use std::error::Error;

use zpd_appkit::runloop::Application;
use zpd_appkit::ui::{
    AsView, Button, CommandItem, CommandMenu, CommandModifier, CommandRole, CommandSet,
    LayoutConstraint, TextField, WindowAppMenu,
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

    let content = window.content_view()?;
    let label = TextField::label_with_string(&application, "Name")?;
    let input = TextField::with_string(&application, "")?;
    let button = Button::with_title(&application, "Save")?;

    input.set_placeholder_string(Some("Your name"));

    content.add_subview(&label);
    content.add_subview(&input);
    content.add_subview(&button);

    label.set_translates_autoresizing_mask_into_constraints(false);
    input.set_translates_autoresizing_mask_into_constraints(false);
    button.set_translates_autoresizing_mask_into_constraints(false);

    let constraints = [
        label
            .leading_anchor()
            .constraint_equal_to(content.leading_anchor(), 20.0),
        label
            .center_y_anchor()
            .constraint_equal_to(input.center_y_anchor(), 0.0),
        input
            .leading_anchor()
            .constraint_equal_to(label.trailing_anchor(), 8.0),
        input
            .trailing_anchor()
            .constraint_equal_to(content.trailing_anchor(), -20.0),
        input
            .top_anchor()
            .constraint_equal_to(content.top_anchor(), 20.0),
        button
            .top_anchor()
            .constraint_equal_to(input.bottom_anchor(), 12.0),
        button
            .trailing_anchor()
            .constraint_equal_to(input.trailing_anchor(), 0.0),
    ];
    LayoutConstraint::activate(&constraints);
    drop(constraints);

    button.set_action(move || println!("{}", input.string_value()));

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
