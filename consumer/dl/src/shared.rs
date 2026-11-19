use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::model::{DeferrableLoad, LoadError, LoadSnapshot};

/// Thread-safe washing-machine state with change notifications.
#[derive(Clone)]
pub struct SharedLoad {
    state: Arc<Mutex<DeferrableLoad>>,
    changes: watch::Sender<LoadSnapshot>,
}

impl SharedLoad {
    pub fn new(load: DeferrableLoad) -> Self {
        let state = Arc::new(Mutex::new(load));
        let snapshot = state.lock().snapshot();
        let (changes, _) = watch::channel(snapshot);
        Self { state, changes }
    }

    pub fn snapshot(&self) -> LoadSnapshot {
        self.state.lock().snapshot()
    }

    pub fn subscribe(&self) -> watch::Receiver<LoadSnapshot> {
        self.changes.subscribe()
    }

    pub fn set_deadline(&self, deadline: DateTime<Utc>) -> Result<(), LoadError> {
        self.mutate(|load| {
            load.set_deadline(deadline);
            Ok(())
        })
    }

    pub fn set_tickgen_connected(&self, connected: bool) {
        self.mutate(|load| {
            load.set_tickgen_connected(connected);
            Ok(())
        })
        .expect("setting the connection flag cannot fail");
    }

    pub fn start(&self) -> Result<(), LoadError> {
        self.mutate(DeferrableLoad::start)
    }

    pub fn pause(&self) -> Result<(), LoadError> {
        self.mutate(DeferrableLoad::pause)
    }

    pub fn resume(&self) -> Result<(), LoadError> {
        self.mutate(DeferrableLoad::resume)
    }

    pub fn stop(&self) -> Result<(), LoadError> {
        self.mutate(DeferrableLoad::stop)
    }

    pub fn on_tick(&self, now: DateTime<Utc>, tick_minutes: u32) {
        self.mutate(|load| {
            load.on_tick(now, tick_minutes);
            Ok(())
        })
        .expect("advancing a wash cycle tick cannot fail");
    }

    fn mutate(
        &self,
        operation: impl FnOnce(&mut DeferrableLoad) -> Result<(), LoadError>,
    ) -> Result<(), LoadError> {
        let snapshot = {
            let mut load = self.state.lock();
            operation(&mut load)?;
            load.snapshot()
        };
        self.changes.send_replace(snapshot);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use crate::LoadState;

    use super::*;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, hour, minute, 0).unwrap()
    }

    #[test]
    fn exposes_control_over_the_shared_state() {
        let shared = SharedLoad::new(DeferrableLoad::new());

        assert_eq!(shared.start(), Err(LoadError::NoDeadline));
        shared.set_deadline(at(10, 0)).unwrap();
        shared.start().unwrap();
        assert_eq!(shared.snapshot().state, LoadState::Running);

        shared.pause().unwrap();
        assert_eq!(shared.snapshot().state, LoadState::Paused);

        shared.resume().unwrap();
        assert_eq!(shared.snapshot().state, LoadState::Running);

        shared.stop().unwrap();
        assert_eq!(shared.snapshot().state, LoadState::Stopped);
    }
}
