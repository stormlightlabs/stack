//! Code `sltk` shares between its crates: the release number the CLI reports,
//! the release train it pins, the install plan, the install itself, the
//! doctor's checks, the repository's host, capped child processes,
//! repository setup, the adoption of a repository set up by hand, the glue
//! both write, and the drift check.

pub mod adopt;
pub mod check;
pub mod doctor;
pub mod glue;
pub mod host;
pub mod install;
pub mod pin;
pub mod platform;
pub mod process;
pub mod setup;
pub mod train;

/// The release of this build, from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
