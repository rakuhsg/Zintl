use std::cell::Cell;
use std::rc::Rc;

use zpd_appkit::actor::{ActorError, ApplicationMessage};
use zpd_appkit::geometry::Rect;
use zpd_appkit::runloop::{Application, ApplicationDelegate};
use zpd_appkit::ui::{
    AsView, Button, CommandItem, CommandMenu, CommandModifier, CommandSet, LayoutConstraint,
    Sidebar, SidebarItem, SidebarSection, TextField, ViewError,
};

struct DropDelegate(Rc<Cell<usize>>);

impl ApplicationDelegate for DropDelegate {}

impl Drop for DropDelegate {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn main() {
    // Verifies the raw Objective-C backend creates, relates, and tears down native actors on main.
    let application = Application::new(()).unwrap();
    application
        .set_commands(
            &CommandSet {
                app_menu: None,
                menus: vec![CommandMenu {
                    title: "Test".into(),
                    items: vec![CommandItem {
                        id: Some("test.run".into()),
                        title: "Run".into(),
                        role: None,
                        key: Some("r".into()),
                        modifiers: vec![CommandModifier::Cmd],
                        enabled: true,
                    }],
                }],
            },
            |_| {},
        )
        .unwrap();
    let window = application.create_window(()).unwrap();
    let content = window.content_view().unwrap();
    let label = TextField::label_with_string(&application, "Actor Tree").unwrap();
    let button = Button::with_title(&application, "Close").unwrap();
    content.add_subview(&label).unwrap();
    content.add_subview(&button).unwrap();
    label.set_identifier(Some("native-label")).unwrap();
    label
        .set_translates_autoresizing_mask_into_constraints(false)
        .unwrap();
    button
        .set_translates_autoresizing_mask_into_constraints(false)
        .unwrap();
    let constraints = [
        label
            .leading_anchor()
            .constraint_equal_to(content.leading_anchor(), 12.0)
            .unwrap(),
        button
            .top_anchor()
            .constraint_equal_to(label.bottom_anchor(), 8.0)
            .unwrap(),
    ];
    LayoutConstraint::activate(&constraints).unwrap();
    window
        .set_sidebar(
            &Sidebar {
                sections: vec![SidebarSection {
                    title: Some("Library".into()),
                    items: vec![SidebarItem {
                        id: "home".into(),
                        title: "Home".into(),
                        system_image: Some("house".into()),
                    }],
                }],
                selected_id: Some("home".into()),
            },
            |_| {},
        )
        .unwrap();
    window
        .set_bounds(Rect::new(100.0, 100.0, 320.0, 200.0))
        .unwrap();
    #[cfg(feature = "wgpu")]
    {
        // Verifies CAMetalLayer ownership follows its surface actor.
        let surface = window
            .create_wgpu_surface(Rect::new(0.0, 0.0, 64.0, 64.0))
            .unwrap();
        assert!(!surface.metal_layer().unwrap().as_ptr().unwrap().is_null());
        assert!(surface.drawable_size().unwrap().width >= 64);
    }
    drop(constraints);
    drop(window);
    assert_eq!(label.set_string_value("expired"), Err(ViewError::Closed));
    drop(button);
    drop(label);
    let stale_application = application.actor_ref();
    drop(application);
    // Verifies the process-wide NSApp root can host a fresh Application session.
    let drops = Rc::new(Cell::new(0));
    let application = Application::new(DropDelegate(drops.clone())).unwrap();
    assert_eq!(
        stale_application.send(ApplicationMessage::Stop),
        Err(ActorError::NotActive)
    );
    drop(application);
    // Verifies the Actor-owned Objective-C delegate releases its Rust state exactly once.
    assert_eq!(drops.get(), 1);
}
