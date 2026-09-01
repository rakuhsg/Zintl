use zintl_ui_desktop::*;

pub struct MainView;

impl View for MainView {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        Window::new(
            Rect::new(100.0, 100.0, 640.0, 400.0),
            "Zintl Full Width Stack UI Test",
        )
        .id("main-window")
        .content(
            VStack::new((
                HStack::new((
                    TextField::new().id("space-leading").placeholder("Leading"),
                    TextField::new()
                        .id("space-trailing")
                        .placeholder("Trailing"),
                ))
                .spacing(24.0)
                .fill_width()
                .space_between(),
                HStack::new((
                    TextField::new().id("equal-leading").placeholder("Leading"),
                    TextField::new()
                        .id("equal-trailing")
                        .placeholder("Trailing"),
                ))
                .spacing(24.0)
                .fill_width()
                .equal_width_children(),
            ))
            .spacing(24.0)
            .fill_width(),
        )
    }
}
