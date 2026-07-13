use crate::command::ExecutionIntent;
use crate::ids::{ClientOrderId, IntentId};
use crate::order::DesiredOrder;

#[must_use]
pub const fn submit_intent(intent_id: IntentId, order: DesiredOrder) -> ExecutionIntent {
    ExecutionIntent::Submit { intent_id, order }
}

#[must_use]
pub const fn cancel_intent(intent_id: IntentId, client_order_id: ClientOrderId) -> ExecutionIntent {
    ExecutionIntent::Cancel {
        intent_id,
        client_order_id,
    }
}
