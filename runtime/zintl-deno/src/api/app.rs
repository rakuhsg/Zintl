use deno_error::JsErrorBox;
use deno_runtime::deno_core::OpState;
use deno_runtime::deno_core::op2;
use serde::Serialize;

use super::{ZintlWindowId, app_host};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all_fields = "camelCase", tag = "type")]
pub enum ZintlAppEvent {
    #[serde(rename = "window.command")]
    WindowCommand {
        window_id: ZintlWindowId,
        command_id: String,
    },
    #[serde(rename = "window.created")]
    WindowCreated { window_id: ZintlWindowId },
    #[serde(rename = "window.willClose")]
    WindowWillClose { window_id: ZintlWindowId },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_window_events_with_js_field_names() {
        let event = serde_json::to_value(ZintlAppEvent::WindowWillClose { window_id: 7 })
            .expect("window event should serialize");

        assert_eq!(event["type"], "window.willClose");
        assert_eq!(event["windowId"], 7);
        assert!(event.get("window_id").is_none());
    }
}

#[op2]
#[serde]
pub(super) fn op_zintl_app_event_bus_poll(
    state: &mut OpState,
) -> Result<Option<ZintlAppEvent>, JsErrorBox> {
    app_host(state)?
        .take_event()
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}
