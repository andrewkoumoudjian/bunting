//! RIT's Trade Blotter: four tabs over your orders and offers.
//!
//! - **Live / Partial**: orders that can still trade, with the quantity
//!   filled out of the order's size and a Cancel button per row. A
//!   cancel on a part-filled order cancels what remains.
//! - **Filled / Cancelled**: the history of done orders.
//! - **Tenders**: the offers made to you, answered here or in Tenders &
//!   OTC.
//! - **Endowments**: securities the case grants; the venue does not
//!   send them yet.

use crate::deals;
use crate::desk::Desk;
use crate::format;
use crate::panels::{cell, colored, dock_panel, panel_body, side_color};
use crate::portfolio;
use crate::ui::{self, Col};
use bunting_trading_state::bunting_client::bnp_wire::TenderStatus;
use bunting_trading_state::{Model, OrderRow, OrderStatus};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{IconName, Sizable as _, v_flex};
use gpui_kit::div;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window,
    uniform_list,
};
use std::rc::Rc;

const LIVE_COLUMNS: [Col; 10] = [
    Col::left("venue", 72.),
    Col::left("symbol", 64.).priority(2),
    Col::left("side", 48.).priority(2),
    Col::right("filled", 88.).priority(2),
    Col::right("price", 72.).priority(2),
    Col::right("left", 56.),
    Col::left("status", 160.).priority(2).weight(3.),
    Col::left("queue", 110.).priority(0),
    Col::right("answer", 72.).priority(0),
    Col::center("", 72.).priority(2),
];

const DONE_COLUMNS: [Col; 8] = [
    Col::left("venue", 72.),
    Col::left("symbol", 64.).priority(2),
    Col::left("side", 48.).priority(2),
    Col::right("filled", 88.).priority(2),
    Col::right("price", 72.).priority(2),
    Col::right("left", 56.).priority(0),
    Col::left("status", 200.).priority(2).weight(3.),
    Col::right("answer", 72.).priority(0),
];

const TENDER_COLUMNS: [Col; 8] = [
    Col::left("symbol", 64.).priority(2),
    Col::left("side", 48.).priority(2),
    Col::right("qty", 64.).priority(2),
    Col::right("price", 64.).priority(2),
    Col::right("bid", 88.).priority(0),
    Col::right("ask", 88.).priority(0),
    Col::left("status", 80.),
    Col::left("", 160.).priority(2),
];

/// The blotter's tabs, in RIT's order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Live,
    Done,
    Tenders,
    Endowments,
}

impl Tab {
    pub const ALL: [Self; 4] = [Self::Live, Self::Done, Self::Tenders, Self::Endowments];

    const fn id(self) -> &'static str {
        match self {
            Self::Live => "blotter-tab-live",
            Self::Done => "blotter-tab-done",
            Self::Tenders => "blotter-tab-tenders",
            Self::Endowments => "blotter-tab-endowments",
        }
    }

    /// The tab's label, as RIT writes it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Live => "Live / Partial",
            Self::Done => "Filled / Cancelled",
            Self::Tenders => "Tenders",
            Self::Endowments => "Endowments",
        }
    }
}

/// Whether `order` is on the Live / Partial tab (it can still trade).
#[must_use]
pub const fn is_live(order: &OrderRow) -> bool {
    order.status.is_live()
}

/// Indices into `model.orders()` for a tab, newest first.
fn indices(model: &Model, tab: Tab) -> Vec<usize> {
    let orders = model.orders();
    (0..orders.len())
        .rev()
        .filter(|&index| match tab {
            Tab::Live => is_live(&orders[index]),
            Tab::Done => !is_live(&orders[index]),
            Tab::Tenders | Tab::Endowments => false,
        })
        .collect()
}

/// The order's text for a tab, one entry per column of that tab.
fn order_texts(model: &Model, order: &OrderRow, tab: Tab) -> Vec<String> {
    let filled = format!(
        "{} / {}",
        format::grouped(order.filled.into()),
        format::grouped(order.quantity.into())
    );
    let answer = order.answer_us.map(format::duration).unwrap_or_default();
    let mut texts = vec![
        format::venue(order.listing),
        model.symbol(order.listing).unwrap_or("?").to_owned(),
        format::side(order.side).to_owned(),
        filled,
        format::price(order.price),
        format::grouped(order.remaining.into()),
        order.status.label(),
    ];
    if tab == Tab::Live {
        texts.push(model.queue_place(order).map_or_else(String::new, |place| {
            format!(
                "#{} · {} ahead",
                place.position_at_price, place.quantity_ahead
            )
        }));
    }
    texts.push(answer);
    texts
}

