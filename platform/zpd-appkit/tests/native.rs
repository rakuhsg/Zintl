use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zpd_appkit::actor::{ActorError, ApplicationMessage, WindowEventKind};
use zpd_appkit::geometry::Rect;
use zpd_appkit::runloop::{Application, ApplicationDelegate};
use zpd_appkit::ui::{
    AsView, Button, CommandItem, CommandMenu, CommandModifier, CommandSet, LayoutConstraint,
    Sidebar, SidebarItem, SidebarSection, TextField, View, ViewError,
};
use zpd_appkit::with_autorelease_pool;

struct DropDelegate(Rc<Cell<usize>>);

struct DropProbe(Rc<Cell<usize>>);

impl ApplicationDelegate for DropDelegate {}

impl Drop for DropDelegate {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn main() {
    // Verifies the raw Objective-C backend creates, relates, and tears down native actors on main.
    let application = Application::new(()).unwrap();

    let (window_native, content_native, button_native) = with_autorelease_pool(|| {
        let window = application.create_window().unwrap();
        let content = window.content_view().unwrap();
        let button = Button::with_title(&application, "Child").unwrap();
        content.add_subview(&button).unwrap();

        // Verifies a real Window -> content view -> Button hierarchy is alive
        // in both the Actor Tree and Objective-C before Window destruction.
        let window_actor = window.actor_ref();
        let content_actor = content.actor_ref();
        let button_actor = button.as_view().actor_ref();
        let window_native = window_actor.downgrade_native().unwrap();
        let content_native = content_actor.downgrade_native().unwrap();
        let button_native = button_actor.downgrade_native().unwrap();
        assert!(window_actor.is_alive());
        assert!(content_actor.is_alive());
        assert!(button_actor.is_alive());
        assert!(window_native.is_alive());
        assert!(content_native.is_alive());
        assert!(button_native.is_alive());

        drop(window);
        assert!(!window_actor.is_alive());
        assert!(!content_actor.is_alive());
        assert!(!button_actor.is_alive());
        assert_eq!(button.set_title("expired"), Err(ViewError::Closed));
        drop(button);
        (window_native, content_native, button_native)
    });
    // Verifies autorelease processing leaves no Objective-C object from the
    // destroyed Window subtree alive.
    assert!(!window_native.is_alive());
    assert!(!content_native.is_alive());
    assert!(!button_native.is_alive());

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
    let window_events = Rc::new(RefCell::new(Vec::new()));
    let received_events = window_events.clone();
    let event_registration = application
        .on(move |event| received_events.borrow_mut().push(event.kind))
        .unwrap();
    let window = application.create_window().unwrap();
    let content = window.content_view().unwrap();
    let first_layout_drops = Rc::new(Cell::new(0));
    let first_probe = DropProbe(first_layout_drops.clone());
    window
        .set_content_layout_handler(move |_| {
            let _ = &first_probe;
        })
        .unwrap();
    let layout_calls = Rc::new(RefCell::new(Vec::new()));
    let received_layouts = layout_calls.clone();
    let second_layout_drops = Rc::new(Cell::new(0));
    let second_probe = DropProbe(second_layout_drops.clone());
    let layout_child = Rc::new(View::new(&application, Rect::new(0.0, 0.0, 0.0, 10.0)).unwrap());
    content.add_subview(layout_child.as_ref()).unwrap();
    let resized_child = layout_child.clone();
    window
        .set_content_layout_handler(move |bounds| {
            let _ = &second_probe;
            received_layouts.borrow_mut().push(bounds);
            resized_child
                .set_frame(Rect::new(0.0, 0.0, bounds.width, 10.0))
                .unwrap();
        })
        .unwrap();
    // Verifies replacing a native layout callback releases the previous Rust state once.
    assert_eq!(first_layout_drops.get(), 1);
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
        .set_bounds(Rect::new(100.0, 100.0, 320.0, 200.0))
        .unwrap();
    // Verifies NSWindow size changes still emit the public semantic resize event.
    assert!(window_events.borrow().contains(&WindowEventKind::DidResize));
    // Verifies callers can read the live content size instead of the outer window frame.
    let content_bounds = content.bounds().unwrap();
    assert_eq!(content_bounds.width, 320.0);
    assert!(content_bounds.height < 200.0);
    // Verifies resizing runs the content layout pass before queued resize events are handled.
    assert_eq!(layout_calls.borrow().last().copied(), Some(content_bounds));
    assert_eq!(layout_child.bounds().unwrap().width, content_bounds.width);
    content.set_needs_layout(true).unwrap();
    content.layout_subtree_if_needed().unwrap();
    // Verifies an explicit layout pass also receives current bounds and applies child geometry.
    assert_eq!(layout_calls.borrow().last().copied(), Some(content_bounds));
    assert_eq!(layout_child.bounds().unwrap().width, content_bounds.width);
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
    // Verifies destroying the content view releases its installed Rust layout callback once.
    assert_eq!(second_layout_drops.get(), 1);
    assert_eq!(label.set_string_value("expired"), Err(ViewError::Closed));
    drop(button);
    drop(label);
    drop(event_registration);
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
