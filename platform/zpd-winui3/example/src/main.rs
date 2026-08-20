#[cfg(target_os = "windows")]
fn main() -> zpd_winui3::Result<()> {
    use std::cell::RefCell;
    use std::thread;
    use std::time::Duration;

    use zpd_winui3::{
        Application, AsElement, Button, DispatcherQueuePriority, Grid, GridLength, Orientation,
        StackPanel, SystemBackdrop, TextBlock, TextBox, Thickness,
    };

    thread_local! {
        static STATUS: RefCell<Option<TextBlock>> = const { RefCell::new(None) };
    }

    Application::new()
        .expect("application already created")
        .run(|context| {
            let window = context.create_window().expect("window creation failed");
            window.set_title("zpd-winui3 example").unwrap();
            window.resize(720, 440).unwrap();
            window.set_system_backdrop(SystemBackdrop::Mica).unwrap();
            window.set_extends_content_into_title_bar(true).unwrap();

            let root = StackPanel::new(Orientation::Vertical, 12.0).unwrap();
            root.set_margin(Thickness::uniform(24.0)).unwrap();
            let heading = TextBlock::new("WinUI 3 from Rust").unwrap();
            let input = TextBox::new("").unwrap();
            input.set_placeholder_text("Your name").unwrap();
            let button = Button::new("Greet").unwrap();
            let status = TextBlock::new("Waiting for DispatcherQueue…").unwrap();

            let form = Grid::new().unwrap();
            form.set_column_definitions(&[GridLength::Star(1.0), GridLength::Auto])
                .unwrap();
            form.add(&input, 0, 0, 1, 1).unwrap();
            form.add(&button, 0, 1, 1, 1).unwrap();
            root.append(&heading).unwrap();
            root.append(&form).unwrap();
            root.append(&status).unwrap();
            STATUS.with(|slot| slot.replace(Some(status)));
            window.set_title_bar(Some(&heading)).unwrap();

            button
                .set_click(move || println!("Hello, {}!", input.text().unwrap()))
                .unwrap();
            window
                .set_menu_bar(&menu_bar(), |id| println!("menu command: {id}"))
                .unwrap();
            window.set_content(Some(&root)).unwrap();
            window.activate().unwrap();

            let dispatcher = context.dispatcher_queue();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(750));
                dispatcher.try_enqueue(DispatcherQueuePriority::Normal, move || {
                    STATUS.with(|slot| {
                        slot.borrow()
                            .as_ref()
                            .unwrap()
                            .set_text("DispatcherQueue task ran on the UI thread")
                            .unwrap();
                    });
                });
            });

            // Application keeps this state alive until the WinUI event loop exits.
            (window, root, heading, form, button)
        })
}

#[cfg(target_os = "windows")]
fn menu_bar() -> zpd_winui3::MenuBar {
    use zpd_winui3::{
        KeyboardAccelerator, MenuBar, MenuBarItem, MenuFlyoutItem, MenuFlyoutItemBase,
        MenuFlyoutSubItem, VirtualKeyModifiers,
    };

    MenuBar {
        items: vec![MenuBarItem {
            title: "File".into(),
            items: vec![
                MenuFlyoutItemBase::Item(MenuFlyoutItem {
                    id: "file.new".into(),
                    text: "New".into(),
                    is_enabled: true,
                    keyboard_accelerator: Some(KeyboardAccelerator {
                        key: "N".into(),
                        modifiers: VirtualKeyModifiers::CONTROL,
                    }),
                }),
                MenuFlyoutItemBase::Separator,
                MenuFlyoutItemBase::SubItem(MenuFlyoutSubItem {
                    text: "Recent".into(),
                    is_enabled: true,
                    items: vec![MenuFlyoutItemBase::Item(MenuFlyoutItem {
                        id: "file.recent.example".into(),
                        text: "Example.zintl".into(),
                        is_enabled: true,
                        keyboard_accelerator: None,
                    })],
                }),
            ],
        }],
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("zpd-winui3-example requires Windows");
}
