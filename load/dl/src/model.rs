use chrono::{DateTime, Duration, Utc};
use thiserror::Error;

use crate::profile;

const COMPLETION_EPSILON_MINUTES: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadState {
    Idle,
    Running,
    Paused,
    Done,
    Stopped,
}

impl LoadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Done => "Done",
            Self::Stopped => "Stopped",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadSnapshot {
    pub state: LoadState,
    pub phase: &'static str,
    pub deadline: Option<DateTime<Utc>>,
    pub latest_start: Option<DateTime<Utc>>,
    pub deadline_met: bool,
    pub cycle_duration_minutes: u32,
    pub cycle_elapsed_minutes: f64,
    pub cycle_remaining_minutes: f64,
    pub progress_percent: f64,
    pub current_power_kw: f64,
    pub energy_delivered_kwh: f64,
    pub cycle_energy_kwh: f64,
    pub simulation_time: Option<DateTime<Utc>>,
    pub tickgen_connected: bool,
}

/// An externally controlled washing machine.
///
/// It never decides when to run. A controller sets `deadline`, then calls
/// `start`, and may `pause`, `resume` or `stop` it over OPC UA.
#[derive(Debug)]
pub struct DeferrableLoad {
    state: LoadState,
    deadline: Option<DateTime<Utc>>,
    cycle_elapsed_minutes: f64,
    energy_delivered_kwh: f64,
    current_power_kw: f64,
    last_phase: &'static str,
    completed_at: Option<DateTime<Utc>>,
    deadline_met: bool,
    last_simulation_time: Option<DateTime<Utc>>,
    tickgen_connected: bool,
}

impl Default for DeferrableLoad {
    fn default() -> Self {
        Self::new()
    }
}

impl DeferrableLoad {
    pub fn new() -> Self {
        Self {
            state: LoadState::Idle,
            deadline: None,
            cycle_elapsed_minutes: 0.0,
            energy_delivered_kwh: 0.0,
            current_power_kw: 0.0,
            last_phase: profile::PHASES[0].name,
            completed_at: None,
            deadline_met: false,
            last_simulation_time: None,
            tickgen_connected: false,
        }
    }

    pub fn snapshot(&self) -> LoadSnapshot {
        let duration = f64::from(profile::CYCLE_DURATION_MINUTES);
        LoadSnapshot {
            state: self.state,
            phase: self.phase_name(),
            deadline: self.deadline,
            latest_start: self
                .deadline
                .map(|deadline| deadline - self.cycle_duration()),
            deadline_met: self.deadline_met,
            cycle_duration_minutes: profile::CYCLE_DURATION_MINUTES,
            cycle_elapsed_minutes: self.cycle_elapsed_minutes,
            cycle_remaining_minutes: (duration - self.cycle_elapsed_minutes).max(0.0),
            progress_percent: (self.cycle_elapsed_minutes / duration * 100.0).clamp(0.0, 100.0),
            current_power_kw: self.current_power_kw,
            energy_delivered_kwh: self.energy_delivered_kwh,
            cycle_energy_kwh: profile::cycle_energy_kwh(),
            simulation_time: self.last_simulation_time,
            tickgen_connected: self.tickgen_connected,
        }
    }

    pub fn state(&self) -> LoadState {
        self.state
    }

    pub fn last_simulation_time(&self) -> Option<DateTime<Utc>> {
        self.last_simulation_time
    }

    /// Record whether the TickGen subscription is currently connected.
    pub fn set_tickgen_connected(&mut self, connected: bool) {
        self.tickgen_connected = connected;
    }

    /// Set the simulation time by which the cycle must finish.
    ///
    /// Changing the deadline after completion re-evaluates `deadline_met`.
    pub fn set_deadline(&mut self, deadline: DateTime<Utc>) {
        self.deadline = Some(deadline);
        self.refresh_deadline_met();
    }

    /// Start a new cycle. A deadline must be configured first.
    pub fn start(&mut self) -> Result<(), LoadError> {
        if matches!(self.state, LoadState::Running | LoadState::Paused) {
            return Err(LoadError::AlreadyRunning);
        }
        if self.deadline.is_none() {
            return Err(LoadError::NoDeadline);
        }

        self.state = LoadState::Running;
        self.cycle_elapsed_minutes = 0.0;
        self.energy_delivered_kwh = 0.0;
        self.current_power_kw = 0.0;
        self.last_phase = profile::PHASES[0].name;
        self.completed_at = None;
        self.deadline_met = false;
        Ok(())
    }

