use crate::actor::ActorRef;
use crate::native;
use zpd_objc::Strong;

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
                unsafe { zpd_objc::msg_send!(item, zpd_objc::sel!("isCollapsed"), () => bool) }
            })
            .unwrap_or(false)
    }

    pub(crate) fn clear(self, window: &ActorRef) -> Result<(), WindowError> {
        window
            .with(|window| {
                // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
                self.content_controller.with(|controller| unsafe {
                    zpd_objc::msg_send!(window, zpd_objc::sel!("setContentViewController:"), ((controller): zpd_objc::Id) => ());
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
    let split = native::alloc_init(zpd_objc::class!("NSSplitViewController"));
    let root = tree
        .insert_child(window, split)
        .map_err(|_| SidebarError::Closed)?;
    let mut rollback = ActorRollback(Some(root.clone()));
    let sidebar_controller = native::alloc_init(zpd_objc::class!("NSViewController"));
    let sidebar_controller_actor = tree
        .insert_child(&root, sidebar_controller.clone())
        .map_err(|_| SidebarError::Closed)?;
    super::sidebar_table::install(&sidebar_controller_actor, sidebar, callback_fn)?;
    // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
    let sidebar_item = unsafe {
        Strong::retain(zpd_objc::msg_send!(zpd_objc::class!("NSSplitViewItem"), zpd_objc::sel!("sidebarWithViewController:"), ((sidebar_controller.as_ptr()): zpd_objc::Id) => zpd_objc::Id))
    }
    .ok_or(SidebarError::NativeCreationFailed)?;
    // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
    let content_item = unsafe {
        Strong::retain(zpd_objc::msg_send!(zpd_objc::class!("NSSplitViewItem"), zpd_objc::sel!("splitViewItemWithViewController:"), ((content_controller
                .with(|id| id)
                .map_err(|_| SidebarError::Closed)?): zpd_objc::Id) => zpd_objc::Id))
    }
    .ok_or(SidebarError::NativeCreationFailed)?;
    // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
    unsafe {
        zpd_objc::msg_send!(sidebar_item.as_ptr(), zpd_objc::sel!("setMinimumThickness:"), ((180.0): f64) => ());
        zpd_objc::msg_send!(sidebar_item.as_ptr(), zpd_objc::sel!("setMaximumThickness:"), ((320.0): f64) => ());
        zpd_objc::msg_send!(root.with(|id| id).map_err(|_| SidebarError::Closed)?, zpd_objc::sel!("addSplitViewItem:"), ((sidebar_item.as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(root.with(|id| id).map_err(|_| SidebarError::Closed)?, zpd_objc::sel!("addSplitViewItem:"), ((content_item.as_ptr()): zpd_objc::Id) => ());
    }
    // SAFETY: sidebar_item is a retained NSSplitViewItem with a live controller.
    unsafe {
        zpd_objc::msg_send!(sidebar_item.as_ptr(), zpd_objc::sel!("setAllowsFullHeightLayout:"), ((true): bool) => ());
        zpd_objc::msg_send!(content_item.as_ptr(), zpd_objc::sel!("setMinimumThickness:"), ((360.0): f64) => ());
        zpd_objc::msg_send!(sidebar_item.as_ptr(), zpd_objc::sel!("setCanCollapse:"), ((true): bool) => ());
        zpd_objc::msg_send!(sidebar_item.as_ptr(), zpd_objc::sel!("setCollapsed:"), ((collapsed): bool) => ());
    }
    window
        .with(|window| {
            // SAFETY: AppKit objects and selectors have matching types and run on the main thread.
            root.with(|split| unsafe {
                zpd_objc::msg_send!(window, zpd_objc::sel!("setContentViewController:"), ((split): zpd_objc::Id) => ())
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
