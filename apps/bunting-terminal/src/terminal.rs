//! Bloomberg-style, single-window GPUI market terminal.
//!
//! The internal floating-panel pointer lifecycle is adapted from the Apache-2.0
//! GPUI `painting.rs` example at Zed commit
//! `1a246efd7e1b83ab568ec5e3e6c1a43a42e1abba`. The outer application shell
//! follows the MIT-licensed Comet GPUI app boundary at commit
//! `e5d8e9fb4c2ffe2350e4114db3bfd89979a2136d`. See
//! `THIRD_PARTY_NOTICES.md` and `docs/gpui-terminal-reference-inventory.md`.

use crate::model::{
    MIN_PANEL_HEIGHT, MIN_PANEL_WIDTH, PanelKind, PanelState, PointerGesture, WorkspacePreset,
    apply_preset, default_panels,
};
use bunting_tui::client::{
    FixClient, IoTask, OutboundCmd, TerminalConfig, UiEvent, book_request, cancel,
    competition_action, competition_requests, new_order,
};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{
    Root,
    button::{Button, ButtonVariants as _},
    chart::CandlestickChart,
    input::{Input, InputState},
};
use std::{env, path::PathBuf, time::Duration};
use tokio::runtime::Runtime;

const BG: u32 = 0x080a0c;
const SURFACE: u32 = 0x101317;
const SURFACE_ALT: u32 = 0x15191e;
const BORDER: u32 = 0x29313a;
const TEXT: u32 = 0xe6edf3;
const MUTED: u32 = 0x84909b;
const GREEN: u32 = 0x2fd08b;
const RED: u32 = 0xff5f6d;
const AMBER: u32 = 0xf0b429;
const BLUE: u32 = 0x58a6ff;

