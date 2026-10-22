//! RIT's Transaction Log: every credit and debit to the cash balance the
//! desk can derive from committed reports: each fill's notional (a buy is
//! a debit, a sell a credit) and each fee or rebate as its own row.
//!
//! The running balance is **derived**: the venue's last account answer,
//! worked back and forward through these rows by committed sequence. It
//! assumes fills and fees are the only cash movements; other ledger
//! postings, and tender and OTC settlements (which carry no cash amount
//! or time on BNP yet), are not listed.

use crate::desk::Desk;
use crate::format;
use crate::panels::{cell, colored, dock_panel, panel_body, signed_color};
use crate::ui::{self, Col};
use bunting_trading_state::Model;
use bunting_trading_state::bunting_client::bnp_wire::Side;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{IconName, v_flex};
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Subscription, TestSupportExt as _,
    Window, uniform_list,
};
use std::rc::Rc;

const COLUMNS: [Col; 7] = [
    Col::right("run time", 80.).priority(1),
    Col::left("ticker", 64.).priority(2),
    Col::left("type", 80.).priority(2),
    Col::right("price", 72.).priority(0),
    Col::right("quantity", 72.).priority(0),
    Col::right("cash flow", 104.).priority(2),
    Col::right("balance", 112.),
];

/// What a cash movement was.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Buy,
    Sell,
    Fee,
    Rebate,
}

impl Kind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Buy => "Buy fill",
            Self::Sell => "Sell fill",
            Self::Fee => "Fee",
            Self::Rebate => "Rebate",
        }
    }
}

/// One credit or debit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub sequence: u64,
    pub time_ns: u64,
    pub symbol: String,
    pub kind: Kind,
    pub price: Option<i64>,
    pub quantity: Option<i64>,
    /// Minor currency units; negative is a debit.
    pub flow: i128,
    /// The derived balance after this movement.
    pub balance: Option<i128>,
    /// Decimal places of the price and of the money.
    pub decimals: u8,
    pub money_decimals: u8,
}

impl Entry {
    /// The row's text, one entry per column.
    #[must_use]
    pub fn texts(&self) -> Vec<String> {
        let dash = || "—".to_owned();
        vec![
            format::logical(self.time_ns),
            self.symbol.clone(),
            self.kind.label().to_owned(),
            self.price
                .map_or_else(dash, |price| format::ticks(price, self.decimals)),
            self.quantity
                .map_or_else(dash, |quantity| format::grouped(quantity.into())),
            format::money_signed(self.flow, self.money_decimals),
            self.balance
                .map_or_else(dash, |balance| format::money(balance, self.money_decimals)),
        ]
    }
}

/// Every movement, oldest first.
#[must_use]
pub fn entries(model: &Model) -> Vec<Entry> {
    let mut list: Vec<Entry> = Vec::new();
    for fill in model.fills() {
        let symbol = model.symbol(fill.listing).unwrap_or("?").to_owned();
        let (multiplier, _) = model.multiplier(fill.listing.instrument_id);
        let decimals = model.quoted_decimals(fill.listing.instrument_id);
        let money_decimals = model.money_decimals(fill.listing.instrument_id);
        let notional = i128::from(fill.price)
            .saturating_mul(i128::from(fill.quantity))
            .saturating_mul(i128::from(multiplier));
        list.push(Entry {
            sequence: fill.sequence,
            time_ns: fill.logical_time_ns,
            symbol: symbol.clone(),
            kind: match fill.side {
                Side::Buy => Kind::Buy,
                Side::Sell => Kind::Sell,
            },
            price: Some(fill.price),
            quantity: Some(fill.quantity),
            flow: match fill.side {
                Side::Buy => -notional,
                Side::Sell => notional,
            },
            balance: None,
            decimals,
            money_decimals,
        });
        if fill.fee != 0 {
            list.push(Entry {
                sequence: fill.sequence,
                time_ns: fill.logical_time_ns,
                symbol,
                kind: if fill.fee > 0 {
                    Kind::Fee
                } else {
                    Kind::Rebate
                },
                price: None,
                quantity: None,
                flow: fill.fee.saturating_neg(),
                balance: None,
                decimals,
                money_decimals,
            });
        }
    }
    // The running balance: the account answer is the balance after every
    // movement committed up to its sequence, so work back from it for
    // earlier rows and forward for later ones. Only with one currency.
    if let Some(account) = model.account()
        && let [cash] = account.cash.as_slice()
    {
        let snapshot = account.run_sequence;
        let anchor = list.iter().rposition(|entry| entry.sequence <= snapshot);
        let mut prefix: Vec<i128> = Vec::with_capacity(list.len());
        let mut sum: i128 = 0;
        for entry in &list {
            sum = sum.saturating_add(entry.flow);
            prefix.push(sum);
        }
        let at_anchor = anchor.map_or(0, |index| prefix[index]);
        for (entry, total) in list.iter_mut().zip(prefix) {
            entry.balance = Some(cash.balance.saturating_add(total).saturating_sub(at_anchor));
        }
    }
    list
}

