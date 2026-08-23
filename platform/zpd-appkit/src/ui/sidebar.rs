use std::cell::RefCell;
use std::rc::Rc;

use crate::actor::ActorRef;
use crate::native::{self, Strong};

use super::WindowError;
use super::callback;

#[derive(Clone, Debug, Default)]
pub struct Sidebar {
    pub sections: Vec<SidebarSection>,
    pub selected_id: Option<String>,
}
#[derive(Clone, Debug)]
pub struct SidebarSection {
    pub title: Option<String>,
    pub items: Vec<SidebarItem>,
}
#[derive(Clone, Debug)]
pub struct SidebarItem {
    pub id: String,
    pub title: String,
    pub system_image: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarError {
    NativeCreationFailed,
    Closed,
}
impl std::fmt::Display for SidebarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NativeCreationFailed => "AppKit failed to install the sidebar",
            Self::Closed => "the window is closed",
        })
    }
}
impl std::error::Error for SidebarError {}

pub(crate) struct SidebarNative {
    content_controller: ActorRef,
    root: ActorRef,
}
impl SidebarNative {
    pub(crate) fn clear(self, window: &ActorRef) -> Result<(), WindowError> {
        window
            .with(|window| {
                self.content_controller.with(|controller| unsafe {
                    native::send_void_id(
                        window,
                        native::sel(b"setContentViewController:\0"),
                        controller,
                    );
                })
            })
            .map_err(WindowError::from)?
            .map_err(WindowError::from)?;
        self.root.remove();
        Ok(())
    }
}

impl Drop for SidebarNative {
    fn drop(&mut self) {
        self.root.remove();
    }
}

struct ActorRollback(Option<ActorRef>);

impl ActorRollback {
    fn disarm(&mut self) {
        self.0.take();
    }
}

impl Drop for ActorRollback {
    fn drop(&mut self) {
        if let Some(actor) = self.0.take() {
            actor.remove();
        }
    }
}

