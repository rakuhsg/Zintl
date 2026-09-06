use super::{SidebarError, WindowError};
use crate::actor::ActorRef;
use crate::native::{self, Strong};

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSToolbarToggleSidebarItemIdentifier: native::Id;
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
                    let existing = native::send_id(window, native::sel(b"toolbar\0"));
                    let created = existing.is_null();
                    let toolbar = if created {
                        let identifier = native::nsstring("zintl.sidebar");
                        Strong::from_retained(native::send_id_id(
                            native::send_id(native::class(b"NSToolbar\0"), native::sel(b"alloc\0")),
                            native::sel(b"initWithIdentifier:\0"),
                            identifier.as_ptr(),
                        ))
                    } else {
                        Strong::retain(existing)
                    }
                    .ok_or(SidebarError::NativeCreationFailed)?;
                    let inserted = Self::toggle_index(&toolbar).is_none();
                    if created {
                        native::send_void_i64(
                            toolbar.as_ptr(),
                            native::sel(b"setDisplayMode:\0"),
                            2,
                        );
                        native::send_void_id(
                            window,
                            native::sel(b"setToolbar:\0"),
                            toolbar.as_ptr(),
                        );
                    }
                    if inserted {
                        native::send_void_id_i64(
                            toolbar.as_ptr(),
                            native::sel(b"insertItemWithItemIdentifier:atIndex:\0"),
                            NSToolbarToggleSidebarItemIdentifier,
                            0,
                        );
                    }
                    if let Some(index) = Self::toggle_index(&toolbar) {
                        let items = native::send_id(toolbar.as_ptr(), native::sel(b"items\0"));
                        let toggle = native::send_id_u64(
                            items,
                            native::sel(b"objectAtIndex:\0"),
                            index as u64,
                        );
                        // Navigation items occupy the leading side of the window toolbar.
                        native::send_void_bool(toggle, native::sel(b"setNavigational:\0"), true);
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
            let items = native::send_id(toolbar.as_ptr(), native::sel(b"items\0"));
            (0..native::send_u64(items, native::sel(b"count\0"))).find_map(|index| {
                let item = native::send_id_u64(items, native::sel(b"objectAtIndex:\0"), index);
                let identifier = native::send_id(item, native::sel(b"itemIdentifier\0"));
                native::send_bool_id(
                    identifier,
                    native::sel(b"isEqualToString:\0"),
                    NSToolbarToggleSidebarItemIdentifier,
                )
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
                        native::send_void_i64(
                            self.toolbar.as_ptr(),
                            native::sel(b"removeItemAtIndex:\0"),
                            index,
                        );
                    }
                    if self.created
                        && native::send_id(window, native::sel(b"toolbar\0"))
                            == self.toolbar.as_ptr()
                    {
                        native::send_void_id(window, native::sel(b"setToolbar:\0"), native::NIL);
                    }
                }
            })
            .map_err(WindowError::from)
    }
}
