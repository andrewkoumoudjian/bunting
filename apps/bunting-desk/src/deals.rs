//! Tenders and OTC negotiations (ADR 0043), as RIT's Blotter "Tenders" tab
//! and its OTC trades: each in the state the venue last committed, with
//! the answers this participant may give.
//!
//! Expiry is shown in run time. The run clock keeps moving between
//! reports, so "left" is measured from the latest committed report this
//! desk received, and says so.

use crate::desk::Desk;
use crate::format;
use crate::panels::{dock_panel, side_color};
use crate::ui::{self, Col, cell, colored, muted};
use bunting_trading_state::Connection;
use bunting_trading_state::bunting_client::bnp_wire::{
    NegotiationInfo, NegotiationStatus, OtcDecision, Side, TenderInfo, TenderStatus,
};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    px,
};

const TENDER_COLUMNS: [Col; 7] = [
    Col::left("symbol", 64.).priority(2),
    Col::left("side", 48.).priority(2),
    Col::right("qty", 64.).priority(2),
    Col::right("price", 64.).priority(2),
    Col::left("expires", 168.).priority(0),
    Col::left("status", 80.),
    Col::left("", 160.).priority(2),
];

const OTC_COLUMNS: [Col; 8] = [
    Col::left("with", 56.),
    Col::left("symbol", 64.).priority(2),
    Col::left("you", 48.).priority(2),
    Col::right("qty", 64.).priority(2),
    Col::right("price", 64.).priority(2),
    Col::left("expires", 168.).priority(0),
    Col::left("status", 150.),
    Col::left("", 232.).priority(2),
];

/// Finished tenders and negotiations listed under the open ones.
pub(crate) const FINISHED_SHOWN: usize = 20;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

pub struct DealsPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    counterparty: Entity<InputState>,
    quantity: Entity<InputState>,
    price: Entity<InputState>,
    expires_in: Entity<InputState>,
    side: Side,
    problem: Option<String>,
    /// The pane width last measured by the body's width probe, so the
    /// scrolling tables fit their columns to it.
    width: std::rc::Rc<std::cell::Cell<f32>>,
    _observe: Subscription,
}

dock_panel!(DealsPanel, "deals", "Tenders & OTC");

fn number(state: &Entity<InputState>, cx: &App) -> i64 {
    state.read(cx).value().trim().parse().unwrap_or(0)
}

pub(crate) const fn tender_status(status: TenderStatus) -> &'static str {
    match status {
        TenderStatus::Open => "open",
        TenderStatus::Accepted => "accepted",
        TenderStatus::Declined => "declined",
        TenderStatus::Expired => "expired",
    }
}

const fn negotiation_status(status: NegotiationStatus) -> &'static str {
    match status {
        NegotiationStatus::Proposed => "proposed",
        NegotiationStatus::Countered => "countered",
        NegotiationStatus::Accepted => "accepted",
        NegotiationStatus::Rejected => "rejected",
        NegotiationStatus::Broken => "broken",
        NegotiationStatus::Expired => "expired",
    }
}

const fn negotiation_open(status: NegotiationStatus) -> bool {
    matches!(
        status,
        NegotiationStatus::Proposed | NegotiationStatus::Countered
    )
}

