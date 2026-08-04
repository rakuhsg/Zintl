//! Rust CLI host built exclusively on the public Zintl embedding contracts.

#![forbid(unsafe_code)]

use runtime_embed::{
    EmbeddedRuntime, EngineSession, HostOpContext, OpLimits, PermissionDecision, PermissionRequest,
    PermissionResponder, RuntimeBuilder, RuntimeError,
};
use runtime_engine::{
    DriveBudget, EngineConfiguration, EngineError, EngineNotifier, EvaluationId, EvaluationOutcome,
    EvaluationRequest,
};
use runtime_jsc::JavaScriptCoreBackend;
use std::fmt;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const DIRECTORY_QUOTA: u64 = 64 * 1024 * 1024;
const DRIVE_BUDGET: DriveBudget = DriveBudget {
    maximum_events: 64,
    maximum_bytes: 1024 * 1024,
};

/// Trusted terminal policy consulted for every requested authority grant.
pub trait PermissionPrompt: Send + Sync + 'static {
    /// Returns true only for an explicit approval of this exact request.
    fn approve(&self, request: &PermissionRequest) -> bool;
}

/// Deny-by-default interactive terminal permission prompt.
#[derive(Default)]
pub struct TerminalPermissionPrompt;

impl PermissionPrompt for TerminalPermissionPrompt {
    fn approve(&self, request: &PermissionRequest) -> bool {
        let directory = request.requested_directory.as_deref().unwrap_or("<none>");
        let mut stderr = io::stderr().lock();
        if writeln!(
            stderr,
            "Permission request:\n  operation: {}\n  kind: {}\n  directory: {}\n  rights: 0x{:x}\nAllow? [y/N] ",
            request.operation, request.kind, directory, request.requested_rights
        )
        .and_then(|()| stderr.flush())
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

#[derive(Default)]
struct WakeState {
    generation: Mutex<u64>,
    ready: Condvar,
}

impl WakeState {
    fn wait_briefly(&self, observed: &mut u64) {
        let Ok(generation) = self.generation.lock() else {
            return;
        };
        if *generation != *observed {
            *observed = *generation;
            return;
        }
        if let Ok((generation, _)) = self
            .ready
            .wait_timeout(generation, Duration::from_millis(2))
        {
            *observed = *generation;
        }
    }
}

impl EngineNotifier for WakeState {
    fn notify(&self) {
        if let Ok(mut generation) = self.generation.lock() {
            *generation = generation.wrapping_add(1);
            self.ready.notify_all();
        }
    }
}

/// A persistent `JavaScriptCore` REPL attached to one Rust-owned runtime.
pub struct JavaScriptRepl {
    session: EngineSession,
    runtime: EmbeddedRuntime,
    wake: Arc<WakeState>,
    next_evaluation: u64,
}

impl JavaScriptRepl {
    /// Builds and starts a Rust-owned runtime with callback-mediated authority.
    ///
    /// # Errors
    ///
    /// Returns sanitized runtime or `JavaScriptCore` initialization failures.
    pub fn new(prompt: Arc<dyn PermissionPrompt>) -> Result<Self, ReplError> {
        let runtime = RuntimeBuilder::new()
            .permission_callback(
                move |request: PermissionRequest, responder: PermissionResponder| {
                    let decision = if prompt.approve(&request) {
                        PermissionDecision::Allow {
                            rights: request.requested_rights,
                            quota: DIRECTORY_QUOTA,
                        }
                    } else {
                        PermissionDecision::Deny
                    };
                    let _ = responder.respond(decision);
                },
            )
            .register_op(
                "dev.zintl.echo",
                1,
                OpLimits::new(1024 * 1024, 1024 * 1024, 30_000_000_000)?,
                "dev.zintl.permission.echo",
                |_context: HostOpContext, input: Vec<u8>| Ok(input),
            )?
            .build()?;
        runtime.start()?;
        let wake = Arc::new(WakeState::default());
        let session = match EngineSession::attach(
            runtime.handle()?,
            Box::new(JavaScriptCoreBackend::new()),
            EngineConfiguration::default(),
            wake.clone(),
        ) {
            Ok(session) => session,
            Err(error) => {
                let _ = runtime.shutdown();
                return Err(error.into());
            }
        };
        Ok(Self {
            session,
            runtime,
            wake,
            next_evaluation: 1,
        })
    }

    /// Evaluates one script and drives bounded runtime turns until it settles.
    ///
    /// # Errors
    ///
    /// Reports sanitized JavaScript, runtime, quota, cancellation, or backend failures.
    pub fn evaluate(&mut self, source: &str) -> Result<String, ReplError> {
        let id = EvaluationId(self.next_evaluation);
        self.next_evaluation = self
            .next_evaluation
            .checked_add(1)
            .ok_or(ReplError::Engine(EngineError::QuotaExceeded))?;
        self.session
            .backend_mut()
            .submit_evaluation(EvaluationRequest {
                id,
                source: source.to_owned(),
            })?;
        let mut wake_generation = 0;
        loop {
            self.session.drive(DRIVE_BUDGET)?;
            while let Some((settled_id, outcome)) = self.session.take_evaluation() {
                if settled_id != id {
                    continue;
                }
                return match outcome {
                    EvaluationOutcome::Value(bytes) => {
                        String::from_utf8(bytes).map_err(|_| ReplError::Protocol)
                    }
                    EvaluationOutcome::Exception(error) => Err(ReplError::JavaScript(format!(
                        "{}: {}",
                        error.name, error.message
                    ))),
                    EvaluationOutcome::Cancelled => Err(ReplError::Cancelled),
                };
            }
            while let Some(output) = self.session.take_console_output() {
                if let Ok(output) = String::from_utf8(output) {
                    eprintln!("{output}");
                }
            }
            self.wake.wait_briefly(&mut wake_generation);
        }
    }

    /// Releases JSC before terminating the Rust runtime.
    ///
    /// # Errors
    ///
    /// Returns sanitized engine or runtime teardown failures.
    pub fn shutdown(mut self) -> Result<(), ReplError> {
        self.session.shutdown()?;
        self.runtime.shutdown()?;
        Ok(())
    }
}

/// Stable errors printed by the CLI without native implementation details.
#[derive(Debug)]
pub enum ReplError {
    /// Rust runtime failure.
    Runtime(RuntimeError),
    /// JavaScript engine boundary failure.
    Engine(EngineError),
    /// Sanitized JavaScript exception.
    JavaScript(String),
    /// Evaluation was cancelled.
    Cancelled,
    /// Engine returned malformed UTF-8 or protocol data.
    Protocol,
    /// Terminal I/O failed.
    Io(io::Error),
}

impl fmt::Display for ReplError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => write!(formatter, "runtime: {error}"),
            Self::Engine(error) => write!(formatter, "engine: {error}"),
            Self::JavaScript(error) => formatter.write_str(error),
            Self::Cancelled => formatter.write_str("evaluation cancelled"),
            Self::Protocol => formatter.write_str("invalid engine protocol"),
            Self::Io(error) => write!(formatter, "terminal I/O: {error}"),
        }
    }
}

