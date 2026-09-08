use zintl_ui_desktop::*;

#[derive(Default)]
struct FullSizeContentViewExample {
    full_size_content_view: Store<bool>,
}

impl View for FullSizeContentViewExample {
    type Output = RenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.full_size_content_view = cx.store(false);
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let full_size_content_view = self.full_size_content_view;

        cx.watch(full_size_content_view, move |enabled| {
            let status = if *enabled { "enabled" } else { "disabled" };
            let button_title = if *enabled {
                "Disable full-size content view"
            } else {
                "Enable full-size content view"
            };
            let content = VStack::new((
                Text::new(format!("Full-size content view is {status}.")),
                Text::new(format!("Full-size content view is {status}.")),
                Button::new(button_title)
                    .minimum_size(Size::new(240.0, 32.0))
                    .on_click(move |cx| {
                        cx.update(full_size_content_view, |enabled| *enabled = !*enabled);
                    }),
            ))
            .minimum_size(Size::new(360.0, 120.0))
            .spacing(24.0);
            let window = Window::new(
                Rect::new(100.0, 100.0, 480.0, 300.0),
                "Full-size Content View",
            )
            .sidebar(Sidebar::new([SidebarSection::new([
                SidebarItem::new("home", "Home").system_image("house"),
                SidebarItem::new("settings", "Settings").system_image("gearshape"),
            ])]))
            .content(content);

            if *enabled {
                window.full_size_content_view()
            } else {
                window
            }
        })
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), AppError> {
    App::new(FullSizeContentViewExample::default()).run()
}

#[cfg(not(target_os = "macos"))]
fn main() {}
