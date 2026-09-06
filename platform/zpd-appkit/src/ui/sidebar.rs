use crate::actor::ActorRef;
use crate::native::{self, Strong};

use super::WindowError;

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
    item: ActorRef,
}
impl SidebarNative {
    pub(crate) fn is_collapsed(&self) -> bool {
        self.item
            .with(|item| {
                // SAFETY: item is a live NSSplitViewItem on the main thread.
                unsafe { native::send_bool(item, native::sel(b"isCollapsed\0")) }
            })
            .unwrap_or(false)
    }

    pub(crate) fn clear(self, window: &ActorRef) -> Result<(), WindowError> {
        window
            .with(|window| {
                // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
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
    collapsed: bool,
    callback_fn: F,
) -> Result<SidebarNative, SidebarError>
where
    F: FnMut(&str) + 'static,
{
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
    super::sidebar_table::install(&sidebar_controller_actor, sidebar, callback_fn)?;
    // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
    let sidebar_item = unsafe {
        Strong::retain(native::send_id_id(
            native::class(b"NSSplitViewItem\0"),
            native::sel(b"sidebarWithViewController:\0"),
            sidebar_controller.as_ptr(),
        ))
    }
    .ok_or(SidebarError::NativeCreationFailed)?;
    // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
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
    // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
    unsafe {
        native::send_void_f64(
            sidebar_item.as_ptr(),
            native::sel(b"setMinimumThickness:\0"),
            180.0,
        );
        native::send_void_f64(
            sidebar_item.as_ptr(),
            native::sel(b"setMaximumThickness:\0"),
            320.0,
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
    // SAFETY: sidebar_item is a retained NSSplitViewItem with a live controller.
    unsafe {
        native::send_void_bool(
            sidebar_item.as_ptr(),
            native::sel(b"setAllowsFullHeightLayout:\0"),
            true,
        );
        native::send_void_f64(
            content_item.as_ptr(),
            native::sel(b"setMinimumThickness:\0"),
            360.0,
        );
        native::send_void_bool(
            sidebar_item.as_ptr(),
            native::sel(b"setCanCollapse:\0"),
            true,
        );
        native::send_void_bool(
            sidebar_item.as_ptr(),
            native::sel(b"setCollapsed:\0"),
            collapsed,
        );
    }
    window
        .with(|window| {
            // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
            root.with(|split| unsafe {
                native::send_void_id(window, native::sel(b"setContentViewController:\0"), split)
            })
        })
        .map_err(|_| SidebarError::Closed)?
        .map_err(|_| SidebarError::Closed)?;
    let item = tree
        .insert_child(&root, sidebar_item)
        .map_err(|_| SidebarError::Closed)?;
    tree.insert_child(&root, content_item)
        .map_err(|_| SidebarError::Closed)?;
    rollback.disarm();
    Ok(SidebarNative {
        content_controller: content_controller.clone(),
        root,
        item,
    })
}
