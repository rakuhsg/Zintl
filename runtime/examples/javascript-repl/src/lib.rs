//! Rust-hosted JavaScript REPL built on the public Zintl embedding API.

#![forbid(unsafe_code)]

use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source,
    Trace, js_string,
};
use runtime_embed::{
    Directory, EmbeddedRuntime, EngineAdapter, EngineLease, FileKind, FilesystemRights,
    PermissionDecision, PermissionRequest, RuntimeBuilder, RuntimeError, RuntimeHandle,
};
use std::fmt;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const DIRECTORY_QUOTA: u64 = 4 * 1024 * 1024;
const MAX_LOOP_ITERATIONS: u64 = 1_000_000;
const MAX_RECURSION_DEPTH: usize = 256;

const BOOTSTRAP: &str = r#"
(() => {
  "use strict";
  const requestDirectory = globalThis.__zintlRequestDirectory;
  const readTextFile = globalThis.__zintlReadTextFile;
  const writeTextFile = globalThis.__zintlWriteTextFile;
  const stat = globalThis.__zintlStat;
  const closeDirectory = globalThis.__zintlCloseDirectory;
  const print = globalThis.__zintlPrint;
  delete globalThis.__zintlRequestDirectory;
  delete globalThis.__zintlReadTextFile;
  delete globalThis.__zintlWriteTextFile;
  delete globalThis.__zintlStat;
  delete globalThis.__zintlCloseDirectory;
  delete globalThis.__zintlPrint;
  Object.defineProperty(globalThis, "Zintl", {
    value: Object.freeze({
      requestDirectory,
      readTextFile,
      writeTextFile,
      stat(path) { return JSON.parse(stat(path)); },
      closeDirectory,
    }),
    writable: false,
    enumerable: true,
    configurable: false,
  });
  Object.defineProperty(globalThis, "console", {
    value: Object.freeze({ log: print }),
    writable: false,
    enumerable: true,
    configurable: false,
  });
})();
"#;

/// Sanitized permission prompt shown outside the JavaScript engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRequest {
    pub operation: String,
    pub directory: String,
    pub rights: u64,
}

/// Trusted terminal or test boundary that decides one permission request.
pub trait PermissionPrompt: Send + Sync + 'static {
    fn approve(&self, request: &PromptRequest) -> bool;
}

/// Interactive `y/N` permission prompt using the process terminal.
#[derive(Default)]
pub struct TerminalPermissionPrompt {
    terminal: Mutex<()>,
}

impl PermissionPrompt for TerminalPermissionPrompt {
    fn approve(&self, request: &PromptRequest) -> bool {
        let Ok(_terminal) = self.terminal.lock() else {
            return false;
        };
        let mut stdout = io::stdout().lock();
        if writeln!(stdout, "\nJavaScript requests directory access")
            .and_then(|()| writeln!(stdout, "  Operation: {}", request.operation))
            .and_then(|()| writeln!(stdout, "  Rights: {}", rights_description(request.rights)))
            .and_then(|()| writeln!(stdout, "  Directory: {:?}", request.directory))
            .and_then(|()| write!(stdout, "Allow once? [y/N] "))
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return false;
        }
        let mut answer = String::new();
        if io::stdin().lock().read_line(&mut answer).is_err() {
            return false;
        }
        matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes")
    }
}

/// A persistent JavaScript context attached to one embedded runtime.
pub struct JavaScriptRepl {
    engine: Option<EngineLease<BoaAdapter>>,
    runtime: EmbeddedRuntime,
}

impl JavaScriptRepl {
    /// Builds and starts a REPL with deny-by-default terminal permission mediation.
    ///
    /// # Errors
    ///
    /// Returns sanitized runtime or engine initialization failures.
    pub fn new(prompt: Arc<dyn PermissionPrompt>) -> Result<Self, ReplError> {
        let gate = Arc::new(PermissionGate::default());
        let resolver_gate = gate.clone();
        let runtime = RuntimeBuilder::new()
            .permission_resolver(move |request: PermissionRequest| resolver_gate.resolve(request))
            .build()?;
        runtime.start()?;
        let engine = match EngineLease::attach(&runtime, BoaAdapter::new(prompt, gate)) {
            Ok(engine) => engine,
            Err(error) => {
                let _ = runtime.shutdown();
                return Err(error);
            }
        };
        Ok(Self {
            engine: Some(engine),
            runtime,
        })
    }

