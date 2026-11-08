//! The host `sltk` runs on and the install plan for it: which tools the train
//! has an archive for, and which it cannot install there.

use std::fmt;

use crate::train::{Archive, Platform, Tier, Tool, Train};

/// The C library this binary was built against, as `cfg(target_env)` names
/// it: `gnu` or `musl` on Linux, and empty on macOS.
const TARGET_ENV: &str = if cfg!(target_env = "musl") {
    "musl"
} else if cfg!(target_env = "gnu") {
    "gnu"
} else {
    ""
};

/// The pointer width this binary was built for, as `cfg(target_pointer_width)`
/// names it.
const TARGET_POINTER_WIDTH: u32 = usize::BITS;

/// The machine `sltk` installs onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// A platform `sltk` supports.
    Supported(Platform),
    /// Any other machine, named by the architecture, OS and C library Rust
    /// reports.
    Unsupported {
        /// The architecture, as `std::env::consts::ARCH` names it.
        arch: String,
        /// The operating system, as `std::env::consts::OS` names it.
        os: String,
        /// The C library, as `cfg(target_env)` names it; empty when the
        /// target names none.
        env: String,
        /// The pointer width in bits, as `cfg(target_pointer_width)` names
        /// it.
        pointer_width: u32,
    },
}

impl Host {
    /// The host from the build target of this `sltk` binary, not from the
    /// machine it runs on.
    ///
    /// An x86_64 `sltk` running under Rosetta on Apple silicon reports an
    /// unsupported x86_64 macOS; replace it with the aarch64 build.
    pub fn detect() -> Host {
        Host::from_parts(
            std::env::consts::ARCH,
            std::env::consts::OS,
            TARGET_ENV,
            TARGET_POINTER_WIDTH,
        )
    }

    /// The host with this architecture, OS, C library and pointer width,
    /// named as `std::env::consts::ARCH`, `std::env::consts::OS`,
    /// `cfg(target_env)` and `cfg(target_pointer_width)` name them.
    ///
    /// Both supported platforms need 64-bit pointers, so an x32 Linux, which
    /// reports `x86_64` and `gnu` with 32-bit pointers, is unsupported. A musl
    /// Linux is unsupported too: fab's gnu archive is not statically linked,
    /// so it may not run there.
    pub fn from_parts(arch: &str, os: &str, env: &str, pointer_width: u32) -> Host {
        match (arch, os, env, pointer_width) {
            ("aarch64", "macos", _, 64) => Host::Supported(Platform::AppleSilicon),
            ("x86_64", "linux", "gnu", 64) => Host::Supported(Platform::LinuxX86_64),
            _ => Host::Unsupported { arch: arch.to_owned(), os: os.to_owned(), env: env.to_owned(), pointer_width },
        }
    }

    /// The platform, when `sltk` supports the host.
    pub fn platform(&self) -> Option<Platform> {
        match self {
            Host::Supported(platform) => Some(*platform),
            Host::Unsupported { .. } => None,
        }
    }
}

impl fmt::Display for Host {
    /// A supported host shows its target triple. Any other shows its
    /// architecture and OS, then its C library on a Linux whose C library is
    /// not `gnu`, then its pointer width when that is not 64 bits.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Supported(platform) => write!(f, "{platform}"),
            Host::Unsupported { arch, os, env, pointer_width } => {
                write!(f, "{arch} {os}")?;
                if os == "linux" && env != "gnu" && !env.is_empty() {
                    write!(f, " {env}")?;
                }
                if *pointer_width != 64 {
                    write!(f, " {pointer_width}-bit")?;
                }
                Ok(())
            }
        }
    }
}

/// Why a plan could not be made from the tools asked for.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// `with` names a tool the train does not list.
    #[error("sltk: the train has no tool named {tool}")]
    UnknownTool {
        /// The name as given.
        tool: String,
    },
    /// `with` names an essential tool, which every plan installs already.
    #[error("sltk: {tool} is always installed, so it needs no --with")]
    EssentialTool {
        /// The tool's name.
        tool: String,
    },
}

/// A tool the plan installs, with the archive it would download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTool {
    /// The tool's name.
    pub name: String,
    /// The pinned release.
    pub version: String,
    /// The binary the archive holds.
    pub binary: String,
    /// The archive for the host's platform.
    pub archive: Archive,
    /// The archive's download URL: the tool's template with `{version}` and
    /// `{archive}` replaced.
    pub url: String,
}

/// A tool the plan cannot install, because the train has no archive of it for
/// the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedTool {
    /// The tool's name.
    pub name: String,
    /// The pinned release.
    pub version: String,
}

