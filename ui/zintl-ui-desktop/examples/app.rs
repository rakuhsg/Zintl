use zintl_ui_desktop::*;

#[derive(Default)]
pub struct MainView {
    name: Store<String>,
    selection: Store<Option<String>>,
    count: Store<i32>,
}

impl View for MainView {
    type Output = RenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.selection = cx.store(Some("home".into()));
        self.name = cx.store(String::new());
        self.count = cx.store(0);
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        let name = self.name;
        let count = self.count;

        Window::new(Rect::new(100.0, 100.0, 640.0, 400.0), "Zintl")
            .sidebar(
                Sidebar::new([SidebarSection::new([
                    SidebarItem::new("home", "Home").system_image("house"),
                    SidebarItem::new("settings", "Settings").system_image("gearshape"),
                ])
                .title("Navigation")])
                .bind(self.selection)
                .on_select(|_cx, id| println!("Selected sidebar item: {id}")),
            )
            .content(
                VStack::new(list![
                    cx.watch(self.selection, |selection| {
                        Text::new(format!(
                            "Selected: {}",
                            selection.as_deref().unwrap_or("none")
                        ))
                    }),
                    TextField::new().placeholder("Your Name").bind(name),
                    cx.watch(name, |name| Text::new(format!("Stored value: {name:?}"))),
                    {
                        cx.watch(name, |name| {
                            if name.len() >= 8 {
                                Text::new("name is too long")
                            } else {
                                Text::new("")
                            }
                        })
                    },
                    HStack::new(list![Button::new("Continue"), Button::new("Cancel")])
                        .spacing(12.0)
                        .fill_width()
                        .equal_width_children(),
                    cx.watch(count, move |c| {
                        Button::new(format!("Counter: {c}")).on_click(move |cx| {
                            cx.update(count, |value| *value += 1);
                        })
                    }),
                    Button::new("Counter-counter").on_click(move |cx| {
                        cx.update(count, |value| *value -= 1);
                    }),
                ])
                .fill_width()
                .spacing(32.0),
            )
            .extend_client_area()
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), AppError> {
    let app = App::new(MainView::default());
    app.run()
}

#[cfg(not(target_os = "macos"))]
fn main() {}