impl std::error::Error for ReplError {}

impl From<RuntimeError> for ReplError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<EngineError> for ReplError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl From<io::Error> for ReplError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::{JavaScriptRepl, PermissionPrompt};
    use runtime_embed::PermissionRequest;
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Deny;

    impl PermissionPrompt for Deny {
        fn approve(&self, _request: &PermissionRequest) -> bool {
            false
        }
    }

    struct Allow;

    impl PermissionPrompt for Allow {
        fn approve(&self, _request: &PermissionRequest) -> bool {
            true
        }
    }

    #[test]
    // Verifies the Rust REPL evaluates persistent Promise-aware JavaScript through JSC.
    fn evaluates_javascript_with_jsc_backend() {
        let mut repl = JavaScriptRepl::new(Arc::new(Deny)).expect("repl");
        assert_eq!(
            repl.evaluate("globalThis.answer = 40; Promise.resolve(answer + 2)")
                .expect("evaluate"),
            r#"{"type":"value","value":42}"#
        );
        assert_eq!(
            repl.evaluate("answer + 1").expect("persistent context"),
            r#"{"type":"value","value":41}"#
        );
        assert_eq!(
            repl.evaluate("[typeof process, typeof require, typeof Deno, typeof fetch]")
                .expect("no ambient operating-system APIs"),
            r#"{"type":"value","value":["undefined","undefined","undefined","undefined"]}"#
        );
        repl.shutdown().expect("shutdown");
    }

    #[test]
    // Verifies absence of explicit approval rejects directory authority.
    fn permission_callback_denies_by_default() {
        let mut repl = JavaScriptRepl::new(Arc::new(Deny)).expect("repl");
        let error = repl
            .evaluate("Zintl.requestDirectory('/tmp', {read:true})")
            .expect_err("denied");
        assert!(error.to_string().contains("Host operation failed"));
        let unknown = repl
            .evaluate("Zintl.invoke('dev.zintl.unknown', new Uint8Array())")
            .expect_err("unknown operation");
        assert!(unknown.to_string().contains("Host operation failed"));
        repl.shutdown().expect("shutdown");
    }

    #[test]
    // Verifies the public Promise API reaches Rust-owned permission and filesystem resources.
    fn approved_directory_uses_opaque_rust_resources() {
        let root = std::env::temp_dir().join(format!(
            "zintl-jsc-repl-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir(&root).expect("directory");
        let root = root.canonicalize().expect("canonical test directory");
        fs::write(root.join("sample.txt"), b"runtime").expect("sample");
        let mut repl = JavaScriptRepl::new(Arc::new(Allow)).expect("repl");
        let source = format!(
            "(async () => {{ const d = await Zintl.requestDirectory({path:?}, {{read:true, metadata:true}}); globalThis.savedOpen = d.openRelative; const f = await d.openRelative('sample.txt', {{read:true, metadata:true}}); const b = await f.read({{maxBytes:64}}); await f.close(); await d.close(); return b; }})()",
            path = root.to_string_lossy()
        );
        assert_eq!(
            repl.evaluate(&source).expect("evaluate"),
            r#"{"type":"bytes","value":[114,117,110,116,105,109,101]}"#
        );
        let write_source = format!(
            "(async () => {{ const d = await Zintl.requestDirectory({path:?}, {{read:true, write:true, create:true, truncate:true, metadata:true}}); await d.writeRelative('fresh.txt', new Uint8Array([1,2,3]), {{create:true, truncate:true}}); const metadata = await d.statRelative('fresh.txt'); const bytes = await d.readRelative('fresh.txt', {{maxBytes:8}}); await d.close(); return {{size:metadata.size, bytes:Array.from(bytes)}}; }})()",
            path = root.to_string_lossy()
        );
        assert_eq!(
            repl.evaluate(&write_source).expect("write, stat and read"),
            r#"{"type":"value","value":{"size":3,"bytes":[1,2,3]}}"#
        );
        let forged = repl
            .evaluate("savedOpen.call({}, 'sample.txt', {read:true})")
            .expect_err("forged receiver");
        assert!(forged.to_string().contains("Invalid receiver"));
        assert_eq!(
            repl.evaluate("Zintl.sleep(1)").expect("sleep"),
            r#"{"type":"value","value":null}"#
        );
        assert_eq!(
            repl.evaluate("Zintl.invoke('dev.zintl.echo', new Uint8Array([7,8]))")
                .expect("custom op"),
            r#"{"type":"bytes","value":[7,8]}"#
        );
        repl.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