#[derive(Clone)]
struct QuoteCandle {
    label: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

pub struct Terminal {
    panels: Vec<PanelState>,
    pointer_gesture: Option<PointerGesture>,
    next_z: u32,
    next_request_id: u128,
    active_preset: WorkspacePreset,
    market_order: bool,
    status: SharedString,
    client: Option<Box<FixClient>>,
    io: Option<IoTask>,
    _runtime: Option<Runtime>,
    command_input: Entity<InputState>,
    quantity_input: Entity<InputState>,
    price_input: Entity<InputState>,
    cancel_input: Entity<InputState>,
}

impl Terminal {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("FUNCTION / SYMBOL / COMMAND")
                .default_value("BNT")
        });
        let quantity_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Quantity").default_value("10"));
        let price_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Price ticks").default_value("100"));
        let cancel_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Order ID to cancel"));

        let (runtime, io, client, status) = match Self::connect_client() {
            Ok((runtime, io, client, profile)) => (
                Some(runtime),
                Some(io),
                Some(client),
                format!("CONNECTING PROFILE={profile}").into(),
            ),
            Err(error) => (None, None, None, format!("CLIENT ERROR: {error}").into()),
        };

        let mut terminal = Self {
            panels: default_panels(),
            pointer_gesture: None,
            next_z: 32,
            next_request_id: 1,
            active_preset: WorkspacePreset::Trading,
            market_order: false,
            status,
            client,
            io,
            _runtime: runtime,
            command_input,
            quantity_input,
            price_input,
            cancel_input,
        };
        apply_preset(&mut terminal.panels, terminal.active_preset);
        terminal.schedule_poll(cx);
        terminal
    }

    fn connect_client() -> Result<(Runtime, IoTask, Box<FixClient>, String), String> {
        let config_path = env::var_os("BUNTING_TERMINAL_CONFIG").map(PathBuf::from);
        let (config, _) = TerminalConfig::load(config_path)?;
        let profile_name = env::var("BUNTING_TERMINAL_PROFILE")
            .unwrap_or_else(|_| config.selected_profile.clone());
        let mut profile = config.profile(&profile_name)?;
        if let Ok(endpoint) = env::var("BUNTING_TERMINAL_ENDPOINT") {
            if !endpoint.trim().is_empty() {
                profile.endpoint = endpoint;
            }
        }
        let credential_override = env::var("BUNTING_TERMINAL_PASSWORD").ok().or_else(|| {
            (profile_name == "local" && env::var_os(&profile.password_env).is_none())
                .then(|| "bunting-local-dev".to_owned())
        });

        let display_client = FixClient::new(
            profile_name.clone(),
            profile.clone(),
            credential_override.clone(),
        )
        .map_err(|error| error.to_string())?;
        let io_client = FixClient::new(profile_name.clone(), profile, credential_override)
            .map_err(|error| error.to_string())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("bunting-terminal-fix")
            .build()
            .map_err(|error| error.to_string())?;
        let io = {
            let _guard = runtime.enter();
            IoTask::spawn(io_client)
        };
        Ok((runtime, io, Box::new(display_client), profile_name))
    }

    fn schedule_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(33))
                .await;
            let Some(this) = this.upgrade() else {
                break;
            };
            this.update(cx, |terminal, cx| {
                terminal.drain_io();
                cx.notify();
            });
        })
        .detach();
    }

    fn drain_io(&mut self) {
        loop {
            let event = self
                .io
                .as_mut()
                .and_then(|io| io.events.try_recv().ok());
            let Some(UiEvent::Snapshot {
                client,
                recovery_request,
                competition_request,
            }) = event
            else {
                break;
            };
            self.status = client.status.clone().into();
            self.client = Some(client);
            if recovery_request {
                let request = book_request(self.allocate_request_id());
                self.enqueue(OutboundCmd::Send(request));
            }
            if competition_request {
                let request_id = self.allocate_request_id();
                for request in competition_requests(request_id) {
                    self.enqueue(OutboundCmd::Send(request));
                }
            }
        }
    }

    fn allocate_request_id(&mut self) -> u128 {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        id
    }

    fn enqueue(&mut self, command: OutboundCmd) {
        let Some(io) = &self.io else {
            self.status = "FIX I/O IS NOT AVAILABLE".into();
            return;
        };
        match io.outbound.try_send(command) {
            Ok(()) => self.status = "COMMAND QUEUED".into(),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.status = "FIX OUTBOUND QUEUE FULL — COMMAND NOT SENT".into();
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.status = "FIX I/O CLOSED — COMMAND NOT SENT".into();
            }
        }
    }

    fn switch_preset(&mut self, preset: WorkspacePreset, cx: &mut Context<Self>) {
        self.active_preset = preset;
        apply_preset(&mut self.panels, preset);
        self.status = format!("WORKSPACE {} LOADED", preset.label()).into();
        cx.notify();
    }

    fn show_panel(&mut self, kind: PanelKind, cx: &mut Context<Self>) {
        self.next_z = self.next_z.saturating_add(1);
        if let Some(panel) = self.panels.iter_mut().find(|panel| panel.kind == kind) {
            panel.visible = true;
            panel.minimized = false;
            panel.z = self.next_z;
        }
        cx.notify();
    }

    fn bring_to_front(&mut self, panel_id: usize) {
        self.next_z = self.next_z.saturating_add(1);
        if let Some(panel) = self.panels.iter_mut().find(|panel| panel.id == panel_id) {
            panel.z = self.next_z;
        }
    }

    fn begin_move(
        &mut self,
        panel_id: usize,
        pointer_start: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.bring_to_front(panel_id);
        if let Some(panel) = self.panels.iter().find(|panel| panel.id == panel_id) {
            self.pointer_gesture = Some(PointerGesture::Move {
                panel_id,
                pointer_start,
                panel_start: panel.rect.origin,
            });
        }
        cx.notify();
    }

    fn begin_resize(
        &mut self,
        panel_id: usize,
        pointer_start: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.bring_to_front(panel_id);
        if let Some(panel) = self.panels.iter().find(|panel| panel.id == panel_id) {
            self.pointer_gesture = Some(PointerGesture::Resize {
                panel_id,
                pointer_start,
                size_start: panel.rect.size,
            });
        }
        cx.notify();
    }

    fn update_pointer(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(gesture) = self.pointer_gesture else {
            return;
        };
        match gesture {
            PointerGesture::Move {
                panel_id,
                pointer_start,
                panel_start,
            } => {
                let dx = position.x - pointer_start.x;
                let dy = position.y - pointer_start.y;
                if let Some(panel) = self.panels.iter_mut().find(|panel| panel.id == panel_id) {
                    panel.rect.origin.x = panel_start.x + dx;
                    panel.rect.origin.y = panel_start.y + dy;
                }
            }
            PointerGesture::Resize {
                panel_id,
                pointer_start,
                size_start,
            } => {
                let dx = position.x - pointer_start.x;
                let dy = position.y - pointer_start.y;
                if let Some(panel) = self.panels.iter_mut().find(|panel| panel.id == panel_id) {
                    let width = size_start.width + dx;
                    let height = size_start.height + dy;
                    panel.rect.size.width = if width < MIN_PANEL_WIDTH {
                        MIN_PANEL_WIDTH
                    } else {
                        width
                    };
                    panel.rect.size.height = if height < MIN_PANEL_HEIGHT {
                        MIN_PANEL_HEIGHT
                    } else {
                        height
                    };
                }
            }
        }
        cx.notify();
    }

    fn end_pointer(&mut self) {
        self.pointer_gesture = None;
    }

    fn close_panel(&mut self, panel_id: usize, cx: &mut Context<Self>) {
        if let Some(panel) = self.panels.iter_mut().find(|panel| panel.id == panel_id) {
            panel.visible = false;
        }
        cx.notify();
    }

    fn toggle_minimize(&mut self, panel_id: usize, cx: &mut Context<Self>) {
        if let Some(panel) = self.panels.iter_mut().find(|panel| panel.id == panel_id) {
            panel.minimized = !panel.minimized;
        }
        cx.notify();
    }

    fn reconnect(&mut self) {
        self.enqueue(OutboundCmd::Reconnect);
    }

    fn refresh(&mut self) {
        let request = book_request(self.allocate_request_id());
        self.enqueue(OutboundCmd::Send(request));
        let request_id = self.allocate_request_id();
        for request in competition_requests(request_id) {
            self.enqueue(OutboundCmd::Send(request));
        }
    }

    fn input_value(input: &Entity<InputState>, cx: &Context<Self>) -> String {
        input.read(cx).value().to_string()
    }

    fn submit_order(&mut self, side: &'static str, cx: &mut Context<Self>) {
        let quantity = match Self::input_value(&self.quantity_input, cx).parse::<i64>() {
            Ok(quantity) if quantity > 0 => quantity,
            _ => {
                self.status = "ORDER REJECTED LOCALLY: QUANTITY MUST BE POSITIVE".into();
                cx.notify();
                return;
            }
        };
        let price = if self.market_order {
            None
        } else {
            match Self::input_value(&self.price_input, cx).parse::<i64>() {
                Ok(price) if price > 0 => Some(price),
                _ => {
                    self.status = "ORDER REJECTED LOCALLY: LIMIT PRICE MUST BE POSITIVE".into();
                    cx.notify();
                    return;
                }
            }
        };
        let request_id = self.allocate_request_id();
        self.enqueue(OutboundCmd::Send(new_order(
            request_id,
            side,
            quantity,
            price,
        )));
        self.status = format!(
            "{} {} QTY={} PRICE={}",
            if self.market_order { "MARKET" } else { "LIMIT" },
            side.to_ascii_uppercase(),
            quantity,
            price.map_or_else(|| "MKT".to_owned(), |value| value.to_string())
        )
        .into();
        cx.notify();
    }

    fn submit_at_best(&mut self, side: &'static str, cx: &mut Context<Self>) {
        let price = self.client.as_ref().and_then(|client| {
            if side == "buy" {
                client.book.asks.first().map(|level| level.0)
            } else {
                client.book.bids.first().map(|level| level.0)
            }
        });
        let Some(price) = price else {
            self.status = "NO LIVE BEST PRICE".into();
            cx.notify();
            return;
        };
        let quantity = Self::input_value(&self.quantity_input, cx)
            .parse::<i64>()
            .ok()
            .filter(|quantity| *quantity > 0)
            .unwrap_or(1);
        let request_id = self.allocate_request_id();
        self.enqueue(OutboundCmd::Send(new_order(
            request_id,
            side,
            quantity,
            Some(price),
        )));
        self.status = format!("QUICK {} {} @ {}", side.to_ascii_uppercase(), quantity, price).into();
        cx.notify();
    }

    fn cancel_order(&mut self, cx: &mut Context<Self>) {
        let order_id = match Self::input_value(&self.cancel_input, cx).parse::<u128>() {
            Ok(order_id) => order_id,
            Err(_) => {
                self.status = "CANCEL REJECTED LOCALLY: INVALID ORDER ID".into();
                cx.notify();
                return;
            }
        };
        let replacement_id = self.allocate_request_id();
        self.enqueue(OutboundCmd::Send(cancel(order_id, replacement_id)));
        self.status = format!("CANCEL REQUESTED ORDER={order_id}").into();
        cx.notify();
    }

    fn tender_action(&mut self, tender_id: u128, action: &'static str, cx: &mut Context<Self>) {
        let Ok(tender_id) = u64::try_from(tender_id) else {
            self.status = "TENDER IDENTIFIER EXCEEDS FIX ACTION RANGE".into();
            cx.notify();
            return;
        };
        self.enqueue(OutboundCmd::Send(competition_action(
            "U6",
            action,
            Some(tender_id),
            None,
        )));
        self.status = format!("TENDER {tender_id} {action}").into();
        cx.notify();
    }

    fn run_action(&mut self, action: &'static str, cx: &mut Context<Self>) {
        self.enqueue(OutboundCmd::Send(competition_action(
            "UA", action, None, None,
        )));
        self.status = format!("RUN ACTION {action}").into();
        cx.notify();
    }

    fn execute_command(&mut self, cx: &mut Context<Self>) {
        let command = Self::input_value(&self.command_input, cx)
            .trim()
            .to_ascii_uppercase();
        match command.as_str() {
            "TRADING" | "TRADE" => self.switch_preset(WorkspacePreset::Trading, cx),
            "RESEARCH" | "RES" => self.switch_preset(WorkspacePreset::Research, cx),
            "COMPETITION" | "COMP" | "RIT" => {
                self.switch_preset(WorkspacePreset::Competition, cx);
            }
            "REFRESH" | "GO" | "BNT" => self.refresh(),
            "RECONNECT" => self.reconnect(),
            "BOOK" => self.show_panel(PanelKind::OrderBook, cx),
            "ORDERS" => self.show_panel(PanelKind::Orders, cx),
            "NEWS" => self.show_panel(PanelKind::News, cx),
            "SESSION" | "FIX" => self.show_panel(PanelKind::Session, cx),
            _ => {
                self.status = format!("UNKNOWN FUNCTION: {command}").into();
                cx.notify();
            }
        }
    }

    fn quote_candles(&self) -> Vec<QuoteCandle> {
        let Some(client) = &self.client else {
            return Vec::new();
        };
        let samples = client.prices.iter().rev().take(72).collect::<Vec<_>>();
        let mut previous = None;
        samples
            .into_iter()
            .rev()
            .enumerate()
            .map(|(index, sample)| {
                let close = (sample.bid as f64 + sample.ask as f64) / 2.0;
                let open = previous.replace(close).unwrap_or(close);
                QuoteCandle {
                    label: index.to_string(),
                    open,
                    high: sample.ask as f64,
                    low: sample.bid as f64,
                    close,
                }
            })
            .collect()
    }

    fn function_button(
        &self,
        id: &'static str,
        label: &'static str,
        active: bool,
        handler: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .label(label)
            .when(active, |button| button.primary())
            .when(!active, |button| button.secondary())
            .on_click(cx.listener(move |terminal, _, _, cx| handler(terminal, cx)))
    }

    fn render_function_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .h(px(48.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(rgb(SURFACE_ALT))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(TEXT))
                    .child("BUNTING//TERMINAL"),
            )
            .child(
                div()
                    .w(px(330.))
                    .child(Input::new(&self.command_input)),
            )
            .child(
                Button::new("command-go")
                    .primary()
                    .label("GO")
                    .on_click(cx.listener(|terminal, _, _, cx| terminal.execute_command(cx))),
            )
            .child(self.function_button(
                "preset-trading",
                "TRADING",
                self.active_preset == WorkspacePreset::Trading,
                |terminal, cx| terminal.switch_preset(WorkspacePreset::Trading, cx),
                cx,
            ))
            .child(self.function_button(
                "preset-research",
                "RESEARCH",
                self.active_preset == WorkspacePreset::Research,
                |terminal, cx| terminal.switch_preset(WorkspacePreset::Research, cx),
                cx,
            ))
            .child(self.function_button(
                "preset-competition",
                "COMP",
                self.active_preset == WorkspacePreset::Competition,
                |terminal, cx| terminal.switch_preset(WorkspacePreset::Competition, cx),
                cx,
            ))
            .child(div().flex_1())
            .child(
                Button::new("refresh-all")
                    .secondary()
                    .label("REFRESH")
                    .on_click(cx.listener(|terminal, _, _, _| terminal.refresh())),
            )
            .child(
                Button::new("reconnect")
                    .secondary()
                    .label("RECONNECT")
                    .on_click(cx.listener(|terminal, _, _, _| terminal.reconnect())),
            )
            .into_any_element()
    }

    fn render_status_bar(&self) -> AnyElement {
        let (profile, role, sequence, state, stale) = self.client.as_ref().map_or_else(
            || ("-", "-", "-", "DISCONNECTED".to_owned(), true),
            |client| {
                (
                    client.profile_name.as_str(),
                    client.verified_role.as_deref().unwrap_or("unverified"),
                    client.committed_sequence.as_str(),
                    format!("{:?}", client.connection_state()),
                    client.stale,
                )
            },
        );
        div()
            .h(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .bg(rgb(SURFACE_ALT))
            .border_t_1()
            .border_color(rgb(BORDER))
            .text_xs()
            .text_color(rgb(MUTED))
            .child(
                div()
                    .text_color(if stale { rgb(RED) } else { rgb(GREEN) })
                    .child(if stale { "● STALE" } else { "● LIVE" }),
            )
            .child(format!("PROFILE {profile}"))
            .child(format!("ROLE {role}"))
            .child(format!("FIX {state}"))
            .child(format!("COMMIT {sequence}"))
            .child(div().flex_1())
            .child(self.status.clone())
            .into_any_element()
    }

    fn render_panel(&self, panel: PanelState, cx: &mut Context<Self>) -> AnyElement {
        let id = panel.id;
        let body_height = if panel.minimized {
            px(0.)
        } else {
            panel.rect.size.height - px(32.)
        };
        div()
            .id(format!("floating-panel-{id}"))
            .absolute()
            .left(panel.rect.origin.x)
            .top(panel.rect.origin.y)
            .w(panel.rect.size.width)
            .h(if panel.minimized {
                px(32.)
            } else {
                panel.rect.size.height
            })
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |terminal, _, _, _| terminal.bring_to_front(id)),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .bg(rgb(SURFACE_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |terminal, event: &MouseDownEvent, _, cx| {
                            terminal.begin_move(id, event.position, cx);
                        }),
                    )
                    .child(
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(if self.client.as_ref().is_some_and(|client| !client.stale) {
                                rgb(GREEN)
                            } else {
                                rgb(RED)
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(TEXT))
                            .child(panel.kind.title()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(format!("minimize-{id}"))
                            .px_2()
                            .text_color(rgb(MUTED))
                            .hover(|element| element.bg(rgb(BORDER)).text_color(rgb(TEXT)))
                            .child("—")
                            .on_click(cx.listener(move |terminal, _, _, cx| {
                                terminal.toggle_minimize(id, cx);
                            })),
                    )
                    .child(
                        div()
                            .id(format!("close-{id}"))
                            .px_2()
                            .text_color(rgb(MUTED))
                            .hover(|element| element.bg(rgb(RED)).text_color(rgb(TEXT)))
                            .child("×")
                            .on_click(cx.listener(move |terminal, _, _, cx| {
                                terminal.close_panel(id, cx);
                            })),
                    ),
            )
            .when(!panel.minimized, |element| {
                element.child(
                    div()
                        .h(body_height)
                        .overflow_hidden()
                        .child(self.render_panel_body(panel.kind, cx)),
                )
            })
            .when(!panel.minimized, |element| {
                element.child(
                    div()
                        .id(format!("resize-{id}"))
                        .absolute()
                        .right(px(0.))
                        .bottom(px(0.))
                        .w(px(18.))
                        .h(px(18.))
                        .border_l_1()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(SURFACE_ALT))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |terminal, event: &MouseDownEvent, _, cx| {
                                terminal.begin_resize(id, event.position, cx);
                            }),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_panel_body(&self, kind: PanelKind, cx: &mut Context<Self>) -> AnyElement {
        match kind {
            PanelKind::Chart => self.render_chart(),
            PanelKind::OrderBook => self.render_order_book(cx),
            PanelKind::OrderTicket => self.render_order_ticket(cx),
            PanelKind::Orders => self.render_orders(cx),
            PanelKind::Account => self.render_account(),
            PanelKind::News => self.render_news(),
            PanelKind::Tenders => self.render_tenders(cx),
            PanelKind::Risk => self.render_risk(),
            PanelKind::Competition => self.render_competition(cx),
            PanelKind::Session => self.render_session(),
        }
    }

    fn render_chart(&self) -> AnyElement {
        let candles = self.quote_candles();
        let (bid, ask, spread) = self.client.as_ref().map_or((None, None, None), |client| {
            let bid = client.book.bids.first().map(|level| level.0);
            let ask = client.book.asks.first().map(|level| level.0);
            (bid, ask, bid.zip(ask).map(|(bid, ask)| ask - bid))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .items_center()
                    .child(metric("SYMBOL", "BNT", TEXT))
                    .child(metric("BID", value_or_dash(bid), GREEN))
                    .child(metric("ASK", value_or_dash(ask), RED))
                    .child(metric("SPREAD", value_or_dash(spread), AMBER))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("QUOTE CANDLES • FIX L1 SNAPSHOTS"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(150.))
                    .when(candles.is_empty(), |element| {
                        element.flex().items_center().justify_center().child(
                            div()
                                .text_color(rgb(MUTED))
                                .child("Waiting for live FIX market data…"),
                        )
                    })
                    .when(!candles.is_empty(), |element| {
                        element.child(
                            CandlestickChart::new(candles)
                                .x(|candle| candle.label.clone())
                                .open(|candle| candle.open)
                                .high(|candle| candle.high)
                                .low(|candle| candle.low)
                                .close(|candle| candle.close)
                                .body_width_ratio(0.55)
                                .tick_margin(8),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_order_book(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex_1().flex().flex_col().overflow_hidden();
        if let Some(client) = &self.client {
            for (price, quantity) in client.book.asks.iter().take(10).rev() {
                rows = rows.child(book_row("ASK", *price, *quantity, RED));
            }
            rows = rows.child(
                div()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(SURFACE_ALT))
                    .text_xs()
                    .text_color(rgb(AMBER))
                    .child(format!("SEQ {} / COMMIT {}", client.book_sequence, client.committed_sequence)),
            );
            for (price, quantity) in client.book.bids.iter().take(10) {
                rows = rows.child(book_row("BID", *price, *quantity, GREEN));
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_2()
            .gap_2()
            .child(table_header(&["SIDE", "PRICE", "QTY"]))
            .child(rows)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("quick-buy")
                            .success()
                            .label("BUY BEST ASK")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.submit_at_best("buy", cx);
                            })),
                    )
                    .child(
                        Button::new("quick-sell")
                            .danger()
                            .label("SELL BEST BID")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.submit_at_best("sell", cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_order_ticket(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_3()
            .child(section_label("ORDER TYPE"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("limit-order")
                            .label("LIMIT")
                            .when(!self.market_order, |button| button.primary())
                            .when(self.market_order, |button| button.secondary())
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.market_order = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("market-order")
                            .label("MARKET")
                            .when(self.market_order, |button| button.primary())
                            .when(!self.market_order, |button| button.secondary())
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.market_order = true;
                                cx.notify();
                            })),
                    ),
            )
            .child(section_label("QUANTITY (LOTS)"))
            .child(Input::new(&self.quantity_input))
            .child(section_label("PRICE (TICKS)"))
            .child(
                div()
                    .when(self.market_order, |element| {
                        element
                            .h(px(34.))
                            .flex()
                            .items_center()
                            .px_2()
                            .bg(rgb(SURFACE_ALT))
                            .text_color(rgb(MUTED))
                            .child("MARKET — server determines fills")
                    })
                    .when(!self.market_order, |element| {
                        element.child(Input::new(&self.price_input))
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("submit-buy")
                            .success()
                            .label("BUY")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.submit_order("buy", cx);
                            })),
                    )
                    .child(
                        Button::new("submit-sell")
                            .danger()
                            .label("SELL")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.submit_order("sell", cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Orders are sent through the existing bounded FIX session. The GPUI app owns no market state."),
            )
            .into_any_element()
    }

    fn render_orders(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex_1().flex().flex_col().overflow_hidden();
        if let Some(client) = &self.client {
            for execution in client.executions.iter().rev().take(12) {
                list = list.child(
                    div()
                        .h(px(25.))
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(rgb(BORDER))
                        .text_xs()
                        .child(cell(execution.order_id.clone(), 0.28, TEXT))
                        .child(cell(execution.kind.clone(), 0.16, BLUE))
                        .child(cell(execution.order_status.clone(), 0.18, AMBER))
                        .child(cell(execution.reason.clone(), 0.38, MUTED)),
                );
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_2()
            .gap_2()
            .child(table_header(&["ORDER ID", "EVENT", "STATUS", "REASON"]))
            .child(list)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.cancel_input)))
                    .child(
                        Button::new("cancel-order")
                            .danger()
                            .label("CANCEL")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.cancel_order(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_account(&self) -> AnyElement {
        let mut holdings = div().flex_1().flex().flex_col().overflow_hidden();
        let mut cash = div().flex().flex_col();
        if let Some(account) = self
            .client
            .as_ref()
            .and_then(|client| client.authoritative_account.as_ref())
        {
            for holding in account.holdings.iter().take(10) {
                holdings = holdings.child(
                    div()
                        .h(px(25.))
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(rgb(BORDER))
                        .text_xs()
                        .child(cell(holding.instrument_id.to_string(), 0.18, TEXT))
                        .child(cell(holding.position.to_string(), 0.18, GREEN))
                        .child(cell(holding.reserved.to_string(), 0.18, AMBER))
                        .child(cell(holding.realized_pnl.to_string(), 0.23, BLUE))
                        .child(cell(holding.unrealized_pnl.to_string(), 0.23, BLUE)),
                );
            }
            for balance in account.cash.iter().take(4) {
                cash = cash.child(
                    div()
                        .flex()
                        .gap_2()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(format!("CCY {}", balance.currency_id))
                        .child(format!("SETTLED {}", balance.settled))
                        .child(format!("RESERVED {}", balance.reserved))
                        .child(format!("MARGIN {}", balance.margin)),
                );
            }
        } else if let Some(client) = &self.client {
            holdings = holdings.child(
                div()
                    .p_3()
                    .text_color(rgb(MUTED))
                    .child(format!(
                        "Local fill projection: position={} cash={} marked={}",
                        client.portfolio.position,
                        client.portfolio.cash,
                        client
                            .book
                            .bids
                            .first()
                            .map_or(client.portfolio.cash, |level| client.portfolio.marked_value(level.0))
                    )),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_2()
            .gap_2()
            .child(table_header(&[
                "INSTR", "POSITION", "RESERVED", "REALIZED", "UNREALIZED",
            ]))
            .child(holdings)
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .pt_2()
                    .child(cash),
            )
            .into_any_element()
    }

    fn render_news(&self) -> AnyElement {
        let mut list = div().size_full().flex().flex_col().overflow_hidden();
        if let Some(client) = &self.client {
            for item in client.news.iter().rev().take(10) {
                list = list.child(
                    div()
                        .p_2()
                        .border_b_1()
                        .border_color(rgb(BORDER))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(TEXT))
                                .child(item.headline.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(item.body.clone()),
                        ),
                );
            }
        }
        div().size_full().p_2().child(list).into_any_element()
    }

    fn render_tenders(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().size_full().flex().flex_col().gap_2().p_2().overflow_hidden();
        if let Some(client) = &self.client {
            for tender in client.tenders.iter().take(8) {
                let tender_id = tender.tender_id.get();
                list = list.child(
                    div()
                        .p_2()
                        .border_1()
                        .border_color(rgb(BORDER))
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(metric("ID", tender_id.to_string(), TEXT))
                        .child(metric("SIDE", format!("{:?}", tender.side), BLUE))
                        .child(metric("QTY", tender.quantity.to_string(), TEXT))
                        .child(metric("PX", tender.price.to_string(), AMBER))
                        .child(metric("STATUS", tender.status.clone(), MUTED))
                        .child(div().flex_1())
                        .child(
                            Button::new(format!("accept-tender-{tender_id}"))
                                .success()
                                .label("ACCEPT")
                                .on_click(cx.listener(move |terminal, _, _, cx| {
                                    terminal.tender_action(tender_id, "accept", cx);
                                })),
                        )
                        .child(
                            Button::new(format!("decline-tender-{tender_id}"))
                                .danger()
                                .label("DECLINE")
                                .on_click(cx.listener(move |terminal, _, _, cx| {
                                    terminal.tender_action(tender_id, "decline", cx);
                                })),
                        ),
                );
            }
        }
        list.into_any_element()
    }

    fn render_risk(&self) -> AnyElement {
        let risk = self
            .client
            .as_ref()
            .and_then(|client| client.risk.as_ref())
            .and_then(|risk| serde_json::to_string_pretty(risk).ok())
            .unwrap_or_else(|| "Risk projection is not available yet.".to_owned());
        let score = self
            .client
            .as_ref()
            .and_then(|client| client.score)
            .map_or_else(
                || "SCORE - / RANK -".to_owned(),
                |score| format!("SCORE {} / RANK {}", score.score, score.rank),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(AMBER))
                    .child(score),
            )
            .child(
                div()
                    .flex_1()
                    .p_2()
                    .bg(rgb(BG))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(risk),
            )
            .into_any_element()
    }

    fn render_competition(&self, cx: &mut Context<Self>) -> AnyElement {
        let discovery = self
            .client
            .as_ref()
            .and_then(|client| client.discovery.as_ref());
        let (run, scenario, lifecycle, logical_time, listings) = discovery.map_or_else(
            || ("-".to_owned(), "-".to_owned(), "-".to_owned(), "-".to_owned(), "-".to_owned()),
            |view| {
                (
                    view.run_id.to_string(),
                    format!("{} v{}", view.scenario_id, view.scenario_version),
                    format!("{:?}", view.lifecycle),
                    view.logical_time.to_string(),
                    view.listings
                        .iter()
                        .map(|listing| listing.symbol.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            },
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(metric("RUN", run, TEXT))
                    .child(metric("SCENARIO", scenario, BLUE))
                    .child(metric("STATE", lifecycle, AMBER))
                    .child(metric("LOGICAL NS", logical_time, MUTED)),
            )
            .child(
                div()
                    .p_2()
                    .bg(rgb(BG))
                    .text_sm()
                    .text_color(rgb(TEXT))
                    .child(format!("LISTINGS {listings}")),
            )
            .child(section_label("INSTRUCTOR / ADMINISTRATOR ACTIONS"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("run-start")
                            .success()
                            .label("START")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.run_action("start", cx);
                            })),
                    )
                    .child(
                        Button::new("run-pause")
                            .secondary()
                            .label("PAUSE")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.run_action("pause", cx);
                            })),
                    )
                    .child(
                        Button::new("run-resume")
                            .primary()
                            .label("RESUME")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.run_action("resume", cx);
                            })),
                    )
                    .child(
                        Button::new("run-score")
                            .secondary()
                            .label("SCORE")
                            .on_click(cx.listener(|terminal, _, _, cx| {
                                terminal.run_action("score", cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("The server verifies role and lifecycle. Displaying a control never grants authority."),
            )
            .into_any_element()
    }

    fn render_session(&self) -> AnyElement {
        let mut logs = div().flex_1().flex().flex_col().overflow_hidden();
        if let Some(client) = &self.client {
            for line in client.logs.iter().rev().take(18) {
                logs = logs.child(
                    div()
                        .text_xs()
                        .text_color(if line.starts_with("OUT") {
                            rgb(BLUE)
                        } else {
                            rgb(MUTED)
                        })
                        .child(line.clone()),
                );
            }
            div()
                .size_full()
                .flex()
                .flex_col()
                .p_2()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .gap_4()
                        .child(metric("PROFILE", client.profile_name.clone(), TEXT))
                        .child(metric("ENDPOINT", client.profile().endpoint.clone(), BLUE))
                        .child(metric("BOOK SEQ", client.book_sequence.clone(), AMBER))
                        .child(metric("COMMIT", client.committed_sequence.clone(), GREEN)),
                )
                .child(
                    div()
                        .flex_1()
                        .p_2()
                        .bg(rgb(BG))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .child(logs),
                )
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(RED))
                .child("FIX client initialization failed")
                .into_any_element()
        }
    }
}

impl Render for Terminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panels = self
            .panels
            .iter()
            .filter(|panel| panel.visible)
            .cloned()
            .collect::<Vec<_>>();
        panels.sort_by_key(|panel| panel.z);

        let mut workspace = div()
            .id("floating-workspace")
            .relative()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(BG))
            .on_mouse_move(cx.listener(|terminal, event: &MouseMoveEvent, _, cx| {
                terminal.update_pointer(event.position, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|terminal, _, _, _| terminal.end_pointer()),
            );
        for panel in panels {
            workspace = workspace.child(self.render_panel(panel, cx));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .font_family("monospace")
            .text_color(rgb(TEXT))
            .child(self.render_function_bar(cx))
            .child(workspace)
            .child(self.render_status_bar())
    }
}

pub fn root(view: Entity<Terminal>, window: &mut Window, cx: &mut gpui::App) -> Entity<Root> {
    cx.new(|cx| Root::new(view, window, cx).bg(rgb(BG)))
}

fn section_label(label: &'static str) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(MUTED))
        .child(label)
        .into_any_element()
}

fn metric(label: &'static str, value: impl Into<SharedString>, color: u32) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(color))
                .child(value.into()),
        )
        .into_any_element()
}

fn table_header(columns: &[&'static str]) -> AnyElement {
    let mut header = div()
        .h(px(24.))
        .flex()
        .items_center()
        .bg(rgb(SURFACE_ALT))
        .border_b_1()
        .border_color(rgb(BORDER));
    let width = 1.0 / columns.len().max(1) as f32;
    for column in columns {
        header = header.child(cell(*column, width, MUTED));
    }
    header.into_any_element()
}

fn book_row(side: &'static str, price: i64, quantity: i64, color: u32) -> AnyElement {
    div()
        .h(px(22.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(rgb(BORDER))
        .text_xs()
        .child(cell(side, 0.25, color))
        .child(cell(price.to_string(), 0.4, color))
        .child(cell(quantity.to_string(), 0.35, TEXT))
        .into_any_element()
}

fn cell(value: impl Into<SharedString>, width: f32, color: u32) -> AnyElement {
    div()
        .w(gpui::relative(width))
        .px_1()
        .text_color(rgb(color))
        .child(value.into())
        .into_any_element()
}

fn value_or_dash(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.to_string())
}