/// A tab's table as tab-separated text with a header row, newest first.
#[must_use]
pub fn copy_text(model: &Model, tab: Tab) -> String {
    match tab {
        Tab::Live | Tab::Done => {
            let columns: &[Col] = if tab == Tab::Live {
                &LIVE_COLUMNS
            } else {
                &DONE_COLUMNS
            };
            // The cancel button column has no label and no text.
            let shown: Vec<usize> = (0..columns.len())
                .filter(|&index| !columns[index].label.is_empty())
                .collect();
            let header: Vec<&str> = shown.iter().map(|&index| columns[index].label).collect();
            let orders = model.orders();
            let body: Vec<Vec<String>> = indices(model, tab)
                .into_iter()
                .map(|index| {
                    let texts = order_texts(model, &orders[index], tab);
                    shown
                        .iter()
                        .map(|&column| texts.get(column).cloned().unwrap_or_default())
                        .collect()
                })
                .collect();
            ui::tsv(&header, &body)
        }
        Tab::Tenders => {
            let header: Vec<&str> = TENDER_COLUMNS
                .iter()
                .map(|col| col.label)
                .filter(|label| !label.is_empty())
                .collect();
            let body: Vec<Vec<String>> = model
                .tenders()
                .iter()
                .rev()
                .map(|held| tender_texts(model, &held.info))
                .collect();
            ui::tsv(&header, &body)
        }
        Tab::Endowments => String::new(),
    }
}

fn tender_texts(
    model: &Model,
    tender: &bunting_trading_state::bunting_client::bnp_wire::TenderInfo,
) -> Vec<String> {
    let touch = portfolio::touch(model, tender.instrument_id);
    let multi = portfolio::venue_count(model, tender.instrument_id) > 1;
    let symbol = model
        .instruments()
        .into_iter()
        .find(|instrument| instrument.instrument_id == tender.instrument_id)
        .map_or_else(|| tender.instrument_id.to_string(), |i| i.symbol);
    vec![
        symbol,
        format::side(tender.side).to_owned(),
        format::grouped(tender.quantity.into()),
        tender.price.to_string(),
        portfolio::priced(touch.0, multi),
        portfolio::priced(touch.1, multi),
        deals::tender_status(tender.status).to_owned(),
    ]
}

/// The status text, one line with an ellipsis, whole in a tooltip.
fn status_cell(id: u64, text: String) -> AnyElement {
    let tip = SharedString::from(text.clone());
    div()
        .id(SharedString::from(format!("order-status-{id}")))
        .test_support()
        .min_w_0()
        .truncate()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(text)
        .into_any_element()
}

pub struct OrdersPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    tab: Tab,
    _observe: Subscription,
}

dock_panel!(OrdersPanel, "orders", "Blotter");

