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

    /// The architecture and OS an archive name must contain to be built for
    /// the platform. Linux matches on `linux` alone so a musl build can serve
    /// the gnu target.
    pub fn archive_markers(self) -> [&'static str; 2] {
        match self {
            Platform::AppleSilicon => ["aarch64", "apple-darwin"],
            Platform::LinuxX86_64 => ["x86_64", "linux"],
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
    /// A URL template does not start with `https://`.
    #[error("train.toml: the url of {tool} does not start with https://")]
    InsecureUrl { tool: String },
    /// A tool has an empty name.
    #[error("train.toml: a tool has an empty name")]
    EmptyName,
    /// A field of a tool is empty.
    #[error("train.toml: {tool} has an empty {field}")]
    EmptyField { tool: String, field: String },
    /// A field holds a path separator, `..`, or a URL delimiter, so it could
    /// leave the directory or URL it is placed in.
    #[error("train.toml: the {field} of {tool} contains {found:?}")]
    BadCharacter {
        tool: String,
        field: String,
        found: &'static str,
    },
    /// The tool name or binary is `.`.
    #[error("train.toml: the {field} of {tool} is \".\"")]
    DotName { tool: String, field: &'static str },
    /// A setup command has no program.
    #[error("train.toml: {tool} has an empty setup command")]
    EmptySetup { tool: String },
    /// A setup command runs a program other than the tool's binary.
    #[error("train.toml: a setup command of {tool} runs {program}, not {binary}")]
    ForeignSetup {
        tool: String,
        program: String,
        binary: String,
    },
    /// An archive name lacks the architecture or OS of its platform.
    #[error("train.toml: the archive of {tool} for {platform} is not named for {marker}")]
    WrongArchive {
        tool: String,
        platform: Platform,
        marker: &'static str,
    },
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
    /// Checks every field before a tool reaches an install.
    ///
    /// The name and binary must be plain file names, since the install joins
    /// `binary` onto a directory. The version and archive names must not leave
    /// the URL path segment they replace. The URL must be https and have both
    /// placeholders: `{version}` because dist's archive names carry no
    /// version, so without it every version would download the same file.
    /// Each setup command must run the tool's own binary. Each platform needs
    /// a well-formed sha256 and an archive named for its architecture and OS.
    fn check(self) -> Result<Tool, TrainError> {
        if self.name.is_empty() {
            return Err(TrainError::EmptyName);
        }
        check_file_name(&self.name, "name", &self.name)?;
        check_file_name(&self.name, "binary", &self.binary)?;
        check_segment(&self.name, "version", &self.version)?;
        if !self.url.starts_with("https://") {
            return Err(TrainError::InsecureUrl { tool: self.name });
        }
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
            check_segment(&self.name, &format!("archive for {platform}"), &archive.archive)?;
            for marker in platform.archive_markers() {
                if !archive.archive.contains(marker) {
                    return Err(TrainError::WrongArchive { tool: self.name, platform, marker });
                }
            }
            platforms.insert(platform, Archive { name: archive.archive, sha256 });
        }
        for command in &self.setup {
            let Some(program) = command.first() else {
                return Err(TrainError::EmptySetup { tool: self.name });
            };
            if *program != self.binary {
                return Err(TrainError::ForeignSetup {
                    tool: self.name.clone(),
                    program: program.clone(),
                    binary: self.binary.clone(),
                });
            }
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

/// Checks that `value` is a plain file name: not empty, not `.`, and free of
/// `/`, `\` and `..`.
fn check_file_name(tool: &str, field: &'static str, value: &str) -> Result<(), TrainError> {
    if value == "." {
        return Err(TrainError::DotName { tool: tool.to_owned(), field });
    }
    check_chars(tool, field, value, &["/", "\\", ".."])
}

/// Checks that `value` can replace a placeholder in a URL path segment: not
/// empty, and free of `/`, `?`, `#` and `..`.
fn check_segment(tool: &str, field: &str, value: &str) -> Result<(), TrainError> {
    check_chars(tool, field, value, &["/", "?", "#", ".."])
}

/// Checks that `value` is not empty and contains none of `banned`.
fn check_chars(tool: &str, field: &str, value: &str, banned: &[&'static str]) -> Result<(), TrainError> {
    if value.is_empty() {
        return Err(TrainError::EmptyField { tool: tool.to_owned(), field: field.to_owned() });
    }
    match banned.iter().find(|b| value.contains(**b)) {
        Some(found) => Err(TrainError::BadCharacter { tool: tool.to_owned(), field: field.to_owned(), found }),
        None => Ok(()),
    }
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

    fn expect_error(text: &str, message: &str) {
        let err = Train::parse(text).unwrap_err();
        assert_eq!(err.to_string(), message);
    }

    #[test]
    fn an_empty_name_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace("name = \"fab\"", "name = \"\"");
        expect_error(&text, "train.toml: a tool has an empty name");
    }

    #[test]
    fn empty_fields_are_rejected() {
        let tool = one_tool("essential", Some(HASH));
        for (from, to, message) in [
            (
                "binary = \"fab\"",
                "binary = \"\"",
                "train.toml: fab has an empty binary",
            ),
            (
                "version = \"0.8.0\"",
                "version = \"\"",
                "train.toml: fab has an empty version",
            ),
            (
                "archive = \"fab-cli-aarch64-apple-darwin.tar.xz\"",
                "archive = \"\"",
                "train.toml: fab has an empty archive for aarch64-apple-darwin",
            ),
        ] {
            expect_error(&tool.replace(from, to), message);
        }
    }

    #[test]
    fn a_name_or_binary_that_is_not_a_plain_file_name_is_rejected() {
        let tool = one_tool("essential", Some(HASH));
        for (from, to, message) in [
            (
                "binary = \"fab\"",
                r#"binary = "../../.bashrc""#,
                r#"train.toml: the binary of fab contains "/""#,
            ),
            (
                "binary = \"fab\"",
                r#"binary = "/etc/passwd""#,
                r#"train.toml: the binary of fab contains "/""#,
            ),
            (
                "binary = \"fab\"",
                r#"binary = 'fab\..'"#,
                r#"train.toml: the binary of fab contains "\\""#,
            ),
            (
                "binary = \"fab\"",
                r#"binary = "..""#,
                r#"train.toml: the binary of fab contains "..""#,
            ),
            (
                "binary = \"fab\"",
                r#"binary = ".""#,
                r#"train.toml: the binary of fab is ".""#,
            ),
            (
                "name = \"fab\"",
                r#"name = "a/b""#,
                r#"train.toml: the name of a/b contains "/""#,
            ),
            (
                "name = \"fab\"",
                r#"name = "..""#,
                r#"train.toml: the name of .. contains "..""#,
            ),
            ("name = \"fab\"", r#"name = ".""#, r#"train.toml: the name of . is ".""#),
        ] {
            expect_error(&tool.replace(from, to), message);
        }
    }

    #[test]
    fn an_empty_setup_command_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace("url =", "setup = [[]]\nurl =");
        expect_error(&text, "train.toml: fab has an empty setup command");
    }

    #[test]
    fn a_setup_command_for_another_program_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace(
            "url =",
            "setup = [[\"fab\", \"init\"], [\"sh\", \"-c\", \"true\"]]\nurl =",
        );
        expect_error(&text, "train.toml: a setup command of fab runs sh, not fab");
    }

    #[test]
    fn a_darwin_archive_under_the_linux_key_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace(
            "fab-cli-x86_64-unknown-linux-gnu.tar.xz",
            "fab-cli-aarch64-apple-darwin.tar.xz",
        );
        expect_error(
            &text,
            "train.toml: the archive of fab for x86_64-unknown-linux-gnu is not named for x86_64",
        );
    }

    #[test]
    fn a_linux_archive_under_the_darwin_key_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace(
            "fab-cli-aarch64-apple-darwin.tar.xz",
            "fab-cli-aarch64-unknown-linux-gnu.tar.xz",
        );
        expect_error(
            &text,
            "train.toml: the archive of fab for aarch64-apple-darwin is not named for apple-darwin",
        );
    }

    #[test]
    fn a_musl_archive_serves_the_linux_key() {
        let text = one_tool("essential", Some(HASH)).replace(
            "fab-cli-x86_64-unknown-linux-gnu.tar.xz",
            "fab-cli-x86_64-unknown-linux-musl.tar.xz",
        );
        Train::parse(&text).unwrap();
    }

    #[test]
    fn a_url_without_https_is_rejected() {
        let text = one_tool("essential", Some(HASH)).replace("https://", "http://");
        expect_error(&text, "train.toml: the url of fab does not start with https://");
    }

    #[test]
    fn a_version_or_archive_that_leaves_its_url_segment_is_rejected() {
        let tool = one_tool("essential", Some(HASH));
        let darwin = "fab-cli-aarch64-apple-darwin.tar.xz";
        for (field, found, from, to) in [
            ("version", "/", "0.8.0", "0.8.0/x".to_owned()),
            ("version", "?", "0.8.0", "0.8.0?x".to_owned()),
            ("version", "#", "0.8.0", "0.8.0#x".to_owned()),
            ("version", "..", "0.8.0", "0.8..0".to_owned()),
            ("archive for aarch64-apple-darwin", "/", darwin, format!("x/{darwin}")),
            ("archive for aarch64-apple-darwin", "?", darwin, format!("{darwin}?x")),
            ("archive for aarch64-apple-darwin", "#", darwin, format!("{darwin}#x")),
            ("archive for aarch64-apple-darwin", "..", darwin, format!("..{darwin}")),
        ] {
            let text = tool.replace(&format!("\"{from}\""), &format!("\"{to}\""));
            expect_error(&text, &format!("train.toml: the {field} of fab contains {found:?}"));
        }
    }
}
