use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use thiserror::Error;

pub const SIMULATED_TICK_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StateError {
    #[error("the tick count must be greater than zero")]
    InvalidTickCount,
    #[error("the tick interval must be finite and greater than zero")]
    InvalidTickInterval,
    #[error("the generator is already running")]
    AlreadyRunning,
    #[error("the generator is not running")]
    NotRunning,
    #[error("there is no paused run to resume")]
    NoActiveRun,
    #[error("a run is already configured; resume it or reset the generator")]
    RunAlreadyConfigured,
    #[error("the tick counter overflowed")]
    TickOverflow,
}

/// A consistent snapshot of the generator state.
#[derive(Debug, Clone)]
pub struct TickSnapshot {
    /// Zero means that no tick has been emitted. Emitted ticks are numbered from one.
    pub tick: u64,
    pub simulation_time: DateTime<Utc>,
    pub running: bool,
    pub remaining_ticks: u64,
    pub start_time: DateTime<Utc>,
    pub tick_interval: Duration,
    pub time_since_last_tick: Option<Duration>,
}

/// Transport-independent simulation state. Time-based advancement is driven by the server.
#[derive(Debug)]
pub struct TickState {
    tick: u64,
    start_time: DateTime<Utc>,
    running: bool,
    remaining_ticks: u64,
    tick_interval: Duration,
    tick_in_progress: bool,
    last_tick_wall_time: Option<Instant>,
}

impl TickState {
    /// Create a paused generator with no emitted ticks.
    pub fn new(start_time: DateTime<Utc>, tick_interval: Duration) -> Result<Self, StateError> {
        Self::validate_interval(tick_interval)?;
        Ok(Self {
            tick: 0,
            start_time,
            running: false,
            remaining_ticks: 0,
            tick_interval,
            tick_in_progress: false,
            last_tick_wall_time: None,
        })
    }

    /// Capture the current state and elapsed wall-clock time.
    pub fn snapshot(&self) -> TickSnapshot {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&self, now: Instant) -> TickSnapshot {
        TickSnapshot {
            tick: self.tick,
            simulation_time: self.simulation_time(),
            running: self.running,
            remaining_ticks: self.remaining_ticks,
            start_time: self.start_time,
            tick_interval: self.tick_interval,
            time_since_last_tick: self
                .last_tick_wall_time
                .map(|last| now.saturating_duration_since(last)),
        }
    }

    /// Emit the first tick immediately and run for exactly `ticks` ticks.
    pub fn start(&mut self, ticks: u64) -> Result<(), StateError> {
        self.start_at(ticks, Instant::now())
    }

    fn start_at(&mut self, ticks: u64, now: Instant) -> Result<(), StateError> {
        if ticks == 0 {
            return Err(StateError::InvalidTickCount);
        }
        if self.running {
            return Err(StateError::AlreadyRunning);
        }
        if self.tick_in_progress {
            return Err(StateError::RunAlreadyConfigured);
        }

        self.tick = self.tick.checked_add(1).ok_or(StateError::TickOverflow)?;
        self.remaining_ticks = ticks;
        self.running = true;
        self.tick_in_progress = true;
        self.last_tick_wall_time = Some(now);
        Ok(())
    }

    /// Pause before the next tick without changing the run.
    pub fn pause(&mut self) -> Result<(), StateError> {
        if !self.running {
            return Err(StateError::NotRunning);
        }
        self.running = false;
        Ok(())
    }

    /// Resume a paused run without emitting an extra tick.
    pub fn resume(&mut self) -> Result<(), StateError> {
        if self.running {
            return Err(StateError::AlreadyRunning);
        }
        if !self.tick_in_progress || self.remaining_ticks == 0 {
            return Err(StateError::NoActiveRun);
        }
        self.running = true;
        Ok(())
    }

    /// Stop the run and reset the tick counter and simulation epoch.
    pub fn reset(&mut self, start_time: DateTime<Utc>) {
        self.tick = 0;
        self.start_time = start_time;
        self.running = false;
        self.remaining_ticks = 0;
        self.tick_in_progress = false;
        self.last_tick_wall_time = None;
    }

    /// Set the wall-clock duration between ticks.
    pub fn set_tick_interval(&mut self, interval: Duration) -> Result<(), StateError> {
        Self::validate_interval(interval)?;
        self.tick_interval = interval;
        Ok(())
    }

    /// Return whether ticks are currently being generated.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Emit the next tick, or complete the run after the final tick interval.
    /// Returns whether the state changed.
    pub fn advance(&mut self) -> bool {
        self.advance_at(Instant::now())
    }

