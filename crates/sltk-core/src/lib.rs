//! Code `sltk` shares between its crates. It holds the release number the CLI
//! reports.

/// The release of this build, from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
