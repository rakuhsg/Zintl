//! V8 implementation of the engine-neutral JavaScript backend contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use runtime_engine::{
    EngineConfiguration, EngineError, EngineEvent, EngineNotifier, EngineObjectKind, EvaluationId,
    EvaluationOutcome, EvaluationRequest, HostCompletion, HostErrorCode, HostRequest,
    HostRequestId, JavaScriptEngineBackend, JavaScriptException, MountRequest,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Once};

const MAXIMUM_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAXIMUM_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
const MAXIMUM_MOUNT_URL_BYTES: usize = 4096;
const MAXIMUM_SLEEP_MILLISECONDS: u64 = 86_400_000;

/// A single-thread-owned V8 implementation of [`JavaScriptEngineBackend`].
pub struct V8Backend {
    isolate: Option<v8::OwnedIsolate>,
    context: Option<v8::Global<v8::Context>>,
    evaluate: Option<v8::Global<v8::Function>>,
    state: Rc<RefCell<BackendState>>,
    local_only: PhantomData<Rc<()>>,
}

struct PendingPromise {
    resolve: v8::Global<v8::Function>,
    reject: v8::Global<v8::Function>,
}

struct BackendState {
    configuration: EngineConfiguration,
    notifier: Option<Arc<dyn EngineNotifier>>,
    events: VecDeque<EngineEvent>,
    terminal_events: VecDeque<EngineEvent>,
    queued_event_bytes: usize,
    pending_evaluations: HashSet<u64>,
    cancelled_evaluations: HashSet<u64>,
    pending_promises: HashMap<u64, PendingPromise>,
    next_host_id: u64,
    shutting_down: bool,
}

impl BackendState {
    fn unstarted() -> Self {
        Self {
            configuration: EngineConfiguration::default(),
            notifier: None,
            events: VecDeque::new(),
            terminal_events: VecDeque::new(),
            queued_event_bytes: 0,
            pending_evaluations: HashSet::new(),
            cancelled_evaluations: HashSet::new(),
            pending_promises: HashMap::new(),
            next_host_id: 1,
            shutting_down: false,
        }
    }

    fn enqueue(&mut self, event: EngineEvent) -> bool {
        let bytes = event.payload_len();
        let accepted = self.terminal_events.is_empty()
            && bytes <= self.configuration.maximum_event_bytes
            && self.queued_event_bytes <= self.configuration.maximum_event_bytes - bytes;
        if accepted {
            self.queued_event_bytes += bytes;
            self.events.push_back(event);
            self.notify();
        }
        accepted
    }

    fn settle_evaluation(&mut self, id: u64, mut outcome: EvaluationOutcome) {
        if !self.pending_evaluations.remove(&id) {
            return;
        }
        if self.cancelled_evaluations.remove(&id) {
            outcome = EvaluationOutcome::Cancelled;
        }
        let event = EngineEvent::EvaluationSettled {
            id: EvaluationId(id),
            outcome,
        };
        if self.enqueue(event) {
            return;
        }
        let fallback = EngineEvent::EvaluationSettled {
            id: EvaluationId(id),
            outcome: EvaluationOutcome::Exception(JavaScriptException {
                name: "QuotaExceeded".to_owned(),
                message: "Engine event queue exhausted".to_owned(),
            }),
        };
        if self.terminal_events.len() < self.configuration.maximum_pending_evaluations {
            self.terminal_events.push_back(fallback);
            self.notify();
        }
    }

    fn notify(&self) {
        if let Some(notifier) = &self.notifier {
            notifier.notify();
        }
    }
}

impl V8Backend {
    /// Creates an unstarted V8 backend.
    #[must_use]
    pub fn new() -> Self {
        Self {
            isolate: None,
            context: None,
            evaluate: None,
            state: Rc::new(RefCell::new(BackendState::unstarted())),
            local_only: PhantomData,
        }
    }

    fn enter<R>(
        &mut self,
        operation: impl FnOnce(&mut v8::PinScope<'_, '_>) -> Result<R, EngineError>,
    ) -> Result<R, EngineError> {
        let context = self.context.as_ref().ok_or(EngineError::InvalidState)?;
        let isolate = self.isolate.as_mut().ok_or(EngineError::InvalidState)?;
        v8::scope!(let handle_scope, isolate);
        let context = v8::Local::new(handle_scope, context);
        let scope = &mut v8::ContextScope::new(handle_scope, context);
        operation(scope)
    }
}