/// What `sltk install` would do on a host, worked out before anything is
/// downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The host the plan is for.
    pub host: Host,
    /// The selected tools with an archive for the host, in train order.
    pub install: Vec<PlannedTool>,
    /// The selected tools with no archive for the host, in train order.
    pub unsupported: Vec<UnsupportedTool>,
}

impl Plan {
    /// Plans the install of the essential tools, and of each optional tool
    /// named in `with`, onto `host`. On an unsupported host every selected
    /// tool is unsupported.
    ///
    /// Fails when `with` names a tool the train does not list, or an
    /// essential tool.
    pub fn new(train: &Train, host: Host, with: &[&str]) -> Result<Plan, PlanError> {
        for &name in with {
            match train.tools.iter().find(|tool| tool.name == name) {
                None => return Err(PlanError::UnknownTool { tool: name.to_owned() }),
                Some(tool) if tool.tier == Tier::Essential => {
                    return Err(PlanError::EssentialTool { tool: name.to_owned() });
                }
                Some(_) => {}
            }
        }
        let mut install = Vec::new();
        let mut unsupported = Vec::new();
        for tool in train.tools.iter().filter(|tool| selected(tool, with)) {
            match host.platform().and_then(|platform| tool.platforms.get(&platform)) {
                Some(archive) => install.push(PlannedTool {
                    name: tool.name.clone(),
                    version: tool.version.clone(),
                    binary: tool.binary.clone(),
                    archive: archive.clone(),
                    url: tool
                        .url
                        .replace("{version}", &tool.version)
                        .replace("{archive}", &archive.name),
                }),
                None => unsupported.push(UnsupportedTool { name: tool.name.clone(), version: tool.version.clone() }),
            }
        }
        Ok(Plan { host, install, unsupported })
    }

    /// Whether every selected tool has an archive for the host.
    pub fn is_complete(&self) -> bool {
        self.unsupported.is_empty()
    }
}

