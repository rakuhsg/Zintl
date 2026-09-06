// Runs native sidebar contracts on AppKit's main thread.
#[path = "../src/native.rs"]
mod native;

use native::sel;
use zpd_appkit::runloop::Application;
use zpd_appkit::ui::{Sidebar, SidebarItem, SidebarSection};

struct SidebarChecks;

impl SidebarChecks {
    fn with_window(operation: impl FnOnce(native::Id)) {
        // SAFETY: This isolated main-thread test creates exactly one NSWindow.
        unsafe {
            let app = native::send_id(
                native::class(b"NSApplication\0"),
                sel(b"sharedApplication\0"),
            );
            let windows = native::send_id(app, sel(b"windows\0"));
            assert_eq!(native::send_u64(windows, sel(b"count\0")), 1);
            operation(native::send_id_u64(windows, sel(b"objectAtIndex:\0"), 0));
        }
    }

    fn model(selected: &str) -> Sidebar {
        Sidebar {
            sections: vec![SidebarSection {
                title: Some("Navigation".into()),
                items: vec![
                    SidebarItem {
                        id: "home".into(),
                        title: "Home".into(),
                        system_image: Some("house".into()),
                    },
                    SidebarItem {
                        id: "settings".into(),
                        title: "Settings".into(),
                        system_image: None,
                    },
                ],
            }],
            selected_id: Some(selected.into()),
        }
    }

    fn item(window: native::Id) -> native::Id {
        // SAFETY: Called while the window owns an installed NSSplitViewController.
        unsafe {
            let split = native::send_id(window, sel(b"contentViewController\0"));
            let items = native::send_id(split, sel(b"splitViewItems\0"));
            native::send_id_u64(items, sel(b"objectAtIndex:\0"), 0)
        }
    }

    fn table(window: native::Id) -> native::Id {
        // SAFETY: The installed sidebar controller owns a scroll view and table.
        unsafe {
            let controller = native::send_id(Self::item(window), sel(b"viewController\0"));
            let scroll = native::send_id(controller, sel(b"view\0"));
            native::send_id(scroll, sel(b"documentView\0"))
        }
    }

    fn toolbar_items(window: native::Id) -> native::Id {
        // SAFETY: The live window has an NSToolbar whose items are retained by it.
        unsafe {
            let toolbar = native::send_id(window, sel(b"toolbar\0"));
            assert!(!toolbar.is_null());
            native::send_id(toolbar, sel(b"items\0"))
        }
    }
}

