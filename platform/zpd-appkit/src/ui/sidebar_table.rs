use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::OnceLock;

use super::{Sidebar, SidebarError};
use crate::actor::ActorRef;
use crate::native::{self, Id, Strong, sel};

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
                native::get_pointer_ivar::<Rc<Self>>(object, c"_sidebarState".as_ptr())
                    .as_ref()
                    .cloned()
            };
            state.as_deref().map(operation).unwrap_or_default()
        }))
        .unwrap_or_else(|_| std::process::abort())
    }
}

unsafe extern "C" fn row_count(object: Id, _: native::Sel, _: Id) -> i64 {
    TableState::with(object, |state| state.rows.len() as i64)
}

unsafe extern "C" fn row_view(object: Id, _: native::Sel, _: Id, _: Id, row: i64) -> Id {
    TableState::with(object, |state| {
        state
            .rows
            .get(row as usize)
            .map_or(native::NIL, |row| row.cell.as_ptr())
    })
}

unsafe extern "C" fn is_group(object: Id, _: native::Sel, _: Id, row: i64) -> bool {
    TableState::with(object, |state| {
        state
            .rows
            .get(row as usize)
            .is_some_and(|row| row.id.is_none())
    })
}

unsafe extern "C" fn should_select(object: Id, _: native::Sel, _: Id, row: i64) -> bool {
    TableState::with(object, |state| {
        state
            .rows
            .get(row as usize)
            .is_some_and(|row| row.id.is_some())
    })
}

unsafe extern "C" fn selection_changed(object: Id, _: native::Sel, notification: Id) {
    TableState::with(object, |state| {
        if !state.ready.get() {
            return;
        }
        // SAFETY: NSTableView sends this notification with itself as its object.
        let row = unsafe {
            let table = native::send_id(notification, sel(b"object\0"));
            native::send_i64(table, sel(b"selectedRow\0"))
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
        let state = native::get_pointer_ivar::<Rc<TableState>>(object, c"_sidebarState".as_ptr());
        native::set_pointer_ivar(
            object,
            c"_sidebarState".as_ptr(),
            std::ptr::null_mut::<Rc<TableState>>(),
        );
        if !state.is_null() {
            drop(Box::from_raw(state));
        }
    }
}

unsafe extern "C" fn dealloc(object: Id, _: native::Sel) {
    if catch_unwind(AssertUnwindSafe(|| release(object))).is_err() {
        std::process::abort();
    }
    // SAFETY: Our NSObject subclass has released its Rust state and now invokes superclass dealloc.
    unsafe {
        native::send_super_void(object, native::class(b"NSObject\0"), sel(b"dealloc\0"));
    }
}

fn delegate_class() -> native::Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        // SAFETY: Register once; each method signature matches its AppKit delegate selector.
        unsafe {
            let class = native::objc_allocateClassPair(
                native::class(b"NSObject\0"),
                c"ZpdSidebarTableDelegate".as_ptr(),
                0,
            );
            assert!(!class.is_null());
            assert!(native::class_addIvar(
                class,
                c"_sidebarState".as_ptr(),
                std::mem::size_of::<Id>(),
                3,
                c"^v".as_ptr()
            ));
            native::add_method(
                class,
                b"numberOfRowsInTableView:\0",
                row_count as unsafe extern "C" fn(_, _, _) -> _,
                b"q@:@\0",
            );
            native::add_method(
                class,
                b"tableView:viewForTableColumn:row:\0",
                row_view as unsafe extern "C" fn(_, _, _, _, _) -> _,
                b"@@:@@q\0",
            );
            native::add_method(
                class,
                b"tableView:isGroupRow:\0",
                is_group as unsafe extern "C" fn(_, _, _, _) -> _,
                b"B@:@q\0",
            );
            native::add_method(
                class,
                b"tableView:shouldSelectRow:\0",
                should_select as unsafe extern "C" fn(_, _, _, _) -> _,
                b"B@:@q\0",
            );
            native::add_method(
                class,
                b"tableViewSelectionDidChange:\0",
                selection_changed as unsafe extern "C" fn(_, _, _),
                b"v@:@\0",
            );
            native::add_method(
                class,
                b"dealloc\0",
                dealloc as unsafe extern "C" fn(_, _),
                b"v@:\0",
            );
            native::objc_registerClassPair(class);
            class as usize
        }
    }) as native::Class
}

