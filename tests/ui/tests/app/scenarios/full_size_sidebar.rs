use zintl_ui_desktop::*;

#[derive(Default)]
pub struct MainView {
    selection: Store<Option<String>>,
}

impl View for MainView {
    type Output = RenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.selection = cx.store(Some("home".into()));
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        Window::new(
            Rect::new(100.0, 100.0, 640.0, 400.0),
            "Zintl Full-size Sidebar UI Test",
        )
        .id("main-window")
        .full_size_content_view()
        .sidebar(
            Sidebar::new([SidebarSection::new([
                SidebarItem::new("home", "Home").system_image("house"),
                SidebarItem::new("settings", "Settings").system_image("gearshape"),
                SidebarItem::new("library", "Library").system_image("books.vertical"),
            ])
            .title("Navigation")])
            .bind(self.selection),
        )
        .content(cx.watch(self.selection, |selection| {
            Text::new(format!(
                "Selected: {}",
                selection.as_deref().unwrap_or("none")
            ))
            .id("selected-item")
        }))
    }
}
