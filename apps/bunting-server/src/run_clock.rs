//! One run clock (ADR 0037 §1).
//!
//! The run's logical clock (`simulation.clock.now`) is the only time the
//! engine knows. The venue stamps every released input with the run time
//! at its release instant: under `Paced`, run time follows venue time at
//! `step_ns` per `step_interval_ns` from an anchor, and freezes while the
//! run is not active; under `Lockstep` and `Accelerated` it is whatever the
//! last operator `Advance` left. A stamp never falls behind the clock, so
//! an operator `Advance` simply re-anchors the mapping.
//!
//! The venue timer submits a clock tick through the admission sequencer
//! whenever something falls due (`RunState::next_due`), so scheduled
//! actions and expiries apply on time with no other input. Ticks are
//! journaled inputs: replay needs no timer.

use crate::admission::{AdmissionService, JobContext, Task};
use crate::distributor::PublishingOrigin;
use bunting_application::ApplicationService;
use bunting_engine::RunState;
use bunting_engine::simulation::RunLifecycle;
use bunting_market_events::ClockMode;
use bunting_market_types::{LogicalTimeNs, RunId};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// How often the timer looks for newly due work. A due instant known
/// further ahead is released exactly on time; one created closer than this
/// is applied at most this late (or earlier, by any other input).
const TIMER_POLL: Duration = Duration::from_millis(5);

/// What the stamp needs from the committed run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ClockView {
    pub(crate) now: LogicalTimeNs,
    pub(crate) step_ns: u64,
    pub(crate) mode: ClockMode,
    pub(crate) active: bool,
}

impl ClockView {
    pub(crate) fn of(state: &RunState) -> Self {
        let simulation = state.simulation();
        Self {
            now: simulation.clock.now,
            step_ns: simulation.clock.step_ns,
            mode: simulation.clock.mode,
            active: simulation.lifecycle == RunLifecycle::Active,
        }
    }

    /// Run nanoseconds per venue nanosecond, when run time follows venue
    /// time.
    const fn pace(&self) -> Option<(u64, u64)> {
        match self.mode {
            ClockMode::Paced { step_interval_ns }
                if self.active && self.step_ns > 0 && step_interval_ns > 0 =>
            {
                Some((self.step_ns, step_interval_ns))
            }
            _ => None,
        }
    }
}

/// A run instant paired with the venue instant it was at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Anchor {
    run_ns: u64,
    venue_us: u64,
    pace: (u64, u64),
}

impl Anchor {
    fn run_at(&self, venue_us: u64) -> u64 {
        let (step, interval) = self.pace;
        let elapsed_ns = u128::from(venue_us.saturating_sub(self.venue_us)) * 1_000;
        let run = elapsed_ns * u128::from(step) / u128::from(interval);
        self.run_ns
            .saturating_add(u64::try_from(run).unwrap_or(u64::MAX))
    }

    /// The first venue microsecond at which run time reaches `run_ns`.
    fn venue_at(&self, run_ns: u64) -> u64 {
        let (step, interval) = self.pace;
        let ahead = u128::from(run_ns.saturating_sub(self.run_ns));
        let venue_ns = (ahead * u128::from(interval)).div_ceil(u128::from(step));
        self.venue_us
            .saturating_add(u64::try_from(venue_ns.div_ceil(1_000)).unwrap_or(u64::MAX))
    }
}

/// Venue-to-run time mapping per run. Only the sequencer thread stamps;
/// the timer reads the same anchors to know when to tick.
#[derive(Debug, Default)]
pub(crate) struct RunClock {
    anchors: Mutex<BTreeMap<RunId, Anchor>>,
}

impl RunClock {
    /// The run time of venue instant `venue_us`. Never behind `view.now`.
    pub(crate) fn stamp(&self, run_id: RunId, view: ClockView, venue_us: u64) -> LogicalTimeNs {
        let Ok(mut anchors) = self.anchors.lock() else {
            return view.now;
        };
        let Some(pace) = view.pace() else {
            // Not following venue time: stamps are the clock itself, and
            // the next paced stamp starts a fresh anchor.
            anchors.remove(&run_id);
            return view.now;
        };
        let fresh = Anchor {
            run_ns: view.now.get(),
            venue_us,
            pace,
        };
        let anchor = anchors.entry(run_id).or_insert(fresh);
        let mapped = anchor.run_at(venue_us);
        if anchor.pace != pace || venue_us < anchor.venue_us || mapped < view.now.get() {
            // The pace changed, or an operator moved the clock past the
            // mapping: continue from the clock.
            *anchor = fresh;
            return view.now;
        }
        LogicalTimeNs::new(mapped)
    }

    /// The venue instant at which run time reaches `at`, if run time is
    /// following venue time.
    fn venue_instant(
        &self,
        run_id: RunId,
        view: ClockView,
        at: LogicalTimeNs,
        now_us: u64,
    ) -> Option<u64> {
        let pace = view.pace()?;
        let mut anchors = self.anchors.lock().ok()?;
        let anchor = anchors.entry(run_id).or_insert(Anchor {
            run_ns: view.now.get(),
            venue_us: now_us,
            pace,
        });
        (anchor.pace == pace).then(|| anchor.venue_at(at.get()))
    }
}

