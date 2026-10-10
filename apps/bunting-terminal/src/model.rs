#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum WorkspacePreset {
    Trading,
    Research,
    Competition,
}

impl WorkspacePreset {
    pub const ALL: [Self; 3] = [Self::Trading, Self::Research, Self::Competition];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Trading => "TRADING",
            Self::Research => "RESEARCH",
            Self::Competition => "COMPETITION",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
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
    pub const ALL: [Self; 10] = [
        Self::Chart,
        Self::OrderBook,
        Self::OrderTicket,
        Self::Orders,
        Self::Account,
        Self::News,
        Self::Tenders,
        Self::Risk,
        Self::Competition,
        Self::Session,
    ];

    pub const fn panel_name(self) -> &'static str {
        match self {
            Self::Chart => "bunting.market-chart",
            Self::OrderBook => "bunting.order-book",
            Self::OrderTicket => "bunting.order-ticket",
            Self::Orders => "bunting.orders-fills",
            Self::Account => "bunting.account-positions",
            Self::News => "bunting.news",
            Self::Tenders => "bunting.tenders",
            Self::Risk => "bunting.risk-score",
            Self::Competition => "bunting.competition-control",
            Self::Session => "bunting.fix-session",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Chart => "Market Chart",
            Self::OrderBook => "Order Book",
            Self::OrderTicket => "Order Entry",
            Self::Orders => "Orders & Fills",
            Self::Account => "Account & Positions",
            Self::News => "News",
            Self::Tenders => "Tenders",
            Self::Risk => "Risk & Score",
            Self::Competition => "Competition Control",
            Self::Session => "FIX Session",
        }
    }

    pub const fn tab_name(self) -> &'static str {
        match self {
            Self::Chart => "CHART",
            Self::OrderBook => "BOOK",
            Self::OrderTicket => "ORDER",
            Self::Orders => "ORDERS",
            Self::Account => "ACCOUNT",
            Self::News => "NEWS",
            Self::Tenders => "TENDERS",
            Self::Risk => "RISK",
            Self::Competition => "COMP",
            Self::Session => "FIX",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_names_are_stable_and_unique() {
        let mut names = PanelKind::ALL
            .into_iter()
            .map(PanelKind::panel_name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PanelKind::ALL.len());
    }
}
