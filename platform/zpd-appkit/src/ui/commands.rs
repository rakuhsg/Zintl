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
