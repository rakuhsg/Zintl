#[cfg(target_os = "macos")]
mod appkit;

pub mod actor;
pub mod messageloop;

pub use actor::*;
pub use messageloop::*;

#[cfg(target_os = "macos")]
pub use appkit::AppkitMessageLoop as PlatformMessageLoop;
