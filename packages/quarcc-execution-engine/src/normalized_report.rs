use crate::ids::{ClientOrderId, LocalOrderId, ReportId, VenueOrderId};
use bunting_market_types::{PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueReportKind {
    Accepted,
    Rejected {
        reason: String,
    },
    Fill {
        last_quantity: QuantityLots,
        cumulative_quantity: QuantityLots,
        price: PriceTicks,
    },
    Cancelled,
    CancelRejected {
        reason: String,
    },
    Replaced,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NormalizedVenueReport {
    pub report_id: ReportId,
    pub source_sequence: Option<u64>,
    pub client_order_id: Option<ClientOrderId>,
    pub local_order_id: Option<LocalOrderId>,
    pub venue_order_id: Option<VenueOrderId>,
    /// Authoritative remaining quantity when supplied by the venue.
    pub leaves_quantity: Option<QuantityLots>,
    pub kind: VenueReportKind,
}
