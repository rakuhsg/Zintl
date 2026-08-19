use zintl_desktop::*;

pub struct MainView {}

impl View for MainView {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        Window::new(Rect::new(100.0, 100.0, 640.0, 400.0), "Zintl")
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), AppError> {
    let app = App::new(MainView {});
    app.run()
}

#[cfg(not(target_os = "macos"))]
fn main() {}
