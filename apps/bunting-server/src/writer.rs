use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Single authoritative commit gate with deterministic interval admission.
pub(crate) struct AuthoritativeWriter {
    started: Instant,
    interval: Duration,
    max_queue: usize,
    next_arrival: AtomicU64,
    queued: AtomicUsize,
    next_commit: Mutex<u64>,
    turn: Condvar,
    gate: Mutex<()>,
}

impl AuthoritativeWriter {
    pub(crate) fn new(interval: Duration, max_queue: usize) -> Self {
        Self {
            started: Instant::now(),
            interval,
            max_queue,
            next_arrival: AtomicU64::new(0),
            queued: AtomicUsize::new(0),
            next_commit: Mutex::new(0),
            turn: Condvar::new(),
            gate: Mutex::new(()),
        }
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.gate
            .lock()
            .map_err(|_| "authoritative writer lock is unavailable".to_owned())
    }

    pub(crate) fn execute_interval<T>(
        &self,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let prior = self.queued.fetch_add(1, Ordering::AcqRel);
        if prior >= self.max_queue {
            self.queued.fetch_sub(1, Ordering::AcqRel);
            return Err(format!("max_interval_queue limit {}", self.max_queue));
        }
        let _slot = QueueSlot(&self.queued);
        let arrival = self.next_arrival.fetch_add(1, Ordering::AcqRel);
        let elapsed = self.started.elapsed();
        let interval_nanos = self.interval.as_nanos().max(1);
        let remainder = elapsed.as_nanos() % interval_nanos;
        if remainder != 0 {
            let remaining = interval_nanos.saturating_sub(remainder);
            std::thread::sleep(Duration::from_nanos(
                u64::try_from(remaining).unwrap_or(u64::MAX),
            ));
        }

        let mut next = self
            .next_commit
            .lock()
            .map_err(|_| "arrival sequence lock is unavailable".to_owned())?;
        while *next != arrival {
            next = self
                .turn
                .wait(next)
                .map_err(|_| "arrival sequence wait is unavailable".to_owned())?;
        }
        let _gate = self.lock()?;
        let result = action();
        *next = next.saturating_add(1);
        self.turn.notify_all();
        result
    }
}

struct QueueSlot<'a>(&'a AtomicUsize);

impl Drop for QueueSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn concurrent_arrivals_commit_in_sequence() -> Result<(), String> {
        let writer = Arc::new(AuthoritativeWriter::new(Duration::from_millis(1), 8));
        let committed = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        for value in 0..4 {
            let writer = writer.clone();
            let committed = committed.clone();
            handles.push(thread::spawn(move || {
                writer.execute_interval(|| {
                    committed
                        .lock()
                        .map_err(|_| "test lock".to_owned())?
                        .push(value);
                    Ok(())
                })
            }));
            thread::sleep(Duration::from_micros(50));
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| "test thread panicked".to_owned())??;
        }
        assert_eq!(
            *committed.lock().map_err(|_| "test lock".to_owned())?,
            vec![0, 1, 2, 3]
        );
        Ok(())
    }
}
