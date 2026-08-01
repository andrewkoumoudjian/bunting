use gpui::{Pixels, Point, Size, point, px, size};

pub const MIN_PANEL_WIDTH: Pixels = px(280.0);
pub const MIN_PANEL_HEIGHT: Pixels = px(180.0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspacePreset {
    Trading,
    Research,
    Competition,
}

impl WorkspacePreset {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trading => "TRADING",
            Self::Research => "RESEARCH",
            Self::Competition => "COMPETITION",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelKind {
    Chart,
    OrderBook,
    OrderTicket,
    Orders,
    Account,
    News,
    Tenders,
    Risk,
    Competition,
    Session,
}

impl PanelKind {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Chart => "MARKET CHART",
            Self::OrderBook => "ORDER BOOK",
            Self::OrderTicket => "ORDER ENTRY",
            Self::Orders => "ORDERS & FILLS",
            Self::Account => "ACCOUNT & POSITIONS",
            Self::News => "NEWS",
            Self::Tenders => "TENDERS",
            Self::Risk => "RISK & SCORE",
            Self::Competition => "COMPETITION CONTROL",
            Self::Session => "FIX SESSION",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PanelRect {
    pub origin: Point<Pixels>,
    pub size: Size<Pixels>,
}

impl PanelRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: point(px(x), px(y)),
            size: size(px(width), px(height)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PanelState {
    pub id: usize,
    pub kind: PanelKind,
    pub rect: PanelRect,
    pub z: u32,
    pub visible: bool,
    pub minimized: bool,
}

impl PanelState {
    pub const fn new(id: usize, kind: PanelKind, rect: PanelRect, z: u32) -> Self {
        Self {
            id,
            kind,
            rect,
            z,
            visible: true,
            minimized: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PointerGesture {
    Move {
        panel_id: usize,
        pointer_start: Point<Pixels>,
        panel_start: Point<Pixels>,
    },
    Resize {
        panel_id: usize,
        pointer_start: Point<Pixels>,
        size_start: Size<Pixels>,
    },
}

pub fn default_panels() -> Vec<PanelState> {
    vec![
        PanelState::new(0, PanelKind::Chart, PanelRect::new(12., 12., 710., 430.), 1),
        PanelState::new(
            1,
            PanelKind::OrderBook,
            PanelRect::new(734., 12., 350., 430.),
            2,
        ),
        PanelState::new(
            2,
            PanelKind::OrderTicket,
            PanelRect::new(1096., 12., 360., 430.),
            3,
        ),
        PanelState::new(3, PanelKind::Orders, PanelRect::new(12., 454., 650., 320.), 4),
        PanelState::new(
            4,
            PanelKind::Account,
            PanelRect::new(674., 454., 500., 320.),
            5,
        ),
        PanelState::new(
            5,
            PanelKind::News,
            PanelRect::new(1186., 454., 470., 320.),
            6,
        ),
        PanelState::new(
            6,
            PanelKind::Tenders,
            PanelRect::new(840., 124., 500., 330.),
            7,
        ),
        PanelState::new(
            7,
            PanelKind::Risk,
            PanelRect::new(870., 280., 470., 330.),
            8,
        ),
        PanelState::new(
            8,
            PanelKind::Competition,
            PanelRect::new(340., 160., 620., 390.),
            9,
        ),
        PanelState::new(
            9,
            PanelKind::Session,
            PanelRect::new(180., 110., 760., 470.),
            10,
        ),
    ]
}

pub fn apply_preset(panels: &mut [PanelState], preset: WorkspacePreset) {
    for panel in panels.iter_mut() {
        panel.visible = false;
        panel.minimized = false;
    }

    let layout: &[(PanelKind, PanelRect)] = match preset {
        WorkspacePreset::Trading => &[
            (PanelKind::Chart, PanelRect::new(12., 12., 690., 410.)),
            (
                PanelKind::OrderBook,
                PanelRect::new(714., 12., 350., 410.),
            ),
            (
                PanelKind::OrderTicket,
                PanelRect::new(1076., 12., 380., 410.),
            ),
            (PanelKind::Orders, PanelRect::new(12., 434., 650., 330.)),
            (
                PanelKind::Account,
                PanelRect::new(674., 434., 500., 330.),
            ),
            (PanelKind::Risk, PanelRect::new(1186., 434., 470., 330.)),
        ],
        WorkspacePreset::Research => &[
            (PanelKind::Chart, PanelRect::new(12., 12., 930., 500.)),
            (PanelKind::News, PanelRect::new(954., 12., 702., 500.)),
            (PanelKind::Account, PanelRect::new(12., 524., 650., 300.)),
            (PanelKind::Risk, PanelRect::new(674., 524., 500., 300.)),
            (
                PanelKind::Session,
                PanelRect::new(1186., 524., 470., 300.),
            ),
        ],
        WorkspacePreset::Competition => &[
            (
                PanelKind::Competition,
                PanelRect::new(12., 12., 610., 420.),
            ),
            (PanelKind::Tenders, PanelRect::new(634., 12., 500., 420.)),
            (PanelKind::News, PanelRect::new(1146., 12., 510., 420.)),
            (PanelKind::Chart, PanelRect::new(12., 444., 710., 380.)),
            (PanelKind::Account, PanelRect::new(734., 444., 450., 380.)),
            (PanelKind::Risk, PanelRect::new(1196., 444., 460., 380.)),
        ],
    };

    for (z, (kind, rect)) in layout.iter().copied().enumerate() {
        if let Some(panel) = panels.iter_mut().find(|panel| panel.kind == kind) {
            panel.rect = rect;
            panel.visible = true;
            panel.z = u32::try_from(z + 1).unwrap_or(u32::MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_identifiers_are_unique() {
        let panels = default_panels();
        let mut ids = panels.iter().map(|panel| panel.id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), panels.len());
    }

    #[test]
    fn trading_preset_surfaces_the_execution_workflow() {
        let mut panels = default_panels();
        apply_preset(&mut panels, WorkspacePreset::Trading);
        for kind in [
            PanelKind::Chart,
            PanelKind::OrderBook,
            PanelKind::OrderTicket,
            PanelKind::Orders,
            PanelKind::Account,
            PanelKind::Risk,
        ] {
            assert!(
                panels
                    .iter()
                    .any(|panel| panel.kind == kind && panel.visible)
            );
        }
        assert!(
            panels
                .iter()
                .any(|panel| panel.kind == PanelKind::Competition && !panel.visible)
        );
    }
}
