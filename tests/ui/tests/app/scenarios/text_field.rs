use zintl_desktop::*;

pub struct MainView {
    name: Option<Store<String>>,
}

impl MainView {
    pub const fn new() -> Self {
        Self { name: None }
    }
}

impl View for MainView {
    type Output = RenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.name = Some(cx.store(String::new()));
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        let name = self
            .name
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
                    .id("name-input")
                    .placeholder("Your Name")
                    .bind(name),
                cx.watch(name, |name| {
                    Text::new(format!("Stored value: {name:?}")).id("stored-value")
                }),
            ))
            .spacing(26.0),
        )
    }
}
