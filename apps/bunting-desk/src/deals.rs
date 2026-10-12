//! Tenders and OTC negotiations (ADR 0043), as RIT's Blotter "Tenders" tab
//! and its OTC trades: each in the state the venue last committed, with
//! the answers this participant may give.
//!
//! Expiry is shown in run time. The run clock keeps moving between
//! reports, so "left" is measured from the latest committed report this
//! desk received, and says so.

use crate::desk::Desk;
use crate::format;
use crate::panels::{
    WhenToggled as _, cell, colored, dock_panel, header, muted, panel_body, row, side_color,
};
use bunting_trading_state::Connection;
use bunting_trading_state::bunting_client::bnp_wire::{
    NegotiationInfo, NegotiationStatus, OtcDecision, Side, TenderInfo, TenderStatus,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
};

/// Finished tenders and negotiations listed under the open ones.
const FINISHED_SHOWN: usize = 20;
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
    _observe: Subscription,
}

dock_panel!(DealsPanel, "deals", "Tenders & OTC");

fn number(state: &Entity<InputState>, cx: &App) -> i64 {
    state.read(cx).value().trim().parse().unwrap_or(0)
}

const fn tender_status(status: TenderStatus) -> &'static str {
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

    fn tender_row(&self, tender: &TenderInfo, cx: &mut Context<Self>) -> Div {
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
        row(vec![
            (cell(self.symbol(tender.instrument_id, cx)), 64.),
            (
                colored(format::side(tender.side), side_color(tender.side, cx)),
                48.,
            ),
            (cell(tender.quantity.to_string()), 64.),
            (cell(tender.price.to_string()), 64.),
            (cell(self.expiry(tender.expires_at_ns, cx)), 160.),
            (cell(tender_status(tender.status)), 72.),
            (actions, 150.),
        ])
    }

    fn negotiation_row(&self, info: &NegotiationInfo, me: u128, cx: &mut Context<Self>) -> Div {
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
        row(vec![
            (cell(format!("P{other}")), 56.),
            (cell(self.symbol(info.instrument_id, cx)), 64.),
            (colored(format::side(side), side_color(side, cx)), 48.),
            (cell(info.quantity.to_string()), 64.),
            (cell(info.price.to_string()), 64.),
            (cell(self.expiry(info.expires_at_ns, cx)), 160.),
            (cell(status), 150.),
            (actions.into_any_element(), 220.),
        ])
    }

    fn proposal_form(&self, cx: &mut Context<Self>) -> Div {
        let field =
            |label: &'static str, id: &'static str, state: &Entity<InputState>, cx: &App| {
                v_flex()
                    .gap_1()
                    .w(gpui_kit::px(110.))
                    .child(muted(label, cx))
                    .child(
                        div()
                            .id(id)
                            .test_support()
                            .child(NumberInput::new(state).small()),
                    )
            };
        let symbol = self
            .desk
            .read(cx)
            .instrument()
            .map(|instrument| instrument.symbol)
            .unwrap_or_default();
        let side = self.side;
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
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("otc-buy")
                            .label("Buy")
                            .small()
                            .when_toggled(side == Side::Buy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.side = Side::Buy;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("otc-sell")
                            .label("Sell")
                            .small()
                            .when_toggled(side == Side::Sell)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.side = Side::Sell;
                                cx.notify();
                            })),
                    ),
            )
            .child(field("Quantity", "otc-quantity", &self.quantity, cx))
            .child(field("Price", "otc-price", &self.price, cx))
            .child(field("Expires in (s)", "otc-expires", &self.expires_in, cx))
            .child(
                Button::new("otc-propose")
                    .label(format!("Propose {symbol}"))
                    .small()
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.propose(cx))),
            )
    }
}

fn section(title: &'static str) -> Div {
    div().pt_2().font_weight(FontWeight::SEMIBOLD).child(title)
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
        let mut body = panel_body("deals", &self.focus)
            .overflow_y_scroll()
            .child(muted(as_of, cx))
            .child(section("Tenders"));
        if tenders.is_empty() {
            body = body.child(muted("No tenders offered to you.", cx));
        } else {
            body = body.child(header(
                &[
                    ("symbol", 64.),
                    ("side", 48.),
                    ("qty", 64.),
                    ("price", 64.),
                    ("expires", 160.),
                    ("status", 72.),
                    ("", 150.),
                ],
                cx,
            ));
            let open = tenders
                .iter()
                .filter(|tender| tender.status == TenderStatus::Open)
                .count();
            for tender in tenders.iter().take(open + FINISHED_SHOWN) {
                let line = self.tender_row(tender, cx);
                body = body.child(line);
            }
        }
        body = body.child(section("OTC negotiations"));
        let me = self.me(cx);
        match me {
            Some(me) if !negotiations.is_empty() => {
                body = body.child(header(
                    &[
                        ("with", 56.),
                        ("symbol", 64.),
                        ("you", 48.),
                        ("qty", 64.),
                        ("price", 64.),
                        ("expires", 160.),
                        ("status", 150.),
                        ("", 220.),
                    ],
                    cx,
                ));
                let open = negotiations
                    .iter()
                    .filter(|info| negotiation_open(info.status))
                    .count();
                for info in negotiations.iter().take(open + FINISHED_SHOWN) {
                    let line = self.negotiation_row(info, me, cx);
                    body = body.child(line);
                }
            }
            _ => body = body.child(muted("No negotiations.", cx)),
        }
        body = body
            .child(section("Propose an OTC trade"))
            .child(muted(
                "You take the side chosen, at the quantity and price entered; Counter on a negotiation uses the same fields. The venue applies each request through its sequencer.",
                cx,
            ))
            .child(self.proposal_form(cx));
        if let Some(problem) = &self.problem {
            body = body.child(
                div()
                    .id("otc-problem")
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        body
    }
}