pub(crate) fn install<F>(
    window: &ActorRef,
    content_controller: &ActorRef,
    sidebar: &Sidebar,
    callback_fn: F,
) -> Result<SidebarNative, SidebarError>
where
    F: FnMut(&str) + 'static,
{
    let callback_fn = Rc::new(RefCell::new(callback_fn));
    let tree = window.tree_handle().ok_or(SidebarError::Closed)?;
    let split = native::alloc_init(b"NSSplitViewController\0");
    let root = tree
        .insert_child(window, split)
        .map_err(|_| SidebarError::Closed)?;
    let mut rollback = ActorRollback(Some(root.clone()));
    let sidebar_controller = native::alloc_init(b"NSViewController\0");
    let sidebar_controller_actor = tree
        .insert_child(&root, sidebar_controller.clone())
        .map_err(|_| SidebarError::Closed)?;
    let stack = native::alloc_init(b"NSStackView\0");
    let stack_actor = tree
        .insert_child(&sidebar_controller_actor, stack.clone())
        .map_err(|_| SidebarError::Closed)?;
    unsafe {
        native::send_void_i64(stack.as_ptr(), native::sel(b"setOrientation:\0"), 1);
        native::send_void_f64(stack.as_ptr(), native::sel(b"setSpacing:\0"), 6.0);
        native::send_void_id(
            sidebar_controller.as_ptr(),
            native::sel(b"setView:\0"),
            stack.as_ptr(),
        );
    }
    for section in &sidebar.sections {
        if let Some(title) = &section.title {
            let label = text_label(title)?;
            tree.insert_child(&stack_actor, label.clone())
                .map_err(|_| SidebarError::Closed)?;
            unsafe {
                native::send_void_id(
                    stack.as_ptr(),
                    native::sel(b"addArrangedSubview:\0"),
                    label.as_ptr(),
                )
            };
        }
        for item in &section.items {
            let button = native::alloc_init(b"NSButton\0");
            let title = native::nsstring(&item.title);
            unsafe {
                native::send_void_id(button.as_ptr(), native::sel(b"setTitle:\0"), title.as_ptr());
                native::send_void_i64(button.as_ptr(), native::sel(b"setBezelStyle:\0"), 13);
                native::send_void_bool(button.as_ptr(), native::sel(b"setBordered:\0"), false);
                native::send_void_i64(
                    button.as_ptr(),
                    native::sel(b"setState:\0"),
                    i64::from(sidebar.selected_id.as_deref() == Some(item.id.as_str())),
                );
            }
            if let Some(symbol) = item.system_image.as_deref() {
                let symbol = native::nsstring(symbol);
                let image = unsafe {
                    native::send_id_id_id(
                        native::class(b"NSImage\0"),
                        native::sel(b"imageWithSystemSymbolName:accessibilityDescription:\0"),
                        symbol.as_ptr(),
                        title.as_ptr(),
                    )
                };
                if !image.is_null() {
                    unsafe {
                        native::send_void_id(button.as_ptr(), native::sel(b"setImage:\0"), image)
                    }
                }
            }
            let id = item.id.clone();
            let callback_fn = callback_fn.clone();
            let target = callback::target(move |_| {
                let Ok(mut callback) = callback_fn.try_borrow_mut() else {
                    std::process::abort()
                };
                callback(&id);
            });
            unsafe {
                native::send_void_id(
                    button.as_ptr(),
                    native::sel(b"setTarget:\0"),
                    target.as_ptr(),
                );
                native::send_void_id(
                    button.as_ptr(),
                    native::sel(b"setAction:\0"),
                    native::sel(b"invoke:\0"),
                );
                native::send_void_id(
                    stack.as_ptr(),
                    native::sel(b"addArrangedSubview:\0"),
                    button.as_ptr(),
                );
            }
            let button_actor = tree
                .insert_child(&stack_actor, button)
                .map_err(|_| SidebarError::Closed)?;
            tree.add_teardown(&button_actor, |button| unsafe {
                native::send_void_id(button, native::sel(b"setTarget:\0"), native::NIL);
                native::send_void_id(button, native::sel(b"setAction:\0"), native::NIL);
            })
            .map_err(|_| SidebarError::Closed)?;
            let target = tree
                .insert_child(&button_actor, target)
                .map_err(|_| SidebarError::Closed)?;
            tree.add_teardown(&target, |target| unsafe { callback::release(target) })
                .map_err(|_| SidebarError::Closed)?;
        }
    }
    let sidebar_item = unsafe {
        Strong::retain(native::send_id_id(
            native::class(b"NSSplitViewItem\0"),
            native::sel(b"sidebarWithViewController:\0"),
            sidebar_controller.as_ptr(),
        ))
    }
    .ok_or(SidebarError::NativeCreationFailed)?;
    let content_item = unsafe {
        Strong::retain(native::send_id_id(
            native::class(b"NSSplitViewItem\0"),
            native::sel(b"splitViewItemWithViewController:\0"),
            content_controller
                .with(|id| id)
                .map_err(|_| SidebarError::Closed)?,
        ))
    }
    .ok_or(SidebarError::NativeCreationFailed)?;
    unsafe {
        native::send_void_f64(
            sidebar_item.as_ptr(),
            native::sel(b"setMinimumThickness:\0"),
            180.0,
        );
        native::send_void_f64(
            sidebar_item.as_ptr(),
            native::sel(b"setMaximumThickness:\0"),
            360.0,
        );
        native::send_void_id(
            root.with(|id| id).map_err(|_| SidebarError::Closed)?,
            native::sel(b"addSplitViewItem:\0"),
            sidebar_item.as_ptr(),
        );
        native::send_void_id(
            root.with(|id| id).map_err(|_| SidebarError::Closed)?,
            native::sel(b"addSplitViewItem:\0"),
            content_item.as_ptr(),
        );
    }
    window
        .with(|window| {
            root.with(|split| unsafe {
                native::send_void_id(window, native::sel(b"setContentViewController:\0"), split)
            })
        })
        .map_err(|_| SidebarError::Closed)?
        .map_err(|_| SidebarError::Closed)?;
    tree.insert_child(&root, sidebar_item)
        .map_err(|_| SidebarError::Closed)?;
    tree.insert_child(&root, content_item)
        .map_err(|_| SidebarError::Closed)?;
    rollback.disarm();
    Ok(SidebarNative {
        content_controller: content_controller.clone(),
        root,
    })
}

fn text_label(value: &str) -> Result<Strong, SidebarError> {
    let value = native::nsstring(value);
    let field = native::alloc_init(b"NSTextField\0");
    unsafe {
        native::send_void_id(
            field.as_ptr(),
            native::sel(b"setStringValue:\0"),
            value.as_ptr(),
        );
        native::send_void_bool(field.as_ptr(), native::sel(b"setEditable:\0"), false);
        native::send_void_bool(field.as_ptr(), native::sel(b"setBordered:\0"), false);
        native::send_void_bool(field.as_ptr(), native::sel(b"setDrawsBackground:\0"), false);
    }
    Ok(field)
}