/// The table as tab-separated text with a header row, oldest first.
#[must_use]
pub fn copy_text(model: &Model) -> String {
    let header: Vec<&str> = COLUMNS.iter().map(|col| col.label).collect();
    let body: Vec<Vec<String>> = entries(model).iter().map(Entry::texts).collect();
    ui::tsv(&header, &body)
}

pub struct TransactionsPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    _observe: Subscription,
}

dock_panel!(TransactionsPanel, "transactions", "Transaction Log");

impl TransactionsPanel {
    pub fn new(desk: Entity<Desk>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
        }
    }
}

impl Render for TransactionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = panel_body("transactions", &self.focus);
        let list = entries(self.desk.read(cx).model());
        if list.is_empty() {
            return body.child(ui::empty_state(
                "transactions-empty",
                IconName::FileText,
                "No cash movements yet",
                None,
                cx,
            ));
        }
        let model = self.desk.read(cx).model();
        let assumed = model
            .listings()
            .iter()
            .any(|info| !model.multiplier(info.listing.instrument_id).1);
        let full = if assumed {
            "Derived from your fills and fees; assumes contract size 1. Other postings, tenders and OTC settlements are not reported yet."
        } else {
            "Derived from your fills and fees. Other postings, tenders and OTC settlements are not reported yet."
        };
        let count = list.len();
        let rows = Rc::new(list);
        let table = ui::responsive_table(&COLUMNS, move |fitted, _, cx| {
            let fitted: Vec<ui::Fit> = fitted.to_vec();
            v_flex()
                .size_full()
                .child(ui::table_head(&fitted, cx))
                .child(
                    uniform_list("transaction-rows", count, move |range, _, cx| {
                        range
                            .filter_map(|index| rows.get(rows.len().checked_sub(index + 1)?))
                            .map(|entry| {
                                let mut texts = entry.texts().into_iter();
                                let mut next = || texts.next().unwrap_or_default();
                                let (time, symbol, kind) = (next(), next(), next());
                                let (price, quantity, flow, balance) =
                                    (next(), next(), next(), next());
                                ui::table_row(
                                    &fitted,
                                    vec![
                                        cell(time),
                                        cell(symbol),
                                        cell(kind),
                                        cell(price),
                                        cell(quantity),
                                        colored(flow, signed_color(entry.flow, cx)),
                                        cell(balance),
                                    ],
                                    cx,
                                )
                            })
                            .collect()
                    })
                    .flex_1(),
                )
        });
        body.child(ui::note(
            "transactions-note",
            "Derived from fills and fees",
            full,
            cx,
        ))
        .child(ui::copy_menu(
            v_flex()
                .id("transactions-table")
                .test_support()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(table),
            self.desk.clone(),
            Rc::new(|desk: &Desk| copy_text(desk.model())),
        ))
    }
}