/// Whether the plan includes `tool`: every essential, and an optional tool
/// only when `with` names it.
fn selected(tool: &Tool, with: &[&str]) -> bool {
    tool.tier == Tier::Essential || with.contains(&tool.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::{Host, Plan, PlanError};
    use crate::train::{Platform, Train};

    /// One essential tool on both platforms, one essential on Linux only, and
    /// one optional tool.
    const FIXTURE: &str = r#"
[[tool]]
name = "alpha"
tier = "essential"
binary = "alpha"
version = "1.2.3"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.aarch64-apple-darwin]
archive = "alpha-aarch64-apple-darwin.tar.xz"
sha256 = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "alpha-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "81cf21ffd2140482a4ea315e60ca34ead891c18e9aa65e7f05daf7fda2404498"

[[tool]]
name = "beta"
tier = "essential"
binary = "beta"
version = "0.1.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "beta-x86_64-unknown-linux-musl.tar.xz"
sha256 = "7cf3c94e04bac6c9dff4fd357fc8133050fe0dbbca9d4d1dab2e4ecd8c5e13c3"

[[tool]]
name = "gamma"
tier = "optional"
binary = "gamma"
version = "2.0.0"
url = "https://example.test/v{version}/{archive}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "gamma-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "80f6bc7e4ce41f7037c91330c814d94e277aed381d192960d00a130a859675e3"
"#;

    fn names<T>(items: &[T], name: impl Fn(&T) -> &str) -> Vec<&str> {
        items.iter().map(name).collect()
    }

    #[test]
    fn the_supported_hosts_map_to_their_triples() {
        let mac = Host::from_parts("aarch64", "macos", "", 64);
        assert_eq!(mac, Host::Supported(Platform::AppleSilicon));
        assert_eq!(mac.to_string(), "aarch64-apple-darwin");
        let linux = Host::from_parts("x86_64", "linux", "gnu", 64);
        assert_eq!(linux, Host::Supported(Platform::LinuxX86_64));
        assert_eq!(linux.to_string(), "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn other_hosts_are_unsupported() {
        for (arch, os, env) in [
            ("x86_64", "macos", ""),
            ("aarch64", "linux", "gnu"),
            ("x86_64", "windows", ""),
        ] {
            let host = Host::from_parts(arch, os, env, 64);
            assert_eq!(host.platform(), None, "{arch} {os}");
            assert_eq!(host.to_string(), format!("{arch} {os}"));
        }
    }

    #[test]
    fn a_musl_linux_is_unsupported() {
        let train = Train::embedded().unwrap();
        let host = Host::from_parts("x86_64", "linux", "musl", 64);
        assert_eq!(host.platform(), None);
        assert_eq!(host.to_string(), "x86_64 linux musl");
        let plan = Plan::new(&train, host, &[]).unwrap();
        assert!(plan.install.is_empty());
        assert_eq!(names(&plan.unsupported, |t| &t.name), ["fab", "arcl"]);
    }

    #[test]
    fn a_linux_with_another_c_library_shows_it() {
        let host = Host::from_parts("x86_64", "linux", "uclibc", 64);
        assert_eq!(host.platform(), None);
        assert_eq!(host.to_string(), "x86_64 linux uclibc");
    }

    #[test]
    fn an_x32_linux_is_unsupported() {
        let train = Train::embedded().unwrap();
        let host = Host::from_parts("x86_64", "linux", "gnu", 32);
        assert_eq!(host.platform(), None);
        assert_eq!(host.to_string(), "x86_64 linux 32-bit");
        let plan = Plan::new(&train, host, &[]).unwrap();
        assert!(plan.install.is_empty());
        assert_eq!(names(&plan.unsupported, |t| &t.name), ["fab", "arcl"]);
    }

    #[test]
    fn a_32_bit_mac_target_is_unsupported() {
        assert_eq!(Host::from_parts("aarch64", "macos", "", 32).platform(), None);
    }

    #[test]
    fn both_supported_targets_plan_every_embedded_tool() {
        let train = Train::embedded().unwrap();
        for platform in [Platform::AppleSilicon, Platform::LinuxX86_64] {
            let plan = Plan::new(&train, Host::Supported(platform), &[]).unwrap();
            assert!(plan.is_complete(), "{platform}: {:?}", plan.unsupported);
            assert_eq!(names(&plan.install, |t| &t.name), ["fab", "arcl"]);
            for planned in &plan.install {
                let tool = train.tools.iter().find(|t| t.name == planned.name).unwrap();
                assert_eq!(planned.archive, tool.platforms[&platform]);
                assert_eq!(planned.version, tool.version);
                assert_eq!(planned.binary, tool.binary);
            }
        }
    }

    #[test]
    fn the_url_has_the_version_and_archive_filled_in() {
        let train = Train::parse(FIXTURE).unwrap();
        let plan = Plan::new(&train, Host::Supported(Platform::LinuxX86_64), &[]).unwrap();
        assert_eq!(
            plan.install[0].url,
            "https://example.test/v1.2.3/alpha-x86_64-unknown-linux-gnu.tar.xz"
        );
    }

    #[test]
    fn an_unsupported_host_lists_every_tool_as_unsupported() {
        let train = Train::embedded().unwrap();
        let plan = Plan::new(&train, Host::from_parts("aarch64", "linux", "gnu", 64), &[]).unwrap();
        assert!(!plan.is_complete());
        assert!(plan.install.is_empty());
        assert_eq!(names(&plan.unsupported, |t| &t.name), ["fab", "arcl"]);
    }

    #[test]
    fn a_tool_with_no_archive_for_the_platform_is_unsupported() {
        let train = Train::parse(FIXTURE).unwrap();
        let plan = Plan::new(&train, Host::Supported(Platform::AppleSilicon), &[]).unwrap();
        assert_eq!(names(&plan.install, |t| &t.name), ["alpha"]);
        assert_eq!(names(&plan.unsupported, |t| &t.name), ["beta"]);
    }

    #[test]
    fn an_optional_tool_is_planned_only_when_named() {
        let train = Train::parse(FIXTURE).unwrap();
        let linux = Host::Supported(Platform::LinuxX86_64);
        let without = Plan::new(&train, linux.clone(), &[]).unwrap();
        assert_eq!(names(&without.install, |t| &t.name), ["alpha", "beta"]);
        let with = Plan::new(&train, linux, &["gamma"]).unwrap();
        assert_eq!(names(&with.install, |t| &t.name), ["alpha", "beta", "gamma"]);
        let mac = Plan::new(&train, Host::Supported(Platform::AppleSilicon), &["gamma"]).unwrap();
        assert_eq!(names(&mac.unsupported, |t| &t.name), ["beta", "gamma"]);
    }

    #[test]
    fn with_naming_a_tool_the_train_lacks_is_an_error() {
        let train = Train::parse(FIXTURE).unwrap();
        let err = Plan::new(&train, Host::Supported(Platform::LinuxX86_64), &["gama"]).unwrap_err();
        assert_eq!(err, PlanError::UnknownTool { tool: "gama".to_owned() });
        assert_eq!(err.to_string(), "sltk: the train has no tool named gama");
    }

    #[test]
    fn with_naming_an_essential_tool_is_an_error() {
        let train = Train::parse(FIXTURE).unwrap();
        let err = Plan::new(&train, Host::Supported(Platform::LinuxX86_64), &["gamma", "alpha"]).unwrap_err();
        assert_eq!(err, PlanError::EssentialTool { tool: "alpha".to_owned() });
        assert_eq!(
            err.to_string(),
            "sltk: alpha is always installed, so it needs no --with"
        );
    }
}