    /// Evaluates one script in the persistent REPL realm.
    ///
    /// # Errors
    ///
    /// Rejects oversized source and reports JavaScript or host failures.
    pub fn evaluate(&mut self, source: &str) -> Result<String, ReplError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(ReplError::SourceTooLarge);
        }
        self.engine
            .as_mut()
            .ok_or(ReplError::NotRunning)?
            .adapter_mut()
            .evaluate(source)
    }

    /// Releases the engine context before terminating the runtime.
    ///
    /// # Errors
    ///
    /// Returns a sanitized engine or runtime shutdown failure.
    pub fn shutdown(mut self) -> Result<(), ReplError> {
        if let Some(engine) = self.engine.take() {
            engine.shutdown()?;
        }
        self.runtime.shutdown()?;
        Ok(())
    }
}

impl Drop for JavaScriptRepl {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.take() {
            let _ = engine.shutdown();
        }
        let _ = self.runtime.shutdown();
    }
}

#[derive(Default)]
struct PermissionGate {
    approval: Mutex<Option<PromptRequest>>,
}

impl PermissionGate {
    fn allow_once(&self, request: PromptRequest) {
        if let Ok(mut approval) = self.approval.lock() {
            *approval = Some(request);
        }
    }

    fn clear(&self) {
        if let Ok(mut approval) = self.approval.lock() {
            approval.take();
        }
    }

    fn resolve(&self, request: PermissionRequest) -> PermissionDecision {
        let Some(directory) = request.requested_directory.clone() else {
            return PermissionDecision::Deny;
        };
        let actual = PromptRequest {
            operation: request.operation.clone(),
            directory,
            rights: request.requested_rights,
        };
        let approved = self
            .approval
            .lock()
            .ok()
            .and_then(|mut approval| approval.take())
            .is_some_and(|expected| expected == actual);
        if approved {
            PermissionDecision::Allow {
                scope: request.requested_scope,
                rights: request.requested_rights,
                quota: DIRECTORY_QUOTA,
            }
        } else {
            PermissionDecision::Deny
        }
    }
}

struct BoaAdapter {
    context: Option<Context>,
    prompt: Arc<dyn PermissionPrompt>,
    gate: Arc<PermissionGate>,
}

impl BoaAdapter {
    fn new(prompt: Arc<dyn PermissionPrompt>, gate: Arc<PermissionGate>) -> Self {
        Self {
            context: None,
            prompt,
            gate,
        }
    }

    fn evaluate(&mut self, source: &str) -> Result<String, ReplError> {
        let context = self.context.as_mut().ok_or(ReplError::NotRunning)?;
        let value = context
            .eval(Source::from_bytes(source))
            .map_err(|error| ReplError::JavaScript(error.to_string()))?;
        context.run_jobs();
        Ok(value.display().to_string())
    }
}

impl EngineAdapter for BoaAdapter {
    type Error = ReplError;

    fn start(&mut self, runtime: RuntimeHandle) -> Result<(), Self::Error> {
        if self.context.is_some() {
            return Err(ReplError::NotRunning);
        }
        let mut context = Context::default();
        context
            .runtime_limits_mut()
            .set_loop_iteration_limit(MAX_LOOP_ITERATIONS);
        context
            .runtime_limits_mut()
            .set_recursion_limit(MAX_RECURSION_DEPTH);
        context.insert_data(HostBridge {
            state: Mutex::new(HostState {
                runtime,
                directory: None,
                prompt: self.prompt.clone(),
                gate: self.gate.clone(),
            }),
        });
        register_host_functions(&mut context)?;
        context
            .eval(Source::from_bytes(BOOTSTRAP))
            .map_err(|error| ReplError::Engine(error.to_string()))?;
        self.context = Some(context);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), Self::Error> {
        self.context.take();
        Ok(())
    }
}

struct HostState {
    runtime: RuntimeHandle,
    directory: Option<Directory>,
    prompt: Arc<dyn PermissionPrompt>,
    gate: Arc<PermissionGate>,
}

#[derive(Finalize, JsData, Trace)]
#[boa_gc(unsafe_empty_trace)]
struct HostBridge {
    // HostState contains no Boa GC values, so it has no traceable edges.
    state: Mutex<HostState>,
}

