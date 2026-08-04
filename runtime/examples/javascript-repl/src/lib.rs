//! Rust CLI host built exclusively on the public Zintl embedding contracts.

#![forbid(unsafe_code)]

use runtime_embed::{
    Authority, AuthorizationRequest, AuthorizationResult, EmbeddedRuntime, EngineSession,
    HostOpContext, OpLimits, RuntimeBuilder, RuntimeError, Source, VfsConfig,
};
use runtime_engine::{
    DriveBudget, EngineConfiguration, EngineError, EngineNotifier, EvaluationId, EvaluationOutcome,
    EvaluationRequest,
};
use runtime_jsc::JavaScriptCoreBackend;
use std::fmt;
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const DRIVE_BUDGET: DriveBudget = DriveBudget {
    maximum_events: 64,
    maximum_bytes: 1024 * 1024,
};

/// Deny-by-default authority for the REPL's `fs` virtual filesystem.
#[derive(Default)]
pub struct TerminalFsAuthority;

impl Authority for TerminalFsAuthority {
    fn authorization_requested(&self, request: &AuthorizationRequest<'_>) -> AuthorizationResult {
        let mut stderr = io::stderr().lock();
        if writeln!(
            stderr,
            "Filesystem authorization request:\n  vfs: {}\n  operation: {:?}\n  path: {}\nAllow? [y/N] ",
            request.vfs, request.operation, request.path
        )
        .and_then(|()| stderr.flush())
        .is_err()
        {
            return AuthorizationResult::Deny;
        }
        let mut answer = String::new();
        if read_authorization_answer(&mut answer).is_err() {
            return AuthorizationResult::Deny;
        }
        if matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes") {
            AuthorizationResult::Allow
        } else {
            AuthorizationResult::Deny
        }
    }
}

fn read_authorization_answer(answer: &mut String) -> io::Result<usize> {
    if io::stdin().is_terminal() {
        return io::stdin().lock().read_line(answer);
    }
    #[cfg(unix)]
    {
        io::BufReader::new(std::fs::File::open("/dev/tty")?).read_line(answer)
    }
    #[cfg(not(unix))]
    {
        io::stdin().lock().read_line(answer)
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
    pub fn new(authority: Arc<dyn Authority>) -> Result<Self, ReplError> {
        let current_directory = std::env::current_dir()?;
        let runtime = RuntimeBuilder::new()
            .add_vfs(VfsConfig {
                name: "fs".into(),
                source: Source::LoadDir {
                    path: current_directory,
                },
                authority: Some(authority),
            })?
            .register_op(
                "dev.zintl.echo",
                1,
                OpLimits::new(1024 * 1024, 1024 * 1024, 30_000_000_000)?,
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
            let mut settled = None;
            while let Some((settled_id, outcome)) = self.session.take_evaluation() {
                if settled_id != id {
                    continue;
                }
                settled = Some(outcome);
            }
            while let Some(output) = self.session.take_console_output() {
                if let Ok(output) = String::from_utf8(output) {
                    eprintln!("{output}");
                }
            }
            if let Some(outcome) = settled {
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
            self.wake.wait_briefly(&mut wake_generation);
        }
    }

    /// Evaluates a complete piped script with top-level `await` support.
    ///
    /// # Errors
    ///
    /// Reports the same bounded runtime and JavaScript failures as [`Self::evaluate`].
    pub fn evaluate_batch(&mut self, source: &str) -> Result<String, ReplError> {
        self.evaluate(&format!("(async () => {{\n{source}\n}})()"))
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
    use super::JavaScriptRepl;
    use runtime_embed::{Authority, AuthorizationRequest, AuthorizationResult};
    use std::sync::Arc;

    struct Deny;

    impl Authority for Deny {
        fn authorization_requested(
            &self,
            _request: &AuthorizationRequest<'_>,
        ) -> AuthorizationResult {
            AuthorizationResult::Deny
        }
    }

    struct Allow;

    impl Authority for Allow {
        fn authorization_requested(
            &self,
            _request: &AuthorizationRequest<'_>,
        ) -> AuthorizationResult {
            AuthorizationResult::Allow
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
        assert_eq!(
            repl.evaluate_batch(
                "const batchValue = await Promise.resolve(43); globalThis.batchValue = batchValue;"
            )
            .expect("batch top-level await"),
            r#"{"type":"value","value":null}"#
        );
        assert_eq!(
            repl.evaluate("batchValue").expect("batch context persists"),
            r#"{"type":"value","value":43}"#
        );
        repl.shutdown().expect("shutdown");
    }

    #[test]
    // Verifies absence of explicit approval rejects virtual filesystem reads.
    fn vfs_authority_denies_by_default() {
        let mut repl = JavaScriptRepl::new(Arc::new(Deny)).expect("repl");
        let error = repl
            .evaluate("Zintl.readFile('fs://Cargo.toml', 'utf8')")
            .expect_err("denied");
        assert!(error.to_string().contains("Host operation failed"));
        let unknown = repl
            .evaluate("Zintl.invoke('dev.zintl.unknown', new Uint8Array())")
            .expect_err("unknown operation");
        assert!(unknown.to_string().contains("Host operation failed"));
        repl.shutdown().expect("shutdown");
    }

    #[test]
    // Verifies VFS URLs reach files without exposing an absolute path to JavaScript.
    fn approved_vfs_reads_use_virtual_urls() {
        let mut repl = JavaScriptRepl::new(Arc::new(Allow)).expect("repl");
        let value = repl
            .evaluate(
                "Zintl.readFile('fs://Cargo.toml', 'utf8').then(x => x.includes('[package]'))",
            )
            .expect("read workspace manifest");
        assert_eq!(value, r#"{"type":"value","value":true}"#);
        let absolute = repl
            .evaluate("Zintl.readFile('/etc/passwd', 'utf8')")
            .expect_err("absolute path rejected");
        assert!(absolute.to_string().contains("Host operation failed"));
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
    }
}
