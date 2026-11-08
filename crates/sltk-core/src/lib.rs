//! Code `sltk` shares between its crates: the release number the CLI reports,
//! the release train it pins, the install plan and the install itself.

pub mod install;
pub mod pin;
pub mod platform;
pub mod train;

/// The release of this build, from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