impl OrdersPanel {
    pub fn new(desk: Entity<Desk>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            tab: Tab::Live,
        }
    }

    /// The tab shown.
    #[must_use]
    pub const fn tab(&self) -> Tab {
        self.tab
    }

    fn tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut group = ButtonGroup::new("blotter-tabs").xsmall();
        for tab in Tab::ALL {
            group = group.child(ui::segment(tab.id(), tab.label(), self.tab == tab));
        }
        ui::toolbar(cx)
            .child(
                group.on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    if let Some(tab) = clicks.first().and_then(|index| Tab::ALL.get(*index)) {
                        this.tab = *tab;
                        cx.notify();
                    }
                })),
            )
            .into_any_element()
    }

    fn orders_table(&self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let count = indices(self.desk.read(cx).model(), tab).len();
        if count == 0 {
            let (id, text) = if tab == Tab::Live {
                ("orders-empty", "No open orders")
            } else {
                ("orders-done-empty", "No filled or cancelled orders yet")
            };
            return ui::empty_state(id, IconName::Inbox, text, None, cx).into_any_element();
        }
        let desk = self.desk.clone();
        let cancel = Rc::new(window.listener_for(&self.desk, |desk, id: &u64, _, cx| {
            desk.cancel(*id, cx);
        }));
        let columns: &'static [Col] = if tab == Tab::Live {
            &LIVE_COLUMNS
        } else {
            &DONE_COLUMNS
        };
        let table = ui::responsive_table(columns, move |fitted, _, cx| {
            let fitted: Vec<ui::Fit> = fitted.to_vec();
            v_flex()
                .size_full()
                .child(ui::table_head(&fitted, cx))
                .child(
                    uniform_list("order-rows", count, move |range, _, cx| {
                        let desk = desk.read(cx);
                        let model = desk.model();
                        let orders = model.orders();
                        let shown = indices(model, tab);
                        range
                            .filter_map(|row| orders.get(*shown.get(row)?))
                            .map(|order| {
                                let id = order.client_order_id;
                                let texts = order_texts(model, order, tab);
                                let mut cells: Vec<AnyElement> = texts
                                    .into_iter()
                                    .enumerate()
                                    .map(|(column, text)| match column {
                                        2 => colored(text, side_color(order.side, cx)),
                                        // A rejection reason can be long: it
                                        // truncates with an ellipsis and the
                                        // whole text is in the tooltip.
                                        6 => status_cell(id, text),
                                        _ => cell(text),
                                    })
                                    .collect();
                                if tab == Tab::Live {
                                    // A cancel on a part-filled order
                                    // cancels what remains.
                                    cells.push(
                                        if matches!(
                                            order.status,
                                            OrderStatus::New | OrderStatus::Resting
                                        ) && id != 0
                                        {
                                            let cancel = cancel.clone();
                                            Button::new(SharedString::from(format!("cancel-{id}")))
                                                .label("Cancel")
                                                .xsmall()
                                                .on_click(move |_, window, cx| {
                                                    cancel(&id, window, cx);
                                                })
                                                .into_any_element()
                                        } else {
                                            cell("")
                                        },
                                    );
                                }
                                ui::table_row(&fitted, cells, cx)
                            })
                            .collect()
                    })
                    .flex_1(),
                )
        });
        let desk = self.desk.clone();
        ui::copy_menu(
            v_flex()
                .id("orders-table")
                .test_support()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(table),
            desk,
            Rc::new(move |desk: &Desk| copy_text(desk.model(), tab)),
        )
        .into_any_element()
    }

    fn tenders_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let model = self.desk.read(cx).model();
        let mut tenders: Vec<_> = model.tenders().iter().map(|held| held.info).collect();
        if tenders.is_empty() {
            return ui::empty_state(
                "blotter-tenders-empty",
                IconName::Replace,
                "No tenders offered to you",
                None,
                cx,
            )
            .into_any_element();
        }
        tenders.reverse();
        tenders.sort_by_key(|tender| tender.status != TenderStatus::Open);
        let fitted = ui::fit_fixed(&TENDER_COLUMNS);
        let mut list = v_flex()
            .id("blotter-tenders")
            .overflow_y_scroll()
            .test_support()
            .flex_1()
            .min_h_0()
            .child(ui::table_head(&fitted, cx));
        for tender in tenders.iter().take(deals::FINISHED_SHOWN) {
            let id = tender.tender_id;
            let texts = tender_texts(model, tender);
            let mut cells: Vec<AnyElement> = texts
                .into_iter()
                .enumerate()
                .map(|(column, text)| match column {
                    1 => colored(text, side_color(tender.side, cx)),
                    _ => cell(text),
                })
                .collect();
            cells.push(if tender.status == TenderStatus::Open {
                gpui_kit::component::h_flex()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("blotter-tender-accept-{id}")))
                            .label("Accept")
                            .xsmall()
                            .primary()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.desk
                                    .update(cx, |desk, cx| desk.decide_tender(id, true, cx));
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("blotter-tender-decline-{id}")))
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
            });
            list = list.child(ui::table_row(&fitted, cells, cx));
        }
        let desk = self.desk.clone();
        ui::copy_menu(
            v_flex()
                .id("tenders-table")
                .test_support()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(list),
            desk,
            Rc::new(|desk: &Desk| copy_text(desk.model(), Tab::Tenders)),
        )
        .into_any_element()
    }
}

impl Render for OrdersPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        let body = panel_body("orders", &self.focus).child(self.tabs(cx));
        match tab {
            Tab::Live | Tab::Done => body.child(self.orders_table(tab, window, cx)),
            Tab::Tenders => body.child(self.tenders_table(cx)),
            Tab::Endowments => body.child(ui::empty_state(
                "blotter-endowments-empty",
                IconName::Inbox,
                "The venue does not send endowments yet",
                None,
                cx,
            )),
        }
    }
}
