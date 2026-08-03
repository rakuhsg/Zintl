use std::ffi::c_void;

/// A borrowed UTF-8 string passed across the native ABI.
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct NativeString {
    bytes: *const u8,
    length: usize,
}

impl NativeString {
    pub(crate) fn from_str(value: &str) -> Self {
        Self {
            bytes: value.as_ptr(),
            length: value.len(),
        }
    }

    unsafe fn as_bytes<'a>(self) -> &'a [u8] {
        if self.length == 0 {
            return &[];
        }
        assert!(!self.bytes.is_null(), "native string bytes are null");
        // SAFETY: The native caller guarantees that `bytes` remains valid for
        // `length` bytes during the synchronous FFI call.
        unsafe { std::slice::from_raw_parts(self.bytes, self.length) }
    }

    pub(crate) unsafe fn to_string(self) -> String {
        // SAFETY: The caller upholds the native string buffer contract.
        let bytes = unsafe { self.as_bytes() };
        String::from_utf8(bytes.to_vec()).expect("native string must contain valid UTF-8")
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct NativeOptionalString {
    value: NativeString,
    is_some: bool,
}

impl NativeOptionalString {
    pub(crate) fn from_option(value: Option<&str>) -> Self {
        match value {
            Some(value) => Self {
                value: NativeString::from_str(value),
                is_some: true,
            },
            None => Self {
                value: NativeString::from_str(""),
                is_some: false,
            },
        }
    }
}

pub(crate) type NativeStringCallback =
    unsafe extern "C" fn(user_data: *mut c_void, value: NativeString);

unsafe extern "C" fn copy_native_string(user_data: *mut c_void, value: NativeString) {
    // SAFETY: `user_data` points to the output String for the duration of this
    // synchronous callback, and native code owns `value` for the call.
    let output = unsafe { &mut *user_data.cast::<String>() };
    // SAFETY: The callback receives a valid native UTF-8 buffer.
    *output = unsafe { value.to_string() };
}

pub(crate) fn receive_native_string(
    invoke: impl FnOnce(*mut c_void, NativeStringCallback),
) -> String {
    let mut output = String::new();
    invoke((&mut output as *mut String).cast(), copy_native_string);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_string_preserves_utf8_and_interior_nul() {
        let value = "AppKit ↔ Rust\0string";
        let native = NativeString::from_str(value);
        // SAFETY: `native` borrows `value`, which remains live for the call.
        assert_eq!(unsafe { native.to_string() }, value);

        let received = receive_native_string(|user_data, callback| {
            // SAFETY: The callback is synchronous and `native` remains live.
            unsafe { callback(user_data, native) };
        });
        assert_eq!(received, value);
    }
}
