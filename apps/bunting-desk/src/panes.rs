//! Every kind of pane the workspace can open, any number of times, and the
//! builders that bring panes back from a saved layout.

use crate::deals::DealsPanel;
use crate::desk::Desk;
use crate::ladder::LadderPanel;
use crate::news::NewsPanel;
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
    LadderTrader,
    OrderEntry,
    Tape,
    Blotter,
    Fills,
    Portfolio,
    TraderInfo,
    Notices,
    News,
    Deals,
}

impl PaneKind {
    pub const ALL: [Self; 11] = [
        Self::BookTrader,
        Self::LadderTrader,
        Self::OrderEntry,
        Self::Tape,
        Self::Blotter,
        Self::Fills,
        Self::Portfolio,
        Self::TraderInfo,
        Self::Notices,
        Self::News,
        Self::Deals,
    ];

    /// The name saved layouts use, and the pane's element ID.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BookTrader => "books",
            Self::LadderTrader => "ladder",
            Self::OrderEntry => "ticket",
            Self::Tape => "tape",
            Self::Blotter => "orders",
            Self::Fills => "fills",
            Self::Portfolio => "positions",
            Self::TraderInfo => "account",
            Self::Notices => "notices",
            Self::News => "news",
            Self::Deals => "deals",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::BookTrader => "Book Trader",
            Self::LadderTrader => "Ladder Trader",
            Self::OrderEntry => "Order Entry",
            Self::Tape => "Tape",
            Self::Blotter => "Blotter",
            Self::Fills => "Fills",
            Self::Portfolio => "Portfolio",
            Self::TraderInfo => "Trader Info",
            Self::Notices => "Notices",
            Self::News => "News",
            Self::Deals => "Tenders & OTC",
        }
    }

    #[must_use]
    pub const fn icon(self) -> IconName {
        match self {
            Self::BookTrader => IconName::BookOpen,
            Self::LadderTrader => IconName::ChevronsUpDown,
            Self::OrderEntry => IconName::SquareTerminal,
            Self::Tape => IconName::Globe,
            Self::Blotter => IconName::Inbox,
            Self::Fills => IconName::CircleCheck,
            Self::Portfolio => IconName::ChartPie,
            Self::TraderInfo => IconName::CircleUser,
            Self::Notices => IconName::Bell,
            Self::News => IconName::Info,
            Self::Deals => IconName::Replace,
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The smallest window this kind stays usable in (ADR 0051 decision 2):
    /// resize drags and newly opened windows honour it.
    #[must_use]
    pub const fn min_size(self) -> (f32, f32) {
        match self {
            Self::BookTrader => (520., 320.),
            Self::LadderTrader => (360., 320.),
            Self::OrderEntry => (300., 320.),
            Self::Tape => (320., 240.),
            Self::Blotter => (560., 220.),
            Self::Fills => (480., 220.),
            Self::Portfolio => (360., 220.),
            Self::TraderInfo => (300., 220.),
            Self::Notices => (300., 200.),
            Self::News => (420., 260.),
            Self::Deals => (420., 300.),
        }
    }

    /// The size a newly opened window of this kind gets.
    #[must_use]
    pub const fn default_size(self) -> (f32, f32) {
        match self {
            Self::BookTrader => (760., 500.),
            Self::LadderTrader => (440., 520.),
            Self::OrderEntry => (360., 460.),
            Self::Tape => (420., 420.),
            Self::Blotter => (840., 320.),
            Self::Fills => (560., 320.),
            Self::Portfolio => (440., 300.),
            Self::TraderInfo => (380., 300.),
            Self::Notices => (420., 300.),
            Self::News => (560., 420.),
            Self::Deals => (560., 440.),
        }
    }

    /// Whether the pane follows the desk's instrument, so its window frame
    /// carries the instrument chip.
    #[must_use]
    pub const fn follows_instrument(self) -> bool {
        matches!(
            self,
            Self::BookTrader | Self::LadderTrader | Self::OrderEntry | Self::Tape
        )
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
            Self::LadderTrader => panel_handle(cx.new(|cx| LadderPanel::new(desk, window, cx))),
            Self::OrderEntry => panel_handle(cx.new(|cx| TicketPanel::new(desk, window, cx))),
            Self::Tape => panel_handle(cx.new(|cx| TradesPanel::new(desk, cx))),
            Self::Blotter => panel_handle(cx.new(|cx| OrdersPanel::new(desk, cx))),
            Self::Fills => panel_handle(cx.new(|cx| FillsPanel::new(desk, cx))),
            Self::Portfolio => panel_handle(cx.new(|cx| PositionsPanel::new(desk, cx))),
            Self::TraderInfo => panel_handle(cx.new(|cx| AccountPanel::new(desk, cx))),
            Self::Notices => panel_handle(cx.new(|cx| NoticesPanel::new(desk, cx))),
            Self::News => panel_handle(cx.new(|cx| NewsPanel::new(desk, cx))),
            Self::Deals => panel_handle(cx.new(|cx| DealsPanel::new(desk, window, cx))),
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
