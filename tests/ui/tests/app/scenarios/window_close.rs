use zintl_ui_desktop::*;

pub struct MainView;

impl View for MainView {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        Window::new(
            Rect::new(100.0, 100.0, 640.0, 400.0),
            "Zintl Window Close UI Test",
        )
        .id("main-window")
        .content(Text::new("Close this window").id("close-instruction"))
    }
}
