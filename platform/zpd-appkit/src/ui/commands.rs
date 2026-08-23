use std::cell::RefCell;
use std::rc::Rc;

use crate::native::{self, Strong};
use crate::runloop::{Application, ApplicationDelegate};

use super::callback;

#[derive(Clone, Debug, Default)]
pub struct CommandSet {
    pub app_menu: Option<WindowAppMenu>,
    pub menus: Vec<CommandMenu>,
}
#[derive(Clone, Debug)]
pub struct WindowAppMenu {
    pub items: Vec<CommandItem>,
}
#[derive(Clone, Debug)]
pub struct CommandMenu {
    pub title: String,
    pub items: Vec<CommandItem>,
}
#[derive(Clone, Debug)]
pub struct CommandItem {
    pub id: Option<String>,
    pub title: String,
    pub role: Option<CommandRole>,
    pub key: Option<String>,
    pub modifiers: Vec<CommandModifier>,
    pub enabled: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum CommandModifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}
#[derive(Clone, Copy, Debug)]
pub enum CommandRole {
    About,
    Quit,
}

#[derive(Clone, Copy, Debug)]
pub enum CommandError {
    NativeCreationFailed,
}
impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppKit failed to install commands")
    }
}
impl std::error::Error for CommandError {}

pub(crate) fn install<D, F>(
    application: &Application<D>,
    commands: &CommandSet,
    callback_fn: F,
) -> Result<(), CommandError>
where
    D: ApplicationDelegate,
    F: FnMut(&str) + 'static,
{
    let callback_fn = Rc::new(RefCell::new(callback_fn));
    let mut objects = Vec::new();
    let main_menu = native::alloc_init(b"NSMenu\0");
    if let Some(menu) = &commands.app_menu {
        add_menu(
            application,
            &main_menu,
            "",
            &menu.items,
            &callback_fn,
            &mut objects,
        )?;
    }
    for menu in &commands.menus {
        add_menu(
            application,
            &main_menu,
            &menu.title,
            &menu.items,
            &callback_fn,
            &mut objects,
        )?;
    }
    // SAFETY: NSApplication retains the installed main menu.
    unsafe {
        native::send_void_id(
            application.app(),
            native::sel(b"setMainMenu:\0"),
            main_menu.as_ptr(),
        )
    };
    objects.push(main_menu);
    application.replace_command_objects(objects);
    Ok(())
}

fn add_menu<F>(
    application: &impl ApplicationAccess,
    main: &Strong,
    title: &str,
    items: &[CommandItem],
    callback_fn: &Rc<RefCell<F>>,
    objects: &mut Vec<Strong>,
) -> Result<(), CommandError>
where
    F: FnMut(&str) + 'static,
{
    let title = native::nsstring(title);
    let menu = native::alloc_init(b"NSMenu\0");
    unsafe { native::send_void_id(menu.as_ptr(), native::sel(b"setTitle:\0"), title.as_ptr()) };
    let container = native::alloc_init(b"NSMenuItem\0");
    unsafe {
        native::send_void_id(
            container.as_ptr(),
            native::sel(b"setSubmenu:\0"),
            menu.as_ptr(),
        );
        native::send_void_id(
            main.as_ptr(),
            native::sel(b"addItem:\0"),
            container.as_ptr(),
        );
    }
    objects.push(container);
    for item in items {
        let item_title = native::nsstring(&item.title);
        let key = native::nsstring(
            item.key
                .as_deref()
                .and_then(|v| v.chars().next())
                .map(|v| v.to_lowercase().to_string())
                .as_deref()
                .unwrap_or(""),
        );
        let native_item = unsafe {
            let allocated =
                native::send_id(native::class(b"NSMenuItem\0"), native::sel(b"alloc\0"));
            let value = native::send_id_id_id_id(
                allocated,
                native::sel(b"initWithTitle:action:keyEquivalent:\0"),
                item_title.as_ptr(),
                native::sel(b"invoke:\0"),
                key.as_ptr(),
            );
            Strong::from_retained(value).ok_or(CommandError::NativeCreationFailed)?
        };
        let app = application.app_id();
        let command_id = item.id.clone();
        let role = item.role;
        let callback_fn = callback_fn.clone();
        let target = callback::target(move |_| match role {
            Some(CommandRole::About) => unsafe {
                native::send_void_id(
                    app,
                    native::sel(b"orderFrontStandardAboutPanel:\0"),
                    native::NIL,
                )
            },
            Some(CommandRole::Quit) => unsafe {
                native::send_void_id(app, native::sel(b"terminate:\0"), native::NIL)
            },
            None => {
                if let Some(id) = command_id.as_deref() {
                    let Ok(mut callback) = callback_fn.try_borrow_mut() else {
                        std::process::abort()
                    };
                    callback(id);
                }
            }
        });
        let modifiers = item.modifiers.iter().fold(0_u64, |mask, value| {
            mask | match value {
                CommandModifier::Cmd => 1 << 20,
                CommandModifier::Ctrl => 1 << 18,
                CommandModifier::Alt => 1 << 19,
                CommandModifier::Shift => 1 << 17,
            }
        });
        unsafe {
            native::send_void_id(
                native_item.as_ptr(),
                native::sel(b"setTarget:\0"),
                target.as_ptr(),
            );
            native::send_void_u64(
                native_item.as_ptr(),
                native::sel(b"setKeyEquivalentModifierMask:\0"),
                modifiers,
            );
            native::send_void_bool(
                native_item.as_ptr(),
                native::sel(b"setEnabled:\0"),
                item.enabled,
            );
            native::send_void_id(
                menu.as_ptr(),
                native::sel(b"addItem:\0"),
                native_item.as_ptr(),
            );
        }
        objects.push(target);
        objects.push(native_item);
    }
    objects.push(menu);
    Ok(())
}

trait ApplicationAccess {
    fn app_id(&self) -> native::Id;
}
impl<D: ApplicationDelegate> ApplicationAccess for Application<D> {
    fn app_id(&self) -> native::Id {
        self.app()
    }
}
