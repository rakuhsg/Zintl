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

#[derive(Default)]
struct Handler {
    window: Option<MainActor<Window>>,
    wgpu_surface: Option<MainActor<WgpuSurface>>,
    render_state: Option<RenderState>,
    vello_renderer: Option<VelloRenderer>,
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
    let handler = Handler::default();
    let m = PlatformMessageLoop::new(handler);
    m.run();
}
