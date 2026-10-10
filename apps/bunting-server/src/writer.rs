use std::sync::{Mutex, MutexGuard};

/// The single authoritative commit gate. The admission sequencer and the
/// built-in agent runtime both mutate the run only while holding it, so
/// committed batches are published in commit order. Ordering of
/// participant commands is decided by the admission sequencer (ADR 0034),
/// not here.
pub(crate) struct AuthoritativeWriter {
    gate: Mutex<()>,
}

impl AuthoritativeWriter {
    pub(crate) const fn new() -> Self {
        Self {
            gate: Mutex::new(()),
        }
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.gate
            .lock()
            .map_err(|_| "authoritative writer lock is unavailable".to_owned())
    }
}
