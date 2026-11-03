//! The core of `sltk`. It will read the release train and a repository's pin
//! and plan what to install onto a machine or write into a repository, so the
//! CLI can show a plan before it changes anything. For now it holds the
//! release number.

/// The release of this build, from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn the_version_is_the_workspace_release() {
        assert_eq!(VERSION, "0.0.0");
    }
}
