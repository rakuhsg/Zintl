use zintl_ui_desktop::*;

pub struct MainView {
    single_line_value: Option<Store<String>>,
    multiline_value: Option<Store<String>>,
}

impl MainView {
    pub const fn new() -> Self {
        Self {
            single_line_value: None,
            multiline_value: None,
        }
    }
}

impl View for MainView {
    type Output = RenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.single_line_value = Some(cx.store(String::new()));
        self.multiline_value = Some(cx.store(String::new()));
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        let single_line_value = self
            .single_line_value
            .expect("MainView must be initialized before rendering");
        let multiline_value = self
            .multiline_value
            .expect("MainView must be initialized before rendering");

        Window::new(
            Rect::new(100.0, 100.0, 640.0, 400.0),
            "Zintl Text Field UI Test",
        )
        .id("main-window")
        .content(
            VStack::new((
                Text::new("Welcome to Zintl").id("welcome-text"),
                TextField::new()
                    .id("text-input")
                    .placeholder("Type here")
                    .bind(single_line_value),
                cx.watch(single_line_value, |value| {
                    Text::new(format!("Stored value: {value:?}")).id("stored-value")
                }),
                TextField::new()
                    .id("multiline-input")
                    .placeholder("Type multiple lines here")
                    .multiline()
                    .minimum_size(Size::new(320.0, 100.0))
                    .bind(multiline_value),
                cx.watch(multiline_value, |value| {
                    Text::new(format!("Stored multiline value: {value:?}"))
                        .id("stored-multiline-value")
                }),
            ))
            .spacing(18.0),
        )
    }
}
