//! The trading workspace: one `DockArea` holding every market panel, and a
//! status bar for the connection (ADR 0050, decision 6).
//!
//! Phase 1 lays out the panels and their empty states. Their contents come
//! from the shared client state crate once it is on `main`; until then each
//! panel says what it will show and that no venue is connected.

use gpui_kit::component::dock::{
    BasePanel, DockArea, DockLayout, DockSkin, Panel, PanelEvent, panel_handle,
};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, TestSupportExt as _, Window, div,
    prelude::*, px,
};

/// One panel of the workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanelSpec {
    /// Stable element ID, also the panel's persisted name.
    pub id: &'static str,
    pub title: &'static str,
    /// What the panel shows once a venue is connected.
    pub purpose: &'static str,
}

/// Per-venue books: each venue's book exactly as that venue's feed
/// reported it. Never merged.
pub const BOOKS: PanelSpec = PanelSpec {
    id: "books",
    title: "Books",
    purpose: "Each venue's book as its own feed reports it: price levels, and order by order where the venue publishes it.",
};
/// Order entry. Every order names its venue; there is no default router.
pub const TICKET: PanelSpec = PanelSpec {
    id: "ticket",
    title: "Order ticket",
    purpose: "Side, quantity, price, time in force and the venue for this order. Cancel and kill switch.",
};
pub const TAPE: PanelSpec = PanelSpec {
    id: "tape",
    title: "Consolidated tape",
    purpose: "Trades from every venue as the processor at the hub published them, late by its own distance.",
};
pub const ORDERS: PanelSpec = PanelSpec {
    id: "orders",
    title: "Open orders",
    purpose: "Your live orders on every venue, from committed reports.",
};
pub const FILLS: PanelSpec = PanelSpec {
    id: "fills",
    title: "Fills",
    purpose: "Every fill with its venue, price, quantity, fee and liquidity.",
};
pub const POSITIONS: PanelSpec = PanelSpec {
    id: "positions",
    title: "Positions",
    purpose: "Position per instrument, with open buy and sell quantity.",
};
pub const ACCOUNT: PanelSpec = PanelSpec {
    id: "account",
    title: "Account",
    purpose: "Cash balances and reserved cash.",
};

/// Every panel, in layout order.
pub const PANELS: [PanelSpec; 7] = [BOOKS, TICKET, TAPE, ORDERS, FILLS, POSITIONS, ACCOUNT];

/// A workspace panel. Phase 1 renders its empty state.
pub struct MarketPanel {
    spec: PanelSpec,
    focus: FocusHandle,
}

impl MarketPanel {
    fn new(spec: PanelSpec, cx: &mut Context<Self>) -> Self {
        Self {
            spec,
            focus: cx.focus_handle(),
        }
    }
}

impl BasePanel for MarketPanel {
    fn panel_name(&self) -> &'static str {
        self.spec.id
    }
}

impl Panel for MarketPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.spec.title
    }
}

impl EventEmitter<PanelEvent> for MarketPanel {}

impl Focusable for MarketPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MarketPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id(self.spec.id)
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .p_3()
            .gap_2()
            .child(div().text_sm().child("No venue connected"))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.spec.purpose),
            )
    }
}

/// Connection state shown in the status bar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Connection {
    Disconnected,
    /// Connected to a venue: its address and the last measured round trip.
    Connected {
        venue: SharedString,
        round_trip_us: Option<u64>,
    },
}

impl Connection {
    /// The status bar's text. Latency is the measured round trip only;
    /// nothing is smoothed or estimated (ADR 0035).
    #[must_use]
    pub fn label(&self) -> SharedString {
        match self {
            Self::Disconnected => "Not connected".into(),
            Self::Connected {
                venue,
                round_trip_us: Some(round_trip),
            } => format!("Connected to {venue} · round trip {round_trip} µs").into(),
            Self::Connected {
                venue,
                round_trip_us: None,
            } => format!("Connected to {venue} · round trip not measured yet").into(),
        }
    }
}

/// The window's content: the dock workspace above a status bar.
pub struct Workspace {
    area: Entity<DockArea>,
    connection: Connection,
}

impl Workspace {
    /// Builds the default layout: books in the centre, the ticket and tape
    /// on the right, and the blotter (orders, fills, positions, account)
    /// along the bottom.
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let (area, _skin) = DockSkin::dock_area("bunting-desk", Some(1), window, cx);
        let panel = |spec: PanelSpec, cx: &mut App| {
            let view = cx.new(|cx| MarketPanel::new(spec, cx));
            panel_handle(view)
        };
        let tabs = |specs: &[PanelSpec], cx: &mut App| {
            specs.iter().fold(DockLayout::tabs(), |layout, spec| {
                let handle = panel(*spec, cx);
                layout.panel_view(handle, cx)
            })
        };
        let layout = DockLayout::v_split()
            .child(
                DockLayout::h_split().child(tabs(&[BOOKS], cx), None).child(
                    DockLayout::v_split()
                        .child(tabs(&[TICKET], cx), None)
                        .child(tabs(&[TAPE], cx), None),
                    Some(px(360.)),
                ),
                None,
            )
            .child(
                tabs(&[ORDERS, FILLS, POSITIONS, ACCOUNT], cx),
                Some(px(220.)),
            );
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
        Self {
            area,
            connection: Connection::Disconnected,
        }
    }

    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn set_connection(&mut self, connection: Connection, cx: &mut Context<Self>) {
        self.connection = connection;
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(div().flex_1().min_h_0().child(self.area.clone()))
            .child(
                StatusBar::new()
                    .left(
                        div()
                            .id("connection")
                            .test_support()
                            .child(self.connection.label()),
                    )
                    .right(concat!("bunting-desk ", env!("CARGO_PKG_VERSION"))),
            )
    }
}
