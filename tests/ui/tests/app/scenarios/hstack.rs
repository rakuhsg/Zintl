use zintl_desktop::*;

pub struct MainView;

impl View for MainView {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        Window::new(
            Rect::new(100.0, 100.0, 640.0, 400.0),
            "Zintl HStack UI Test",
        )
        .id("main-window")
        .content(
            HStack::new((
                TextField::new().id("leading-field").placeholder("Leading"),
                TextField::new()
                    .id("trailing-field")
                    .placeholder("Trailing"),
            ))
            .id("field-row")
            .spacing(24.0),
        )
    }
}
