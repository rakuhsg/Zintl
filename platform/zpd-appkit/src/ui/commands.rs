#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_menu: Option<WindowAppMenu>,
    pub menus: Vec<CommandMenu>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WindowAppMenu {
    pub items: Vec<CommandItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CommandMenu {
    pub title: String,
    pub items: Vec<CommandItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CommandItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CommandRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<CommandModifier>,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandModifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandRole {
    About,
    Quit,
}

#[derive(Debug)]
pub enum CommandError {
    Encoding(serde_json::Error),
    InvalidJson(std::ffi::NulError),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encoding(error) => write!(f, "failed to encode commands: {error}"),
            Self::InvalidJson(error) => {
                write!(f, "encoded commands contain an interior NUL byte: {error}")
            }
        }
    }
}

impl std::error::Error for CommandError {}

pub(crate) fn install<F>(commands: &CommandSet, callback: F) -> Result<(), CommandError>
where
    F: FnMut(u32, &str) + 'static,
{
    let json = serde_json::to_string(commands).map_err(CommandError::Encoding)?;
    let json = CString::new(json).map_err(CommandError::InvalidJson)?;
    let callback_state = Rc::into_raw(Rc::new(CommandCallback {
        callback: RefCell::new(callback),
    }));

    // SAFETY: The JSON buffer lives for the call. Swift owns callback_state
    // until it invokes the release callback exactly once.
    unsafe {
        ffi::zintlappkit_set_commands(
            json.as_ptr(),
            callback_state.cast(),
            invoke_command::<F>,
            release_command::<F>,
        );
    }
    Ok(())
}

struct CommandCallback<F> {
    callback: RefCell<F>,
}

unsafe fn clone_command<F>(user_data: *const c_void) -> Option<Rc<CommandCallback<F>>> {
    let state = user_data.cast::<CommandCallback<F>>();
    if state.is_null() {
        return None;
    }

    // SAFETY: Swift holds the transferred strong reference until release.
    unsafe { Rc::increment_strong_count(state) };
    // SAFETY: The increment above created the returned strong reference.
    Some(unsafe { Rc::from_raw(state) })
}

unsafe extern "C" fn invoke_command<F: FnMut(u32, &str) + 'static>(
    user_data: *const c_void,
    window_id: u32,
    command_id: *const c_char,
) {
    if user_data.is_null() || command_id.is_null() {
        return;
    }

    if catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Command callbacks are installed with an Rc-backed state.
        let Some(state) = (unsafe { clone_command::<F>(user_data) }) else {
            return;
        };
        // SAFETY: Swift supplies a NUL-terminated command identifier.
        let command_id = unsafe { CStr::from_ptr(command_id) }.to_string_lossy();
        let Ok(mut callback) = state.callback.try_borrow_mut() else {
            std::process::abort();
        };
        callback(window_id, &command_id);
    }))
    .is_err()
    {
        std::process::abort();
    }
}

unsafe extern "C" fn release_command<F>(user_data: *const c_void) {
    if user_data.is_null() {
        return;
    }

    // SAFETY: Swift invokes this exactly once for the transferred Rc.
    unsafe { drop(Rc::from_raw(user_data.cast::<CommandCallback<F>>())) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_set_uses_swift_field_names() {
        let commands = CommandSet {
            app_menu: Some(WindowAppMenu {
                items: vec![CommandItem {
                    id: None,
                    title: "Quit".into(),
                    role: Some(CommandRole::Quit),
                    key: Some("q".into()),
                    modifiers: vec![CommandModifier::Cmd],
                    enabled: true,
                }],
            }),
            menus: Vec::new(),
        };

        let json = serde_json::to_value(commands).unwrap();
        assert!(json.get("appMenu").is_some());
        assert_eq!(json["appMenu"]["items"][0]["role"], "quit");
    }
}
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use crate::ffi;
