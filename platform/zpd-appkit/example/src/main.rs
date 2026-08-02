use std::error::Error;

use zpd_appkit::runloop::Application;
use zpd_appkit::ui::{
    CommandItem, CommandMenu, CommandModifier, CommandRole, CommandSet, WindowAppMenu,
};

fn main() -> Result<(), Box<dyn Error>> {
    let application = Application::new(())?;
    application.set_commands(&commands(), |command_id| {
        if command_id == "example.hello" {
            println!("Hello from zpd-appkit!");
        }
    })?;

    let window = application.create_window(())?;
    window.set_size(640.0, 400.0)?;
    window.show()?;

    application.run();
    Ok(())
}

fn commands() -> CommandSet {
    CommandSet {
        app_menu: Some(WindowAppMenu {
            items: vec![
                CommandItem {
                    id: None,
                    title: "About zpd-appkit Example".into(),
                    role: Some(CommandRole::About),
                    key: None,
                    modifiers: Vec::new(),
                    enabled: true,
                },
                CommandItem {
                    id: None,
                    title: "Quit zpd-appkit Example".into(),
                    role: Some(CommandRole::Quit),
                    key: Some("q".into()),
                    modifiers: vec![CommandModifier::Cmd],
                    enabled: true,
                },
            ],
        }),
        menus: vec![CommandMenu {
            title: "Example".into(),
            items: vec![CommandItem {
                id: Some("example.hello".into()),
                title: "Print Hello".into(),
                role: None,
                key: Some("h".into()),
                modifiers: vec![CommandModifier::Cmd],
                enabled: true,
            }],
        }],
    }
}
