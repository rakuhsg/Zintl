mod api;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use api::{AppWindowHost, AppWindowState, Message};
use zintl_deno::api::{ZintlApi, ZintlAppApi, ZintlWindowApi};
use zintl_deno::runtime::{DenoRuntime, DenoRuntimeOptions};
use zintl_native::{Context, Event, MainMarker, MessageHandler, PlatformMessageLoop};

struct Handler {
    main_module: PathBuf,
    js_thread: Option<thread::JoinHandle<()>>,
    window_state: Arc<AppWindowState>,
}

impl Handler {
    fn new(main_module: PathBuf) -> Self {
        Handler {
            main_module,
            js_thread: None,
            window_state: Arc::new(AppWindowState::default()),
        }
    }

    fn start_js_thread(&mut self, options: DenoRuntimeOptions) {
        let main_module = self.main_module.clone();
        self.js_thread = Some(
            thread::Builder::new()
                .name("zintl-js".to_string())
                .spawn(move || {
                    if let Err(error) =
                        DenoRuntime::run_file_path_current_thread_with_options(main_module, options)
                    {
                        eprintln!("zintl-js: {error}");
                    }
                })
                .expect("failed to spawn JS thread"),
        );
    }
}

impl MessageHandler<Message> for Handler {
    fn on_init(&mut self, _marker: MainMarker, cx: impl Context<Message>) {
        let host = Arc::new(AppWindowHost::new(cx, self.window_state.clone()));
        let app: Arc<dyn ZintlAppApi> = host.clone();
        let window: Arc<dyn ZintlWindowApi> = host;
        self.start_js_thread(DenoRuntimeOptions {
            api: ZintlApi {
                app: Some(app),
                window: Some(window),
            },
        });
    }

    fn on_event(&mut self, _marker: MainMarker, _cx: impl Context<Message>, event: Event<Message>) {
        match event {
            Event::UserMessage(message) => self.window_state.handle_message(message),
        }
    }
}

fn main() {
    let main_module = main_module_from_args();
    let handler = Handler::new(main_module);
    let m = PlatformMessageLoop::new(handler);
    m.run();
}

fn main_module_from_args() -> PathBuf {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: zintl-app <main.js>");
        std::process::exit(2);
    };

    let path = PathBuf::from(path);
    match path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "failed to resolve main module '{}': {error}",
                path.display()
            );
            std::process::exit(2);
        }
    }
}
