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
    struct Pool(zpd_objc::Id);

    impl Drop for Pool {
        fn drop(&mut self) {
            // SAFETY: The pool was returned by NSAutoreleasePool init and is drained once.
            unsafe { zpd_objc::msg_send!(self.0, zpd_objc::sel!("drain"), () => ()) };
        }
    }

    // SAFETY: NSAutoreleasePool implements the parameterless alloc/init methods.
    let pool = unsafe {
        zpd_objc::msg_send!(zpd_objc::msg_send!(zpd_objc::class!("NSAutoreleasePool"), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("init"), () => zpd_objc::Id)
    };
    assert!(!pool.is_null(), "NSAutoreleasePool initialization failed");
    let _pool = Pool(pool);
    operation()
}
