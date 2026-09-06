#[macro_export]
macro_rules! decl {
    (
        $name:ident : [ $( $superclass:expr )? ] {
            fields { $( $field:ident : $type:ident ),* $(,)? }
            methods {
                $( $selector:literal : $encoding:literal => $implementation:path ),* $(,)?
            }
        }
    ) => {{
        let class_name = concat!(stringify!($name), "\0");
        let superclass: $crate::ffi::Class = $crate::decl!(@superclass $( $superclass )?);
        // SAFETY: The class name is static and NUL-terminated, and superclass is null or a Class.
        let class = unsafe {
            $crate::ffi::objc_allocateClassPair(
                superclass,
                class_name.as_ptr().cast(),
                0,
            )
        };
        assert!(!class.is_null(), "failed to allocate Objective-C class");

        $(
            let (size, alignment, encoding) = $crate::decl!(@field_type $type);
            let field_name = concat!(stringify!($field), "\0");
            // SAFETY: The class is unregistered and all strings are static and NUL-terminated.
            let added = unsafe {
                $crate::ffi::class_addIvar(
                    class,
                    field_name.as_ptr().cast(),
                    size,
                    alignment,
                    encoding,
                )
            };
            assert!(added, "failed to add Objective-C field");
        )*

        $(
            let selector_name = concat!($selector, "\0");
            // SAFETY: The selector name is a static, NUL-terminated string.
            let selector = unsafe {
                $crate::ffi::sel_registerName(selector_name.as_ptr().cast())
            };
            let implementation = $implementation as *const ();
            // SAFETY: Objective-C stores implementations behind a signature-erased function pointer.
            let implementation: $crate::ffi::Imp = unsafe {
                ::std::mem::transmute(implementation)
            };
            let encoding = concat!($encoding, "\0");
            // SAFETY: The class is unregistered, the selector is valid, and the encoding is NUL-terminated.
            let added = unsafe {
                $crate::ffi::class_addMethod(
                    class,
                    selector,
                    implementation,
                    encoding.as_ptr().cast(),
                )
            };
            assert!(added, "failed to add Objective-C method");
        )*

        // SAFETY: The class was allocated above and is registered exactly once by this expansion.
        unsafe { $crate::ffi::objc_registerClassPair(class) };
        class
    }};
    (
        $name:ident : [ $( $superclass:expr )? ] {
            methods {
                $( $selector:literal : $encoding:literal => $implementation:path ),* $(,)?
            }
        }
    ) => {
        $crate::decl!($name : [ $( $superclass )? ] {
            fields {}
            methods {
                $( $selector : $encoding => $implementation ),*
            }
        })
    };
    ( $name:ident : [ $( $superclass:expr )? ] { $( $field:ident : $type:ident ),* $(,)? } ) => {
        $crate::decl!($name : [ $( $superclass )? ] {
            fields { $( $field : $type ),* }
            methods {}
        })
    };
    (@superclass) => {
        ::std::ptr::null_mut()
    };
    (@superclass $superclass:expr) => {
        $superclass
    };
    (@field_type sel) => {{
        (
            ::std::mem::size_of::<$crate::ffi::Sel>(),
            ::std::mem::align_of::<$crate::ffi::Sel>().trailing_zeros() as u8,
            b":\0".as_ptr().cast(),
        )
    }};
    (@field_type $type:ident) => {
        compile_error!(concat!("unsupported Objective-C field type: ", stringify!($type)))
    };
}

#[cfg(test)]
mod tests {
    use crate::ffi;
    use std::ffi::{c_char, c_long, c_void};

    unsafe extern "C" fn inherited_value(_: ffi::Id, _: ffi::Sel) -> c_long {
        1
    }

    unsafe extern "C" fn overridden_value(_: ffi::Id, _: ffi::Sel) -> c_long {
        2
    }

    #[test]
    fn declares_class_with_selector_field() {
        // Verifies that decl! registers a class containing selector-sized storage.
        let custom_class = decl!(ZpdObjcDeclSelectorTest: [] {
            field: sel
        });

        assert!(!custom_class.is_null());
        // SAFETY: The class is registered above and the field name is NUL-terminated.
        let field = unsafe {
            ffi::class_getInstanceVariable(custom_class, c"field".as_ptr().cast::<c_char>())
        };
        assert!(!field.is_null());
    }

    #[test]
    fn inherits_and_overrides_methods() {
        // Verifies inherited dispatch and replacement by a subclass implementation.
        let base_class = decl!(ZpdObjcDeclMethodBase: [] {
            methods {
                "value": "q@:" => inherited_value,
            }
        });
        let inherited_class = decl!(ZpdObjcDeclMethodInherited: [base_class] {
            methods {}
        });
        let overridden_class = decl!(ZpdObjcDeclMethodOverridden: [base_class] {
            methods {
                "value": "q@:" => overridden_value,
            }
        });

        // SAFETY: The classes and selector were registered above.
        unsafe {
            let selector = ffi::sel_registerName(c"value".as_ptr());
            assert_eq!(invoke_long_method(inherited_class, selector), 1);
            assert_eq!(invoke_long_method(overridden_class, selector), 2);
        }
    }

    unsafe fn invoke_long_method(class: ffi::Class, selector: ffi::Sel) -> c_long {
        // SAFETY: The caller provides a class containing a no-argument, long-returning method.
        let method = unsafe { ffi::class_getInstanceMethod(class, selector) };
        assert!(!method.is_null());
        // SAFETY: The method is non-null and its encoding is q@:.
        let implementation = unsafe { ffi::method_getImplementation(method) };
        // SAFETY: The method encoding matches this concrete function pointer type.
        let implementation: unsafe extern "C" fn(ffi::Id, ffi::Sel) -> c_long =
            unsafe { std::mem::transmute(implementation) };
        // SAFETY: The test implementations do not dereference the receiver.
        unsafe { implementation(std::ptr::null_mut::<c_void>(), selector) }
    }
}
