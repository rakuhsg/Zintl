use zintl_native::{
    Context, Event, MainActor, MessageHandler, PlatformMessageLoop, Rect, WgpuSurface, Window,
};

enum Message {
    CreateWindow {
        window: MainActor<Window>,
        wgpu_surface: MainActor<WgpuSurface>,
        surface: wgpu::Surface<'static>,
    },
}

#[derive(Default)]
struct Handler {
    window: Option<MainActor<Window>>,
    wgpu_surface: Option<MainActor<WgpuSurface>>,
    surface: Option<wgpu::Surface<'static>>,
}

impl MessageHandler<Message> for Handler {
    fn on_init(&mut self, cx: impl Context<Message>) {
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
                let surface = create_smoke_surface(&wgpu_surface, marker);
                cx.send_message(Message::CreateWindow {
                    window,
                    wgpu_surface,
                    surface,
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
                surface,
            }) => {
                self.window = Some(window);
                self.wgpu_surface = Some(wgpu_surface);
                self.surface = Some(surface);
            }
        }
    }
}

fn create_smoke_surface(
    wgpu_surface: &MainActor<WgpuSurface>,
    marker: zintl_native::MainMarker,
) -> wgpu::Surface<'static> {
    let surface = wgpu_surface.read(marker).unwrap();
    let drawable_size = surface.drawable_size();
    assert!(drawable_size.width > 0);
    assert!(drawable_size.height > 0);

    let instance = wgpu::Instance::default();
    // SAFETY: The native `WgpuSurface` owns the CAMetalLayer and is stored in
    // `Handler` so it outlives this smoke-test `wgpu::Surface`.
    unsafe {
        instance
            .create_surface_unsafe(surface.surface_target_unsafe())
            .expect("failed to create wgpu surface")
    }
}

fn main() {
    let handler = Handler::default();
    let m = PlatformMessageLoop::new(handler);
    m.run();
}