    fn advance_at(&mut self, now: Instant) -> bool {
        if !self.running {
            return false;
        }

        if self.remaining_ticks > 1 {
            self.remaining_ticks -= 1;
            self.tick += 1;
            self.last_tick_wall_time = Some(now);
        } else {
            self.remaining_ticks = 0;
            self.running = false;
            self.tick_in_progress = false;
        }
        true
    }

    fn simulation_time(&self) -> DateTime<Utc> {
        let completed_intervals = self.tick.saturating_sub(1) as i64;
        let minutes = completed_intervals.saturating_mul(SIMULATED_TICK_MINUTES);
        self.start_time + chrono::Duration::minutes(minutes)
    }

    fn validate_interval(interval: Duration) -> Result<(), StateError> {
        if interval.is_zero() {
            Err(StateError::InvalidTickInterval)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn start_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn state() -> TickState {
        TickState::new(start_time(), Duration::from_secs(9)).unwrap()
    }

    #[test]
    fn starts_paused_without_an_emitted_tick() {
        let snapshot = state().snapshot();

        assert_eq!(snapshot.tick, 0);
        assert!(!snapshot.running);
        assert_eq!(snapshot.remaining_ticks, 0);
        assert_eq!(snapshot.simulation_time, start_time());
        assert_eq!(snapshot.time_since_last_tick, None);
    }

    #[test]
    fn emits_ticks_and_advances_simulated_time() {
        let mut state = state();
        let now = Instant::now();
        state.start_at(2, now).unwrap();

        let snapshot = state.snapshot_at(now);
        assert_eq!(snapshot.tick, 1);
        assert_eq!(snapshot.remaining_ticks, 2);
        assert_eq!(snapshot.simulation_time, start_time());
        assert_eq!(snapshot.time_since_last_tick, Some(Duration::ZERO));

        assert!(state.advance_at(now + Duration::from_secs(9)));
        let snapshot = state.snapshot_at(now + Duration::from_secs(10));
        assert_eq!(snapshot.tick, 2);
        assert_eq!(snapshot.remaining_ticks, 1);
        assert_eq!(
            snapshot.simulation_time,
            start_time() + chrono::Duration::minutes(15)
        );
        assert_eq!(snapshot.time_since_last_tick, Some(Duration::from_secs(1)));
    }

    #[test]
    fn finite_run_stops_after_the_final_tick_interval() {
        let mut state = state();
        state.start(1).unwrap();
        state.advance();

        let snapshot = state.snapshot();
        assert!(!snapshot.running);
        assert_eq!(snapshot.tick, 1);
        assert_eq!(snapshot.remaining_ticks, 0);
        assert!(!state.advance());
    }

    #[test]
    fn a_new_run_continues_with_the_next_tick() {
        let mut state = state();
        state.start(1).unwrap();
        state.advance();
        state.start(1).unwrap();

        assert_eq!(state.tick, 2);
        assert!(state.running);
        assert_eq!(state.remaining_ticks, 1);
    }

    #[test]
    fn pause_and_resume_do_not_emit_an_extra_tick() {
        let mut state = state();
        state.start(2).unwrap();
        state.pause().unwrap();

        assert!(!state.advance());
        state.resume().unwrap();
        assert_eq!(state.tick, 1);
        assert!(state.running);
    }

    #[test]
    fn reset_clears_run_and_elapsed_time() {
        let mut state = state();
        state.start(1).unwrap();
        let new_start = start_time() + chrono::Duration::days(1);
        state.reset(new_start);

        let snapshot = state.snapshot();
        assert_eq!(snapshot.tick, 0);
        assert_eq!(snapshot.start_time, new_start);
        assert_eq!(snapshot.simulation_time, new_start);
        assert_eq!(snapshot.time_since_last_tick, None);
        assert!(!snapshot.running);
    }

    #[test]
    fn rejects_invalid_control_operations() {
        let mut state = state();
        assert_eq!(state.start(0), Err(StateError::InvalidTickCount));
        assert_eq!(state.resume(), Err(StateError::NoActiveRun));
        assert_eq!(state.pause(), Err(StateError::NotRunning));
        assert_eq!(
            state.set_tick_interval(Duration::ZERO),
            Err(StateError::InvalidTickInterval)
        );

        state.start(1).unwrap();
        assert_eq!(state.start(1), Err(StateError::AlreadyRunning));
        state.pause().unwrap();
        assert_eq!(state.start(1), Err(StateError::RunAlreadyConfigured));
    }

    #[test]
    fn tick_interval_is_not_divided() {
        let state = state();
        assert_eq!(state.snapshot().tick_interval, Duration::from_secs(9));
    }
}
