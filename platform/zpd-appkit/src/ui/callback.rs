use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use crate::native::{self, Id, Strong};

struct Callback {
    invoke: RefCell<Box<dyn FnMut(Id)>>,
}

unsafe fn callback(object: Id) -> *mut Callback {
    unsafe { native::get_pointer_ivar(object, c"_zpdCallback".as_ptr()) }
}

unsafe extern "C" fn invoke(object: Id, _: native::Sel, sender: Id) {
    let callback = unsafe { callback(object).as_ref() };
    let Some(callback) = callback else { return };
    if catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut invoke) = callback.invoke.try_borrow_mut() else {
            std::process::abort()
        };
        invoke(sender);
    }))
    .is_err()
    {
        std::process::abort()
    }
}

unsafe extern "C" fn dealloc(object: Id, _: native::Sel) {
    unsafe { release(object) };
    unsafe {
        native::send_super_void(
            object,
            native::class(b"NSObject\0"),
            native::sel(b"dealloc\0"),
        )
    };
}

pub(crate) unsafe fn release(object: Id) {
    let callback = unsafe { callback(object) };
    if !callback.is_null() {
        // SAFETY: Clearing the ivar transfers the sole allocation back to Rust.
        unsafe {
            native::set_pointer_ivar(
                object,
                c"_zpdCallback".as_ptr(),
                std::ptr::null_mut::<Callback>(),
            )
        };
        // SAFETY: The target owns exactly one callback allocation.
        unsafe { drop(Box::from_raw(callback)) };
    }
}

fn target_class() -> native::Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let class = native::objc_allocateClassPair(
            native::class(b"NSObject\0"),
            c"ZpdRustActionTarget".as_ptr(),
            0,
        );
        assert!(!class.is_null());
        assert!(native::class_addIvar(
            class,
            c"_zpdCallback".as_ptr(),
            std::mem::size_of::<Id>(),
            3,
            c"^v".as_ptr()
        ));
        native::add_method(
            class,
            b"invoke:\0",
            invoke as unsafe extern "C" fn(_, _, _),
            b"v@:@\0",
        );
        native::add_method(
            class,
            b"controlTextDidChange:\0",
            invoke as unsafe extern "C" fn(_, _, _),
            b"v@:@\0",
        );
        native::add_method(
            class,
            b"dealloc\0",
            dealloc as unsafe extern "C" fn(_, _),
            b"v@:\0",
        );
        native::objc_registerClassPair(class);
        class as usize
    }) as native::Class
}

pub(crate) fn target(callback: impl FnMut(Id) + 'static) -> Strong {
    let class = target_class();
    // SAFETY: The registered target class owns the installed callback pointer.
    unsafe {
        let object = native::send_id(
            native::send_id(class, native::sel(b"alloc\0")),
            native::sel(b"init\0"),
        );
        native::set_pointer_ivar(
            object,
            c"_zpdCallback".as_ptr(),
            Box::into_raw(Box::new(Callback {
                invoke: RefCell::new(Box::new(callback)),
            })),
        );
        Strong::from_retained(object).expect("callback target allocation failed")
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    struct DropProbe(Rc<Cell<usize>>);
    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn target_releases_callback_state_once() {
        // Verifies replacing or destroying a native target releases its Rust closure exactly once.
        let drops = Rc::new(Cell::new(0));
        let probe = DropProbe(drops.clone());
        let target = super::target(move |_| {
            let _ = &probe;
        });
        drop(target);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn actor_owned_target_replacement_releases_each_callback_once() {
        // Verifies Actor attachment replacement drops old and current callback state once each.
        let tree = crate::actor::ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let owner = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        let drops = Rc::new(Cell::new(0));
        let first = DropProbe(drops.clone());
        tree.replace_owned(
            &owner,
            "target",
            super::target(move |_| {
                let _ = &first;
            }),
        )
        .unwrap();
        let second = DropProbe(drops.clone());
        tree.replace_owned(
            &owner,
            "target",
            super::target(move |_| {
                let _ = &second;
            }),
        )
        .unwrap();
        assert_eq!(drops.get(), 1);
        tree.clear_owned(&owner, "target").unwrap();
        assert_eq!(drops.get(), 2);
    }
}
