//! Message-loop-owned JavaScript host with no OS event loop or I/O resources.

#![forbid(unsafe_code)]

pub use runtime_engine::{
    EngineConfiguration, EngineError, EngineEvent, EngineNotifier, EvaluationId, EvaluationOutcome,
    EvaluationRequest, HostCompletion, HostRequest, HostRequestId, JavaScriptEngineBackend,
};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

/// Engine work observed during one bounded host turn.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostTurn {
    pub events: usize,
    pub payload_bytes: usize,
    pub has_more: bool,
}

/// Builder for a message-loop-owned [`ZjsHost`].
pub struct ZjsHostBuilder {
    backend: Box<dyn JavaScriptEngineBackend>,
    configuration: EngineConfiguration,
    notifier: Arc<dyn EngineNotifier>,
}

impl ZjsHostBuilder {
    #[must_use]
    pub fn new(
        backend: Box<dyn JavaScriptEngineBackend>,
        notifier: Arc<dyn EngineNotifier>,
    ) -> Self {
        Self {
            backend,
            configuration: EngineConfiguration::default(),
            notifier,
        }
    }

    #[must_use]
    pub const fn configuration(mut self, configuration: EngineConfiguration) -> Self {
        self.configuration = configuration;
        self
    }

    /// Starts the engine and binds it to a new host.
    ///
    /// # Errors
    /// Returns invalid configuration or backend startup failures.
    pub fn build(mut self) -> Result<ZjsHost, EngineError> {
        self.backend
            .start(self.configuration, self.notifier.clone())?;
        Ok(ZjsHost {
            backend: self.backend,
            events: VecDeque::new(),
            deferred: None,
            local_only: PhantomData,
        })
    }
}

/// Owns only the JavaScript engine and its engine-local runtime state.
///
/// `Rc` in the marker keeps this value on its owning message-loop thread.
pub struct ZjsHost {
    backend: Box<dyn JavaScriptEngineBackend>,
    events: VecDeque<EngineEvent>,
    deferred: Option<EngineEvent>,
    local_only: PhantomData<Rc<()>>,
}

impl ZjsHost {
    /// Enters JavaScript to evaluate source and immediately reaches a microtask checkpoint.
    ///
    /// # Errors
    /// Returns an engine lifecycle, quota, or evaluation submission error.
    pub fn evaluate(&mut self, request: EvaluationRequest) -> Result<(), EngineError> {
        self.backend.submit_evaluation(request)?;
        self.perform_microtask_checkpoint()
    }

    /// Resolves or rejects one engine-owned Promise, then drains its microtasks.
    ///
    /// # Errors
    /// Returns an error for an unknown request or failed engine entry.
    pub fn complete_host_request(
        &mut self,
        request_id: HostRequestId,
        completion: HostCompletion,
    ) -> Result<(), EngineError> {
        self.backend.complete_host_request(request_id, completion)?;
        self.perform_microtask_checkpoint()
    }

    /// Runs Promise jobs and `queueMicrotask` in the engine's single logical queue.
    ///
    /// # Errors
    /// Returns a backend checkpoint failure.
    pub fn perform_microtask_checkpoint(&mut self) -> Result<(), EngineError> {
        self.backend.perform_microtask_checkpoint()
    }

    /// Moves a bounded number of already-produced engine events into host state.
    ///
    /// # Errors
    /// Rejects zero limits, quota overflow, or malformed backend events.
    pub fn drain_events(
        &mut self,
        maximum_events: usize,
        maximum_bytes: usize,
    ) -> Result<HostTurn, EngineError> {
        if maximum_events == 0 || maximum_bytes == 0 {
            return Err(EngineError::InvalidConfiguration);
        }
        let mut turn = HostTurn::default();
        while turn.events < maximum_events {
            let event = if self.deferred.is_some() {
                self.deferred.take()
            } else {
                self.backend.next_event()?
            };
            let Some(event) = event else {
                break;
            };
            let bytes = event.payload_len();
            if bytes > maximum_bytes {
                return Err(EngineError::QuotaExceeded);
            }
            if turn.payload_bytes > maximum_bytes - bytes {
                self.deferred = Some(event);
                turn.has_more = true;
                break;
            }
            turn.events += 1;
            turn.payload_bytes += bytes;
            self.events.push_back(event);
        }
        turn.has_more |= turn.events == maximum_events || self.deferred.is_some();
        Ok(turn)
    }

    #[must_use]
    pub fn next_event(&mut self) -> Option<EngineEvent> {
        self.events.pop_front()
    }

    /// Cancels a pending evaluation.
    ///
    /// # Errors
    /// Returns an error for an unknown evaluation or unavailable engine.
    pub fn cancel_evaluation(&mut self, id: EvaluationId) -> Result<(), EngineError> {
        self.backend.cancel_evaluation(id)
    }

    /// Releases all engine-owned state.
    ///
    /// # Errors
    /// Returns a sanitized backend shutdown failure.
    pub fn shutdown(&mut self) -> Result<(), EngineError> {
        self.backend.shutdown()
    }
}

impl Drop for ZjsHost {
    fn drop(&mut self) {
        let _ = self.backend.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct NoopNotifier;
    impl EngineNotifier for NoopNotifier {
        fn notify(&self) {}
    }

    #[derive(Default)]
    struct TraceBackend {
        trace: Arc<Mutex<Vec<&'static str>>>,
    }

    impl JavaScriptEngineBackend for TraceBackend {
        fn start(
            &mut self,
            _: EngineConfiguration,
            _: Arc<dyn EngineNotifier>,
        ) -> Result<(), EngineError> {
            self.trace.lock().unwrap().push("start");
            Ok(())
        }
        fn submit_evaluation(&mut self, _: EvaluationRequest) -> Result<(), EngineError> {
            self.trace.lock().unwrap().push("evaluate");
            Ok(())
        }
        fn next_event(&mut self) -> Result<Option<EngineEvent>, EngineError> {
            Ok(None)
        }
        fn complete_host_request(
            &mut self,
            _: HostRequestId,
            _: HostCompletion,
        ) -> Result<(), EngineError> {
            self.trace.lock().unwrap().push("complete");
            Ok(())
        }
        fn perform_microtask_checkpoint(&mut self) -> Result<(), EngineError> {
            self.trace.lock().unwrap().push("microtasks");
            Ok(())
        }
        fn cancel_evaluation(&mut self, _: EvaluationId) -> Result<(), EngineError> {
            Ok(())
        }
        fn shutdown(&mut self) -> Result<(), EngineError> {
            self.trace.lock().unwrap().push("shutdown");
            Ok(())
        }
    }

    #[test]
    fn javascript_entries_end_with_a_microtask_checkpoint() {
        // Verifies evaluation and Promise completion preserve task/microtask ordering.
        let trace = Arc::new(Mutex::new(Vec::new()));
        let backend = TraceBackend {
            trace: trace.clone(),
        };
        let mut host = ZjsHostBuilder::new(Box::new(backend), Arc::new(NoopNotifier))
            .build()
            .unwrap();
        host.evaluate(EvaluationRequest {
            id: EvaluationId(1),
            source: "1".into(),
        })
        .unwrap();
        host.complete_host_request(HostRequestId(1), HostCompletion::Unit)
            .unwrap();
        assert_eq!(
            *trace.lock().unwrap(),
            vec!["start", "evaluate", "microtasks", "complete", "microtasks"]
        );
    }
}
