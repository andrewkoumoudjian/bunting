use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ExecutionCapabilities {
    pub submit: bool,
    pub cancel: bool,
    pub replace: bool,
    pub reconcile: bool,
    pub snapshot_restore: bool,
    pub kill_switch: bool,
}

impl Default for ExecutionCapabilities {
    fn default() -> Self {
        Self {
            submit: true,
            cancel: true,
            replace: true,
            reconcile: true,
            snapshot_restore: true,
            kill_switch: true,
        }
    }
}
