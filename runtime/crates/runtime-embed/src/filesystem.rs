use crate::RuntimeError;
use runtime_event_loop::CompletionNotifier;
use runtime_event_loop::worker::{WorkerCompletion, WorkerError};
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct FilesystemCompletions {
    pending_signal: Mutex<bool>,
    changed: Condvar,
    results: Mutex<HashMap<u64, Result<Vec<u8>, WorkerError>>>,
    drain: Mutex<()>,
}

impl CompletionNotifier for FilesystemCompletions {
    fn notify_drain_needed(&self) {
        if let Ok(mut pending) = self.pending_signal.lock() {
            *pending = true;
            self.changed.notify_all();
        }
    }
}

impl FilesystemCompletions {
    pub(crate) fn store(&self, completion: WorkerCompletion) -> Result<(), RuntimeError> {
        self.results
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .insert(completion.request_id, completion.result);
        Ok(())
    }

    pub(crate) fn take(
        &self,
        request_id: u64,
    ) -> Result<Option<Result<Vec<u8>, WorkerError>>, RuntimeError> {
        Ok(self
            .results
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .remove(&request_id))
    }

    pub(crate) fn wait_signal(&self) -> Result<(), RuntimeError> {
        let pending = self
            .pending_signal
            .lock()
            .map_err(|_| RuntimeError::Internal)?;
        let (mut pending, _) = self
            .changed
            .wait_timeout_while(pending, Duration::from_secs(1), |pending| !*pending)
            .map_err(|_| RuntimeError::Internal)?;
        *pending = false;
        Ok(())
    }

    pub(crate) fn drain_guard(&self) -> Result<std::sync::MutexGuard<'_, ()>, RuntimeError> {
        self.drain.lock().map_err(|_| RuntimeError::Internal)
    }
}
