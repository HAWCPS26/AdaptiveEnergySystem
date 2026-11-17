use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::{StateError, TickSnapshot, TickState};

use super::config::duration_from_seconds;

#[derive(Clone)]
pub(super) struct SharedState {
    state: Arc<Mutex<TickState>>,
    changes: watch::Sender<TickSnapshot>,
}

impl SharedState {
    /// Wrap the simulation state with change notifications.
    pub(super) fn new(state: TickState) -> Self {
        let state = Arc::new(Mutex::new(state));
        let snapshot = state.lock().snapshot();
        let (changes, _) = watch::channel(snapshot);
        Self { state, changes }
    }

    pub(super) fn snapshot(&self) -> TickSnapshot {
        self.state.lock().snapshot()
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<TickSnapshot> {
        self.changes.subscribe()
    }

    pub(super) fn start(&self, ticks: u64) -> Result<(), StateError> {
        self.mutate(|state| state.start(ticks))
    }

    pub(super) fn pause(&self) -> Result<(), StateError> {
        self.mutate(|state| state.pause())
    }

    pub(super) fn resume(&self) -> Result<(), StateError> {
        self.mutate(|state| state.resume())
    }

    pub(super) fn reset(&self, start_time: DateTime<Utc>) {
        self.mutate(|state| {
            state.reset(start_time);
            Ok(())
        })
        .expect("reset cannot fail");
    }

    pub(super) fn set_tick_interval(&self, seconds: f64) -> Result<(), StateError> {
        let interval =
            duration_from_seconds(seconds).map_err(|_| StateError::InvalidTickInterval)?;
        self.mutate(|state| state.set_tick_interval(interval))
    }

    pub(super) fn advance(&self) {
        self.mutate(|state| {
            state.advance();
            Ok(())
        })
        .expect("advancing cannot fail");
    }

    fn mutate(
        &self,
        operation: impl FnOnce(&mut TickState) -> Result<(), StateError>,
    ) -> Result<(), StateError> {
        let snapshot = {
            let mut state = self.state.lock();
            operation(&mut state)?;
            state.snapshot()
        };
        self.changes.send_replace(snapshot);
        Ok(())
    }

    /// Return milliseconds since the latest emitted tick.
    pub(super) fn time_since_last_tick_ms(&self) -> f64 {
        self.snapshot()
            .time_since_last_tick
            .map_or(0.0, |duration| duration.as_secs_f64() * 1_000.0)
    }
}
