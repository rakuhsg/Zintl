// Runs native sidebar contracts on AppKit's main thread.
#[path = "../src/native.rs"]
mod native;

use zpd_appkit::runloop::Application;
use zpd_appkit::ui::{Sidebar, SidebarItem, SidebarSection};

struct SidebarChecks;

impl SidebarChecks {
    fn with_window(operation: impl FnOnce(zpd_objc::Id)) {
        // SAFETY: This isolated main-thread test creates exactly one NSWindow.
        unsafe {
            let app = zpd_objc::msg_send!(zpd_objc::class!("NSApplication"), zpd_objc::sel!("sharedApplication"), () => zpd_objc::Id);
            let windows = zpd_objc::msg_send!(app, zpd_objc::sel!("windows"), () => zpd_objc::Id);
            assert_eq!(
                zpd_objc::msg_send!(windows, zpd_objc::sel!("count"), () => u64),
                1
            );
            operation(
                zpd_objc::msg_send!(windows, zpd_objc::sel!("objectAtIndex:"), ((0): u64) => zpd_objc::Id),
            );
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

    fn item(window: zpd_objc::Id) -> zpd_objc::Id {
        // SAFETY: Called while the window owns an installed NSSplitViewController.
        unsafe {
            let split = zpd_objc::msg_send!(window, zpd_objc::sel!("contentViewController"), () => zpd_objc::Id);
            let items =
                zpd_objc::msg_send!(split, zpd_objc::sel!("splitViewItems"), () => zpd_objc::Id);
            zpd_objc::msg_send!(items, zpd_objc::sel!("objectAtIndex:"), ((0): u64) => zpd_objc::Id)
        }
    }

    fn table(window: zpd_objc::Id) -> zpd_objc::Id {
        // SAFETY: The installed sidebar controller owns a scroll view and table.
        unsafe {
            let controller = zpd_objc::msg_send!(Self::item(window), zpd_objc::sel!("viewController"), () => zpd_objc::Id);
            let scroll =
                zpd_objc::msg_send!(controller, zpd_objc::sel!("view"), () => zpd_objc::Id);
            zpd_objc::msg_send!(scroll, zpd_objc::sel!("documentView"), () => zpd_objc::Id)
        }
    }

    fn toolbar_items(window: zpd_objc::Id) -> zpd_objc::Id {
        // SAFETY: The live window has an NSToolbar whose items are retained by it.
        unsafe {
            let toolbar =
                zpd_objc::msg_send!(window, zpd_objc::sel!("toolbar"), () => zpd_objc::Id);
            assert!(!toolbar.is_null());
            zpd_objc::msg_send!(toolbar, zpd_objc::sel!("items"), () => zpd_objc::Id)
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
            let controller =
                zpd_objc::msg_send!(item, zpd_objc::sel!("viewController"), () => zpd_objc::Id);
            let scroll =
                zpd_objc::msg_send!(controller, zpd_objc::sel!("view"), () => zpd_objc::Id);
            let table =
                zpd_objc::msg_send!(scroll, zpd_objc::sel!("documentView"), () => zpd_objc::Id);
            // Verifies a scrollable source-list table with fixed-height, headerless rows.
            assert!(zpd_objc::msg_send!(scroll, zpd_objc::sel!("hasVerticalScroller"), () => bool));
            assert_eq!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("numberOfRows"), () => i64),
                3
            );
            assert_eq!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("style"), () => i64),
                3
            );
            assert_eq!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("rowHeight"), () => f64),
                32.0
            );
            assert!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("headerView"), () => zpd_objc::Id)
                    .is_null()
            );
            assert_eq!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("selectedRow"), () => i64),
                1
            );
            assert!(clicked.borrow().is_empty());
            assert!(events.borrow().is_empty());
            // Verifies native cells contain the title and symbol, with no button rows.
            let cell = zpd_objc::msg_send!(table, zpd_objc::sel!("viewAtColumn:row:makeIfNecessary:"), ((0): i64, (1): i64, (true): bool) => zpd_objc::Id);
            assert!(!cell.is_null());
            assert!(
                zpd_objc::msg_send!(cell, zpd_objc::sel!("isKindOfClass:"), ((zpd_objc::class!("NSTableCellView")): zpd_objc::Id) => bool)
            );
            let text = zpd_objc::msg_send!(cell, zpd_objc::sel!("textField"), () => zpd_objc::Id);
            assert_eq!(
                native::rust_string(
                    zpd_objc::msg_send!(text, zpd_objc::sel!("stringValue"), () => zpd_objc::Id)
                ),
                "Home"
            );
            let image = zpd_objc::msg_send!(cell, zpd_objc::sel!("imageView"), () => zpd_objc::Id);
            assert!(
                !zpd_objc::msg_send!(image, zpd_objc::sel!("image"), () => zpd_objc::Id).is_null()
            );
            let delegate =
                zpd_objc::msg_send!(table, zpd_objc::sel!("delegate"), () => zpd_objc::Id);
            // Verifies group headings cannot be selected through normal table interaction.
            assert!(
                !zpd_objc::msg_send!(delegate, zpd_objc::sel!("tableView:shouldSelectRow:"), ((table): zpd_objc::Id, (0): i64) => bool)
            );
            assert!(
                zpd_objc::msg_send!(delegate, zpd_objc::sel!("tableView:isGroupRow:"), ((table): zpd_objc::Id, (0): i64) => bool)
            );
            assert!(
                zpd_objc::msg_send!(delegate, zpd_objc::sel!("tableView:shouldSelectRow:"), ((table): zpd_objc::Id, (2): i64) => bool)
            );
            // Verifies table selection notifications deliver stable item IDs.
            let indexes = zpd_objc::msg_send!(zpd_objc::class!("NSIndexSet"), zpd_objc::sel!("indexSetWithIndex:"), ((2): u64) => zpd_objc::Id);
            zpd_objc::msg_send!(table, zpd_objc::sel!("selectRowIndexes:byExtendingSelection:"), ((indexes): zpd_objc::Id, (false): bool) => ());
            assert_eq!(&*clicked.borrow(), "settings");
            // Verifies forced group selection and deselection do not emit invalid IDs.
            let heading = zpd_objc::msg_send!(zpd_objc::class!("NSIndexSet"), zpd_objc::sel!("indexSetWithIndex:"), ((0): u64) => zpd_objc::Id);
            zpd_objc::msg_send!(table, zpd_objc::sel!("selectRowIndexes:byExtendingSelection:"), ((heading): zpd_objc::Id, (false): bool) => ());
            zpd_objc::msg_send!(table, zpd_objc::sel!("deselectAll:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
            assert_eq!(&*clicked.borrow(), "settings");
            assert_eq!(&*events.borrow(), &["settings"]);
            // Verifies the automatically installed item uses AppKit's sidebar action.
            let toolbar_items = SidebarChecks::toolbar_items(window);
            assert_eq!(
                zpd_objc::msg_send!(toolbar_items, zpd_objc::sel!("count"), () => u64),
                1
            );
            let toggle = zpd_objc::msg_send!(toolbar_items, zpd_objc::sel!("objectAtIndex:"), ((0): u64) => zpd_objc::Id);
            assert_eq!(
                zpd_objc::msg_send!(toggle, zpd_objc::sel!("action"), () => zpd_objc::Id),
                zpd_objc::sel!("toggleSidebar:")
            );
            // Verifies the first toolbar item is placed in the window's leading navigation area.
            assert!(zpd_objc::msg_send!(toggle, zpd_objc::sel!("isNavigational"), () => bool));
            assert!(zpd_objc::msg_send!(item, zpd_objc::sel!("canCollapse"), () => bool));
            // Verifies the actual toolbar action resolves through the responder chain.
            let app = zpd_objc::msg_send!(zpd_objc::class!("NSApplication"), zpd_objc::sel!("sharedApplication"), () => zpd_objc::Id);
            let content =
                zpd_objc::msg_send!(window, zpd_objc::sel!("contentView"), () => zpd_objc::Id);
            zpd_objc::msg_send!(window, zpd_objc::sel!("makeFirstResponder:"), ((content): zpd_objc::Id) => bool);
            let action = zpd_objc::msg_send!(toggle, zpd_objc::sel!("action"), () => zpd_objc::Id);
            let target = zpd_objc::msg_send!(toggle, zpd_objc::sel!("target"), () => zpd_objc::Id);
            assert!(
                zpd_objc::msg_send!(app, zpd_objc::sel!("sendAction:to:from:"), ((action): zpd_objc::Id, (target): zpd_objc::Id, (toggle): zpd_objc::Id) => bool)
            );
            assert!(zpd_objc::msg_send!(item, zpd_objc::sel!("isCollapsed"), () => bool));
            // Verifies the same toolbar action can reopen the sidebar.
            assert!(
                zpd_objc::msg_send!(app, zpd_objc::sel!("sendAction:to:from:"), ((action): zpd_objc::Id, (target): zpd_objc::Id, (toggle): zpd_objc::Id) => bool)
            );
            assert!(!zpd_objc::msg_send!(item, zpd_objc::sel!("isCollapsed"), () => bool));
            zpd_objc::msg_send!(item, zpd_objc::sel!("setCollapsed:"), ((true): bool) => ());
        }
    });
    // Verifies reactive sidebar replacement preserves collapse state and avoids duplicate toggles.
    window
        .set_sidebar(&SidebarChecks::model("settings"), |_| {})
        .unwrap();
    SidebarChecks::with_window(|window| {
        // SAFETY: Both the split item and toolbar belong to the live window.
        unsafe {
            assert!(
                zpd_objc::msg_send!(SidebarChecks::item(window), zpd_objc::sel!("isCollapsed"), () => bool)
            );
            assert_eq!(
                zpd_objc::msg_send!(SidebarChecks::toolbar_items(window), zpd_objc::sel!("count"), () => u64),
                1
            );
        }
    });
    SidebarChecks::with_window(|window| {
        // SAFETY: A replacement sidebar owns a new live NSTableView.
        unsafe {
            assert_eq!(
                zpd_objc::msg_send!(SidebarChecks::table(window), zpd_objc::sel!("selectedRow"), () => i64),
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
            zpd_objc::msg_send!(SidebarChecks::item(window), zpd_objc::sel!("setCollapsed:"), ((false): bool) => ());
            let table = SidebarChecks::table(window);
            assert_eq!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("numberOfRows"), () => i64),
                103
            );
            zpd_objc::msg_send!(table, zpd_objc::sel!("scrollRowToVisible:"), ((102): i64) => ());
            assert!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("visibleRect"), () => native::Rect)
                    .origin
                    .y
                    > 0.0
            );
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
            assert_eq!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("numberOfRows"), () => i64),
                0
            );
            assert_eq!(
                zpd_objc::msg_send!(table, zpd_objc::sel!("selectedRow"), () => i64),
                -1
            );
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
            assert!(
                zpd_objc::msg_send!(window, zpd_objc::sel!("toolbar"), () => zpd_objc::Id)
                    .is_null()
            );
        }
    });
    // Verifies an existing toolbar and its unrelated items survive sidebar installation/removal.
    let toolbar = native::alloc_init(zpd_objc::class!("NSToolbar"));
    SidebarChecks::with_window(|window| {
        // SAFETY: toolbar is retained for the lifetime of these native checks.
        unsafe {
            zpd_objc::msg_send!(window, zpd_objc::sel!("setToolbar:"), ((toolbar.as_ptr()): zpd_objc::Id) => ());
            zpd_objc::msg_send!(toolbar.as_ptr(), zpd_objc::sel!("insertItemWithItemIdentifier:atIndex:"), ((native::nsstring("NSToolbarFlexibleSpaceItem").as_ptr()): zpd_objc::Id, (0): i64) => ());
        }
    });
    window
        .set_sidebar(&SidebarChecks::model("home"), |_| {})
        .unwrap();
    window.clear_sidebar().unwrap();
    SidebarChecks::with_window(|window| {
        // SAFETY: The original toolbar is retained by both the window and this test.
        unsafe {
            assert_eq!(
                zpd_objc::msg_send!(window, zpd_objc::sel!("toolbar"), () => zpd_objc::Id),
                toolbar.as_ptr()
            );
            assert_eq!(
                zpd_objc::msg_send!(SidebarChecks::toolbar_items(window), zpd_objc::sel!("count"), () => u64),
                1
            );
        }
    });
    drop(window);
    drop(toolbar);
}