fn register_host_functions(context: &mut Context) -> Result<(), ReplError> {
    for (name, length, function) in [
        (
            js_string!("__zintlRequestDirectory"),
            2,
            NativeFunction::from_fn_ptr(request_directory),
        ),
        (
            js_string!("__zintlReadTextFile"),
            1,
            NativeFunction::from_fn_ptr(read_text_file),
        ),
        (
            js_string!("__zintlWriteTextFile"),
            2,
            NativeFunction::from_fn_ptr(write_text_file),
        ),
        (
            js_string!("__zintlStat"),
            1,
            NativeFunction::from_fn_ptr(stat_file),
        ),
        (
            js_string!("__zintlCloseDirectory"),
            0,
            NativeFunction::from_fn_ptr(close_directory),
        ),
        (
            js_string!("__zintlPrint"),
            1,
            NativeFunction::from_fn_ptr(print_values),
        ),
    ] {
        context
            .register_global_builtin_callable(name, length, function)
            .map_err(|error| ReplError::Engine(error.to_string()))?;
    }
    Ok(())
}

fn request_directory(
    _this: &JsValue,
    arguments: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let path = required_string(arguments, 0, "directory path", context)?;
    let requested = optional_string(arguments, 1, "read,metadata", context)?;
    let rights = parse_rights(&requested).map_err(type_error)?;
    with_host(context, |state| {
        let prompt_request = PromptRequest {
            operation: "zintl.builtin.fs.request-directory".to_owned(),
            directory: path.clone(),
            rights: rights.bits(),
        };
        state.gate.clear();
        if state.prompt.approve(&prompt_request) {
            state.gate.allow_once(prompt_request);
        }
        let directory = state
            .runtime
            .request_directory(path, rights)
            .and_then(runtime_embed::RuntimeTask::wait);
        state.gate.clear();
        let directory = directory?;
        state.directory = Some(directory);
        Ok(JsValue::from(true))
    })
}

fn read_text_file(
    _this: &JsValue,
    arguments: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let path = required_string(arguments, 0, "relative file path", context)?;
    with_host(context, |state| {
        let directory = state
            .directory
            .as_ref()
            .ok_or(RuntimeError::PermissionDenied)?;
        let file = directory
            .open_file(path, FilesystemRights::READ, false, false)?
            .wait()?;
        let bytes = file.read(MAX_FILE_BYTES)?.wait()?;
        let text = String::from_utf8(bytes).map_err(|_| RuntimeError::Protocol)?;
        Ok(JsValue::from(JsString::from(text)))
    })
}

fn write_text_file(
    _this: &JsValue,
    arguments: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let path = required_string(arguments, 0, "relative file path", context)?;
    let text = required_string(arguments, 1, "text", context)?;
    if text.len() > MAX_FILE_BYTES {
        return Err(type_error("text exceeds the 1 MiB limit"));
    }
    with_host(context, |state| {
        let directory = state
            .directory
            .as_ref()
            .ok_or(RuntimeError::PermissionDenied)?;
        let file = directory
            .open_file(path, FilesystemRights::WRITE, false, true)?
            .wait()?;
        file.write(text.into_bytes())?.wait()?;
        Ok(JsValue::undefined())
    })
}

fn stat_file(_this: &JsValue, arguments: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = required_string(arguments, 0, "relative file path", context)?;
    with_host(context, |state| {
        let directory = state
            .directory
            .as_ref()
            .ok_or(RuntimeError::PermissionDenied)?;
        let file = directory
            .open_file(path, FilesystemRights::METADATA, false, false)?
            .wait()?;
        let metadata = file.stat()?.wait()?;
        let kind = match metadata.kind {
            FileKind::File => "file",
            FileKind::Directory => "directory",
        };
        Ok(JsValue::from(JsString::from(format!(
            "{{\"kind\":\"{kind}\",\"size\":{}}}",
            metadata.size
        ))))
    })
}

fn close_directory(
    _this: &JsValue,
    _arguments: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    with_host(context, |state| {
        if let Some(directory) = state.directory.take() {
            directory.close()?;
        }
        Ok(JsValue::undefined())
    })
}

fn print_values(
    _this: &JsValue,
    arguments: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let mut output = io::stdout().lock();
    for (index, value) in arguments.iter().enumerate() {
        if index != 0 {
            write!(output, " ").map_err(|_| host_error(RuntimeError::Io))?;
        }
        let value = value.to_string(context)?.to_std_string_escaped();
        write!(output, "{value}").map_err(|_| host_error(RuntimeError::Io))?;
    }
    writeln!(output).map_err(|_| host_error(RuntimeError::Io))?;
    Ok(JsValue::undefined())
}

fn with_host(
    context: &Context,
    operation: impl FnOnce(&mut HostState) -> Result<JsValue, RuntimeError>,
) -> JsResult<JsValue> {
    let bridge = context
        .get_data::<HostBridge>()
        .ok_or_else(|| host_error(RuntimeError::ShuttingDown))?;
    let mut state = bridge
        .state
        .lock()
        .map_err(|_| host_error(RuntimeError::Internal))?;
    operation(&mut state).map_err(host_error)
}

