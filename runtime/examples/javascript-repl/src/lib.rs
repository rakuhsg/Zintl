//! REPL composed from a main-thread JS loop and a separate mount IO loop.

#![forbid(unsafe_code)]

use messageloop_core::Sender;
#[cfg(unix)]
use messageloop_io::SourceFd;
use messageloop_io::{
    Event, Interest, IoContext, IoMessageHandler, IoSender, MessageLoopIo, Token,
};
use runtime_engine::{
    EngineEvent, EngineNotifier, EvaluationId, EvaluationOutcome, EvaluationRequest,
    HostCompletion, HostErrorCode, HostRequest, HostRequestId, JavaScriptEngineBackend,
    MountRequest,
};
use runtime_jsc::JavaScriptCoreBackend;
use runtime_v8::V8Backend;
use std::fmt;
use std::io::{self, IsTerminal, Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use zintl_host::{ZjsHost, ZjsHostBuilder};
use zintl_io_mounts::{
    BoundaryResolve, DirResolve, MountConfig, MountError, MountService, Source as MountSource,
    SymlinkPolicy,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EngineKind {
    JavaScriptCore,
    V8,
}

impl EngineKind {
    const fn default_for_platform() -> Self {
        if cfg!(target_os = "macos") {
            Self::JavaScriptCore
        } else {
            Self::V8
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::JavaScriptCore => "JavaScriptCore",
            Self::V8 => "V8",
        }
    }

    fn backend(self) -> Box<dyn JavaScriptEngineBackend> {
        match self {
            Self::JavaScriptCore => Box::new(JavaScriptCoreBackend::new()),
            Self::V8 => Box::new(V8Backend::new()),
        }
    }
}

enum MainMessage {
    EngineReady,
    MountCreated(Result<(), MountError>),
    IoCompleted {
        request_id: HostRequestId,
        completion: HostCompletion,
    },
}

enum IoMessage {
    Mount {
        request_id: HostRequestId,
        request: MountRequest,
    },
    Sleep {
        request_id: HostRequestId,
        duration: Duration,
    },
    WorkerCompleted {
        request_id: HostRequestId,
        result: Result<IoValue, MountError>,
    },
    Stop,
}

enum IoValue {
    Unit,
    Bytes(Vec<u8>),
}

struct LoopNotifier(IoSender<MainMessage>);
impl EngineNotifier for LoopNotifier {
    fn notify(&self) {
        let _ = self.0.send(MainMessage::EngineReady);
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;
struct WorkerPool {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}
impl WorkerPool {
    fn new(count: usize) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::new();
        for index in 0..count {
            let receiver = receiver.clone();
            workers.push(
                thread::Builder::new()
                    .name(format!("zintl-io-worker-{index}"))
                    .spawn(move || {
                        loop {
                            let job = receiver
                                .lock()
                                .ok()
                                .and_then(|receiver| receiver.recv().ok());
                            let Some(job) = job else { break };
                            job();
                        }
                    })?,
            );
        }
        Ok(Self {
            sender: Some(sender),
            workers,
        })
    }
    fn submit(&self, job: impl FnOnce() + Send + 'static) -> io::Result<()> {
        self.sender
            .as_ref()
            .ok_or_else(|| io::Error::other("worker pool stopped"))?
            .send(Box::new(job))
            .map_err(|_| io::Error::other("worker pool stopped"))
    }
}
impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

struct IoHandler {
    main_sender: IoSender<MainMessage>,
    mounts: Arc<Mutex<MountService>>,
    workers: WorkerPool,
    root: std::path::PathBuf,
}

impl IoMessageHandler<IoMessage> for IoHandler {
    fn init(&mut self, _cx: &mut IoContext<'_, IoMessage>) -> io::Result<()> {
        let result = self
            .mounts
            .lock()
            .map_err(|_| io::Error::other("mount lock poisoned"))?
            .create_mount(MountConfig {
                name: "project".into(),
                source: MountSource::Directory {
                    root_path: self.root.clone(),
                    readonly: false,
                    resolve: DirResolve {
                        boundary: BoundaryResolve::InRoot,
                        symlinks: SymlinkPolicy::Allow,
                    },
                },
            })
            .map(|_| ());
        self.main_sender
            .send(MainMessage::MountCreated(result))
            .map_err(|_| io::Error::other("JS loop closed"))
    }

    fn on(&mut self, cx: &mut IoContext<'_, IoMessage>, message: IoMessage) -> io::Result<()> {
        match message {
            IoMessage::Mount {
                request_id,
                request,
            } => {
                let mounts = self.mounts.clone();
                let sender = cx.sender();
                self.workers.submit(move || {
                    let result = mounts
                        .lock()
                        .map_err(|_| MountError::Io)
                        .and_then(|mut mounts| run_mount_request(&mut mounts, request));
                    let _ = sender.send(IoMessage::WorkerCompleted { request_id, result });
                })?;
            }
            IoMessage::Sleep {
                request_id,
                duration,
            } => {
                let sender = cx.sender();
                self.workers.submit(move || {
                    thread::sleep(duration);
                    let _ = sender.send(IoMessage::WorkerCompleted {
                        request_id,
                        result: Ok(IoValue::Unit),
                    });
                })?;
            }
            IoMessage::WorkerCompleted { request_id, result } => {
                let completion = match result {
                    Ok(IoValue::Unit) => HostCompletion::Unit,
                    Ok(IoValue::Bytes(bytes)) => HostCompletion::Bytes(bytes),
                    Err(error) => HostCompletion::Failed(mount_error_code(&error)),
                };
                let _ = self.main_sender.send(MainMessage::IoCompleted {
                    request_id,
                    completion,
                });
            }
            IoMessage::Stop => cx.request_termination(),
        }
        Ok(())
    }
}

fn run_mount_request(
    mounts: &mut MountService,
    request: MountRequest,
) -> Result<IoValue, MountError> {
    match request {
        MountRequest::ReadFile { url, maximum_bytes } => {
            mounts.read_uri(&url, maximum_bytes).map(IoValue::Bytes)
        }
        MountRequest::WriteFile { url, bytes } => {
            mounts.write_uri(&url, bytes)?;
            Ok(IoValue::Unit)
        }
        MountRequest::CreateDirectory { url } => {
            mounts.create_dir_uri(&url)?;
            Ok(IoValue::Unit)
        }
        MountRequest::RemoveFile { url } => {
            mounts.remove_file_uri(&url)?;
            Ok(IoValue::Unit)
        }
        MountRequest::RemoveDirectory { url } => {
            mounts.remove_dir_uri(&url)?;
            Ok(IoValue::Unit)
        }
        MountRequest::Rename { from, to } => {
            mounts.rename_uri(&from, &to)?;
            Ok(IoValue::Unit)
        }
    }
}

const fn mount_error_code(error: &MountError) -> HostErrorCode {
    match error {
        MountError::MalformedUri
        | MountError::EmptyMountName
        | MountError::InvalidMountName
        | MountError::MalformedPercentEncoding
        | MountError::NulByte
        | MountError::EncodedSeparator
        | MountError::InvalidUtf8
        | MountError::InvalidPath
        | MountError::BoundaryViolation
        | MountError::SymlinkDisallowed
        | MountError::SymlinkLoop
        | MountError::MountNotFound
        | MountError::HandleNotFound => HostErrorCode::InvalidRequest,
        MountError::ReadOnly => HostErrorCode::PermissionDenied,
        MountError::DuplicateMountName | MountError::NotDirectory | MountError::Io => {
            HostErrorCode::OperationFailed
        }
    }
}

struct MainHandler {
    io_sender: Arc<Mutex<Option<IoSender<IoMessage>>>>,
    js: Option<ZjsHost>,
    stdin_token: Option<Token>,
    next_evaluation: u64,
    input: Vec<u8>,
    interactive: bool,
    eof: bool,
    pending_evaluations: usize,
    engine: EngineKind,
}

impl MainHandler {
    fn send_io(&self, message: IoMessage) -> io::Result<()> {
        self.io_sender
            .lock()
            .map_err(|_| io::Error::other("IO sender lock poisoned"))?
            .as_ref()
            .ok_or_else(|| io::Error::other("IO loop unavailable"))?
            .send(message)
            .map_err(|_| io::Error::other("IO loop closed"))
    }

    fn process_engine(&mut self, cx: &mut IoContext<'_, MainMessage>) -> io::Result<()> {
        let Some(js) = self.js.as_mut() else {
            return Ok(());
        };
        let turn = js.drain_events(64, 4 * 1024 * 1024).map_err(engine_io)?;
        while let Some(event) = self.js.as_mut().and_then(ZjsHost::next_event) {
            match event {
                EngineEvent::EvaluationSettled { outcome, .. } => {
                    self.pending_evaluations = self.pending_evaluations.saturating_sub(1);
                    match outcome {
                        EvaluationOutcome::Value(bytes) => {
                            println!("{}", String::from_utf8_lossy(&bytes));
                        }
                        EvaluationOutcome::Exception(error) => {
                            eprintln!("Uncaught {}: {}", error.name, error.message);
                        }
                        EvaluationOutcome::Cancelled => eprintln!("evaluation cancelled"),
                    }
                    if self.interactive {
                        print!("js> ");
                        io::stdout().flush()?;
                    }
                    if self.eof && self.pending_evaluations == 0 {
                        let _ = self.send_io(IoMessage::Stop);
                        cx.request_termination();
                    }
                }
                EngineEvent::ConsoleOutput(bytes) => {
                    eprintln!("{}", String::from_utf8_lossy(&bytes));
                }
                EngineEvent::HostRequest { id, request } => match request {
                    HostRequest::Mount(request) => {
                        self.send_io(IoMessage::Mount {
                            request_id: id,
                            request,
                        })?;
                    }
                    HostRequest::Sleep { nanoseconds } => {
                        self.send_io(IoMessage::Sleep {
                            request_id: id,
                            duration: Duration::from_nanos(nanoseconds),
                        })?;
                    }
                    HostRequest::Invoke { name, input, .. } if name == "dev.zintl.echo" => self
                        .js
                        .as_mut()
                        .unwrap()
                        .complete_host_request(id, HostCompletion::Bytes(input))
                        .map_err(engine_io)?,
                    HostRequest::Invoke { .. } => self
                        .js
                        .as_mut()
                        .unwrap()
                        .complete_host_request(
                            id,
                            HostCompletion::Failed(HostErrorCode::InvalidRequest),
                        )
                        .map_err(engine_io)?,
                },
            }
        }
        if turn.has_more {
            let _ = cx.sender().send(MainMessage::EngineReady);
        }
        Ok(())
    }

    fn evaluate(&mut self, source: String) -> io::Result<()> {
        let id = EvaluationId(self.next_evaluation);
        self.next_evaluation = self
            .next_evaluation
            .checked_add(1)
            .ok_or_else(|| io::Error::other("evaluation IDs exhausted"))?;
        self.js
            .as_mut()
            .ok_or_else(|| io::Error::other("JavaScript host unavailable"))?
            .evaluate(EvaluationRequest { id, source })
            .map_err(engine_io)?;
        self.pending_evaluations += 1;
        Ok(())
    }

    fn consume_input(&mut self, cx: &mut IoContext<'_, MainMessage>, eof: bool) -> io::Result<()> {
        if self.interactive {
            while let Some(newline) = self.input.iter().position(|byte| *byte == b'\n') {
                let line = self.input.drain(..=newline).collect::<Vec<_>>();
                self.consume_line(cx, &String::from_utf8_lossy(&line))?;
            }
            if eof && !self.input.is_empty() {
                let line = std::mem::take(&mut self.input);
                self.consume_line(cx, &String::from_utf8_lossy(&line))?;
            }
        } else if eof && !self.input.is_empty() {
            let source = String::from_utf8(std::mem::take(&mut self.input))
                .map_err(|_| io::Error::other("stdin is not UTF-8"))?;
            self.evaluate(source)?;
        }
        if eof {
            self.eof = true;
            if self.pending_evaluations == 0 {
                let _ = self.send_io(IoMessage::Stop);
                cx.request_termination();
            }
        }
        Ok(())
    }

    fn consume_line(&mut self, cx: &mut IoContext<'_, MainMessage>, line: &str) -> io::Result<()> {
        let source = line.trim();
        match source {
            "" => Ok(()),
            ".exit" | ".quit" => {
                let _ = self.send_io(IoMessage::Stop);
                cx.request_termination();
                Ok(())
            }
            ".help" => {
                println!("Zintl.readFile(uri, 'utf8')");
                println!("Zintl.writeFile(uri, stringOrBytes)");
                println!("Zintl.mkdir(uri)");
                println!("Zintl.rename(from, to)");
                println!("Zintl.removeFile(uri)");
                println!("Zintl.removeDirectory(uri)");
                println!(
                    "Demo: cargo run -p javascript-repl < examples/javascript-repl/demo/mount-operations.js"
                );
                Ok(())
            }
            _ => self.evaluate(source.to_owned()),
        }
    }
}

impl IoMessageHandler<MainMessage> for MainHandler {
    fn init(&mut self, cx: &mut IoContext<'_, MainMessage>) -> io::Result<()> {
        #[cfg(unix)]
        {
            rustix::io::ioctl_fionbio(io::stdin(), true)?;
            let fd = io::stdin().as_raw_fd();
            self.stdin_token = Some(cx.register(&mut SourceFd(&fd), Interest::READABLE)?);
        }
        let notifier = Arc::new(LoopNotifier(cx.sender()));
        self.js = Some(
            ZjsHostBuilder::new(self.engine.backend(), notifier)
                .build()
                .map_err(engine_io)?,
        );
        if self.interactive {
            println!(
                "Zintl JavaScript REPL ({})\nType .help or .exit.",
                self.engine.name()
            );
            print!("js> ");
            io::stdout().flush()?;
        }
        Ok(())
    }
    fn on(&mut self, cx: &mut IoContext<'_, MainMessage>, message: MainMessage) -> io::Result<()> {
        match message {
            MainMessage::EngineReady => self.process_engine(cx),
            MainMessage::MountCreated(Ok(())) => Ok(()),
            MainMessage::MountCreated(Err(error)) => Err(io::Error::other(error)),
            MainMessage::IoCompleted {
                request_id,
                completion,
            } => {
                self.js
                    .as_mut()
                    .ok_or_else(|| io::Error::other("JavaScript host unavailable"))?
                    .complete_host_request(request_id, completion)
                    .map_err(engine_io)?;
                self.process_engine(cx)
            }
        }
    }
    fn on_ready(&mut self, cx: &mut IoContext<'_, MainMessage>, event: &Event) -> io::Result<()> {
        if Some(event.token()) != self.stdin_token {
            return Ok(());
        }
        let mut eof = false;
        let mut chunk = [0; 4096];
        loop {
            match io::stdin().read(&mut chunk) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(count) => self.input.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        self.consume_input(cx, eof)
    }
    fn terminate(&mut self, _cx: &mut IoContext<'_, MainMessage>) -> io::Result<()> {
        if let Some(js) = self.js.as_mut() {
            js.shutdown().map_err(engine_io)?;
        }
        Ok(())
    }
}

fn engine_io(error: runtime_engine::EngineError) -> io::Error {
    io::Error::other(error)
}

/// Runs both message loops until stdin reaches EOF or `.exit` is entered.
///
/// # Errors
/// Returns terminal, engine, mount startup, or message-loop failures.
pub fn run() -> Result<(), ReplError> {
    let Some(engine) = parse_arguments(std::env::args_os().skip(1))? else {
        return Ok(());
    };
    let interactive = io::stdin().is_terminal();
    let io_slot = Arc::new(Mutex::new(None));
    let main_loop = MessageLoopIo::new(MainHandler {
        io_sender: io_slot.clone(),
        js: None,
        stdin_token: None,
        next_evaluation: 1,
        input: Vec::new(),
        interactive,
        eof: false,
        pending_evaluations: 0,
        engine,
    })?;
    let main_sender = main_loop.sender();
    let io_loop = MessageLoopIo::new(IoHandler {
        main_sender,
        mounts: Arc::new(Mutex::new(MountService::default())),
        workers: WorkerPool::new(2)?,
        root: std::env::current_dir()?,
    })?;
    let io_sender = io_loop.sender();
    *io_slot
        .lock()
        .map_err(|_| io::Error::other("IO sender lock poisoned"))? = Some(io_sender);
    let io_thread = thread::Builder::new()
        .name("zintl-io-loop".into())
        .spawn(move || io_loop.run())?;
    let main_result = main_loop.run();
    let io_result = io_thread
        .join()
        .map_err(|_| io::Error::other("IO loop panicked"))?;
    main_result?;
    io_result?;
    Ok(())
}

fn parse_arguments(
    arguments: impl IntoIterator<Item = impl Into<std::ffi::OsString>>,
) -> io::Result<Option<EngineKind>> {
    let mut arguments = arguments.into_iter().map(Into::into);
    let mut engine = None;
    while let Some(argument) = arguments.next() {
        let argument = argument
            .into_string()
            .map_err(|_| io::Error::other("arguments must be UTF-8"))?;
        if argument == "--help" || argument == "-h" {
            print_usage();
            return Ok(None);
        }
        let value = if argument == "--engine" {
            arguments
                .next()
                .ok_or_else(|| io::Error::other("--engine requires jsc or v8"))?
                .into_string()
                .map_err(|_| io::Error::other("engine name must be UTF-8"))?
        } else if let Some(value) = argument.strip_prefix("--engine=") {
            value.to_owned()
        } else {
            return Err(io::Error::other(format!("unknown argument: {argument}")));
        };
        let selected = match value.as_str() {
            "jsc" => EngineKind::JavaScriptCore,
            "v8" => EngineKind::V8,
            _ => return Err(io::Error::other("--engine must be jsc or v8")),
        };
        if engine.replace(selected).is_some() {
            return Err(io::Error::other("--engine may only be specified once"));
        }
    }
    Ok(Some(
        engine.unwrap_or_else(EngineKind::default_for_platform),
    ))
}

fn print_usage() {
    println!("Usage: javascript-repl [--engine jsc|v8]");
    println!("  jsc  JavaScriptCore (macOS only)");
    println!("  v8   V8 through rusty_v8");
}

#[derive(Debug)]
pub struct ReplError(io::Error);
impl fmt::Display for ReplError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl std::error::Error for ReplError {}
impl From<io::Error> for ReplError {
    fn from(error: io::Error) -> Self {
        Self(error)
    }
}

#[cfg(test)]
mod tests {
    use super::{EngineKind, parse_arguments};

    #[test]
    // Verifies both supported spellings select V8 without starting a message loop.
    fn parses_v8_engine_option() {
        assert_eq!(
            parse_arguments(["--engine", "v8"]).unwrap(),
            Some(EngineKind::V8)
        );
        assert_eq!(
            parse_arguments(["--engine=v8"]).unwrap(),
            Some(EngineKind::V8)
        );
    }

    #[test]
    // Verifies an unknown engine is rejected before either backend is initialized.
    fn rejects_unknown_engine() {
        assert!(parse_arguments(["--engine", "unknown"]).is_err());
    }
}
