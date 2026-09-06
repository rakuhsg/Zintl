use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::OnceLock;

use super::{Sidebar, SidebarError};
use crate::actor::ActorRef;
use crate::native;
use zpd_objc::{Id, Strong};

struct Row {
    id: Option<String>,
    cell: Strong,
}

struct TableState {
    rows: Vec<Row>,
    ready: Cell<bool>,
    callback: RefCell<Box<dyn FnMut(&str)>>,
}

impl TableState {
    fn with<R: Default>(object: Id, operation: impl FnOnce(&Self) -> R) -> R {
        catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: Our registered delegate owns a boxed Rc until teardown clears its ivar.
            let state = unsafe {
                zpd_objc::get_pointer_ivar::<Rc<Self>>(object, c"_sidebarState".as_ptr())
                    .as_ref()
                    .cloned()
            };
            state.as_deref().map(operation).unwrap_or_default()
        }))
        .unwrap_or_else(|_| std::process::abort())
    }
}

unsafe extern "C" fn row_count(object: Id, _: zpd_objc::Sel, _: Id) -> i64 {
    TableState::with(object, |state| state.rows.len() as i64)
}

unsafe extern "C" fn row_view(object: Id, _: zpd_objc::Sel, _: Id, _: Id, row: i64) -> Id {
    TableState::with(object, |state| {
        state
            .rows
            .get(row as usize)
            .map_or(zpd_objc::NIL, |row| row.cell.as_ptr())
    })
}

unsafe extern "C" fn is_group(object: Id, _: zpd_objc::Sel, _: Id, row: i64) -> bool {
    TableState::with(object, |state| {
        state
            .rows
            .get(row as usize)
            .is_some_and(|row| row.id.is_none())
    })
}

unsafe extern "C" fn should_select(object: Id, _: zpd_objc::Sel, _: Id, row: i64) -> bool {
    TableState::with(object, |state| {
        state
            .rows
            .get(row as usize)
            .is_some_and(|row| row.id.is_some())
    })
}

unsafe extern "C" fn selection_changed(object: Id, _: zpd_objc::Sel, notification: Id) {
    TableState::with(object, |state| {
        if !state.ready.get() {
            return;
        }
        // SAFETY: NSTableView sends this notification with itself as its object.
        let row = unsafe {
            let table =
                zpd_objc::msg_send!(notification, zpd_objc::sel!("object"), () => zpd_objc::Id);
            zpd_objc::msg_send!(table, zpd_objc::sel!("selectedRow"), () => i64)
        };
        if let Some(id) = state
            .rows
            .get(row as usize)
            .and_then(|row| row.id.as_deref())
        {
            (state.callback.borrow_mut())(id);
        }
    });
}

fn release(object: Id) {
    // SAFETY: Clearing the ivar transfers the delegate's sole boxed Rc to Rust.
    unsafe {
        let state = zpd_objc::get_pointer_ivar::<Rc<TableState>>(object, c"_sidebarState".as_ptr());
        zpd_objc::set_pointer_ivar(
            object,
            c"_sidebarState".as_ptr(),
            std::ptr::null_mut::<Rc<TableState>>(),
        );
        if !state.is_null() {
            drop(Box::from_raw(state));
        }
    }
}

unsafe extern "C" fn dealloc(object: Id, _: zpd_objc::Sel) {
    if catch_unwind(AssertUnwindSafe(|| release(object))).is_err() {
        std::process::abort();
    }
    // SAFETY: Our NSObject subclass has released its Rust state and now invokes superclass dealloc.
    unsafe {
        zpd_objc::msg_send_super!(object, zpd_objc::class!("NSObject"), zpd_objc::sel!("dealloc"), () => ());
    }
}

fn delegate_class() -> zpd_objc::Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        zpd_objc::decl!(ZpdSidebarTableDelegate: [zpd_objc::class!("NSObject")] {
            fields { _sidebarState: ptr }
            methods {
                "numberOfRowsInTableView:": "q@:@" => row_count,
                "tableView:viewForTableColumn:row:": "@@:@@q" => row_view,
                "tableView:isGroupRow:": "B@:@q" => is_group,
                "tableView:shouldSelectRow:": "B@:@q" => should_select,
                "tableViewSelectionDidChange:": "v@:@" => selection_changed,
                "dealloc": "v@:" => dealloc,
            }
        }) as usize
    }) as zpd_objc::Class
}

fn constrain(first: Id, attribute: i64, second: Id, second_attribute: i64, constant: f64) {
    // SAFETY: All views share a cell ancestor; attributes and constants describe equalities.
    unsafe {
        let constraint = zpd_objc::msg_send!(zpd_objc::class!("NSLayoutConstraint"), zpd_objc::sel!("constraintWithItem:attribute:relatedBy:toItem:attribute:multiplier:constant:"), ((first): zpd_objc::Id, (attribute): i64, (0): i64, (second): zpd_objc::Id, (second_attribute): i64, (1.0): f64, (constant): f64) => zpd_objc::Id);
        zpd_objc::msg_send!(constraint, zpd_objc::sel!("setActive:"), ((true): bool) => ());
    }
}