fn main() {
    let application = Application::new(()).unwrap();
    let window = application.create_window().unwrap();
    let content = window.content_view().unwrap();
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = events.clone();
    let _registration = application
        .on(move |event| {
            if let zpd_appkit::actor::WindowEventKind::SidebarSelectionChanged { id } = event.kind {
                observed.borrow_mut().push(id);
            }
        })
        .unwrap();
    let clicked = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let received = clicked.clone();
    window
        .set_sidebar(&SidebarChecks::model("home"), move |id| {
            *received.borrow_mut() = id.into()
        })
        .unwrap();
    window.show().unwrap();
    SidebarChecks::with_window(|window| {
        // SAFETY: These AppKit objects are owned by the live window on the main thread.
        unsafe {
            let item = SidebarChecks::item(window);
            let controller = native::send_id(item, sel(b"viewController\0"));
            let scroll = native::send_id(controller, sel(b"view\0"));
            let table = native::send_id(scroll, sel(b"documentView\0"));
            // Verifies a scrollable source-list table with fixed-height, headerless rows.
            assert!(native::send_bool(scroll, sel(b"hasVerticalScroller\0")));
            assert_eq!(native::send_i64(table, sel(b"numberOfRows\0")), 3);
            assert_eq!(native::send_i64(table, sel(b"style\0")), 3);
            assert_eq!(native::send_f64(table, sel(b"rowHeight\0")), 32.0);
            assert!(native::send_id(table, sel(b"headerView\0")).is_null());
            assert_eq!(native::send_i64(table, sel(b"selectedRow\0")), 1);
            assert!(clicked.borrow().is_empty());
            assert!(events.borrow().is_empty());
            // Verifies native cells contain the title and symbol, with no button rows.
            let cell = native::send_id_i64_i64_bool(
                table,
                sel(b"viewAtColumn:row:makeIfNecessary:\0"),
                0,
                1,
                true,
            );
            assert!(!cell.is_null());
            assert!(native::send_bool_id(
                cell,
                sel(b"isKindOfClass:\0"),
                native::class(b"NSTableCellView\0")
            ));
            let text = native::send_id(cell, sel(b"textField\0"));
            assert_eq!(
                native::rust_string(native::send_id(text, sel(b"stringValue\0"))),
                "Home"
            );
            let image = native::send_id(cell, sel(b"imageView\0"));
            assert!(!native::send_id(image, sel(b"image\0")).is_null());
            let delegate = native::send_id(table, sel(b"delegate\0"));
            // Verifies group headings cannot be selected through normal table interaction.
            assert!(!native::send_bool_id_i64(
                delegate,
                sel(b"tableView:shouldSelectRow:\0"),
                table,
                0
            ));
            assert!(native::send_bool_id_i64(
                delegate,
                sel(b"tableView:isGroupRow:\0"),
                table,
                0
            ));
            assert!(native::send_bool_id_i64(
                delegate,
                sel(b"tableView:shouldSelectRow:\0"),
                table,
                2
            ));
            // Verifies table selection notifications deliver stable item IDs.
            let indexes = native::send_id_u64(
                native::class(b"NSIndexSet\0"),
                sel(b"indexSetWithIndex:\0"),
                2,
            );
            native::send_void_id_bool(
                table,
                sel(b"selectRowIndexes:byExtendingSelection:\0"),
                indexes,
                false,
            );
            assert_eq!(&*clicked.borrow(), "settings");
            // Verifies forced group selection and deselection do not emit invalid IDs.
            let heading = native::send_id_u64(
                native::class(b"NSIndexSet\0"),
                sel(b"indexSetWithIndex:\0"),
                0,
            );
            native::send_void_id_bool(
                table,
                sel(b"selectRowIndexes:byExtendingSelection:\0"),
                heading,
                false,
            );
            native::send_void_id(table, sel(b"deselectAll:\0"), native::NIL);
            assert_eq!(&*clicked.borrow(), "settings");
            assert_eq!(&*events.borrow(), &["settings"]);
            // Verifies the automatically installed item uses AppKit's sidebar action.
            let toolbar_items = SidebarChecks::toolbar_items(window);
            assert_eq!(native::send_u64(toolbar_items, sel(b"count\0")), 1);
            let toggle = native::send_id_u64(toolbar_items, sel(b"objectAtIndex:\0"), 0);
            assert_eq!(
                native::send_id(toggle, sel(b"action\0")),
                sel(b"toggleSidebar:\0")
            );
            // Verifies the first toolbar item is placed in the window's leading navigation area.
            assert!(native::send_bool(toggle, sel(b"isNavigational\0")));
            assert!(native::send_bool(item, sel(b"canCollapse\0")));
            // Verifies the actual toolbar action resolves through the responder chain.
            let app = native::send_id(
                native::class(b"NSApplication\0"),
                sel(b"sharedApplication\0"),
            );
            let content = native::send_id(window, sel(b"contentView\0"));
            native::send_bool_id(window, sel(b"makeFirstResponder:\0"), content);
            let action = native::send_id(toggle, sel(b"action\0"));
            let target = native::send_id(toggle, sel(b"target\0"));
            assert!(native::send_bool_id_id_id(
                app,
                sel(b"sendAction:to:from:\0"),
                action,
                target,
                toggle
            ));
            assert!(native::send_bool(item, sel(b"isCollapsed\0")));
            // Verifies the same toolbar action can reopen the sidebar.
            assert!(native::send_bool_id_id_id(
                app,
                sel(b"sendAction:to:from:\0"),
                action,
                target,
                toggle
            ));
            assert!(!native::send_bool(item, sel(b"isCollapsed\0")));
            native::send_void_bool(item, sel(b"setCollapsed:\0"), true);
        }
    });
    // Verifies reactive sidebar replacement preserves collapse state and avoids duplicate toggles.
    window
        .set_sidebar(&SidebarChecks::model("settings"), |_| {})
        .unwrap();
    SidebarChecks::with_window(|window| {
        // SAFETY: Both the split item and toolbar belong to the live window.
        unsafe {
            assert!(native::send_bool(
                SidebarChecks::item(window),
                sel(b"isCollapsed\0")
            ));
            assert_eq!(
                native::send_u64(SidebarChecks::toolbar_items(window), sel(b"count\0")),
                1
            );
        }
    });
    SidebarChecks::with_window(|window| {
        // SAFETY: A replacement sidebar owns a new live NSTableView.
        unsafe {
            assert_eq!(
                native::send_i64(SidebarChecks::table(window), sel(b"selectedRow\0")),
                2
            );
        }
    });
    // Verifies long lists keep all rows and can scroll to the last item.
    let mut long = SidebarChecks::model("home");
    long.sections[0]
        .items
        .extend((0..100).map(|index| SidebarItem {
            id: format!("item-{index}"),
            title: format!("Item {index}"),
            system_image: None,
        }));
    window.set_sidebar(&long, |_| {}).unwrap();
    SidebarChecks::with_window(|window| {
        // SAFETY: The split item and table belong to the live test window.
        unsafe {
            native::send_void_bool(SidebarChecks::item(window), sel(b"setCollapsed:\0"), false);
            let table = SidebarChecks::table(window);
            assert_eq!(native::send_i64(table, sel(b"numberOfRows\0")), 103);
            native::send_void_i64(table, sel(b"scrollRowToVisible:\0"), 102);
            assert!(native::send_rect(table, sel(b"visibleRect\0")).origin.y > 0.0);
        }
    });
    // Verifies an empty sidebar has no selected row and emits no initialization callback.
    window
        .set_sidebar(&Sidebar::default(), |_| {
            panic!("unexpected initial selection")
        })
        .unwrap();
    SidebarChecks::with_window(|window| {
        // SAFETY: The empty table remains owned by the installed sidebar.
        unsafe {
            let table = SidebarChecks::table(window);
            assert_eq!(native::send_i64(table, sel(b"numberOfRows\0")), 0);
            assert_eq!(native::send_i64(table, sel(b"selectedRow\0")), -1);
        }
    });
    // Verifies clearing the delegate releases its captured Rust callback state.
    let probe = std::rc::Rc::new(());
    let weak = std::rc::Rc::downgrade(&probe);
    window
        .set_sidebar(&SidebarChecks::model("home"), move |_| {
            let _ = &probe;
        })
        .unwrap();
    assert!(weak.upgrade().is_some());
    // Verifies removing the sidebar also removes its generated toolbar, preserving content.
    window.clear_sidebar().unwrap();
    assert!(weak.upgrade().is_none());
    assert!(content.actor_ref().is_alive());
    SidebarChecks::with_window(|window| {
        // SAFETY: The window remains live after sidebar removal.
        unsafe {
            assert!(native::send_id(window, sel(b"toolbar\0")).is_null());
        }
    });
    // Verifies an existing toolbar and its unrelated items survive sidebar installation/removal.
    let toolbar = native::alloc_init(b"NSToolbar\0");
    SidebarChecks::with_window(|window| {
        // SAFETY: toolbar is retained for the lifetime of these native checks.
        unsafe {
            native::send_void_id(window, sel(b"setToolbar:\0"), toolbar.as_ptr());
            native::send_void_id_i64(
                toolbar.as_ptr(),
                sel(b"insertItemWithItemIdentifier:atIndex:\0"),
                native::nsstring("NSToolbarFlexibleSpaceItem").as_ptr(),
                0,
            );
        }
    });
    window
        .set_sidebar(&SidebarChecks::model("home"), |_| {})
        .unwrap();
    window.clear_sidebar().unwrap();
    SidebarChecks::with_window(|window| {
        // SAFETY: The original toolbar is retained by both the window and this test.
        unsafe {
            assert_eq!(native::send_id(window, sel(b"toolbar\0")), toolbar.as_ptr());
            assert_eq!(
                native::send_u64(SidebarChecks::toolbar_items(window), sel(b"count\0")),
                1
            );
        }
    });
    drop(window);
    drop(toolbar);
}
