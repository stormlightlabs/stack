//! The host `sltk` runs on and the install plan for it: which tools the train
//! has an archive for, and which it cannot install there.

use std::fmt;

use crate::train::{Archive, Platform, Tier, Tool, Train};

/// The machine `sltk` installs onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// A platform `sltk` supports.
    Supported(Platform),
    /// Any other machine, named by the architecture and OS Rust reports.
    Unsupported {
        /// The architecture, as `std::env::consts::ARCH` names it.
        arch: String,
        /// The operating system, as `std::env::consts::OS` names it.
        os: String,
    },
}

impl Host {
    /// The host this binary runs on.
    pub fn detect() -> Host {
        Host::from_parts(std::env::consts::ARCH, std::env::consts::OS)
    }

    /// The host with this architecture and OS, named as
    /// `std::env::consts::ARCH` and `std::env::consts::OS` name them.
    pub fn from_parts(arch: &str, os: &str) -> Host {
        match (arch, os) {
            ("aarch64", "macos") => Host::Supported(Platform::AppleSilicon),
            ("x86_64", "linux") => Host::Supported(Platform::LinuxX86_64),
            _ => Host::Unsupported { arch: arch.to_owned(), os: os.to_owned() },
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
    /// A supported host shows its target triple; any other shows its
    /// architecture and OS.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Supported(platform) => write!(f, "{platform}"),
            Host::Unsupported { arch, os } => write!(f, "{arch} {os}"),
        }
    }
}

/// A tool the plan installs, with the archive it would download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTool {
    /// The tool's name.
    pub name: String,
    /// The pinned release.
    pub version: String,
    /// The archive for the host's platform.
    pub archive: Archive,
}

/// A tool the plan cannot install, because the train has no archive of it for
/// the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
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
    pub unsupported: Vec<Unsupported>,
}

impl Plan {
    /// Plans the install of the essential tools, and of each optional tool
    /// named in `with`, onto `host`. On an unsupported host every selected
    /// tool is unsupported.
    pub fn new(train: &Train, host: Host, with: &[&str]) -> Plan {
        let mut install = Vec::new();
        let mut unsupported = Vec::new();
        for tool in train.tools.iter().filter(|tool| selected(tool, with)) {
            match host.platform().and_then(|platform| tool.platforms.get(&platform)) {
                Some(archive) => install.push(PlannedTool {
                    name: tool.name.clone(),
                    version: tool.version.clone(),
                    archive: archive.clone(),
                }),
                None => unsupported.push(Unsupported { name: tool.name.clone(), version: tool.version.clone() }),
            }
        }
        Plan { host, install, unsupported }
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
    use super::{Host, Plan};
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
        let mac = Host::from_parts("aarch64", "macos");
        assert_eq!(mac, Host::Supported(Platform::AppleSilicon));
        assert_eq!(mac.to_string(), "aarch64-apple-darwin");
        let linux = Host::from_parts("x86_64", "linux");
        assert_eq!(linux, Host::Supported(Platform::LinuxX86_64));
        assert_eq!(linux.to_string(), "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn other_hosts_are_unsupported() {
        for (arch, os) in [("x86_64", "macos"), ("aarch64", "linux"), ("x86_64", "windows")] {
            let host = Host::from_parts(arch, os);
            assert_eq!(host.platform(), None, "{arch} {os}");
            assert_eq!(host.to_string(), format!("{arch} {os}"));
        }
    }

    #[test]
    fn detect_reads_the_build_target() {
        assert_eq!(
            Host::detect(),
            Host::from_parts(std::env::consts::ARCH, std::env::consts::OS)
        );
    }

    #[test]
    fn both_supported_targets_plan_every_embedded_tool() {
        let train = Train::embedded().unwrap();
        for platform in [Platform::AppleSilicon, Platform::LinuxX86_64] {
            let plan = Plan::new(&train, Host::Supported(platform), &[]);
            assert!(plan.is_complete(), "{platform}: {:?}", plan.unsupported);
            assert_eq!(names(&plan.install, |t| &t.name), ["fab", "arcl"]);
            for planned in &plan.install {
                let tool = train.tools.iter().find(|t| t.name == planned.name).unwrap();
                assert_eq!(planned.archive, tool.platforms[&platform]);
                assert_eq!(planned.version, tool.version);
            }
        }
    }

    #[test]
    fn an_unsupported_host_lists_every_tool_as_unsupported() {
        let train = Train::embedded().unwrap();
        let plan = Plan::new(&train, Host::from_parts("aarch64", "linux"), &[]);
        assert!(!plan.is_complete());
        assert!(plan.install.is_empty());
        assert_eq!(names(&plan.unsupported, |t| &t.name), ["fab", "arcl"]);
    }

    #[test]
    fn a_tool_with_no_archive_for_the_platform_is_unsupported() {
        let train = Train::parse(FIXTURE).unwrap();
        let plan = Plan::new(&train, Host::Supported(Platform::AppleSilicon), &[]);
        assert_eq!(names(&plan.install, |t| &t.name), ["alpha"]);
        assert_eq!(names(&plan.unsupported, |t| &t.name), ["beta"]);
    }

    #[test]
    fn an_optional_tool_is_planned_only_when_named() {
        let train = Train::parse(FIXTURE).unwrap();
        let linux = Host::Supported(Platform::LinuxX86_64);
        let without = Plan::new(&train, linux.clone(), &[]);
        assert_eq!(names(&without.install, |t| &t.name), ["alpha", "beta"]);
        let with = Plan::new(&train, linux, &["gamma"]);
        assert_eq!(names(&with.install, |t| &t.name), ["alpha", "beta", "gamma"]);
        let mac = Plan::new(&train, Host::Supported(Platform::AppleSilicon), &["gamma"]);
        assert_eq!(names(&mac.unsupported, |t| &t.name), ["beta", "gamma"]);
    }
}
