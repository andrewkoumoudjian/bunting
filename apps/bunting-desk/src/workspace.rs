//! The trading workspace: one `DockArea` holding every market panel, and a
//! status bar for the connection (ADR 0050, decision 6).

use crate::desk::Desk;
use crate::panels::{
    AccountPanel, BooksPanel, FillsPanel, NoticesPanel, OrdersPanel, PositionsPanel, TicketPanel,
    TradesPanel, connection_label,
};
use gpui_kit::component::dock::{DockArea, DockLayout, DockSkin, panel_handle};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, TestSupportExt as _, Window, div, prelude::*, px,
};

/// Element IDs of every panel, in layout order.
pub const PANELS: [&str; 8] = [
    "books",
    "ticket",
    "tape",
    "orders",
    "fills",
    "positions",
    "account",
    "notices",
];

/// The window's content: the dock workspace above a status bar.
pub struct Workspace {
    desk: Entity<Desk>,
    area: Entity<DockArea>,
    _observe: Subscription,
}

impl Workspace {
    /// Builds the default layout: books in the centre, the ticket and
    /// trades on the right, and orders, fills, positions, account and
    /// notices along the bottom.
    pub fn new(desk: Entity<Desk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (area, _skin) = DockSkin::dock_area("bunting-desk", Some(2), window, cx);
        let books = cx.new(|cx| BooksPanel::new(desk.clone(), cx));
        let ticket = cx.new(|cx| TicketPanel::new(desk.clone(), window, cx));
        let trades = cx.new(|cx| TradesPanel::new(desk.clone(), cx));
        let orders = cx.new(|cx| OrdersPanel::new(desk.clone(), cx));
        let fills = cx.new(|cx| FillsPanel::new(desk.clone(), cx));
        let positions = cx.new(|cx| PositionsPanel::new(desk.clone(), cx));
        let account = cx.new(|cx| AccountPanel::new(desk.clone(), cx));
        let notices = cx.new(|cx| NoticesPanel::new(desk.clone(), cx));
        let app: &App = cx;
        let layout = DockLayout::v_split()
            .child(
                DockLayout::h_split()
                    .child(
                        DockLayout::tabs().panel_view(panel_handle(books), app),
                        None,
                    )
                    .child(
                        DockLayout::v_split()
                            .child(
                                DockLayout::tabs().panel_view(panel_handle(ticket), app),
                                None,
                            )
                            .child(
                                DockLayout::tabs().panel_view(panel_handle(trades), app),
                                Some(px(240.)),
                            ),
                        Some(px(380.)),
                    ),
                None,
            )
            .child(
                DockLayout::tabs()
                    .panel_view(panel_handle(orders), app)
                    .panel_view(panel_handle(fills), app)
                    .panel_view(panel_handle(positions), app)
                    .panel_view(panel_handle(account), app)
                    .panel_view(panel_handle(notices), app),
                Some(px(240.)),
            );
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            area,
        }
    }

    #[must_use]
    pub fn desk(&self) -> &Entity<Desk> {
        &self.desk
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = connection_label(self.desk.read(cx));
        v_flex()
            .size_full()
            .child(div().flex_1().min_h_0().child(self.area.clone()))
            .child(
                StatusBar::new()
                    .left(div().id("connection").test_support().child(status))
                    .right(concat!("bunting-desk ", env!("CARGO_PKG_VERSION"))),
            )
    }
}