    /// Pause a running cycle without losing progress.
    pub fn pause(&mut self) -> Result<(), LoadError> {
        if self.state != LoadState::Running {
            return Err(LoadError::NotRunning);
        }
        self.state = LoadState::Paused;
        self.current_power_kw = 0.0;
        Ok(())
    }

    /// Resume a paused cycle.
    pub fn resume(&mut self) -> Result<(), LoadError> {
        if self.state != LoadState::Paused {
            return Err(LoadError::NotPaused);
        }
        self.state = LoadState::Running;
        Ok(())
    }

    /// Stop a running or paused cycle and discard its progress.
    pub fn stop(&mut self) -> Result<(), LoadError> {
        if !matches!(self.state, LoadState::Running | LoadState::Paused) {
            return Err(LoadError::NotRunning);
        }
        self.state = LoadState::Stopped;
        self.cycle_elapsed_minutes = 0.0;
        self.energy_delivered_kwh = 0.0;
        self.current_power_kw = 0.0;
        self.last_phase = profile::PHASES[0].name;
        self.completed_at = None;
        self.deadline_met = false;
        Ok(())
    }

    /// Advance the appliance by one simulation tick.
    pub fn on_tick(&mut self, now: DateTime<Utc>, tick_minutes: u32) {
        self.last_simulation_time = Some(now);

        if self.state != LoadState::Running {
            self.current_power_kw = 0.0;
            return;
        }

        let duration = f64::from(profile::CYCLE_DURATION_MINUTES);
        let advance = f64::from(tick_minutes).min((duration - self.cycle_elapsed_minutes).max(0.0));
        if advance <= 0.0 {
            self.current_power_kw = 0.0;
            return;
        }

        let tick = profile::tick(self.cycle_elapsed_minutes, advance);
        self.cycle_elapsed_minutes += advance;
        self.energy_delivered_kwh += tick.energy_kwh;
        self.current_power_kw = tick.average_power_kw;
        self.last_phase = tick.phase;

        if self.cycle_elapsed_minutes >= duration - COMPLETION_EPSILON_MINUTES {
            self.cycle_elapsed_minutes = duration;
            self.state = LoadState::Done;
            self.current_power_kw = 0.0;
            let completed_at = now + Duration::seconds((advance * 60.0).round() as i64);
            self.completed_at = Some(completed_at);
            self.refresh_deadline_met();
        }
    }

    fn refresh_deadline_met(&mut self) {
        self.deadline_met = match (self.state, self.deadline, self.completed_at) {
            (LoadState::Done, Some(deadline), Some(completed_at)) => completed_at <= deadline,
            _ => false,
        };
    }

