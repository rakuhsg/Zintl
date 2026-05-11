mod appkit;
pub mod messageloop;

pub use messageloop::*;

#[cfg(target_os = "macos")]
pub use appkit::AppkitMessageLoop as PlatformMessageLoop;