/// The side `me` takes in a negotiation: the proposer's side, or the
/// other one for the counterparty.
fn my_side(info: &NegotiationInfo, me: u128) -> Side {
    if info.proposer_id == me {
        info.side
    } else {
        match info.side {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

impl DealsPanel {
    pub fn new(desk: Entity<Desk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            counterparty: cx.new(|cx| InputState::new(window, cx)),
            quantity: cx.new(|cx| InputState::new(window, cx).default_value("100")),
            price: cx.new(|cx| InputState::new(window, cx)),
            expires_in: cx.new(|cx| InputState::new(window, cx).default_value("60")),
            side: Side::Buy,
            problem: None,
            width: std::rc::Rc::new(std::cell::Cell::new(0.)),
        }
    }

    fn me(&self, cx: &App) -> Option<u128> {
        match self.desk.read(cx).model().connection() {
            Connection::Connected { welcome, .. } => Some(welcome.participant_id),
            _ => None,
        }
    }

    fn symbol(&self, instrument_id: u128, cx: &App) -> String {
        self.desk
            .read(cx)
            .instruments()
            .into_iter()
            .find(|instrument| instrument.instrument_id == instrument_id)
            .map_or_else(|| instrument_id.to_string(), |instrument| instrument.symbol)
    }

    /// "95.000 (12.3 s left)" from the latest report's run time.
    fn expiry(&self, expires_at_ns: u64, cx: &App) -> String {
        let now = self.desk.read(cx).model().run_time_ns();
        let left = expires_at_ns.saturating_sub(now) / 1_000;
        format!(
            "{} ({} left)",
            format::logical(expires_at_ns),
            format::duration(left)
        )
    }

    fn propose(&mut self, cx: &mut Context<Self>) {
        let Some(instrument) = self.desk.read(cx).instrument() else {
            return;
        };
        let counterparty = number(&self.counterparty, cx);
        let quantity = number(&self.quantity, cx);
        let price = number(&self.price, cx);
        let seconds = number(&self.expires_in, cx);
        self.problem = if counterparty <= 0 {
            Some("Enter the counterparty's participant number.".to_owned())
        } else if Some(counterparty.unsigned_abs().into()) == self.me(cx) {
            Some("You cannot trade with yourself.".to_owned())
        } else if quantity <= 0 || price <= 0 || seconds <= 0 {
            Some("Quantity, price and expiry must be positive.".to_owned())
        } else {
            None
        };
        if self.problem.is_some() {
            cx.notify();
            return;
        }
        let expires_at_ns = self
            .desk
            .read(cx)
            .model()
            .run_time_ns()
            .saturating_add(seconds.unsigned_abs().saturating_mul(NANOS_PER_SECOND));
        let side = self.side;
        self.desk.update(cx, |desk, cx| {
            desk.propose_otc(
                counterparty.unsigned_abs().into(),
                instrument.instrument_id,
                side,
                quantity,
                price,
                expires_at_ns,
                cx,
            );
        });
        cx.notify();
    }

    fn tender_row(&self, tender: &TenderInfo, fitted: &[ui::Fit], cx: &mut Context<Self>) -> Div {
        let id = tender.tender_id;
        let actions: AnyElement = if tender.status == TenderStatus::Open {
            h_flex()
                .gap_1()
                .child(
                    Button::new(SharedString::from(format!("tender-accept-{id}")))
                        .label("Accept")
                        .xsmall()
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.desk
                                .update(cx, |desk, cx| desk.decide_tender(id, true, cx));
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("tender-decline-{id}")))
                        .label("Decline")
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.desk
                                .update(cx, |desk, cx| desk.decide_tender(id, false, cx));
                        })),
                )
                .into_any_element()
        } else {
            cell("")
        };
        ui::table_row(
            fitted,
            vec![
                cell(self.symbol(tender.instrument_id, cx)),
                colored(format::side(tender.side), side_color(tender.side, cx)),
                cell(tender.quantity.to_string()),
                cell(tender.price.to_string()),
                cell(self.expiry(tender.expires_at_ns, cx)),
                cell(tender_status(tender.status)),
                actions,
            ],
            cx,
        )
    }

    fn negotiation_row(
        &self,
        info: &NegotiationInfo,
        me: u128,
        fitted: &[ui::Fit],
        cx: &mut Context<Self>,
    ) -> Div {
        let id = info.negotiation_id;
        let other = if info.proposer_id == me {
            info.counterparty_id
        } else {
            info.proposer_id
        };
        let side = my_side(info, me);
        let mut actions = h_flex().gap_1();
        if negotiation_open(info.status) {
            if info.awaiting == me {
                let decide =
                    |label: &'static str, decision: OtcDecision, cx: &mut Context<Self>| {
                        Button::new(SharedString::from(format!(
                            "otc-{}-{id}",
                            label.to_lowercase()
                        )))
                        .label(label)
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.desk
                                .update(cx, |desk, cx| desk.decide_otc(id, decision, cx));
                        }))
                    };
                actions = actions
                    .child(decide("Accept", OtcDecision::Accept, cx).primary())
                    .child(decide("Reject", OtcDecision::Reject, cx))
                    .child(
                        Button::new(SharedString::from(format!("otc-counter-{id}")))
                            .label("Counter")
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let quantity = number(&this.quantity, cx);
                                let price = number(&this.price, cx);
                                this.desk.update(cx, |desk, cx| {
                                    desk.counter_otc(id, quantity, price, cx);
                                });
                            })),
                    );
            }
            actions = actions.child(
                Button::new(SharedString::from(format!("otc-break-{id}")))
                    .label("Break")
                    .xsmall()
                    .danger()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.desk
                            .update(cx, |desk, cx| desk.decide_otc(id, OtcDecision::Break, cx));
                    })),
            );
        }
        let status = if negotiation_open(info.status) {
            if info.awaiting == me {
                format!("{} · your answer", negotiation_status(info.status))
            } else {
                format!("{} · their answer", negotiation_status(info.status))
            }
        } else {
            negotiation_status(info.status).to_owned()
        };
        ui::table_row(
            fitted,
            vec![
                cell(format!("P{other}")),
                cell(self.symbol(info.instrument_id, cx)),
                colored(format::side(side), side_color(side, cx)),
                cell(info.quantity.to_string()),
                cell(info.price.to_string()),
                cell(self.expiry(info.expires_at_ns, cx)),
                cell(status),
                actions.into_any_element(),
            ],
            cx,
        )
    }

    fn proposal_form(&self, cx: &mut Context<Self>) -> Div {
        let field =
            |label: &'static str, id: &'static str, state: &Entity<InputState>, cx: &App| {
                ui::field(
                    label,
                    div()
                        .id(id)
                        .test_support()
                        .child(NumberInput::new(state).small())
                        .into_any_element(),
                    cx,
                )
                .w(px(110.))
            };
        let symbol = self
            .desk
            .read(cx)
            .instrument()
            .map(|instrument| instrument.symbol)
            .unwrap_or_default();
        h_flex()
            .gap_2()
            .items_end()
            .flex_wrap()
            .child(field(
                "Counterparty",
                "otc-counterparty",
                &self.counterparty,
                cx,
            ))
            .child(ui::field(
                "Side",
                ButtonGroup::new("otc-side")
                    .small()
                    .child(ui::segment("otc-buy", "Buy", self.side == Side::Buy))
                    .child(ui::segment("otc-sell", "Sell", self.side == Side::Sell))
                    .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                        this.side = if clicks.contains(&0) {
                            Side::Buy
                        } else {
                            Side::Sell
                        };
                        cx.notify();
                    }))
                    .into_any_element(),
                cx,
            ))
            .child(field("Quantity", "otc-quantity", &self.quantity, cx))
            .child(field("Price", "otc-price", &self.price, cx))
            .child(field("Expires in (s)", "otc-expires", &self.expires_in, cx))
            .child(
                Button::new("otc-propose")
                    .label(format!("Propose {symbol}"))
                    .small()
                    .primary()
                    .tooltip(
                        "You take the side chosen, at the quantity and price entered; Counter on a negotiation uses the same fields",
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.propose(cx))),
            )
    }
}