fn required_string(
    arguments: &[JsValue],
    index: usize,
    label: &str,
    context: &mut Context,
) -> JsResult<String> {
    let value = arguments
        .get(index)
        .ok_or_else(|| type_error(format!("missing {label}")))?;
    if !value.is_string() {
        return Err(type_error(format!("{label} must be a string")));
    }
    Ok(value.to_string(context)?.to_std_string_escaped())
}

fn optional_string(
    arguments: &[JsValue],
    index: usize,
    default: &str,
    context: &mut Context,
) -> JsResult<String> {
    match arguments.get(index) {
        None => Ok(default.to_owned()),
        Some(value) if value.is_undefined() => Ok(default.to_owned()),
        Some(value) if value.is_string() => Ok(value.to_string(context)?.to_std_string_escaped()),
        Some(_) => Err(type_error("rights must be a comma-separated string")),
    }
}

fn parse_rights(value: &str) -> Result<FilesystemRights, &'static str> {
    let mut bits = 0_u64;
    for name in value.split(',').map(str::trim) {
        bits |= match name {
            "read" => FilesystemRights::READ.bits(),
            "write" => FilesystemRights::WRITE.bits(),
            "create" => FilesystemRights::CREATE.bits(),
            "metadata" => FilesystemRights::METADATA.bits(),
            "enumerate" => FilesystemRights::ENUMERATE.bits(),
            "truncate" => FilesystemRights::TRUNCATE.bits(),
            _ => return Err("unknown or empty filesystem right"),
        };
    }
    FilesystemRights::from_bits(bits).map_err(|_| "invalid filesystem rights")
}

fn rights_description(rights: u64) -> String {
    [
        (FilesystemRights::READ, "read"),
        (FilesystemRights::WRITE, "write"),
        (FilesystemRights::CREATE, "create"),
        (FilesystemRights::METADATA, "metadata"),
        (FilesystemRights::ENUMERATE, "enumerate"),
        (FilesystemRights::TRUNCATE, "truncate"),
    ]
    .into_iter()
    .filter_map(|(right, name)| (rights & right.bits() != 0).then_some(name))
    .collect::<Vec<_>>()
    .join(", ")
}

fn type_error(message: impl Into<String>) -> boa_engine::JsError {
    JsNativeError::typ().with_message(message.into()).into()
}

fn host_error(error: RuntimeError) -> boa_engine::JsError {
    JsNativeError::error()
        .with_message(format!("Zintl host error: {error}"))
        .into()
}

#[derive(Debug)]
pub enum ReplError {
    Runtime(RuntimeError),
    Engine(String),
    JavaScript(String),
    SourceTooLarge,
    NotRunning,
    Io(io::Error),
}

impl fmt::Display for ReplError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => write!(formatter, "runtime error: {error}"),
            Self::Engine(_) => formatter.write_str("JavaScript engine initialization failed"),
            Self::JavaScript(error) => write!(formatter, "{error}"),
            Self::SourceTooLarge => formatter.write_str("source exceeds the 1 MiB limit"),
            Self::NotRunning => formatter.write_str("REPL is not running"),
            Self::Io(error) => write!(formatter, "terminal I/O failed: {error}"),
        }
    }
}

impl std::error::Error for ReplError {}

