use crate::ids::{ClientOrderId, IntentId, LocalOrderId};
use crate::order::DesiredOrder;
use bunting_market_events::OrderKind;
use bunting_market_types::QuantityLots;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionIntent {
    Submit {
        intent_id: IntentId,
        order: DesiredOrder,
    },
    Cancel {
        intent_id: IntentId,
        client_order_id: ClientOrderId,
    },
    Replace {
        intent_id: IntentId,
        client_order_id: ClientOrderId,
        quantity: QuantityLots,
        kind: OrderKind,
    },
    Query {
        intent_id: IntentId,
        local_order_id: LocalOrderId,
    },
    ActivateKillSwitch {
        intent_id: IntentId,
    },
}

impl ExecutionIntent {
    #[must_use]
    pub const fn intent_id(&self) -> IntentId {
        match self {
            Self::Submit { intent_id, .. }
            | Self::Cancel { intent_id, .. }
            | Self::Replace { intent_id, .. }
            | Self::Query { intent_id, .. }
            | Self::ActivateKillSwitch { intent_id } => *intent_id,
        }
    }
}
