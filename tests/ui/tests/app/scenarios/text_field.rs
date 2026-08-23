use zintl_desktop::*;

pub struct MainView {
    value: Option<Store<String>>,
}

impl MainView {
    pub const fn new() -> Self {
        Self { value: None }
    }
}

impl View for MainView {
    type Output = RenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.value = Some(cx.store(String::new()));
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        let value = self
            .value
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
                    .bind(value),
                cx.watch(value, |value| {
                    Text::new(format!("Stored value: {value:?}")).id("stored-value")
                }),
            ))
            .spacing(26.0),
        )
    }
}