impl From<RuntimeError> for ReplError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<io::Error> for ReplError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RecordingPrompt {
        allow: AtomicBool,
        requests: Mutex<Vec<PromptRequest>>,
        threads: Mutex<Vec<std::thread::ThreadId>>,
    }

    impl RecordingPrompt {
        fn new(allow: bool) -> Self {
            Self {
                allow: AtomicBool::new(allow),
                requests: Mutex::new(Vec::new()),
                threads: Mutex::new(Vec::new()),
            }
        }
    }

    impl PermissionPrompt for RecordingPrompt {
        fn approve(&self, request: &PromptRequest) -> bool {
            self.requests
                .lock()
                .expect("requests")
                .push(request.clone());
            self.threads
                .lock()
                .expect("threads")
                .push(std::thread::current().id());
            self.allow.load(Ordering::Acquire)
        }
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "zintl-javascript-repl-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).expect("temporary directory");
        directory.canonicalize().expect("canonical directory")
    }

    fn js_string(path: &Path) -> String {
        format!("{:?}", path.to_string_lossy())
    }

    // Verifies the Rust engine keeps one realm alive across separate REPL evaluations.
    #[test]
    fn evaluates_javascript_in_a_persistent_context() {
        let prompt = Arc::new(RecordingPrompt::new(false));
        let mut repl = JavaScriptRepl::new(prompt).expect("repl");
        assert_eq!(
            repl.evaluate("let answer = 40 + 2; answer").expect("eval"),
            "42"
        );
        assert_eq!(repl.evaluate("answer + 1").expect("eval"), "43");
        repl.shutdown().expect("shutdown");
    }

    // Verifies denial occurs before a JavaScript-visible directory capability is installed.
    #[test]
    fn permission_prompt_denies_by_default() {
        let root = temporary_directory("deny");
        let prompt = Arc::new(RecordingPrompt::new(false));
        let mut repl = JavaScriptRepl::new(prompt.clone()).expect("repl");
        let source = format!(
            "Zintl.requestDirectory({}, 'read,metadata')",
            js_string(&root)
        );
        assert!(repl.evaluate(&source).is_err());
        let requests = prompt.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].directory, root.to_string_lossy());
        assert_eq!(
            requests[0].rights,
            FilesystemRights::READ
                .union(FilesystemRights::METADATA)
                .bits()
        );
        drop(requests);
        assert_eq!(
            prompt.threads.lock().expect("threads").as_slice(),
            &[std::thread::current().id()]
        );
        repl.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    // Verifies approved scope supports bounded relative reads without exposing a native handle.
    #[test]
    fn approved_directory_reads_through_opaque_host_state() {
        let root = temporary_directory("read");
        fs::write(root.join("hello.txt"), "hello from Rust").expect("fixture");
        let prompt = Arc::new(RecordingPrompt::new(true));
        let mut repl = JavaScriptRepl::new(prompt).expect("repl");
        let request = format!(
            "Zintl.requestDirectory({}, 'read,metadata')",
            js_string(&root)
        );
        assert_eq!(repl.evaluate(&request).expect("request"), "true");
        assert_eq!(
            repl.evaluate("Zintl.readTextFile('hello.txt')")
                .expect("read"),
            "\"hello from Rust\""
        );
        assert!(
            repl.evaluate("Zintl.readTextFile('../escape.txt')")
                .is_err()
        );
        assert_eq!(
            repl.evaluate("Object.keys(Zintl).sort().join(',')")
                .expect("surface"),
            "\"closeDirectory,readTextFile,requestDirectory,stat,writeTextFile\""
        );
        repl.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    // Verifies writes require both write and truncate rights granted by the prompt.
    #[test]
    fn write_api_cannot_escalate_a_read_only_grant() {
        let root = temporary_directory("write-denied");
        fs::write(root.join("value.txt"), "original").expect("fixture");
        let prompt = Arc::new(RecordingPrompt::new(true));
        let mut repl = JavaScriptRepl::new(prompt).expect("repl");
        let request = format!(
            "Zintl.requestDirectory({}, 'read,metadata')",
            js_string(&root)
        );
        repl.evaluate(&request).expect("request");
        assert!(
            repl.evaluate("Zintl.writeTextFile('value.txt', 'changed')")
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(root.join("value.txt")).expect("contents"),
            "original"
        );
        repl.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    // Verifies an explicitly approved write/truncate grant updates only a relative file.
    #[test]
    fn approved_write_uses_the_declared_rights() {
        let root = temporary_directory("write-approved");
        fs::write(root.join("value.txt"), "original").expect("fixture");
        let prompt = Arc::new(RecordingPrompt::new(true));
        let mut repl = JavaScriptRepl::new(prompt).expect("repl");
        let request = format!(
            "Zintl.requestDirectory({}, 'read,write,metadata,truncate')",
            js_string(&root)
        );
        repl.evaluate(&request).expect("request");
        assert_eq!(
            repl.evaluate("Zintl.writeTextFile('value.txt', 'changed')")
                .expect("write"),
            "undefined"
        );
        assert_eq!(
            repl.evaluate("Zintl.readTextFile('value.txt')")
                .expect("read"),
            "\"changed\""
        );
        repl.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    // Verifies oversized source and non-terminating loops are rejected by explicit engine limits.
    #[test]
    fn evaluation_limits_prevent_unbounded_repl_work() {
        let prompt = Arc::new(RecordingPrompt::new(false));
        let mut repl = JavaScriptRepl::new(prompt).expect("repl");
        let oversized = " ".repeat(MAX_SOURCE_BYTES + 1);
        assert!(matches!(
            repl.evaluate(&oversized),
            Err(ReplError::SourceTooLarge)
        ));
        assert!(repl.evaluate("while (true) {}").is_err());
        repl.shutdown().expect("shutdown");
    }
}
