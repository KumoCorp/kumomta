//! Coordinates write pauses and automatic retries after spool errors.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Timing and toggle that govern when the gate latches and when it may
/// reopen on its own.
#[derive(Clone, Copy)]
pub(super) struct Policy {
    /// Delay from the first error of an incident until the gate latches.
    pub latch_duration: Duration,
    /// Quiet interval required, once latched, before an automatic reopen.
    pub unlatch_duration: Duration,
    /// Whether the gate may reopen on its own. False requires a restart.
    pub allow_unlatch: bool,
}

impl Policy {
    /// Rejects a configuration that could never keep the gate open: with
    /// unlatching enabled, a zero unlatch interval would clear and re-latch
    /// on the same monitor tick.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.allow_unlatch || !self.unlatch_duration.is_zero(),
            "error_unlatch_duration must be greater than zero when allow_error_unlatch is true"
        );
        Ok(())
    }
}

/// Serializes error observations and gate transitions while allowing
/// lock-free checks.
pub(super) struct Health {
    state: Mutex<State>,
    active: AtomicBool,
    policy: Policy,
    path: String,
}

impl Health {
    pub fn new(background_errors: u64, policy: Policy, path: String) -> Self {
        Self {
            state: Mutex::new(State::new(background_errors)),
            active: AtomicBool::new(false),
            policy,
            path,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Returns the delay from the first observed error in an incident before
    /// the gate latches.
    pub fn latch_duration(&self) -> Duration {
        self.policy.latch_duration
    }

    /// Records an error and returns whether it starts an incident or
    /// latches the gate.
    pub fn record_foreground_error(&self, fatal: bool) -> bool {
        self.update(|state| state.foreground_error(Instant::now(), fatal))
    }

    /// Feeds the latest background-error count from the periodic monitor into
    /// the state machine, driving the latch and reopen timers. Called on every
    /// monitor tick with a fresh sample, not just when the count changed.
    /// Growth over the previous sample is a new error.
    pub fn sample_background_errors(&self, background_errors: u64) {
        let mut growth = None;
        self.update(|state| {
            if background_errors > state.background_errors {
                growth = Some((state.background_errors, background_errors));
            }
            state.tick(Instant::now(), background_errors, self.policy)
        });
        // Logged after `update` releases the state lock: `tracing` dispatch
        // is synchronous and can block on a slow sink, and holding the lock
        // across that would serialize every concurrent error reporter
        // behind it.
        if let Some((prev, new)) = growth {
            tracing::error!(
                "rocksdb at {}: background error count increased from {prev} to {new}; \
                 check the LOG file in that directory for details",
                self.path,
            );
        }
    }

    /// Runs one state mutation under the lock. It mirrors the result into the
    /// lock-free gate and logs it. Returns true when the caller should log the
    /// underlying cause: the mutation either started a fresh incident or
    /// latched the gate.
    fn update(&self, change: impl FnOnce(&mut State) -> Option<Transition>) -> bool {
        let (started_incident, latched, log) = {
            let mut state = self.state.lock();
            let was_healthy = state.unhealthy_since.is_none();
            let transition = change(&mut state);
            let started_incident = was_healthy && state.unhealthy_since.is_some();
            let latched = matches!(transition, Some(Transition::Latched { .. }));
            // Store the mirrored flag and snapshot the log fields while
            // still holding the lock. Otherwise a clear could overwrite the
            // immediate latch from a concurrent fatal error, or the snapshot
            // could observe a later state than the transition it describes.
            let log = self.publish(&state, transition);
            (started_incident, latched, log)
        };
        // Emit the log after releasing the lock: `tracing` dispatch is
        // synchronous and can block on a slow sink, and holding the lock
        // across that would serialize every concurrent error reporter
        // behind it.
        if let Some(log) = log {
            self.log_transition(log);
        }
        started_incident || latched
    }

    /// Mirrors the latched flag into the lock-free `active` gate that the
    /// hot write path reads. Returns the fields of a one-shot transition log
    /// for a latch or clear, or `None` if `transition` is `None`.
    fn publish(&self, state: &State, transition: Option<Transition>) -> Option<LogTransition> {
        self.active
            .store(state.latched_at.is_some(), Ordering::Relaxed);
        match transition {
            Some(Transition::Latched { fatal }) => Some(LogTransition::Latched {
                fatal,
                foreground_errors: state
                    .foreground_errors
                    .saturating_sub(state.foreground_baseline),
                background_errors: state
                    .background_errors
                    .saturating_sub(state.background_baseline),
                latch_delay: if fatal {
                    Duration::ZERO
                } else {
                    self.policy.latch_duration
                },
            }),
            Some(Transition::Cleared) => Some(LogTransition::Cleared {
                unlatch_duration: self.policy.unlatch_duration,
            }),
            None => None,
        }
    }

    /// Emits the one-shot transition log built by `publish`.
    fn log_transition(&self, log: LogTransition) {
        match log {
            LogTransition::Latched {
                fatal,
                foreground_errors,
                background_errors,
                latch_delay,
            } => tracing::error!(
                "rocksdb at {}: load-shedding gate latched; fatal={fatal}, \
                 foreground errors={foreground_errors}, \
                 background errors={background_errors}, latch delay={latch_delay:?}. \
                 Ingress paths will reject traffic. Inspect the LOG file \
                 for the underlying cause.",
                self.path,
            ),
            LogTransition::Cleared { unlatch_duration } => tracing::info!(
                "rocksdb at {}: load-shedding gate cleared after {:?} \
                 latched and without newly observed errors; ingress may retry \
                 the database. Database recovery has not been verified.",
                self.path,
                unlatch_duration,
            ),
        }
    }
}

/// The fields of a one-shot transition log, snapshotted from `State` under
/// the lock.
enum LogTransition {
    Latched {
        fatal: bool,
        foreground_errors: u64,
        background_errors: u64,
        latch_delay: Duration,
    },
    Cleared {
        unlatch_duration: Duration,
    },
}

/// A change in the open/closed state of the gate that the caller should log
/// once.
#[derive(Debug, PartialEq, Eq)]
enum Transition {
    /// The gate closed to writes. `fatal` distinguishes the causes: a
    /// Corruption or IOError that latched immediately, versus a milder signal
    /// that latched only after `latch_duration` elapsed. It does not mean the
    /// gate is permanent. A fatal latch can still reopen automatically.
    Latched { fatal: bool },
    /// The gate reopened to writes after the quiet interval.
    Cleared,
}

/// Per-incident error tracking. Separates the errors of the current incident
/// from counts retained from earlier incidents, and holds the timing that the
/// latch and reopen decisions are made from.
struct State {
    /// Background-error count captured when the current incident's baseline
    /// was set (construction or the last clear). Only growth above this
    /// counts as an error in this incident.
    background_baseline: u64,
    /// Foreground-error count captured at the same point. Only growth above
    /// it counts as a foreground error in this incident.
    foreground_baseline: u64,
    /// Highest background-error count seen so far. A failed property read
    /// samples as zero. Keeping the maximum prevents that dip from looking like
    /// the count moved backwards.
    background_errors: u64,
    /// Running count of foreground errors reported to this tracker.
    foreground_errors: u64,
    /// When the first error of the current incident was seen. None while
    /// healthy. Drives the latch delay and is cleared on reopen.
    unhealthy_since: Option<Instant>,
    /// When the most recent error in the current incident was seen. The
    /// reopen interval is measured from this (or the latch time, if later).
    last_error_at: Option<Instant>,
    /// When the gate latched. None while it is open.
    latched_at: Option<Instant>,
}

impl State {
    fn new(background_errors: u64) -> Self {
        Self {
            background_baseline: background_errors,
            foreground_baseline: 0,
            background_errors,
            foreground_errors: 0,
            unhealthy_since: None,
            last_error_at: None,
            latched_at: None,
        }
    }

    /// Records that a fresh error occurred now: opens the incident if this
    /// is its first error, and advances the last-error timestamp that the
    /// reopen interval is measured from.
    fn record_error(&mut self, now: Instant) {
        self.unhealthy_since.get_or_insert(now);
        self.last_error_at = Some(now);
    }

    /// Reports a foreground (load/store/remove/enumerate) error. A fatal class
    /// latches the gate immediately. Anything else only starts (or extends) the
    /// incident and leaves latching to the delayed path in `tick`. Returns the
    /// latch transition when this call caused it.
    fn foreground_error(&mut self, now: Instant, fatal: bool) -> Option<Transition> {
        self.foreground_errors += 1;
        self.record_error(now);
        if fatal && self.latched_at.is_none() {
            self.latched_at = Some(now);
            return Some(Transition::Latched { fatal: true });
        }
        None
    }

    /// Advances the state machine on a periodic monitor sample: folds in the
    /// latest background-error count, then applies whichever timer is due --
    /// reopen if latched and quiet long enough, otherwise latch if the
    /// incident has run past `latch_duration`. Returns the resulting
    /// transition, if any.
    fn tick(&mut self, now: Instant, background_errors: u64, policy: Policy) -> Option<Transition> {
        if background_errors > self.background_errors {
            self.record_error(now);
        }
        self.background_errors = self.background_errors.max(background_errors);

        if let Some(latched_at) = self.latched_at {
            let quiet_since = self.last_error_at.unwrap_or(latched_at).max(latched_at);
            if policy.allow_unlatch && now.duration_since(quiet_since) >= policy.unlatch_duration {
                self.latched_at = None;
                self.unhealthy_since = None;
                self.last_error_at = None;
                self.background_baseline = self.background_errors;
                self.foreground_baseline = self.foreground_errors;
                return Some(Transition::Cleared);
            }
        } else if let Some(since) = self.unhealthy_since {
            // Latch even for an isolated error, since a stuck database can stop
            // reporting errors entirely. RocksDB can pause background
            // scheduling after a paranoid-check failure, dropping
            // compaction-pending and is-write-stopped to zero. Use error
            // history to cover that case.
            if now.duration_since(since) >= policy.latch_duration {
                self.latched_at = Some(now);
                return Some(Transition::Latched { fatal: false });
            }
        }
        None
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use k9::assert_equal;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn policy() -> Policy {
        Policy {
            latch_duration: Duration::from_secs(15),
            unlatch_duration: Duration::from_secs(30),
            allow_unlatch: true,
        }
    }

    #[test]
    fn validate_rejects_zero_unlatch_only_when_unlatching_enabled() {
        let mut p = policy();
        p.unlatch_duration = Duration::ZERO;
        p.allow_unlatch = true;
        assert_equal!(
            format!("{:#}", p.validate().unwrap_err()),
            "error_unlatch_duration must be greater than zero when allow_error_unlatch is true"
        );
        p.allow_unlatch = false;
        p.validate().unwrap();
    }

    /// `State::new` takes a starting background-error count as its baseline,
    /// and only growth above it opens an incident. This checks the non-growth
    /// cases against a nonzero baseline: re-observing the baseline, and a dip
    /// to zero from a failed property read, neither of which opens an incident.
    /// `RocksSpool::new` always baselines at zero in practice, but `State`
    /// itself accepts any starting count.
    #[test]
    fn nonzero_baseline_does_not_start_an_incident_on_its_own() {
        let now = Instant::now();
        let mut state = State::new(7);
        for (seconds, bg) in [(0, 7), (60, 0), (120, 7)] {
            assert_equal!(
                state.tick(now + Duration::from_secs(seconds), bg, policy()),
                None
            );
            assert_equal!(state.unhealthy_since, None);
            assert_equal!(state.latched_at, None);
        }
        assert_equal!(state.background_errors, 7);
    }

    #[test]
    fn zero_latch_delay() {
        let mut policy = policy();
        policy.latch_duration = Duration::ZERO;
        let now = Instant::now();
        let mut state = State::new(0);
        assert_equal!(
            state.tick(now, 1, policy),
            Some(Transition::Latched { fatal: false })
        );
        assert_equal!(state.tick(now, 1, policy), None);
    }

    /// One error of one signal (foreground or background, with the other
    /// silent) latches after `latch_duration` and reopens after the quiet
    /// interval. Running the cycle three times, an hour apart, checks that each
    /// incident is measured on its own and gets a fresh delay rather than
    /// inheriting a stale timestamp from the previous one.
    #[test]
    fn isolated_errors_and_repeated_incidents() {
        for foreground in [false, true] {
            let mut state = State::new(7);
            let start = Instant::now();
            let mut bg = 7;
            assert_equal!(state.tick(start, bg, policy()), None);
            for incident in 0..3 {
                let now = start + Duration::from_secs(incident * 3600);
                if foreground {
                    assert_equal!(state.foreground_error(now, false), None);
                } else {
                    bg += 1;
                }
                assert_equal!(state.tick(now, bg, policy()), None);
                assert_equal!(
                    state.tick(now + Duration::from_secs(14), bg, policy()),
                    None
                );
                assert_equal!(
                    state.tick(now + Duration::from_secs(15), bg, policy()),
                    Some(Transition::Latched { fatal: false })
                );
                assert_equal!(
                    state.tick(now + Duration::from_secs(44), bg, policy()),
                    None
                );
                assert_equal!(
                    state.tick(now + Duration::from_secs(45), bg, policy()),
                    Some(Transition::Cleared)
                );
                assert_equal!(
                    state.tick(now + Duration::from_secs(3599), bg, policy()),
                    None
                );
                assert_equal!(state.latched_at, None);
            }
            assert_equal!(state.foreground_errors, if foreground { 3 } else { 0 });
        }
    }

    #[test]
    fn either_signal_extends_quiet_period() {
        for foreground_last in [false, true] {
            let now = Instant::now();
            let mut state = State::new(0);
            assert_equal!(
                state.foreground_error(now, true),
                Some(Transition::Latched { fatal: true })
            );
            assert_equal!(state.tick(now + Duration::from_secs(10), 1, policy()), None);
            if foreground_last {
                assert_equal!(
                    state.foreground_error(now + Duration::from_secs(20), false),
                    None
                );
            }
            let quiet_end = if foreground_last { 50 } else { 40 };
            assert_equal!(
                state.tick(now + Duration::from_secs(quiet_end - 1), 1, policy()),
                None
            );
            assert_equal!(
                state.tick(now + Duration::from_secs(quiet_end), 1, policy()),
                Some(Transition::Cleared)
            );
        }
    }

    #[test]
    fn disabled_unlatch() {
        let mut policy = policy();
        policy.allow_unlatch = false;
        let now = Instant::now();
        let mut state = State::new(0);
        assert_equal!(
            state.foreground_error(now, true),
            Some(Transition::Latched { fatal: true })
        );
        assert_equal!(state.tick(now + Duration::from_secs(3600), 0, policy), None);
        assert_equal!(state.latched_at, Some(now));
    }

    #[test]
    fn short_unlatch_waits_from_latch() {
        let mut policy = policy();
        policy.unlatch_duration = Duration::from_secs(1);
        let now = Instant::now();
        let mut state = State::new(0);
        assert_equal!(state.tick(now, 1, policy), None);
        assert_equal!(
            state.tick(now + Duration::from_secs(15), 1, policy),
            Some(Transition::Latched { fatal: false })
        );
        assert_equal!(state.tick(now + Duration::from_secs(15), 1, policy), None);
        assert_equal!(
            state.tick(now + Duration::from_secs(16), 1, policy),
            Some(Transition::Cleared)
        );
    }

    #[test]
    fn foreground_diagnostics_only_on_transitions() {
        let health = Health::new(0, policy(), "test".to_string());
        assert_equal!(health.record_foreground_error(false), true);
        for _ in 0..100 {
            assert_equal!(health.record_foreground_error(false), false);
        }
        assert_equal!(health.record_foreground_error(true), true);
        for _ in 0..100 {
            assert_equal!(health.record_foreground_error(true), false);
        }
        assert_equal!(health.state.lock().foreground_errors, 202);
        let retry_at = Instant::now() + policy().unlatch_duration;
        assert_equal!(
            health.update(|state| state.tick(retry_at, 0, policy())),
            false
        );
        assert_equal!(health.is_active(), false);
        assert_equal!(
            health.update(|state| state.foreground_error(retry_at, false)),
            true
        );
    }

    #[test]
    fn fatal_error_and_clear_are_serialized() {
        for _ in 0..100 {
            let health = Arc::new(Health::new(0, policy(), "test".to_string()));
            let start = Instant::now();
            let retry_at = start + Duration::from_secs(60);
            health.update(|state| state.foreground_error(start, true));
            let barrier = Arc::new(Barrier::new(2));
            thread::scope(|scope| {
                scope.spawn(|| {
                    barrier.wait();
                    health.update(|state| state.foreground_error(retry_at, true));
                });
                barrier.wait();
                health.update(|state| state.tick(retry_at, 0, health.policy));
            });
            assert_equal!(health.is_active(), true);
            assert_equal!(health.state.lock().foreground_errors, 2);
        }
    }
}
