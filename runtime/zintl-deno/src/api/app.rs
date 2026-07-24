use deno_error::JsErrorBox;
use deno_runtime::deno_core::OpState;
use deno_runtime::deno_core::op2;
use serde::Serialize;
use std::cell::RefCell;
use std::rc::Rc;

use super::{ZintlAppCommands, ZintlWindowId, app_host};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all_fields = "camelCase", tag = "type")]
pub enum ZintlAppEvent {
    #[serde(rename = "click")]
    Click { command_id: String },
    #[serde(rename = "onload")]
    WindowCreated { window_id: ZintlWindowId },
    #[serde(rename = "willclose")]
    WindowWillClose { window_id: ZintlWindowId },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_window_events_with_js_field_names() {
        let event = serde_json::to_value(ZintlAppEvent::WindowWillClose { window_id: 7 })
            .expect("window event should serialize");

        assert_eq!(event["type"], "willclose");
        assert_eq!(event["windowId"], 7);
        assert!(event.get("window_id").is_none());
    }
}

#[op2]
#[serde]
pub(super) async fn op_zintl_app_next_event(
    state: Rc<RefCell<OpState>>,
) -> Result<ZintlAppEvent, JsErrorBox> {
    let host = {
        let state = state.borrow();
        app_host(&state)?
    };
    host.next_event()
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
pub(super) fn op_zintl_app_set_commands(
    state: &mut OpState,
    #[serde] commands: ZintlAppCommands,
) -> Result<(), JsErrorBox> {
    app_host(state)?
        .set_commands(commands)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}