impl Render for DealsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.desk.read(cx).model();
        let mut tenders: Vec<TenderInfo> = model.tenders().iter().map(|held| held.info).collect();
        let mut negotiations: Vec<NegotiationInfo> =
            model.negotiations().iter().map(|held| held.info).collect();
        let as_of = format!(
            "As of the latest report, run time {}.",
            format::logical(model.run_time_ns())
        );
        // Open ones first, newest first within each group.
        tenders.reverse();
        tenders.sort_by_key(|tender| tender.status != TenderStatus::Open);
        negotiations.reverse();
        negotiations.sort_by_key(|info| !negotiation_open(info.status));
        // The body scrolls, so a probe measures its width (one frame
        // behind on a resize) and both tables fit their columns to it.
        // The probe is a body child, so it measures the content width
        // inside the pane padding; 0 until the first frame.
        let available = self.width.get();
        let tender_fit = ui::fit(&TENDER_COLUMNS, available);
        let otc_fit = ui::fit(&OTC_COLUMNS, available);
        let mut body = ui::pane_body("deals", &self.focus)
            .overflow_y_scroll()
            .child(ui::width_probe(cx.entity().downgrade(), self.width.clone()))
            .child(muted(as_of, cx))
            .child(ui::section_label("Tenders", cx));
        if tenders.is_empty() {
            body = body.child(muted("No tenders offered to you", cx));
        } else {
            body = body.child(ui::table_head(&tender_fit, cx));
            let open = tenders
                .iter()
                .filter(|tender| tender.status == TenderStatus::Open)
                .count();
            for tender in tenders.iter().take(open + FINISHED_SHOWN) {
                let line = self.tender_row(tender, &tender_fit, cx);
                body = body.child(line);
            }
        }
        body = body.child(ui::section_label("OTC negotiations", cx));
        let me = self.me(cx);
        match me {
            Some(me) if !negotiations.is_empty() => {
                body = body.child(ui::table_head(&otc_fit, cx));
                let open = negotiations
                    .iter()
                    .filter(|info| negotiation_open(info.status))
                    .count();
                for info in negotiations.iter().take(open + FINISHED_SHOWN) {
                    let line = self.negotiation_row(info, me, &otc_fit, cx);
                    body = body.child(line);
                }
            }
            _ => body = body.child(muted("No negotiations", cx)),
        }
        body = body
            .child(ui::section_label("Propose an OTC trade", cx))
            .child(self.proposal_form(cx));
        if let Some(problem) = &self.problem {
            body = body.child(
                div()
                    .id("otc-problem")
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        body
    }
}
