use std::cell::RefCell;
use std::rc::Rc;

use crate::actor::{ActorRef, ActorTree};
use crate::native;
use crate::runloop::{Application, ApplicationDelegate};
use zpd_objc::Strong;

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
        zpd_objc::msg_send!(app, zpd_objc::sel!("setMainMenu:"), ((zpd_objc::NIL): zpd_objc::Id) => ())
    })
    .map_err(|_| CommandError::Closed)?;
    let main_menu = native::alloc_init(zpd_objc::class!("NSMenu"));
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
        main.with(|main| unsafe {
            zpd_objc::msg_send!(app, zpd_objc::sel!("setMainMenu:"), ((main): zpd_objc::Id) => ())
        })
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
    let menu = native::alloc_init(zpd_objc::class!("NSMenu"));
    unsafe {
        zpd_objc::msg_send!(menu.as_ptr(), zpd_objc::sel!("setTitle:"), ((title.as_ptr()): zpd_objc::Id) => ())
    };
    let container = native::alloc_init(zpd_objc::class!("NSMenuItem"));
    unsafe {
        zpd_objc::msg_send!(container.as_ptr(), zpd_objc::sel!("setSubmenu:"), ((menu.as_ptr()): zpd_objc::Id) => ());
        zpd_objc::msg_send!(main.with(|id| id).map_err(|_| CommandError::Closed)?, zpd_objc::sel!("addItem:"), ((container.as_ptr()): zpd_objc::Id) => ());
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
            let allocated = zpd_objc::msg_send!(zpd_objc::class!("NSMenuItem"), zpd_objc::sel!("alloc"), () => zpd_objc::Id);
            let value = zpd_objc::msg_send!(allocated, zpd_objc::sel!("initWithTitle:action:keyEquivalent:"), ((item_title.as_ptr()): zpd_objc::Id, (zpd_objc::sel!("invoke:")): zpd_objc::Id, (key.as_ptr()): zpd_objc::Id) => zpd_objc::Id);
            Strong::from_retained(value).ok_or(CommandError::NativeCreationFailed)?
        };
        let app = application.clone();
        let command_id = item.id.clone();
        let role = item.role;
        let callback_fn = callback_fn.clone();
        let target = callback::target(move |_| match role {
            Some(CommandRole::About) => {
                let _ = app.with(|app| unsafe {
                    zpd_objc::msg_send!(app, zpd_objc::sel!("orderFrontStandardAboutPanel:"), ((zpd_objc::NIL): zpd_objc::Id) => ())
                });
            }
            Some(CommandRole::Quit) => {
                let _ = app.with(|app| unsafe {
                    zpd_objc::msg_send!(app, zpd_objc::sel!("terminate:"), ((zpd_objc::NIL): zpd_objc::Id) => ())
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
                CommandModifier::Cmd => native::NS_EVENT_MODIFIER_FLAG_COMMAND,
                CommandModifier::Ctrl => native::NS_EVENT_MODIFIER_FLAG_CONTROL,
                CommandModifier::Alt => native::NS_EVENT_MODIFIER_FLAG_OPTION,
                CommandModifier::Shift => native::NS_EVENT_MODIFIER_FLAG_SHIFT,
            }
        });
        unsafe {
            zpd_objc::msg_send!(native_item.as_ptr(), zpd_objc::sel!("setTarget:"), ((target.as_ptr()): zpd_objc::Id) => ());
            zpd_objc::msg_send!(native_item.as_ptr(), zpd_objc::sel!("setKeyEquivalentModifierMask:"), ((modifiers): u64) => ());
            zpd_objc::msg_send!(native_item.as_ptr(), zpd_objc::sel!("setEnabled:"), ((item.enabled): bool) => ());
            zpd_objc::msg_send!(menu.with(|id| id).map_err(|_| CommandError::Closed)?, zpd_objc::sel!("addItem:"), ((native_item.as_ptr()): zpd_objc::Id) => ());
        }
        let item = tree
            .insert_child(&menu, native_item)
            .map_err(|_| CommandError::Closed)?;
        tree.add_teardown(&item, |item| unsafe {
            zpd_objc::msg_send!(item, zpd_objc::sel!("setTarget:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
            zpd_objc::msg_send!(item, zpd_objc::sel!("setAction:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
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