fn constrain(first: Id, attribute: i64, second: Id, second_attribute: i64, constant: f64) {
    // SAFETY: All views share a cell ancestor; attributes and constants describe equalities.
    unsafe {
        let constraint = native::send_constraint(
            native::class(b"NSLayoutConstraint\0"),
            sel(b"constraintWithItem:attribute:relatedBy:toItem:attribute:multiplier:constant:\0"),
            first,
            attribute,
            0,
            second,
            second_attribute,
            1.0,
            constant,
        );
        native::send_void_bool(constraint, sel(b"setActive:\0"), true);
    }
}

fn cell(title: &str, symbol: Option<&str>, group: bool) -> Strong {
    let cell = native::alloc_init(b"NSTableCellView\0");
    let text = native::alloc_init(b"NSTextField\0");
    // SAFETY: The retained cell owns its text/image subviews and active layout constraints.
    unsafe {
        native::send_void_id(
            text.as_ptr(),
            sel(b"setStringValue:\0"),
            native::nsstring(title).as_ptr(),
        );
        native::send_void_bool(text.as_ptr(), sel(b"setEditable:\0"), false);
        native::send_void_bool(text.as_ptr(), sel(b"setSelectable:\0"), false);
        native::send_void_bool(text.as_ptr(), sel(b"setBordered:\0"), false);
        native::send_void_bool(text.as_ptr(), sel(b"setDrawsBackground:\0"), false);
        native::send_void_i64(text.as_ptr(), sel(b"setLineBreakMode:\0"), 4);
        native::send_void_bool(
            text.as_ptr(),
            sel(b"setTranslatesAutoresizingMaskIntoConstraints:\0"),
            false,
        );
        native::send_void_id(cell.as_ptr(), sel(b"addSubview:\0"), text.as_ptr());
        native::send_void_id(cell.as_ptr(), sel(b"setTextField:\0"), text.as_ptr());
        constrain(text.as_ptr(), 6, cell.as_ptr(), 6, -4.0);
        constrain(text.as_ptr(), 10, cell.as_ptr(), 10, 0.0);
        if group {
            constrain(text.as_ptr(), 5, cell.as_ptr(), 5, 4.0);
        } else {
            let image = native::alloc_init(b"NSImageView\0");
            native::send_void_bool(
                image.as_ptr(),
                sel(b"setTranslatesAutoresizingMaskIntoConstraints:\0"),
                false,
            );
            native::send_void_id(cell.as_ptr(), sel(b"addSubview:\0"), image.as_ptr());
            native::send_void_id(cell.as_ptr(), sel(b"setImageView:\0"), image.as_ptr());
            let configuration = native::send_id_f64_f64(
                native::class(b"NSImageSymbolConfiguration\0"),
                sel(b"configurationWithPointSize:weight:\0"),
                15.0,
                0.0,
            );
            native::send_void_id(
                image.as_ptr(),
                sel(b"setSymbolConfiguration:\0"),
                configuration,
            );
            native::send_void_id(
                image.as_ptr(),
                sel(b"setContentTintColor:\0"),
                native::send_id(native::class(b"NSColor\0"), sel(b"labelColor\0")),
            );
            if let Some(symbol) = symbol {
                let icon = native::send_id_id_id(
                    native::class(b"NSImage\0"),
                    sel(b"imageWithSystemSymbolName:accessibilityDescription:\0"),
                    native::nsstring(symbol).as_ptr(),
                    native::nsstring(title).as_ptr(),
                );
                native::send_void_id(image.as_ptr(), sel(b"setImage:\0"), icon);
            }
            constrain(image.as_ptr(), 5, cell.as_ptr(), 5, 4.0);
            constrain(image.as_ptr(), 10, cell.as_ptr(), 10, 0.0);
            constrain(image.as_ptr(), 7, native::NIL, 0, 18.0);
            constrain(image.as_ptr(), 8, native::NIL, 0, 18.0);
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
    let scroll = native::alloc_init(b"NSScrollView\0");
    let scroll_actor = tree
        .insert_child(controller, scroll.clone())
        .map_err(|_| SidebarError::Closed)?;
    let table = native::alloc_init(b"NSTableView\0");
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
        let delegate = Strong::from_retained(native::send_id(
            native::send_id(delegate_class(), sel(b"alloc\0")),
            sel(b"init\0"),
        ))
        .ok_or(SidebarError::NativeCreationFailed)?;
        native::set_pointer_ivar(
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
            native::send_void_id(table, sel(b"setDelegate:\0"), native::NIL);
            native::send_void_id(table, sel(b"setDataSource:\0"), native::NIL);
        }
    })
    .map_err(|_| SidebarError::Closed)?;
    // SAFETY: The controller, scroll view, table, column and delegate are live on the main thread.
    unsafe {
        native::send_void_bool(scroll.as_ptr(), sel(b"setDrawsBackground:\0"), false);
        native::send_void_bool(scroll.as_ptr(), sel(b"setHasVerticalScroller:\0"), true);
        native::send_void_bool(scroll.as_ptr(), sel(b"setAutohidesScrollers:\0"), true);
        let column = Strong::from_retained(native::send_id_id(
            native::send_id(native::class(b"NSTableColumn\0"), sel(b"alloc\0")),
            sel(b"initWithIdentifier:\0"),
            native::nsstring("items").as_ptr(),
        ))
        .ok_or(SidebarError::NativeCreationFailed)?;
        native::send_void_u64(column.as_ptr(), sel(b"setResizingMask:\0"), 1);
        native::send_void_id(table.as_ptr(), sel(b"addTableColumn:\0"), column.as_ptr());
        native::send_void_id(table.as_ptr(), sel(b"setHeaderView:\0"), native::NIL);
        native::send_void_i64(table.as_ptr(), sel(b"setStyle:\0"), 3);
        native::send_void_i64(table.as_ptr(), sel(b"setRowSizeStyle:\0"), 0);
        native::send_void_id(
            table.as_ptr(),
            sel(b"setBackgroundColor:\0"),
            native::send_id(native::class(b"NSColor\0"), sel(b"clearColor\0")),
        );
        native::send_void_f64(table.as_ptr(), sel(b"setRowHeight:\0"), 32.0);

        native::send_void_bool(table.as_ptr(), sel(b"setAllowsMultipleSelection:\0"), false);
        native::send_void_id(table.as_ptr(), sel(b"setDataSource:\0"), delegate.as_ptr());
        native::send_void_id(table.as_ptr(), sel(b"setDelegate:\0"), delegate.as_ptr());
        native::send_void_id(scroll.as_ptr(), sel(b"setDocumentView:\0"), table.as_ptr());
        native::send_void(table.as_ptr(), sel(b"reloadData\0"));
        if let Some(row) = selected {
            let indexes = native::send_id_u64(
                native::class(b"NSIndexSet\0"),
                sel(b"indexSetWithIndex:\0"),
                row as u64,
            );
            native::send_void_id_bool(
                table.as_ptr(),
                sel(b"selectRowIndexes:byExtendingSelection:\0"),
                indexes,
                false,
            );
        }
    }
    controller
        .with(|controller| {
            // SAFETY: The controller retains the installed scroll view.
            unsafe {
                native::send_void_id(controller, sel(b"setView:\0"), scroll.as_ptr());
            }
        })
        .map_err(|_| SidebarError::Closed)?;
    state.ready.set(true);
    Ok(())
}
