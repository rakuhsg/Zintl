use zintl_native::{
    Context, Event, MainActor, MessageHandler, PlatformMessageLoop, Rect, WgpuSurface, Window,
};

enum Message {
    CreateWindow {
        window: MainActor<Window>,
        wgpu_surface: MainActor<WgpuSurface>,
        render_state: RenderState,
    },
}

#[derive(Default)]
struct Handler {
    window: Option<MainActor<Window>>,
    wgpu_surface: Option<MainActor<WgpuSurface>>,
    render_state: Option<RenderState>,
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
            }) => {
                self.window = Some(window);
                self.wgpu_surface = Some(wgpu_surface);
                self.render_state = Some(render_state);
            }
        }
    }
}

struct RenderState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl RenderState {
    fn clear(&mut self, color: wgpu::Color) {
        let frame = self
            .surface
            .get_current_texture()
            .expect("failed to acquire surface texture");
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clear encoder"),
            });

        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        self.queue.submit([encoder.finish()]);
        frame.present();
    }
}

fn create_render_state(
    wgpu_surface: &MainActor<WgpuSurface>,
    marker: zintl_native::MainMarker,
) -> RenderState {
    let native_surface = wgpu_surface.read(marker).unwrap();
    let drawable_size = native_surface.drawable_size();
    assert!(drawable_size.width > 0);
    assert!(drawable_size.height > 0);

    let instance = wgpu::Instance::default();
    // SAFETY: The native `WgpuSurface` owns the CAMetalLayer and is stored in
    // `Handler` so it outlives this smoke-test `wgpu::Surface`.
    let surface = unsafe {
        instance
            .create_surface_unsafe(native_surface.surface_target_unsafe())
            .expect("failed to create wgpu surface")
    };

    let adapter =
        futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("failed to find a compatible wgpu adapter");

    let (device, queue) =
        futures::executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("failed to create wgpu device");

    let capabilities = surface.get_capabilities(&adapter);
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .unwrap_or(capabilities.formats[0]);
    let present_mode = capabilities
        .present_modes
        .iter()
        .copied()
        .find(|mode| *mode == wgpu::PresentMode::Fifo)
        .unwrap_or(capabilities.present_modes[0]);
    let alpha_mode = capabilities.alpha_modes[0];
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: drawable_size.width,
        height: drawable_size.height,
        present_mode,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    RenderState {
        surface,
        device,
        queue,
    }
}

fn main() {
    let handler = Handler::default();
    let m = PlatformMessageLoop::new(handler);
    m.run();
}
