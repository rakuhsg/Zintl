use zintl_desktop::*;

pub struct MainView {
    name: Option<Store<String>>,
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

        Window::new(Rect::new(100.0, 100.0, 640.0, 400.0), "Zintl").content(
            VStack::new((
                Text::new("Welcome to Zintl"),
                TextField::new().placeholder("Your Name").bind(name),
                cx.watch(name, |name| Text::new(format!("Stored value: {name:?}"))),
                HStack::new((Button::new("Continue"), Button::new("Cancel"))).spacing(12.0),
            ))
            .spacing(26.0),
        )
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), AppError> {
    let app = App::new(MainView { name: None });
    app.run()
}

#[cfg(not(target_os = "macos"))]
fn main() {}
