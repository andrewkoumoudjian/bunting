use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct NbcProfileCapabilities {
    pub submit_limit: bool,
    pub cancel: bool,
    pub replace: bool,
    pub explicit_done: bool,
    pub logical_steps: bool,
    pub scoring: bool,
}

impl Default for NbcProfileCapabilities {
    fn default() -> Self {
        Self {
            submit_limit: true,
            cancel: true,
            replace: false,
            explicit_done: true,
            logical_steps: true,
            scoring: false,
        }
    }
}