    fn phase_name(&self) -> &'static str {
        match self.state {
            LoadState::Running | LoadState::Paused => self.last_phase,
            LoadState::Idle => "Idle",
            LoadState::Done => "Done",
            LoadState::Stopped => "Stopped",
        }
    }

    fn cycle_duration(&self) -> Duration {
        Duration::minutes(i64::from(profile::CYCLE_DURATION_MINUTES))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoadError {
    #[error("a cycle is already running or paused")]
    AlreadyRunning,
    #[error("no cycle is running")]
    NotRunning,
    #[error("the cycle is not paused")]
    NotPaused,
    #[error("set a deadline before starting the cycle")]
    NoDeadline,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, hour, minute, 0).unwrap()
    }

    fn load_with_deadline() -> DeferrableLoad {
        let mut load = DeferrableLoad::new();
        load.set_deadline(at(10, 0));
        load
    }

    fn run_six_ticks(load: &mut DeferrableLoad) {
        for tick in 0..6 {
            load.on_tick(at(8, 0) + Duration::minutes(tick * 15), 15);
        }
    }

    #[test]
    fn start_requires_a_deadline() {
        let mut load = DeferrableLoad::new();
        assert_eq!(load.start(), Err(LoadError::NoDeadline));

        load.set_deadline(at(10, 0));
        assert_eq!(load.start(), Ok(()));
        assert_eq!(load.state(), LoadState::Running);
    }

    #[test]
    fn follows_the_wash_profile_and_completes() {
        let mut load = load_with_deadline();
        load.start().unwrap();

        let expected_power = [0.733_333_32, 2.0, 0.2, 0.2, 0.366_666_68];
        for (tick, expected) in expected_power.iter().enumerate() {
            let time = at(8, 0) + Duration::minutes(tick as i64 * 15);
            load.on_tick(time, 15);
            let snapshot = load.snapshot();
            assert!(
                (snapshot.current_power_kw - expected).abs() < 1e-5,
                "tick {tick} power was {}",
                snapshot.current_power_kw
            );
        }

        // The sixth tick completes the cycle and power drops back to zero.
        load.on_tick(at(8, 0) + Duration::minutes(75), 15);
        let snapshot = load.snapshot();
        assert_eq!(snapshot.state, LoadState::Done);
        assert_eq!(snapshot.phase, "Done");
        assert!((snapshot.energy_delivered_kwh - 0.9875).abs() < 1e-9);
        assert_eq!(snapshot.cycle_elapsed_minutes, 90.0);
        assert!((snapshot.progress_percent - 100.0).abs() < 1e-9);
        assert_eq!(snapshot.current_power_kw, 0.0);
        assert!(snapshot.deadline_met);
    }

    #[test]
    fn pause_freezes_progress_and_resume_continues() {
        let mut load = load_with_deadline();
        load.start().unwrap();
        load.on_tick(at(8, 0), 15);
        let energy_after_first = load.snapshot().energy_delivered_kwh;

        load.pause().unwrap();
        load.on_tick(at(8, 15), 15);
        let paused = load.snapshot();
        assert_eq!(paused.state, LoadState::Paused);
        assert_eq!(paused.energy_delivered_kwh, energy_after_first);
        assert_eq!(paused.cycle_elapsed_minutes, 15.0);

        load.resume().unwrap();
        load.on_tick(at(8, 30), 15);
        let resumed = load.snapshot();
        assert_eq!(resumed.state, LoadState::Running);
        assert!(resumed.energy_delivered_kwh > energy_after_first);
        assert_eq!(resumed.cycle_elapsed_minutes, 30.0);
    }

    #[test]
    fn stop_discards_progress_and_allows_a_new_start() {
        let mut load = load_with_deadline();
        load.start().unwrap();
        load.on_tick(at(8, 0), 15);
        load.stop().unwrap();

        let stopped = load.snapshot();
        assert_eq!(stopped.state, LoadState::Stopped);
        assert_eq!(stopped.phase, "Stopped");
        assert_eq!(stopped.cycle_elapsed_minutes, 0.0);
        assert_eq!(stopped.energy_delivered_kwh, 0.0);

        assert_eq!(load.start(), Ok(()));
        assert_eq!(load.state(), LoadState::Running);
    }

    #[test]
    fn reports_a_missed_deadline() {
        let mut load = DeferrableLoad::new();
        load.set_deadline(at(8, 30));
        load.start().unwrap();
        run_six_ticks(&mut load);

        let snapshot = load.snapshot();
        assert_eq!(snapshot.state, LoadState::Done);
        assert!(!snapshot.deadline_met);
    }

    #[test]
    fn changing_the_deadline_after_completion_rechecks_it() {
        let mut load = DeferrableLoad::new();
        load.set_deadline(at(8, 30));
        load.start().unwrap();
        run_six_ticks(&mut load);
        assert!(!load.snapshot().deadline_met);

        load.set_deadline(at(9, 30));
        assert!(load.snapshot().deadline_met);
    }

    #[test]
    fn exposes_the_latest_start_time() {
        let mut load = DeferrableLoad::new();
        load.set_deadline(at(10, 0));
        assert_eq!(load.snapshot().latest_start, Some(at(8, 30)));
    }

    #[test]
    fn rejects_invalid_control_transitions() {
        let mut load = load_with_deadline();
        assert_eq!(load.pause(), Err(LoadError::NotRunning));
        assert_eq!(load.resume(), Err(LoadError::NotPaused));
        assert_eq!(load.stop(), Err(LoadError::NotRunning));

        load.start().unwrap();
        assert_eq!(load.start(), Err(LoadError::AlreadyRunning));
        assert_eq!(load.resume(), Err(LoadError::NotPaused));

        load.pause().unwrap();
        assert_eq!(load.start(), Err(LoadError::AlreadyRunning));
        assert_eq!(load.pause(), Err(LoadError::NotRunning));
    }
}
