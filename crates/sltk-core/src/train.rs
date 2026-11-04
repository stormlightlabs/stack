//! The release train: the version of each tool an `sltk` release pins, read
//! from the `train.toml` the binary embeds.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;

/// The text of `train.toml`, embedded at build time.
pub const EMBEDDED: &str = include_str!("../../../train.toml");

/// Whether a tool always installs or only when asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Always installed; every repository `sltk` sets up has it.
    Essential,
    /// Installed only when asked for.
    Optional,
}

impl Tier {
    /// The name `train.toml` uses for the tier.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Essential => "essential",
            Tier::Optional => "optional",
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A platform `sltk` installs tools on, named by its Rust target triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub enum Platform {
    /// macOS on Apple silicon.
    #[serde(rename = "aarch64-apple-darwin")]
    AppleSilicon,
    /// Linux on x86-64.
    #[serde(rename = "x86_64-unknown-linux-gnu")]
    LinuxX86_64,
}

impl Platform {
    /// The target triple `train.toml` uses for the platform.
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::AppleSilicon => "aarch64-apple-darwin",
            Platform::LinuxX86_64 => "x86_64-unknown-linux-gnu",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a train could not be read.
#[derive(Debug, thiserror::Error)]
pub enum TrainError {
    /// The text is not TOML, or does not have the train's shape.
    #[error("train.toml: {0}")]
    Parse(#[from] toml::de::Error),
    /// A platform a tool lists has no sha256 for its archive.
    #[error("train.toml: {tool} has no sha256 for {platform}")]
    MissingHash { tool: String, platform: Platform },
    /// A sha256 is not 64 lowercase hexadecimal digits.
    #[error("train.toml: the sha256 of {tool} for {platform} is not 64 lowercase hexadecimal digits")]
    BadHash { tool: String, platform: Platform },
    /// The train lists no tools.
    #[error("train.toml: the train lists no tools")]
    NoTools,
    /// A tool lists no platforms.
    #[error("train.toml: {tool} lists no platforms")]
    NoPlatforms { tool: String },
    /// Two tools share a name.
    #[error("train.toml: {tool} is listed more than once")]
    RepeatedTool { tool: String },
    /// A URL template lacks a placeholder it needs.
    #[error("train.toml: the url of {tool} has no {placeholder}")]
    MissingPlaceholder { tool: String, placeholder: &'static str },
}

/// One release archive of a tool, for one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// The archive's file name, which replaces `{archive}` in the URL template.
    pub name: String,
    /// The archive's sha256, in lowercase hexadecimal.
    pub sha256: String,
}

/// A tool the train pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    /// The tool's name, as `--with` takes it.
    pub name: String,
    /// Whether it always installs.
    pub tier: Tier,
    /// The binary its archive holds.
    pub binary: String,
    /// The pinned release.
    pub version: String,
    /// The download URL template, with `{version}` and `{archive}`.
    pub url: String,
    /// The archive for each platform the tool supports.
    pub platforms: BTreeMap<Platform, Archive>,
    /// The commands that set a repository up, each as program and arguments.
    pub setup: Vec<Vec<String>>,
}

/// The pinned set of tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Train {
    /// The tools, in the order `train.toml` lists them.
    pub tools: Vec<Tool>,
}

impl Train {
    /// Reads the train this binary embeds.
    pub fn embedded() -> Result<Train, TrainError> {
        Train::parse(EMBEDDED)
    }

    /// Reads a train from the text of a `train.toml`. A train must list at
    /// least one tool, and no tool name twice.
    pub fn parse(text: &str) -> Result<Train, TrainError> {
        let raw: RawTrain = toml::from_str(text)?;
        if raw.tool.is_empty() {
            return Err(TrainError::NoTools);
        }
        let mut names = BTreeSet::new();
        let mut tools = Vec::with_capacity(raw.tool.len());
        for raw_tool in raw.tool {
            if !names.insert(raw_tool.name.clone()) {
                return Err(TrainError::RepeatedTool { tool: raw_tool.name });
            }
            tools.push(raw_tool.check()?);
        }
        Ok(Train { tools })
    }
}

/// `train.toml` as written, before each hash is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrain {
    #[serde(default)]
    tool: Vec<RawTool>,
}

/// A tool as written, before each hash is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTool {
    name: String,
    tier: Tier,
    binary: String,
    version: String,
    url: String,
    platforms: BTreeMap<Platform, RawArchive>,
    #[serde(default)]
    setup: Vec<Vec<String>>,
}

impl RawTool {
    /// Checks that the URL template has both placeholders, that the tool
    /// lists a platform, and that every listed platform has a well-formed
    /// sha256.
    ///
    /// `{version}` is required because dist's archive names carry no version:
    /// without it in the URL, every version would download the same file.
    fn check(self) -> Result<Tool, TrainError> {
        for placeholder in ["{version}", "{archive}"] {
            if !self.url.contains(placeholder) {
                return Err(TrainError::MissingPlaceholder { tool: self.name, placeholder });
            }
        }
        if self.platforms.is_empty() {
            return Err(TrainError::NoPlatforms { tool: self.name });
        }
        let mut platforms = BTreeMap::new();
        for (platform, archive) in self.platforms {
            let Some(sha256) = archive.sha256 else {
                return Err(TrainError::MissingHash { tool: self.name, platform });
            };
            if sha256.len() != 64 || !sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err(TrainError::BadHash { tool: self.name, platform });
            }
            platforms.insert(platform, Archive { name: archive.archive, sha256 });
        }
        Ok(Tool {
            name: self.name,
            tier: self.tier,
            binary: self.binary,
            version: self.version,
            url: self.url,
            platforms,
            setup: self.setup,
        })
    }
}

