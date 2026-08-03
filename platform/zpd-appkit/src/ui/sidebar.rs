use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::rc::Rc;

use crate::ffi;
use crate::string::NativeString;

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidebar {
    pub sections: Vec<SidebarSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SidebarSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub items: Vec<SidebarItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarItem {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_image: Option<String>,
}

#[derive(Debug)]
pub enum SidebarError {
    Encoding(serde_json::Error),
    NativeCreationFailed,
    Closed,
}

impl std::fmt::Display for SidebarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encoding(error) => write!(f, "failed to encode sidebar: {error}"),
            Self::NativeCreationFailed => write!(f, "AppKit failed to install the sidebar"),
            Self::Closed => write!(f, "the window is closed"),
        }
    }
}

impl std::error::Error for SidebarError {}

pub(crate) fn install<F>(
    window: NonNull<c_void>,
    sidebar: &Sidebar,
    callback: F,
) -> Result<(), SidebarError>
where
    F: FnMut(&str) + 'static,
{
    let json = serde_json::to_string(sidebar).map_err(SidebarError::Encoding)?;
    let callback_state = Rc::into_raw(Rc::new(SidebarCallback {
        callback: RefCell::new(callback),
    }));

    // SAFETY: The JSON buffer lives for the call. Swift consumes the
    // transferred callback state and releases it exactly once.
    let installed = unsafe {
        ffi::zintlappkit_window_set_sidebar(
            window.as_ptr(),
            NativeString::from_str(&json),
            callback_state.cast(),
            invoke_selection::<F>,
            release_selection::<F>,
        )
    };
    if installed {
        Ok(())
    } else {
        Err(SidebarError::NativeCreationFailed)
    }
}

struct SidebarCallback<F> {
    callback: RefCell<F>,
}

unsafe fn clone_callback<F>(user_data: *const c_void) -> Option<Rc<SidebarCallback<F>>> {
    let state = user_data.cast::<SidebarCallback<F>>();
    if state.is_null() {
        return None;
    }

    // SAFETY: Swift holds the transferred strong reference until release.
    unsafe { Rc::increment_strong_count(state) };
    // SAFETY: The increment above created the returned strong reference.
    Some(unsafe { Rc::from_raw(state) })
}

unsafe extern "C" fn invoke_selection<F: FnMut(&str) + 'static>(
    user_data: *const c_void,
    item_id: NativeString,
) {
    if catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Sidebar callbacks are installed with an Rc-backed state.
        let Some(state) = (unsafe { clone_callback::<F>(user_data) }) else {
            return;
        };
        // SAFETY: Swift supplies a borrowed UTF-8 item identifier.
        let item_id = unsafe { item_id.to_string() };
        let Ok(mut callback) = state.callback.try_borrow_mut() else {
            std::process::abort();
        };
        callback(&item_id);
    }))
    .is_err()
    {
        std::process::abort();
    }
}

unsafe extern "C" fn release_selection<F>(user_data: *const c_void) {
    if user_data.is_null() {
        return;
    }

    // SAFETY: Swift invokes this exactly once for the transferred Rc.
    unsafe { drop(Rc::from_raw(user_data.cast::<SidebarCallback<F>>())) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_uses_swift_field_names() {
        let sidebar = Sidebar {
            sections: vec![SidebarSection {
                title: Some("Library".into()),
                items: vec![SidebarItem {
                    id: "recent".into(),
                    title: "Recent".into(),
                    system_image: Some("clock".into()),
                }],
            }],
            selected_id: Some("recent".into()),
        };

        let json = serde_json::to_value(sidebar).unwrap();
        assert_eq!(json["selectedId"], "recent");
        assert_eq!(json["sections"][0]["items"][0]["systemImage"], "clock");
    }
}
