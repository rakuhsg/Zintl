use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use zintl_deno::api::{ZintlApi, ZintlWindow, ZintlWindowError};
use zintl_deno::runtime::{DenoRuntime, DenoRuntimeOptions};
use zintl_native::{Context, Event, MainActor, MessageHandler, PlatformMessageLoop, Window};

enum Message {
    WindowCreated { window: MainActor<Window> },
}

struct Handler {
    main_module: PathBuf,
    js_thread: Option<thread::JoinHandle<()>>,
    windows: Vec<MainActor<Window>>,
}

impl Handler {
    fn new(main_module: PathBuf) -> Self {
        Handler {
            main_module,
            js_thread: None,
            windows: Vec::new(),
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
    fn on_init(&mut self, cx: impl Context<Message>) {
        self.start_js_thread(DenoRuntimeOptions {
            api: ZintlApi {
                window: Some(Arc::new(AppWindowHost { cx })),
            },
        });
    }

    fn on_event(&mut self, _cx: impl Context<Message>, event: Event<Message>) {
        match event {
            Event::UserMessage(Message::WindowCreated { window }) => {
                self.windows.push(window);
            }
        }
    }
}

struct AppWindowHost<C> {
    cx: C,
}

impl<C> ZintlWindow for AppWindowHost<C>
where
    C: Context<Message> + Send + Sync,
{
    fn create_window(&self) -> Result<(), ZintlWindowError> {
        let wm = self.cx.window_manager();
        self.cx.perform_main(
            move |marker, cx| {
                let window = wm.create_window(marker);
                window.read(marker).unwrap().show();
                cx.send_message(Message::WindowCreated { window });
            },
            None,
        );
        Ok(())
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