fn cell(title: &str, symbol: Option<&str>, group: bool) -> Strong {
    let cell = native::alloc_init(zpd_objc::class!("NSTableCellView"));
    let text = native::alloc_init(zpd_objc::class!("NSTextField"));
    // SAFETY: The retained cell owns its text/image subviews and active layout constraints.
    unsafe {
        zpd_objc::msg_send!(text.as_ptr(), zpd_objc::sel!("setStringValue:"), ((native::nsstring(title).as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(text.as_ptr(), zpd_objc::sel!("setEditable:"), ((false): bool) => ());
        zpd_objc::msg_send!(text.as_ptr(), zpd_objc::sel!("setSelectable:"), ((false): bool) => ());
        zpd_objc::msg_send!(text.as_ptr(), zpd_objc::sel!("setBordered:"), ((false): bool) => ());
        zpd_objc::msg_send!(text.as_ptr(), zpd_objc::sel!("setDrawsBackground:"), ((false): bool) => ());
        zpd_objc::msg_send!(text.as_ptr(), zpd_objc::sel!("setLineBreakMode:"), ((4): i64) => ());
        zpd_objc::msg_send!(text.as_ptr(), zpd_objc::sel!("setTranslatesAutoresizingMaskIntoConstraints:"), ((false): bool) => ());
        zpd_objc::msg_send!(cell.as_ptr(), zpd_objc::sel!("addSubview:"), ((text.as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(cell.as_ptr(), zpd_objc::sel!("setTextField:"), ((text.as_ptr()): zpd_objc::Id) => ());
        constrain(text.as_ptr(), 6, cell.as_ptr(), 6, -4.0);
        constrain(text.as_ptr(), 10, cell.as_ptr(), 10, 0.0);
        if group {
            constrain(text.as_ptr(), 5, cell.as_ptr(), 5, 4.0);
        } else {
            let image = native::alloc_init(zpd_objc::class!("NSImageView"));
            zpd_objc::msg_send!(image.as_ptr(), zpd_objc::sel!("setTranslatesAutoresizingMaskIntoConstraints:"), ((false): bool) => ());
            zpd_objc::msg_send!(cell.as_ptr(), zpd_objc::sel!("addSubview:"), ((image.as_ptr()): zpd_objc::Id) => ());
            zpd_objc::msg_send!(cell.as_ptr(), zpd_objc::sel!("setImageView:"), ((image.as_ptr()): zpd_objc::Id) => ());
            let configuration = zpd_objc::msg_send!(zpd_objc::class!("NSImageSymbolConfiguration"), zpd_objc::sel!("configurationWithPointSize:weight:"), ((15.0): f64, (0.0): f64) => zpd_objc::Id);
            zpd_objc::msg_send!(image.as_ptr(), zpd_objc::sel!("setSymbolConfiguration:"), ((configuration): zpd_objc::Id) => ());
            zpd_objc::msg_send!(image.as_ptr(), zpd_objc::sel!("setContentTintColor:"), ((zpd_objc::msg_send!(zpd_objc::class!("NSColor"), zpd_objc::sel!("labelColor"), () => zpd_objc::Id)): zpd_objc::Id) => ());
            if let Some(symbol) = symbol {
                let icon = zpd_objc::msg_send!(zpd_objc::class!("NSImage"), zpd_objc::sel!("imageWithSystemSymbolName:accessibilityDescription:"), ((native::nsstring(symbol).as_ptr()): zpd_objc::Id, (native::nsstring(title).as_ptr()): zpd_objc::Id) => zpd_objc::Id);
                zpd_objc::msg_send!(image.as_ptr(), zpd_objc::sel!("setImage:"), ((icon): zpd_objc::Id) => ());
            }
            constrain(image.as_ptr(), 5, cell.as_ptr(), 5, 4.0);
            constrain(image.as_ptr(), 10, cell.as_ptr(), 10, 0.0);
            constrain(image.as_ptr(), 7, zpd_objc::NIL, 0, 18.0);
            constrain(image.as_ptr(), 8, zpd_objc::NIL, 0, 18.0);
            constrain(text.as_ptr(), 5, image.as_ptr(), 6, 8.0);
        }
    }
    cell
}

pub(crate) fn install(
    controller: &ActorRef,
    sidebar: &Sidebar,
    callback: impl FnMut(&str) + 'static,
) -> Result<(), SidebarError> {
    let tree = controller.tree_handle().ok_or(SidebarError::Closed)?;
    let scroll = native::alloc_init(zpd_objc::class!("NSScrollView"));
    let scroll_actor = tree
        .insert_child(controller, scroll.clone())
        .map_err(|_| SidebarError::Closed)?;
    let table = native::alloc_init(zpd_objc::class!("NSTableView"));
    let table_actor = tree
        .insert_child(&scroll_actor, table.clone())
        .map_err(|_| SidebarError::Closed)?;
    let mut rows = Vec::new();
    for section in &sidebar.sections {
        if let Some(title) = &section.title {
            rows.push(Row {
                id: None,
                cell: cell(title, None, true),
            });
        }
        for item in &section.items {
            rows.push(Row {
                id: Some(item.id.clone()),
                cell: cell(&item.title, item.system_image.as_deref(), false),
            });
        }
    }
    let selected = rows
        .iter()
        .position(|row| row.id.is_some() && row.id == sidebar.selected_id);
    let state = Rc::new(TableState {
        rows,
        ready: Cell::new(false),
        callback: RefCell::new(Box::new(callback)),
    });
    // SAFETY: The registered NSObject subclass stores a boxed Rc with matching teardown.
    let delegate = unsafe {
        let delegate = Strong::from_retained(zpd_objc::msg_send!(zpd_objc::msg_send!(delegate_class(), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("init"), () => zpd_objc::Id))
        .ok_or(SidebarError::NativeCreationFailed)?;
        zpd_objc::set_pointer_ivar(
            delegate.as_ptr(),
            c"_sidebarState".as_ptr(),
            Box::into_raw(Box::new(state.clone())),
        );
        delegate
    };
    let delegate_actor = tree
        .insert_child(&table_actor, delegate.clone())
        .map_err(|_| SidebarError::Closed)?;
    tree.add_teardown(&delegate_actor, release)
        .map_err(|_| SidebarError::Closed)?;
    tree.add_teardown(&table_actor, |table| {
        // SAFETY: Disconnect non-owning AppKit delegates before their Rust state is released.
        unsafe {
            zpd_objc::msg_send!(table, zpd_objc::sel!("setDelegate:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
            zpd_objc::msg_send!(table, zpd_objc::sel!("setDataSource:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
        }
    })
    .map_err(|_| SidebarError::Closed)?;
    // SAFETY: The controller, scroll view, table, column and delegate are live on the main thread.
    unsafe {
        zpd_objc::msg_send!(scroll.as_ptr(), zpd_objc::sel!("setDrawsBackground:"), ((false): bool) => ());
        zpd_objc::msg_send!(scroll.as_ptr(), zpd_objc::sel!("setHasVerticalScroller:"), ((true): bool) => ());
        zpd_objc::msg_send!(scroll.as_ptr(), zpd_objc::sel!("setAutohidesScrollers:"), ((true): bool) => ());
        let column = Strong::from_retained(zpd_objc::msg_send!(zpd_objc::msg_send!(zpd_objc::class!("NSTableColumn"), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("initWithIdentifier:"), ((native::nsstring("items").as_ptr()): zpd_objc::Id) => zpd_objc::Id))
        .ok_or(SidebarError::NativeCreationFailed)?;
        zpd_objc::msg_send!(column.as_ptr(), zpd_objc::sel!("setResizingMask:"), ((1): u64) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("addTableColumn:"), ((column.as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setHeaderView:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setStyle:"), ((3): i64) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setRowSizeStyle:"), ((0): i64) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setBackgroundColor:"), ((zpd_objc::msg_send!(zpd_objc::class!("NSColor"), zpd_objc::sel!("clearColor"), () => zpd_objc::Id)): zpd_objc::Id) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setRowHeight:"), ((32.0): f64) => ());

        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setAllowsMultipleSelection:"), ((false): bool) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setDataSource:"), ((delegate.as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("setDelegate:"), ((delegate.as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(scroll.as_ptr(), zpd_objc::sel!("setDocumentView:"), ((table.as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("reloadData"), () => ());
        if let Some(row) = selected {
            let indexes = zpd_objc::msg_send!(zpd_objc::class!("NSIndexSet"), zpd_objc::sel!("indexSetWithIndex:"), ((row as u64): u64) => zpd_objc::Id);
            zpd_objc::msg_send!(table.as_ptr(), zpd_objc::sel!("selectRowIndexes:byExtendingSelection:"), ((indexes): zpd_objc::Id, (false): bool) => ());
        }
    }
    controller
        .with(|controller| {
            // SAFETY: The controller retains the installed scroll view.
            unsafe {
                zpd_objc::msg_send!(controller, zpd_objc::sel!("setView:"), ((scroll.as_ptr()): zpd_objc::Id) => ());
            }
        })
        .map_err(|_| SidebarError::Closed)?;
    state.ready.set(true);
    Ok(())
}