impl Default for V8Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaScriptEngineBackend for V8Backend {
    fn start(
        &mut self,
        configuration: EngineConfiguration,
        notifier: Arc<dyn EngineNotifier>,
    ) -> Result<(), EngineError> {
        if self.isolate.is_some() {
            return Err(EngineError::InvalidState);
        }
        validate_configuration(configuration)?;
        initialize_v8();

        self.state = Rc::new(RefCell::new(BackendState {
            configuration,
            notifier: Some(notifier),
            ..BackendState::unstarted()
        }));
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        if !isolate.set_slot(self.state.clone()) {
            return Err(EngineError::Backend);
        }

        let (context_global, evaluate) = {
            v8::scope!(let handle_scope, &mut isolate);
            let context = v8::Context::new(handle_scope, v8::ContextOptions::default());
            let context_global = v8::Global::new(handle_scope, context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            install_callback(scope, "__zintlEvaluationDone", evaluation_done)?;
            install_callback(scope, "__zintlInvoke", invoke)?;
            install_callback(scope, "__zintlSleep", sleep)?;
            install_callback(scope, "__zintlReadFile", read_file)?;
            install_callback(scope, "__zintlMountOperation", mount_operation)?;
            install_callback(scope, "__zintlConsole", console_output)?;

            v8::tc_scope!(let try_catch, scope);
            let source = v8::String::new(try_catch, include_str!("bootstrap.js"))
                .ok_or(EngineError::Backend)?;
            let script =
                v8::Script::compile(try_catch, source, None).ok_or(EngineError::Backend)?;
            let bridge = script.run(try_catch).ok_or(EngineError::Backend)?;
            let bridge =
                v8::Local::<v8::Object>::try_from(bridge).map_err(|_| EngineError::Backend)?;
            let key = v8::String::new(try_catch, "evaluate").ok_or(EngineError::Backend)?;
            let evaluate = bridge
                .get(try_catch, key.into())
                .ok_or(EngineError::Backend)?;
            let evaluate =
                v8::Local::<v8::Function>::try_from(evaluate).map_err(|_| EngineError::Backend)?;
            (context_global, v8::Global::new(try_catch, evaluate))
        };

        self.context = Some(context_global);
        self.evaluate = Some(evaluate);
        self.isolate = Some(isolate);
        Ok(())
    }

    fn submit_evaluation(&mut self, request: EvaluationRequest) -> Result<(), EngineError> {
        let configuration = self.state.borrow().configuration;
        if request.id.0 == 0
            || request.id.0 > MAXIMUM_SAFE_INTEGER
            || request.source.len() > configuration.maximum_source_bytes
        {
            return Err(EngineError::InvalidRequest);
        }
        {
            let mut state = self.state.borrow_mut();
            if state.shutting_down {
                return Err(EngineError::InvalidState);
            }
            if state.pending_evaluations.contains(&request.id.0) {
                return Err(EngineError::InvalidRequest);
            }
            if state.pending_evaluations.len() >= configuration.maximum_pending_evaluations
                || state.terminal_events.len() >= configuration.maximum_pending_evaluations
            {
                return Err(EngineError::QuotaExceeded);
            }
            state.pending_evaluations.insert(request.id.0);
        }

        let evaluate = self
            .evaluate
            .as_ref()
            .ok_or(EngineError::InvalidState)?
            .clone();
        let id = request.id.0;
        let result = self.enter(|scope| {
            let evaluate = v8::Local::new(scope, &evaluate);
            let receiver = v8::undefined(scope).into();
            let id_value = safe_integer_number(scope, id).into();
            let source = v8::String::new(scope, &request.source)
                .ok_or(EngineError::Backend)?
                .into();
            evaluate
                .call(scope, receiver, &[id_value, source])
                .ok_or(EngineError::Backend)?;
            Ok(())
        });
        if result.is_err() {
            self.state.borrow_mut().settle_evaluation(
                id,
                EvaluationOutcome::Exception(JavaScriptException {
                    name: "Error".to_owned(),
                    message: "JavaScript evaluation failed".to_owned(),
                }),
            );
        }
        result
    }

    fn next_event(&mut self) -> Result<Option<EngineEvent>, EngineError> {
        if self.isolate.is_none() {
            return Err(EngineError::InvalidState);
        }
        let mut state = self.state.borrow_mut();
        if let Some(event) = state.events.pop_front() {
            state.queued_event_bytes = state.queued_event_bytes.saturating_sub(event.payload_len());
            return Ok(Some(event));
        }
        Ok(state.terminal_events.pop_front())
    }

    fn complete_host_request(
        &mut self,
        request_id: HostRequestId,
        completion: HostCompletion,
    ) -> Result<(), EngineError> {
        if request_id.0 == 0 {
            return Err(EngineError::InvalidRequest);
        }
        let promise = self
            .state
            .borrow_mut()
            .pending_promises
            .remove(&request_id.0)
            .ok_or(EngineError::InvalidRequest)?;
        self.enter(|scope| complete_promise(scope, &promise, completion))
    }

    fn perform_microtask_checkpoint(&mut self) -> Result<(), EngineError> {
        self.enter(|scope| {
            scope.perform_microtask_checkpoint();
            Ok(())
        })
    }

    fn cancel_evaluation(&mut self, evaluation_id: EvaluationId) -> Result<(), EngineError> {
        let mut state = self.state.borrow_mut();
        if !state.pending_evaluations.contains(&evaluation_id.0) {
            return Err(EngineError::InvalidRequest);
        }
        state.cancelled_evaluations.insert(evaluation_id.0);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), EngineError> {
        let Some(mut isolate) = self.isolate.take() else {
            return Ok(());
        };
        {
            let mut state = self.state.borrow_mut();
            state.shutting_down = true;
            state.pending_promises.clear();
            state.pending_evaluations.clear();
            state.cancelled_evaluations.clear();
            state.notifier = None;
        }
        self.evaluate.take();
        self.context.take();
        isolate.remove_slot::<Rc<RefCell<BackendState>>>();
        drop(isolate);
        Ok(())
    }
}

impl Drop for V8Backend {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn validate_configuration(configuration: EngineConfiguration) -> Result<(), EngineError> {
    if configuration.maximum_pending_evaluations == 0
        || configuration.maximum_pending_host_requests == 0
        || configuration.maximum_source_bytes == 0
        || configuration.maximum_event_bytes == 0
    {
        Err(EngineError::InvalidConfiguration)
    } else {
        Ok(())
    }
}

fn initialize_v8() {
    static INITIALIZE: Once = Once::new();
    INITIALIZE.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

fn install_callback<F>(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
    callback: F,
) -> Result<(), EngineError>
where
    F: v8::MapFnTo<v8::FunctionCallback>,
{
    let function = v8::Function::new(scope, callback).ok_or(EngineError::Backend)?;
    let name = v8::String::new(scope, name).ok_or(EngineError::Backend)?;
    let context = scope.get_current_context();
    if context
        .global(scope)
        .set(scope, name.into(), function.into())
        == Some(true)
    {
        Ok(())
    } else {
        Err(EngineError::Backend)
    }
}

fn callback_state(scope: &v8::PinScope<'_, '_>) -> Option<Rc<RefCell<BackendState>>> {
    scope.get_slot::<Rc<RefCell<BackendState>>>().map(Rc::clone)
}

#[allow(clippy::needless_pass_by_value)]
fn evaluation_done(
    scope: &mut v8::PinScope<'_, '_>,
    arguments: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(id) = exact_u64(scope, arguments.get(0)) else {
        return;
    };
    let succeeded = arguments.get(1).boolean_value(scope);
    let Some(value) = rust_string(scope, arguments.get(2)) else {
        return;
    };
    let outcome = if succeeded {
        EvaluationOutcome::Value(value.into_bytes())
    } else {
        let (name, message) = value.split_once(':').unwrap_or(("Error", &value));
        EvaluationOutcome::Exception(JavaScriptException {
            name: name.trim().to_owned(),
            message: message.trim().to_owned(),
        })
    };
    if let Some(state) = callback_state(scope) {
        state.borrow_mut().settle_evaluation(id, outcome);
    }
}

#[allow(clippy::needless_pass_by_value)]
fn invoke(
    scope: &mut v8::PinScope<'_, '_>,
    arguments: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(name) = rust_string(scope, arguments.get(0)) else {
        reject_callback(
            scope,
            arguments.get(3),
            "Invalid operation input",
            "InvalidRequest",
        );
        return;
    };
    let maximum =
        callback_state(scope).map_or(0, |state| state.borrow().configuration.maximum_event_bytes);
    let Some(input) = byte_array(scope, arguments.get(1), maximum) else {
        reject_callback(
            scope,
            arguments.get(3),
            "Invalid operation input",
            "InvalidRequest",
        );
        return;
    };
    submit_host(
        scope,
        HostRequest::Invoke {
            name,
            version: 1,
            input,
        },
        arguments.get(2),
        arguments.get(3),
    );
}

#[allow(clippy::needless_pass_by_value)]
fn sleep(
    scope: &mut v8::PinScope<'_, '_>,
    arguments: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(milliseconds) = exact_u64(scope, arguments.get(0)) else {
        reject_callback(
            scope,
            arguments.get(2),
            "Invalid timer delay",
            "InvalidRequest",
        );
        return;
    };
    if milliseconds > MAXIMUM_SLEEP_MILLISECONDS {
        reject_callback(
            scope,
            arguments.get(2),
            "Invalid timer delay",
            "InvalidRequest",
        );
        return;
    }
    submit_host(
        scope,
        HostRequest::Sleep {
            nanoseconds: milliseconds * 1_000_000,
        },
        arguments.get(1),
        arguments.get(2),
    );
}

#[allow(clippy::needless_pass_by_value)]
fn read_file(
    scope: &mut v8::PinScope<'_, '_>,
    arguments: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(url) = rust_string(scope, arguments.get(0)) else {
        reject_callback(
            scope,
            arguments.get(3),
            "Invalid mount read request",
            "InvalidRequest",
        );
        return;
    };
    let Some(maximum_bytes) =
        exact_u64(scope, arguments.get(1)).and_then(|value| usize::try_from(value).ok())
    else {
        reject_callback(
            scope,
            arguments.get(3),
            "Invalid mount read request",
            "InvalidRequest",
        );
        return;
    };
    if maximum_bytes == 0 || url.len() > MAXIMUM_MOUNT_URL_BYTES {
        reject_callback(
            scope,
            arguments.get(3),
            "Invalid mount read request",
            "InvalidRequest",
        );
        return;
    }
    submit_host(
        scope,
        HostRequest::Mount(MountRequest::ReadFile { url, maximum_bytes }),
        arguments.get(2),
        arguments.get(3),
    );
}

#[allow(clippy::needless_pass_by_value)]
fn mount_operation(
    scope: &mut v8::PinScope<'_, '_>,
    arguments: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(kind) = exact_u64(scope, arguments.get(0)) else {
        reject_callback(
            scope,
            arguments.get(4),
            "Invalid mount operation",
            "InvalidRequest",
        );
        return;
    };
    let Some(first) = rust_string(scope, arguments.get(1)) else {
        reject_callback(
            scope,
            arguments.get(4),
            "Invalid mount operation",
            "InvalidRequest",
        );
        return;
    };
    if first.len() > MAXIMUM_MOUNT_URL_BYTES {
        reject_callback(
            scope,
            arguments.get(4),
            "Invalid mount operation",
            "InvalidRequest",
        );
        return;
    }
    let maximum =
        callback_state(scope).map_or(0, |state| state.borrow().configuration.maximum_event_bytes);
    let request = match kind {
        11 => byte_array(scope, arguments.get(2), maximum)
            .map(|bytes| HostRequest::Mount(MountRequest::WriteFile { url: first, bytes })),
        12 => Some(HostRequest::Mount(MountRequest::CreateDirectory {
            url: first,
        })),
        13 => Some(HostRequest::Mount(MountRequest::RemoveFile { url: first })),
        14 => Some(HostRequest::Mount(MountRequest::RemoveDirectory {
            url: first,
        })),
        15 => rust_string(scope, arguments.get(2))
            .filter(|to| to.len() <= MAXIMUM_MOUNT_URL_BYTES)
            .map(|to| HostRequest::Mount(MountRequest::Rename { from: first, to })),
        _ => None,
    };
    let Some(request) = request else {
        reject_callback(
            scope,
            arguments.get(4),
            "Invalid mount operation",
            "InvalidRequest",
        );
        return;
    };
    submit_host(scope, request, arguments.get(3), arguments.get(4));
}

#[allow(clippy::needless_pass_by_value)]
fn console_output(
    scope: &mut v8::PinScope<'_, '_>,
    arguments: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(message) = rust_string(scope, arguments.get(0)) else {
        return;
    };
    if let Some(state) = callback_state(scope) {
        state
            .borrow_mut()
            .enqueue(EngineEvent::ConsoleOutput(message.into_bytes()));
    }
}

fn submit_host(
    scope: &mut v8::PinScope<'_, '_>,
    request: HostRequest,
    resolve: v8::Local<'_, v8::Value>,
    reject: v8::Local<'_, v8::Value>,
) {
    let Ok(resolve) = v8::Local::<v8::Function>::try_from(resolve) else {
        return;
    };
    let Ok(reject) = v8::Local::<v8::Function>::try_from(reject) else {
        return;
    };
    let Some(state) = callback_state(scope) else {
        return;
    };
    let mut state = state.borrow_mut();
    if state.shutting_down
        || state.pending_promises.len() >= state.configuration.maximum_pending_host_requests
        || state.next_host_id > MAXIMUM_SAFE_INTEGER
    {
        drop(state);
        reject_function(
            scope,
            reject,
            "Host request limit exceeded",
            "QuotaExceeded",
        );
        return;
    }
    let id = state.next_host_id;
    state.next_host_id += 1;
    let event = EngineEvent::HostRequest {
        id: HostRequestId(id),
        request,
    };
    if !state.enqueue(event) {
        drop(state);
        reject_function(
            scope,
            reject,
            "Engine event queue exhausted",
            "QuotaExceeded",
        );
        return;
    }
    state.pending_promises.insert(
        id,
        PendingPromise {
            resolve: v8::Global::new(scope, resolve),
            reject: v8::Global::new(scope, reject),
        },
    );
}

fn complete_promise(
    scope: &mut v8::PinScope<'_, '_>,
    promise: &PendingPromise,
    completion: HostCompletion,
) -> Result<(), EngineError> {
    let receiver = v8::undefined(scope).into();
    match completion {
        HostCompletion::Unit => {
            let resolve = v8::Local::new(scope, &promise.resolve);
            resolve
                .call(scope, receiver, &[])
                .ok_or(EngineError::Backend)?;
        }
        HostCompletion::Bytes(bytes) => {
            let values = bytes
                .into_iter()
                .map(|byte| v8::Integer::new_from_unsigned(scope, u32::from(byte)).into())
                .collect::<Vec<v8::Local<'_, v8::Value>>>();
            let value = v8::Array::new_with_elements(scope, &values).into();
            let resolve = v8::Local::new(scope, &promise.resolve);
            resolve
                .call(scope, receiver, &[value])
                .ok_or(EngineError::Backend)?;
        }
        HostCompletion::Object { id, kind } => {
            let object = v8::Object::new(scope);
            let id_key = v8::Private::for_api(scope, v8::String::new(scope, "zintl.object.id"));
            let kind_key = v8::Private::for_api(scope, v8::String::new(scope, "zintl.object.kind"));
            let id_value = safe_integer_number(scope, id.0).into();
            let kind_value = v8::Integer::new(
                scope,
                match kind {
                    EngineObjectKind::Directory => 1,
                    EngineObjectKind::File => 2,
                },
            )
            .into();
            object.set_private(scope, id_key, id_value);
            object.set_private(scope, kind_key, kind_value);
            let resolve = v8::Local::new(scope, &promise.resolve);
            resolve
                .call(scope, receiver, &[object.into()])
                .ok_or(EngineError::Backend)?;
        }
        HostCompletion::Failed(error) => {
            let reject = v8::Local::new(scope, &promise.reject);
            let value = error_value(scope, "Host operation failed", host_error_name(error))?;
            reject
                .call(scope, receiver, &[value])
                .ok_or(EngineError::Backend)?;
        }
    }
    Ok(())
}

fn reject_callback(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    message: &str,
    code: &str,
) {
    if let Ok(reject) = v8::Local::<v8::Function>::try_from(value) {
        reject_function(scope, reject, message, code);
    }
}

fn reject_function(
    scope: &mut v8::PinScope<'_, '_>,
    reject: v8::Local<'_, v8::Function>,
    message: &str,
    code: &str,
) {
    let Ok(value) = error_value(scope, message, code) else {
        return;
    };
    let receiver = v8::undefined(scope).into();
    let _ = reject.call(scope, receiver, &[value]);
}

fn error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    code: &str,
) -> Result<v8::Local<'s, v8::Value>, EngineError> {
    let message = v8::String::new(scope, message).ok_or(EngineError::Backend)?;
    let error = v8::Exception::error(scope, message);
    let object = v8::Local::<v8::Object>::try_from(error).map_err(|_| EngineError::Backend)?;
    let key = v8::String::new(scope, "code").ok_or(EngineError::Backend)?;
    let code = v8::String::new(scope, code).ok_or(EngineError::Backend)?;
    object.set(scope, key.into(), code.into());
    Ok(error)
}

fn rust_string(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<String> {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn exact_u64(scope: &mut v8::PinScope<'_, '_>, value: v8::Local<'_, v8::Value>) -> Option<u64> {
    let value = value.number_value(scope)?;
    if value.is_finite()
        && (0.0..=MAXIMUM_SAFE_INTEGER_F64).contains(&value)
        && value.fract() == 0.0
    {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(value as u64)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
fn safe_integer_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: u64,
) -> v8::Local<'s, v8::Number> {
    debug_assert!(value <= MAXIMUM_SAFE_INTEGER);
    v8::Number::new(scope, value as f64)
}

fn byte_array(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    maximum: usize,
) -> Option<Vec<u8>> {
    let array = v8::Local::<v8::Array>::try_from(value).ok()?;
    let length = usize::try_from(array.length()).ok()?;
    if length > maximum {
        return None;
    }
    let mut bytes = Vec::with_capacity(length);
    for index in 0..array.length() {
        let value = array.get_index(scope, index)?.uint32_value(scope)?;
        bytes.push(u8::try_from(value).ok()?);
    }
    Some(bytes)
}

const fn host_error_name(error: HostErrorCode) -> &'static str {
    match error {
        HostErrorCode::PermissionDenied => "PermissionDenied",
        HostErrorCode::InvalidRequest => "InvalidRequest",
        HostErrorCode::QuotaExceeded => "QuotaExceeded",
        HostErrorCode::Cancelled => "Cancelled",
        HostErrorCode::TimedOut => "TimedOut",
        HostErrorCode::ShuttingDown => "ShuttingDown",
        HostErrorCode::OperationFailed => "OperationFailed",
    }
}

#[cfg(test)]
mod tests {
    use super::V8Backend;
    use runtime_engine::{
        EngineConfiguration, EngineEvent, EngineNotifier, EvaluationId, EvaluationOutcome,
        EvaluationRequest, JavaScriptEngineBackend,
    };
    use std::sync::Arc;

    struct NoopNotifier;

    impl EngineNotifier for NoopNotifier {
        fn notify(&self) {}
    }

    #[test]
    // Verifies V8 evaluates a Promise while the host controls the microtask checkpoint.
    fn evaluates_promise_at_explicit_checkpoint() {
        let mut backend = V8Backend::new();
        backend
            .start(EngineConfiguration::default(), Arc::new(NoopNotifier))
            .expect("start");
        backend
            .submit_evaluation(EvaluationRequest {
                id: EvaluationId(1),
                source: "Promise.resolve(6 * 7)".to_owned(),
            })
            .expect("submit");
        assert_eq!(backend.next_event().expect("event"), None);
        backend.perform_microtask_checkpoint().expect("checkpoint");
        assert_eq!(
            backend.next_event().expect("event"),
            Some(EngineEvent::EvaluationSettled {
                id: EvaluationId(1),
                outcome: EvaluationOutcome::Value(br#"{"type":"value","value":42}"#.to_vec()),
            })
        );
    }

    #[test]
    // Verifies console output and the terminal result preserve their enqueue order.
    fn emits_console_before_settlement() {
        let mut backend = V8Backend::new();
        backend
            .start(EngineConfiguration::default(), Arc::new(NoopNotifier))
            .expect("start");
        backend
            .submit_evaluation(EvaluationRequest {
                id: EvaluationId(1),
                source: "console.log('hello'); 7".to_owned(),
            })
            .expect("submit");
        backend.perform_microtask_checkpoint().expect("checkpoint");
        assert_eq!(
            backend.next_event().expect("event"),
            Some(EngineEvent::ConsoleOutput(b"hello".to_vec()))
        );
        assert!(matches!(
            backend.next_event().expect("event"),
            Some(EngineEvent::EvaluationSettled { .. })
        ));
    }
}
