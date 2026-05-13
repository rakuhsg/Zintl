use std::path::PathBuf;
use std::thread;

use zintl_native::{
    Context, Event, MainActor, MessageHandler, PlatformMessageLoop, Rect, WgpuSurface, Window,
};
use zintl_render::VelloRenderer;
use zintl_wgpu::RenderState;

enum Message {
    CreateWindow {
        window: MainActor<Window>,
        wgpu_surface: MainActor<WgpuSurface>,
        render_state: RenderState,
        vello_renderer: VelloRenderer,
    },
}

struct Handler {
    main_module: PathBuf,
    js_thread: Option<thread::JoinHandle<()>>,
    window: Option<MainActor<Window>>,
    wgpu_surface: Option<MainActor<WgpuSurface>>,
    render_state: Option<RenderState>,
    vello_renderer: Option<VelloRenderer>,
}

impl Handler {
    fn new(main_module: PathBuf) -> Self {
        Handler {
            main_module,
            js_thread: None,
            window: None,
            wgpu_surface: None,
            render_state: None,
            vello_renderer: None,
        }
    }
}

impl MessageHandler<Message> for Handler {
    fn on_init(&mut self, cx: impl Context<Message>) {
        self.js_thread = Some(start_js_thread(self.main_module.clone()));

        let wm = cx.window_manager();
        cx.perform_main(
            move |marker, cx| {
                let window = wm.create_window(marker);
                window.read(marker).unwrap().show();
                let wgpu_surface = window.read(marker).unwrap().create_wgpu_surface(
                    marker,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 480.0,
                        height: 300.0,
                    },
                );
                let mut render_state = create_render_state(&wgpu_surface, marker);
                let vello_renderer = VelloRenderer::new(render_state.device())
                    .expect("failed to create vello renderer");
                render_state.clear(wgpu::Color {
                    r: 0.08,
                    g: 0.12,
                    b: 1.0,
                    a: 1.0,
                });
                cx.send_message(Message::CreateWindow {
                    window,
                    wgpu_surface,
                    render_state,
                    vello_renderer,
                });
            },
            None,
        );
    }

    fn on_event(&mut self, _cx: impl Context<Message>, event: Event<Message>) {
        match event {
            Event::UserMessage(Message::CreateWindow {
                window,
                wgpu_surface,
                render_state,
                vello_renderer,
            }) => {
                self.window = Some(window);
                self.wgpu_surface = Some(wgpu_surface);
                self.render_state = Some(render_state);
                self.vello_renderer = Some(vello_renderer);
            }
        }
    }
}

fn start_js_thread(main_module: PathBuf) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("zintl-js".to_string())
        .spawn(move || {
            let runtime = zintl_deno::DenoRuntime::from_file_path(main_module);
            if let Err(error) = runtime.run_current_thread() {
                eprintln!("zintl-js: {error}");
            }
        })
        .expect("failed to spawn JS thread")
}

fn create_render_state(
    wgpu_surface: &MainActor<WgpuSurface>,
    marker: zintl_native::MainMarker,
) -> RenderState {
    let native_surface = wgpu_surface.read(marker).unwrap();
    let drawable_size = native_surface.drawable_size();

    // SAFETY: The native `WgpuSurface` owns the CAMetalLayer and is stored in
    // `Handler` so it outlives the `wgpu::Surface` owned by `RenderState`.
    unsafe {
        RenderState::new_from_surface_target(
            native_surface.surface_target_unsafe(),
            drawable_size.width,
            drawable_size.height,
        )
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