/// An archive as written; the hash is optional here so a missing one gets an
/// error that names the tool and platform.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArchive {
    archive: String,
    sha256: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{Platform, Tier, Train, TrainError};

    const HASH: &str = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0";

    fn one_tool(tier: &str, linux_hash: Option<&str>) -> String {
        let linux_hash = linux_hash.map(|h| format!("sha256 = \"{h}\"\n")).unwrap_or_default();
        format!(
            r#"
[[tool]]
name = "fab"
tier = "{tier}"
binary = "fab"
version = "0.8.0"
url = "https://example.test/v{{version}}/{{archive}}"

[tool.platforms.aarch64-apple-darwin]
archive = "fab-cli-aarch64-apple-darwin.tar.xz"
sha256 = "{HASH}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "fab-cli-x86_64-unknown-linux-gnu.tar.xz"
{linux_hash}"#
        )
    }

    #[test]
    fn the_embedded_train_parses() {
        let train = Train::embedded().unwrap();
        let names: Vec<_> = train.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["fab", "arcl"]);
        for tool in &train.tools {
            assert_eq!(tool.tier, Tier::Essential);
            assert!(tool.platforms.contains_key(&Platform::AppleSilicon), "{}", tool.name);
            assert!(tool.platforms.contains_key(&Platform::LinuxX86_64), "{}", tool.name);
        }
        let arcl = &train.tools[1];
        assert_eq!(
            arcl.setup,
            [vec!["arcl", "init", "--workspace"], vec!["arcl", "skill", "install"]]
        );
    }

    #[test]
    fn a_complete_train_parses() {
        let train = Train::parse(&one_tool("optional", Some(HASH))).unwrap();
        assert_eq!(train.tools[0].tier, Tier::Optional);
        assert!(train.tools[0].setup.is_empty());
    }

    #[test]
    fn a_missing_hash_names_the_tool_and_platform() {
        let err = Train::parse(&one_tool("essential", None)).unwrap_err();
        assert!(
            matches!(&err, TrainError::MissingHash { tool, platform: Platform::LinuxX86_64 } if tool == "fab"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "train.toml: fab has no sha256 for x86_64-unknown-linux-gnu"
        );
    }

    #[test]
    fn a_malformed_hash_is_rejected() {
        let err = Train::parse(&one_tool("essential", Some("abc123"))).unwrap_err();
        assert!(matches!(err, TrainError::BadHash { .. }), "{err:?}");
    }

    #[test]
    fn a_hash_with_a_non_hex_digit_is_rejected() {
        let hash = format!("{}g", &HASH[..63]);
        let err = Train::parse(&one_tool("essential", Some(&hash))).unwrap_err();
        assert!(matches!(err, TrainError::BadHash { .. }), "{err:?}");
    }

    #[test]
    fn an_uppercase_hash_is_rejected() {
        let err = Train::parse(&one_tool("essential", Some(&HASH.to_uppercase()))).unwrap_err();
        assert_eq!(
            err.to_string(),
            "train.toml: the sha256 of fab for x86_64-unknown-linux-gnu is not 64 lowercase hexadecimal digits"
        );
    }

    #[test]
    fn an_empty_train_is_rejected() {
        let err = Train::parse("").unwrap_err();
        assert!(matches!(err, TrainError::NoTools), "{err:?}");
    }

    #[test]
    fn a_tool_with_no_platforms_is_rejected() {
        let text = r#"
[[tool]]
name = "fab"
tier = "essential"
binary = "fab"
version = "0.8.0"
url = "https://example.test/v{version}/{archive}"
platforms = {}
"#;
        let err = Train::parse(text).unwrap_err();
        assert_eq!(err.to_string(), "train.toml: fab lists no platforms");
    }

    #[test]
    fn a_repeated_tool_is_rejected() {
        let tool = one_tool("essential", Some(HASH));
        let err = Train::parse(&format!("{tool}\n{tool}")).unwrap_err();
        assert_eq!(err.to_string(), "train.toml: fab is listed more than once");
    }

    #[test]
    fn a_url_without_archive_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace("/{archive}", "/fab.tar.xz");
        let err = Train::parse(&text).unwrap_err();
        assert_eq!(err.to_string(), "train.toml: the url of fab has no {archive}");
    }

    #[test]
    fn a_url_without_version_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace("/v{version}", "/latest");
        let err = Train::parse(&text).unwrap_err();
        assert_eq!(err.to_string(), "train.toml: the url of fab has no {version}");
    }

    #[test]
    fn an_unknown_tier_is_rejected() {
        let err = Train::parse(&one_tool("nice-to-have", Some(HASH))).unwrap_err();
        assert!(matches!(err, TrainError::Parse(_)), "{err:?}");
        assert!(err.to_string().contains("unknown variant `nice-to-have`"), "{err}");
    }

    #[test]
    fn an_unsupported_platform_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace("x86_64-unknown-linux-gnu]", "x86_64-pc-windows-msvc]");
        let err = Train::parse(&text).unwrap_err();
        assert!(matches!(err, TrainError::Parse(_)), "{err:?}");
    }
}
