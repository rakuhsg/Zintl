//! Safe, main-thread-aware Rust ownership wrappers for AppKit.

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("zpd-appkit supports Apple Silicon macOS only");

pub mod actor;
pub mod geometry;
mod native;
pub mod runloop;
pub mod ui;

/// Runs an operation inside an Objective-C autorelease pool.
pub fn with_autorelease_pool<R>(operation: impl FnOnce() -> R) -> R {
    struct Pool(native::Id);

    impl Drop for Pool {
        fn drop(&mut self) {
            // SAFETY: The pool was returned by NSAutoreleasePool init and is drained once.
            unsafe { native::send_void(self.0, native::sel(b"drain\0")) };
        }
    }

    // SAFETY: NSAutoreleasePool implements the parameterless alloc/init methods.
    let pool = unsafe {
        native::send_id(
            native::send_id(
                native::class(b"NSAutoreleasePool\0"),
                native::sel(b"alloc\0"),
            ),
            native::sel(b"init\0"),
        )
    };
    assert!(!pool.is_null(), "NSAutoreleasePool initialization failed");
    let _pool = Pool(pool);
    operation()
}
