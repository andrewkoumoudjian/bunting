//! Every kind of pane the workspace can open, any number of times, and the
//! builders that bring panes back from a saved layout.

use crate::desk::Desk;
use crate::panels::{
    AccountPanel, BooksPanel, FillsPanel, NoticesPanel, OrdersPanel, PositionsPanel, TicketPanel,
    TradesPanel,
};
use gpui_kit::component::IconName;
use gpui_kit::component::dock::{BasePanelView, panel_handle, register_panel};
use gpui_kit::{App, AppContext as _, Entity, Global, Window};
use std::sync::Arc;

/// A kind of pane. Its [`name`](Self::name) is written into saved layouts
/// and never changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneKind {
    BookTrader,
    OrderEntry,
    Tape,
    Blotter,
    Fills,
    Portfolio,
    TraderInfo,
    Notices,
}

impl PaneKind {
    pub const ALL: [Self; 8] = [
        Self::BookTrader,
        Self::OrderEntry,
        Self::Tape,
        Self::Blotter,
        Self::Fills,
        Self::Portfolio,
        Self::TraderInfo,
        Self::Notices,
    ];

    /// The name saved layouts use, and the pane's element ID.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BookTrader => "books",
            Self::OrderEntry => "ticket",
            Self::Tape => "tape",
            Self::Blotter => "orders",
            Self::Fills => "fills",
            Self::Portfolio => "positions",
            Self::TraderInfo => "account",
            Self::Notices => "notices",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::BookTrader => "Book Trader",
            Self::OrderEntry => "Order Entry",
            Self::Tape => "Tape",
            Self::Blotter => "Blotter",
            Self::Fills => "Fills",
            Self::Portfolio => "Portfolio",
            Self::TraderInfo => "Trader Info",
            Self::Notices => "Notices",
        }
    }

    #[must_use]
    pub const fn icon(self) -> IconName {
        match self {
            Self::BookTrader => IconName::BookOpen,
            Self::OrderEntry => IconName::SquareTerminal,
            Self::Tape => IconName::Globe,
            Self::Blotter => IconName::Inbox,
            Self::Fills => IconName::CircleCheck,
            Self::Portfolio => IconName::ChartPie,
            Self::TraderInfo => IconName::CircleUser,
            Self::Notices => IconName::Bell,
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// A new pane of this kind on `desk`.
    pub fn build(
        self,
        desk: Entity<Desk>,
        window: &mut Window,
        cx: &mut App,
    ) -> Arc<dyn BasePanelView> {
        match self {
            Self::BookTrader => panel_handle(cx.new(|cx| BooksPanel::new(desk, window, cx))),
            Self::OrderEntry => panel_handle(cx.new(|cx| TicketPanel::new(desk, window, cx))),
            Self::Tape => panel_handle(cx.new(|cx| TradesPanel::new(desk, cx))),
            Self::Blotter => panel_handle(cx.new(|cx| OrdersPanel::new(desk, cx))),
            Self::Fills => panel_handle(cx.new(|cx| FillsPanel::new(desk, cx))),
            Self::Portfolio => panel_handle(cx.new(|cx| PositionsPanel::new(desk, cx))),
            Self::TraderInfo => panel_handle(cx.new(|cx| AccountPanel::new(desk, cx))),
            Self::Notices => panel_handle(cx.new(|cx| NoticesPanel::new(desk, cx))),
        }
    }
}

/// The desk restored panes attach to.
struct DeskHandle(Entity<Desk>);

impl Global for DeskHandle {}

/// Lets saved layouts rebuild every kind of pane on `desk`.
pub fn register(desk: &Entity<Desk>, cx: &mut App) {
    cx.set_global(DeskHandle(desk.clone()));
    for kind in PaneKind::ALL {
        register_panel(cx, kind.name(), move |_, window, cx| {
            let desk = cx.global::<DeskHandle>().0.clone();
            kind.build(desk, window, cx)
        });
    }
}
