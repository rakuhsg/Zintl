pub use vello::{Scene, peniko};

pub struct VelloRenderer {
    renderer: vello::Renderer,
}

impl VelloRenderer {
    pub fn new(device: &wgpu::Device) -> Result<Self, vello::Error> {
        let renderer = vello::Renderer::new(device, vello::RendererOptions::default())?;

        Ok(VelloRenderer { renderer })
    }

    pub fn renderer(&mut self) -> &mut vello::Renderer {
        &mut self.renderer
    }
}