impl JobContext<'_> {
    /// The run time at which this job applies: its release instant on the
    /// run's clock. Read and used on the sequencer thread, the only
    /// committer, so no other input can move the clock in between.
    pub(crate) fn run_time(&self, run_id: RunId) -> Result<LogicalTimeNs, String> {
        let view = ApplicationService::new(self.origin)
            .read(run_id, ClockView::of)
            .map_err(|error| format!("run read failed: {error}"))?;
        Ok(self
            .run_clock
            .stamp(run_id, view, self.admission.release_us))
    }
}

/// The venue timer for `run_id`: forever, queues a clock tick for the
/// venue instant at which the next item falls due.
pub(crate) fn run_timer(
    admission: &AdmissionService,
    origin: &PublishingOrigin,
    run_id: RunId,
) -> Result<(), String> {
    let mut queued: Option<u64> = None;
    loop {
        let (view, due) = ApplicationService::new(origin)
            .read(run_id, |state| (ClockView::of(state), state.next_due()))
            .map_err(|error| format!("timer run read failed: {error}"))?;
        let now_us = admission.clock().now_us();
        let at = due.and_then(|due| {
            admission
                .run_clock()
                .venue_instant(run_id, view, due, now_us)
        });
        // Queue a tick for a due instant inside the next poll, once; a
        // tick released when nothing is due any more commits nothing.
        if let Some(at) = at
            && at <= now_us.saturating_add(duration_us(TIMER_POLL))
            && queued.is_none_or(|queued| queued != at || queued < now_us)
        {
            admission
                .schedule(at, tick_task(run_id))
                .map_err(|error| format!("clock tick admission: {error}"))?;
            queued = Some(at);
        }
        std::thread::sleep(TIMER_POLL);
    }
}

fn tick_task(run_id: RunId) -> Task {
    Box::new(move |context| {
        let outcome = context.run_time(run_id).and_then(|logical_time| {
            ApplicationService::new(context.origin)
                .execute_clock_tick(run_id, logical_time, context.admission)
                .map_err(|error| error.to_string())
        });
        if let Err(error) = outcome {
            eprintln!("bunting-server: clock tick failed: {error}");
        }
    })
}

fn duration_us(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn paced(now: u64, step_ns: u64, step_interval_ns: u64) -> ClockView {
        ClockView {
            now: LogicalTimeNs::new(now),
            step_ns,
            mode: ClockMode::Paced { step_interval_ns },
            active: true,
        }
    }

    #[test]
    fn paced_run_time_follows_venue_time_at_its_pace() {
        let clock = RunClock::default();
        let run = RunId::new(1);
        // Real time: one run nanosecond per venue nanosecond.
        assert_eq!(clock.stamp(run, paced(0, 1_000, 1_000), 500).get(), 0);
        assert_eq!(
            clock.stamp(run, paced(0, 1_000, 1_000), 1_500).get(),
            1_000_000
        );
        // A tick due at 2 ms run time is released at 2.5 ms venue time.
        assert_eq!(
            clock.venue_instant(
                run,
                paced(1_000_000, 1_000, 1_000),
                LogicalTimeNs::new(2_000_000),
                1_500
            ),
            Some(2_500)
        );
    }

    #[test]
    fn compressed_time_rounds_ticks_up_so_they_find_their_item_due() {
        let clock = RunClock::default();
        let run = RunId::new(1);
        // Sixty run seconds per venue second.
        let view = paced(0, 60_000, 1_000);
        assert_eq!(clock.stamp(run, view, 0).get(), 0);
        let at = clock
            .venue_instant(run, view, LogicalTimeNs::new(1_000_000_001), 0)
            .unwrap();
        assert!(clock.stamp(run, view, at).get() >= 1_000_000_001);
        assert!(clock.stamp(run, view, at - 1).get() < 1_000_000_001);
    }

    #[test]
    fn an_advance_or_pause_reanchors_and_stamps_never_regress() {
        let clock = RunClock::default();
        let run = RunId::new(1);
        assert_eq!(clock.stamp(run, paced(0, 1_000, 1_000), 0).get(), 0);
        // An operator advance moved the clock to 10 ms at venue 1 ms.
        assert_eq!(
            clock
                .stamp(run, paced(10_000_000, 1_000, 1_000), 1_000)
                .get(),
            10_000_000
        );
        assert_eq!(
            clock
                .stamp(run, paced(10_000_000, 1_000, 1_000), 2_000)
                .get(),
            11_000_000
        );
        // Paused: frozen.
        let mut paused = paced(11_000_000, 1_000, 1_000);
        paused.active = false;
        assert_eq!(clock.stamp(run, paused, 50_000).get(), 11_000_000);
        // Resumed at venue 60 ms: continues from the frozen clock.
        assert_eq!(
            clock
                .stamp(run, paced(11_000_000, 1_000, 1_000), 60_000)
                .get(),
            11_000_000
        );
        assert_eq!(
            clock
                .stamp(run, paced(11_000_000, 1_000, 1_000), 61_000)
                .get(),
            12_000_000
        );
    }

    #[test]
    fn lockstep_stamps_the_clock_and_never_ticks() {
        let clock = RunClock::default();
        let run = RunId::new(1);
        let view = ClockView {
            now: LogicalTimeNs::new(7),
            step_ns: 1_000_000,
            mode: ClockMode::Lockstep,
            active: true,
        };
        assert_eq!(clock.stamp(run, view, 99_999).get(), 7);
        assert_eq!(
            clock.venue_instant(run, view, LogicalTimeNs::new(8), 0),
            None
        );
    }
}
