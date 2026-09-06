use super::{SidebarError, WindowError};
use crate::actor::ActorRef;
use crate::native;
use zpd_objc::Strong;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSToolbarToggleSidebarItemIdentifier: zpd_objc::Id;
}

/// Owns only the toolbar or item added for the sidebar.
pub(crate) struct SidebarToolbar {
    toolbar: Strong,
    created: bool,
    inserted: bool,
}

impl SidebarToolbar {
    pub(crate) fn install(window: &ActorRef) -> Result<Self, SidebarError> {
        window
            .with(|window| {
                // SAFETY: The window is live; AppKit toolbar selectors use the declared ABI.
                unsafe {
                    let existing = zpd_objc::msg_send!(window, zpd_objc::sel!("toolbar"), () => zpd_objc::Id);
                    let created = existing.is_null();
                    let toolbar = if created {
                        let identifier = native::nsstring("zintl.sidebar");
                        Strong::from_retained(zpd_objc::msg_send!(zpd_objc::msg_send!(zpd_objc::class!("NSToolbar"), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("initWithIdentifier:"), ((identifier.as_ptr()): zpd_objc::Id) => zpd_objc::Id))
                    } else {
                        Strong::retain(existing)
                    }
                    .ok_or(SidebarError::NativeCreationFailed)?;
                    let inserted = Self::toggle_index(&toolbar).is_none();
                    if created {
                        zpd_objc::msg_send!(toolbar.as_ptr(), zpd_objc::sel!("setDisplayMode:"), ((2): i64) => ());
                        zpd_objc::msg_send!(window, zpd_objc::sel!("setToolbar:"), ((toolbar.as_ptr()): zpd_objc::Id) => ());
                    }
                    if inserted {
                        zpd_objc::msg_send!(toolbar.as_ptr(), zpd_objc::sel!("insertItemWithItemIdentifier:atIndex:"), ((NSToolbarToggleSidebarItemIdentifier): zpd_objc::Id, (0): i64) => ());
                    }
                    if let Some(index) = Self::toggle_index(&toolbar) {
                        let items = zpd_objc::msg_send!(toolbar.as_ptr(), zpd_objc::sel!("items"), () => zpd_objc::Id);
                        let toggle = zpd_objc::msg_send!(items, zpd_objc::sel!("objectAtIndex:"), ((index as u64): u64) => zpd_objc::Id);
                        // Navigation items occupy the leading side of the window toolbar.
                        zpd_objc::msg_send!(toggle, zpd_objc::sel!("setNavigational:"), ((true): bool) => ());
                    }
                    Ok(Self {
                        toolbar,
                        created,
                        inserted,
                    })
                }
            })
            .map_err(|_| SidebarError::Closed)?
    }

    fn toggle_index(toolbar: &Strong) -> Option<i64> {
        // SAFETY: NSToolbar owns its items, whose identifiers are NSString objects.
        unsafe {
            let items =
                zpd_objc::msg_send!(toolbar.as_ptr(), zpd_objc::sel!("items"), () => zpd_objc::Id);
            (0..zpd_objc::msg_send!(items, zpd_objc::sel!("count"), () => u64)).find_map(|index| {
                let item = zpd_objc::msg_send!(items, zpd_objc::sel!("objectAtIndex:"), ((index): u64) => zpd_objc::Id);
                let identifier = zpd_objc::msg_send!(item, zpd_objc::sel!("itemIdentifier"), () => zpd_objc::Id);
                zpd_objc::msg_send!(identifier, zpd_objc::sel!("isEqualToString:"), ((NSToolbarToggleSidebarItemIdentifier): zpd_objc::Id) => bool)
                .then_some(index as i64)
            })
        }
    }

    pub(crate) fn clear(self, window: &ActorRef) -> Result<(), WindowError> {
        window
            .with(|window| {
                // SAFETY: The retained toolbar and live window remain valid during removal.
                unsafe {
                    if self.inserted
                        && let Some(index) = Self::toggle_index(&self.toolbar)
                    {
                        zpd_objc::msg_send!(self.toolbar.as_ptr(), zpd_objc::sel!("removeItemAtIndex:"), ((index): i64) => ());
                    }
                    if self.created
                        && zpd_objc::msg_send!(window, zpd_objc::sel!("toolbar"), () => zpd_objc::Id)
                            == self.toolbar.as_ptr()
                    {
                        zpd_objc::msg_send!(window, zpd_objc::sel!("setToolbar:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
                    }
                }
            })
            .map_err(WindowError::from)
    }
}
