use std::cell::RefCell;
use std::rc::Rc;

use crate::actor::{ActorRef, ActorTree};
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
    Closed,
}
impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NativeCreationFailed => "AppKit failed to install commands",
            Self::Closed => "the application is not active",
        })
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
    let tree = application.tree();
    let app = application.actor_ref();
    app.with(|app| unsafe {
        native::send_void_id(app, native::sel(b"setMainMenu:\0"), native::NIL)
    })
    .map_err(|_| CommandError::Closed)?;
    let main_menu = native::alloc_init(b"NSMenu\0");
    let main = tree
        .replace_owned(&app, "commands", main_menu)
        .map_err(|_| CommandError::Closed)?;
    let build_result = (|| {
        if let Some(menu) = &commands.app_menu {
            add_menu(tree, &app, &main, "", &menu.items, &callback_fn)?;
        }
        for menu in &commands.menus {
            add_menu(tree, &app, &main, &menu.title, &menu.items, &callback_fn)?;
        }
        Ok::<(), CommandError>(())
    })();
    if let Err(error) = build_result {
        let _ = tree.clear_owned(&app, "commands");
        return Err(error);
    }
    // SAFETY: NSApplication retains the installed main menu.
    let install_result = app.with(|app| {
        main.with(|main| unsafe { native::send_void_id(app, native::sel(b"setMainMenu:\0"), main) })
    });
    match install_result {
        Ok(Ok(())) => {}
        Ok(Err(_)) | Err(_) => {
            let _ = tree.clear_owned(&app, "commands");
            return Err(CommandError::Closed);
        }
    }
    Ok(())
}

fn add_menu<F>(
    tree: &ActorTree,
    application: &ActorRef,
    main: &ActorRef,
    title: &str,
    items: &[CommandItem],
    callback_fn: &Rc<RefCell<F>>,
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
            main.with(|id| id).map_err(|_| CommandError::Closed)?,
            native::sel(b"addItem:\0"),
            container.as_ptr(),
        );
    }
    let container = tree
        .insert_child(main, container)
        .map_err(|_| CommandError::Closed)?;
    let menu = tree
        .insert_child(&container, menu)
        .map_err(|_| CommandError::Closed)?;
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
        let app = application.clone();
        let command_id = item.id.clone();
        let role = item.role;
        let callback_fn = callback_fn.clone();
        let target = callback::target(move |_| match role {
            Some(CommandRole::About) => {
                let _ = app.with(|app| unsafe {
                    native::send_void_id(
                        app,
                        native::sel(b"orderFrontStandardAboutPanel:\0"),
                        native::NIL,
                    )
                });
            }
            Some(CommandRole::Quit) => {
                let _ = app.with(|app| unsafe {
                    native::send_void_id(app, native::sel(b"terminate:\0"), native::NIL)
                });
            }
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
                menu.with(|id| id).map_err(|_| CommandError::Closed)?,
                native::sel(b"addItem:\0"),
                native_item.as_ptr(),
            );
        }
        let item = tree
            .insert_child(&menu, native_item)
            .map_err(|_| CommandError::Closed)?;
        tree.add_teardown(&item, |item| unsafe {
            native::send_void_id(item, native::sel(b"setTarget:\0"), native::NIL);
            native::send_void_id(item, native::sel(b"setAction:\0"), native::NIL);
        })
        .map_err(|_| CommandError::Closed)?;
        let target = tree
            .insert_child(&item, target)
            .map_err(|_| CommandError::Closed)?;
        tree.add_teardown(&target, |target| unsafe { callback::release(target) })
            .map_err(|_| CommandError::Closed)?;
    }
    Ok(())
}
