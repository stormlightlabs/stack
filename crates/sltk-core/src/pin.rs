//! The repository pin: the `sltk` version a repository uses, the sha256 of
//! each platform's `sltk` archive, and the optional tools it chose, kept in
//! `.github/sltk.toml`. The essentials are implied.
//!
//! The session hook reads the version with
//! `sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' .github/sltk.toml`, and its
//! platform's hash from the line `sha256-<target> = "<hex>"` the same way. So
//! the file is written by hand rather than by a TOML serializer, and reading
//! it requires those exact lines.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::train::{Platform, Tier, Train};

/// Where the pin lives, relative to the repository root.
pub const PATH: &str = ".github/sltk.toml";

/// The platforms whose `sltk` archive hash every pin records, in the order
/// the file lists them.
pub const TARGETS: [Platform; 2] = [Platform::AppleSilicon, Platform::LinuxX86_64];

/// The comment at the top of a written pin.
const HEADER: &str = "# The sltk release this repository pins, and the optional tools it uses.\n\
# The session hook reads the version and sha256 lines; sltk writes them, so keep them as is.\n";

/// Why a pin could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PinError {
    /// The file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    /// The text is not TOML, or does not have the pin's shape.
    #[error("{PATH}: {0}")]
    Parse(#[from] toml::de::Error),
    /// No line reads `version = "x.y.z"` exactly, so the session hook would
    /// find no version.
    #[error("{PATH}: no line reads exactly version = \"x.y.z\", which the session hook needs")]
    MissingVersionLine,
    /// No line reads `sha256-<target> = "<hex>"` exactly for this platform, so
    /// the session hook could not check the archive.
    #[error("{PATH}: no line reads exactly sha256-{platform} = \"<hex>\", which the session hook needs")]
    MissingHashLine { platform: Platform },
    /// The `version = "x.y.z"` line the session hook reads and the TOML
    /// version differ, as when the line sits inside a string.
    #[error("{PATH}: the version line and the TOML version differ")]
    VersionMismatch,
    /// The `sha256-<target> = "<hex>"` line the session hook reads and the
    /// TOML hash for this platform differ, as when the line sits inside a
    /// string.
    #[error("{PATH}: the sha256-{platform} line and the TOML hash differ")]
    HashMismatch { platform: Platform },
    /// The pin has a key it does not define.
    #[error("{PATH}: unknown key {key}")]
    UnknownKey { key: String },
    /// The pin has no sha256 for this platform.
    #[error("{PATH}: the pin has no sha256 for {platform}")]
    MissingHash { platform: Platform },
    /// The sha256 is not 64 lowercase hexadecimal digits.
    #[error("{PATH}: the sha256 for {platform} is not 64 lowercase hexadecimal digits")]
    BadHash { platform: Platform },
    /// The version has a character other than a digit or `.`, or does not
    /// start with a digit, so the session hook could not read it.
    #[error("{PATH}: the version {version:?} is not digits and dots")]
    BadVersion { version: String },
    /// The train has no tool by this name.
    #[error("{PATH}: the train has no tool named {tool}")]
    UnknownTool { tool: String },
    /// The tool is in the train but always installs, so the pin cannot list it.
    #[error("{PATH}: {tool} is {tier}; optional lists only optional tools")]
    NotOptional { tool: String, tier: Tier },
    /// A tool is listed more than once.
    #[error("{PATH}: {tool} is listed more than once")]
    RepeatedTool { tool: String },
}

/// What `.github/sltk.toml` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// The `sltk` release, such as `0.1.0`.
    pub version: String,
    /// The sha256 of the `sltk` archive for each platform in [`TARGETS`], in
    /// lowercase hexadecimal.
    pub sha256: BTreeMap<Platform, String>,
    /// The optional tools to install, in the order the file lists them.
    pub optional: Vec<String>,
}

impl Pin {
    /// Reads the pin from `.github/sltk.toml` under `root`, checking each
    /// optional tool against `train`.
    pub fn read(root: &Path, train: &Train) -> Result<Pin, PinError> {
        let path = root.join(PATH);
        let text = std::fs::read_to_string(&path).map_err(|source| PinError::Io { path, source })?;
        Pin::parse(&text, train)
    }

    /// Writes the pin to `.github/sltk.toml` under `root`, creating `.github`
    /// when it is missing.
    pub fn write(&self, root: &Path, train: &Train) -> Result<(), PinError> {
        let text = self.render(train)?;
        let path = root.join(PATH);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| PinError::Io { path: dir.to_owned(), source })?;
        }
        std::fs::write(&path, text).map_err(|source| PinError::Io { path, source })
    }

    /// Reads a pin from the text of a `.github/sltk.toml`. The text must hold
    /// the lines the session hook reads, and each optional tool must be an
    /// optional tool in `train`.
    pub fn parse(text: &str, train: &Train) -> Result<Pin, PinError> {
        let line_version = hook_line(text, "version", is_hook_version).ok_or(PinError::MissingVersionLine)?;
        let mut line_hashes = BTreeMap::new();
        for platform in TARGETS {
            let hash = hook_line(text, &hash_key(platform), is_hash).ok_or(PinError::MissingHashLine { platform })?;
            line_hashes.insert(platform, hash);
        }
        let mut raw: RawPin = toml::from_str(text)?;
        if raw.version != line_version {
            return Err(PinError::VersionMismatch);
        }
        let mut sha256 = BTreeMap::new();
        for platform in TARGETS {
            let hash = raw.sha256.remove(&hash_key(platform));
            if hash.as_deref() != line_hashes.get(&platform).copied() {
                return Err(PinError::HashMismatch { platform });
            }
            sha256.extend(hash.map(|hash| (platform, hash)));
        }
        if let Some(key) = raw.sha256.into_keys().next() {
            return Err(PinError::UnknownKey { key });
        }
        let pin = Pin { version: raw.version, sha256, optional: raw.optional };
        pin.check(train)?;
        Ok(pin)
    }

    /// Returns the text of `.github/sltk.toml` for this pin. The version is
    /// the line `version = "x.y.z"` and each hash the line
    /// `sha256-<target> = "<hex>"`, with no other spacing or comment, and the
    /// file has no tables.
    pub fn render(&self, train: &Train) -> Result<String, PinError> {
        self.check(train)?;
        let mut text = format!("{HEADER}version = \"{}\"\n", self.version);
        for platform in TARGETS {
            text.push_str(&format!("{} = \"{}\"\n", hash_key(platform), self.sha256[&platform]));
        }
        let optional: Vec<String> = self
            .optional
            .iter()
            .map(|tool| toml::Value::String(tool.clone()).to_string())
            .collect();
        text.push_str(&format!("optional = [{}]\n", optional.join(", ")));
        Ok(text)
    }

    /// Checks that the session hook can read the version and each hash, and
    /// that each optional tool is an optional tool in `train`, listed once.
    fn check(&self, train: &Train) -> Result<(), PinError> {
        if !is_hook_version(&self.version) {
            return Err(PinError::BadVersion { version: self.version.clone() });
        }
        for platform in TARGETS {
            match self.sha256.get(&platform) {
                None => return Err(PinError::MissingHash { platform }),
                Some(hash) if !is_hash(hash) => return Err(PinError::BadHash { platform }),
                Some(_) => {}
            }
        }
        let mut seen = BTreeSet::new();
        for name in &self.optional {
            let Some(tool) = train.tools.iter().find(|t| t.name == *name) else {
                return Err(PinError::UnknownTool { tool: name.clone() });
            };
            if tool.tier != Tier::Optional {
                return Err(PinError::NotOptional { tool: name.clone(), tier: tool.tier });
            }
            if !seen.insert(name.as_str()) {
                return Err(PinError::RepeatedTool { tool: name.clone() });
            }
        }
        Ok(())
    }
}

/// `.github/sltk.toml` as written. serde cannot deny unknown fields next to
/// a flattened map, so every key other than `version` and `optional` lands in
/// `sha256`, and [`Pin::parse`] rejects any that is not a [`TARGETS`] hash key.
#[derive(Deserialize)]
struct RawPin {
    version: String,
    #[serde(default)]
    optional: Vec<String>,
    /// The `sha256-<target>` values, by key.
    #[serde(flatten)]
    sha256: BTreeMap<String, String>,
}

/// The key of the line that holds the hash for `platform`.
fn hash_key(platform: Platform) -> String {
    format!("sha256-{platform}")
}

/// Returns the value the session hook's `sed` one-liner would print first for
/// `key`: from a line that is exactly `<key> = "<value>"`, where `valid`
/// accepts the value. Lines split on `\n` alone, as `sed` splits them.
fn hook_line<'a>(text: &'a str, key: &str, valid: fn(&str) -> bool) -> Option<&'a str> {
    text.split('\n').find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix(" = \"")?.strip_suffix('"')?;
        valid(value).then_some(value)
    })
}

/// Whether `hash` is 64 lowercase hexadecimal digits, `[0-9a-f]\{64\}`.
fn is_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `version` matches the one-liner's `[0-9][0-9.]*`.
fn is_hook_version(version: &str) -> bool {
    version.starts_with(|c: char| c.is_ascii_digit()) && version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

#[cfg(test)]
mod tests {
    use super::{PATH, Pin, PinError};
    use crate::train::{Platform, Tier, Train};

    const HASH: &str = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0";
    const DARWIN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    const LINUX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A train with an essential `fab` and an optional `mire`, since the
    /// embedded train has no optional tool yet.
    fn fixture_train() -> Train {
        let tool = |name: &str, tier: &str| {
            format!(
                r#"
[[tool]]
name = "{name}"
tier = "{tier}"
binary = "{name}"
version = "0.1.0"
url = "https://example.test/v{{version}}/{{archive}}"

[tool.platforms.x86_64-unknown-linux-gnu]
archive = "{name}-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "{HASH}"
"#
            )
        };
        Train::parse(&format!("{}{}", tool("fab", "essential"), tool("mire", "optional"))).unwrap()
    }

    fn pin(optional: &[&str]) -> Pin {
        Pin {
            version: "0.1.0".to_owned(),
            sha256: [
                (Platform::AppleSilicon, DARWIN.to_owned()),
                (Platform::LinuxX86_64, LINUX.to_owned()),
            ]
            .into(),
            optional: optional.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// The two hash lines, for pins written out by hand.
    fn hash_lines() -> String {
        format!("sha256-aarch64-apple-darwin = \"{DARWIN}\"\nsha256-x86_64-unknown-linux-gnu = \"{LINUX}\"\n")
    }

    /// Runs the session hook's `sed -n` script on `text`, or returns `None`
    /// when `sed` is not on the PATH.
    fn sed(script: &str, text: &str) -> Option<String> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = match Command::new("sed")
            .args(["-n", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => panic!("sed: {err}"),
        };
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "sed {script}: {out:?}");
        Some(String::from_utf8(out.stdout).unwrap())
    }

    #[test]
    fn the_written_version_and_hashes_are_the_exact_lines_the_hook_reads() {
        let text = pin(&["mire"]).render(&fixture_train()).unwrap();
        let lines: Vec<&str> = text.split('\n').collect();
        for expected in [
            r#"version = "0.1.0""#.to_owned(),
            format!(r#"sha256-aarch64-apple-darwin = "{DARWIN}""#),
            format!(r#"sha256-x86_64-unknown-linux-gnu = "{LINUX}""#),
        ] {
            assert!(lines.contains(&expected.as_str()), "{expected} not in\n{text}");
        }
        assert!(!lines.iter().any(|line| line.starts_with('[')), "a table in\n{text}");
    }

    #[test]
    fn the_session_hook_sed_reads_the_written_version_and_hashes() {
        let text = pin(&["mire"]).render(&fixture_train()).unwrap();
        let hash = |target: &str| format!(r#"s/^sha256-{target} = "\([0-9a-f]\{{64\}}\)"$/\1/p"#);
        for (script, expected) in [
            (r#"s/^version = "\([0-9][0-9.]*\)"$/\1/p"#.to_owned(), "0.1.0"),
            (hash("aarch64-apple-darwin"), DARWIN),
            (hash("x86_64-unknown-linux-gnu"), LINUX),
        ] {
            let Some(out) = sed(&script, &text) else {
                println!("skipped: sed is not on the PATH");
                return;
            };
            assert_eq!(out, format!("{expected}\n"), "{script}");
        }
    }

    #[test]
    fn writing_then_reading_returns_the_same_pin() {
        let train = fixture_train();
        for written in [pin(&[]), pin(&["mire"])] {
            let read = Pin::parse(&written.render(&train).unwrap(), &train).unwrap();
            assert_eq!(read, written);
        }
    }

    #[test]
    fn write_then_read_round_trips_through_the_file() {
        let train = fixture_train();
        let root = std::env::temp_dir().join(format!("sltk-pin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        pin(&["mire"]).write(&root, &train).unwrap();
        assert!(root.join(PATH).is_file());
        assert_eq!(Pin::read(&root, &train).unwrap(), pin(&["mire"]));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_embedded_train_takes_a_pin_with_no_optional_tools() {
        let train = Train::embedded().unwrap();
        let text = pin(&[]).render(&train).unwrap();
        assert_eq!(Pin::parse(&text, &train).unwrap(), pin(&[]));
    }

    #[test]
    fn an_unknown_tool_is_named() {
        let train = fixture_train();
        let err = pin(&["nimbus"]).render(&train).unwrap_err();
        assert!(
            matches!(&err, PinError::UnknownTool { tool } if tool == "nimbus"),
            "{err:?}"
        );
        assert_eq!(err.to_string(), ".github/sltk.toml: the train has no tool named nimbus");

        let text = format!("version = \"0.1.0\"\n{}optional = [\"nimbus\"]\n", hash_lines());
        let err = Pin::parse(&text, &train).unwrap_err();
        assert_eq!(err.to_string(), ".github/sltk.toml: the train has no tool named nimbus");
    }

    #[test]
    fn an_essential_tool_is_not_listed() {
        let err = pin(&["fab"]).render(&fixture_train()).unwrap_err();
        assert!(
            matches!(&err, PinError::NotOptional { tool, tier: Tier::Essential } if tool == "fab"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            ".github/sltk.toml: fab is essential; optional lists only optional tools"
        );
    }

    #[test]
    fn a_repeated_tool_is_rejected() {
        let err = pin(&["mire", "mire"]).render(&fixture_train()).unwrap_err();
        assert_eq!(err.to_string(), ".github/sltk.toml: mire is listed more than once");
    }

    #[test]
    fn a_version_the_hook_cannot_read_is_not_written() {
        let train = fixture_train();
        for version in ["", "v0.1.0", "0.1.0-rc.1", "0.1.0\"\nx = \"1"] {
            let written = Pin { version: version.to_owned(), ..pin(&[]) };
            let err = written.render(&train).unwrap_err();
            assert!(matches!(err, PinError::BadVersion { .. }), "{version:?}: {err:?}");
        }
    }

    #[test]
    fn a_missing_or_malformed_hash_is_not_written() {
        let train = fixture_train();
        let mut written = pin(&[]);
        written.sha256.remove(&Platform::LinuxX86_64);
        let err = written.render(&train).unwrap_err();
        assert_eq!(
            err.to_string(),
            ".github/sltk.toml: the pin has no sha256 for x86_64-unknown-linux-gnu"
        );

        for hash in [&DARWIN[..63], &DARWIN.to_uppercase(), &format!("{}g", &DARWIN[..63])] {
            let mut written = pin(&[]);
            written.sha256.insert(Platform::AppleSilicon, hash.to_owned());
            let err = written.render(&train).unwrap_err();
            assert_eq!(
                err.to_string(),
                ".github/sltk.toml: the sha256 for aarch64-apple-darwin is not 64 lowercase hexadecimal digits"
            );
        }
    }

    #[test]
    fn a_file_without_the_exact_version_line_is_rejected() {
        let train = fixture_train();
        for version in [
            "version=\"0.1.0\"\n",
            "version = \"0.1.0\" # pinned\n",
            "version  = \"0.1.0\"\n",
            " version = \"0.1.0\"\n",
            "version = '0.1.0'\n",
            "version = \"0.1.0\"\r\n",
            "",
        ] {
            let text = format!("{version}{}", hash_lines());
            let err = Pin::parse(&text, &train).unwrap_err();
            assert!(matches!(err, PinError::MissingVersionLine), "{text:?}: {err:?}");
            assert_eq!(
                err.to_string(),
                ".github/sltk.toml: no line reads exactly version = \"x.y.z\", which the session hook needs"
            );
        }
    }

    #[test]
    fn a_file_without_an_exact_hash_line_names_it() {
        let train = fixture_train();
        let darwin = format!("sha256-aarch64-apple-darwin = \"{DARWIN}\"\n");
        let linux_key = "sha256-x86_64-unknown-linux-gnu";
        for linux in [
            String::new(),
            format!("{linux_key}=\"{LINUX}\"\n"),
            format!("{linux_key} = \"{LINUX}\" # pinned\n"),
            format!("{linux_key} = '{LINUX}'\n"),
            format!("{linux_key} = \"{}\"\n", LINUX.to_uppercase()),
            format!("{linux_key} = \"{}\"\n", &LINUX[..63]),
        ] {
            let text = format!("version = \"0.1.0\"\n{darwin}{linux}");
            let err = Pin::parse(&text, &train).unwrap_err();
            assert!(
                matches!(err, PinError::MissingHashLine { platform: Platform::LinuxX86_64 }),
                "{text:?}: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                ".github/sltk.toml: no line reads exactly sha256-x86_64-unknown-linux-gnu = \"<hex>\", \
                 which the session hook needs"
            );
        }
        let text = format!("version = \"0.1.0\"\n{linux_key} = \"{LINUX}\"\n");
        let err = Pin::parse(&text, &train).unwrap_err();
        assert!(
            matches!(err, PinError::MissingHashLine { platform: Platform::AppleSilicon }),
            "{err:?}"
        );
    }

    #[test]
    fn a_version_line_the_toml_does_not_hold_is_rejected() {
        // The hook would read 0.1.0 from inside the string; the TOML version is 0.2.0.
        let text = format!(
            "version = '0.2.0'\n{}optional = [\"\"\"\nversion = \"0.1.0\"\n\"\"\"]\n",
            hash_lines()
        );
        let err = Pin::parse(&text, &fixture_train()).unwrap_err();
        assert!(matches!(err, PinError::VersionMismatch), "{err:?}");
        assert_eq!(
            err.to_string(),
            ".github/sltk.toml: the version line and the TOML version differ"
        );
    }

    #[test]
    fn a_hash_line_the_toml_does_not_hold_is_rejected() {
        // The hook would read LINUX from inside the string; the TOML hash is HASH.
        let text = format!(
            "version = \"0.1.0\"\nsha256-aarch64-apple-darwin = \"{DARWIN}\"\n\
             sha256-x86_64-unknown-linux-gnu = '{HASH}'\n\
             optional = [\"\"\"\nsha256-x86_64-unknown-linux-gnu = \"{LINUX}\"\n\"\"\"]\n"
        );
        let err = Pin::parse(&text, &fixture_train()).unwrap_err();
        assert!(
            matches!(err, PinError::HashMismatch { platform: Platform::LinuxX86_64 }),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            ".github/sltk.toml: the sha256-x86_64-unknown-linux-gnu line and the TOML hash differ"
        );
    }

    #[test]
    fn a_hash_line_inside_a_string_with_no_toml_hash_is_rejected() {
        let text = format!(
            "version = \"0.1.0\"\nsha256-aarch64-apple-darwin = \"{DARWIN}\"\n\
             optional = [\"\"\"\nsha256-x86_64-unknown-linux-gnu = \"{LINUX}\"\n\"\"\"]\n"
        );
        let err = Pin::parse(&text, &fixture_train()).unwrap_err();
        assert!(
            matches!(err, PinError::HashMismatch { platform: Platform::LinuxX86_64 }),
            "{err:?}"
        );
    }

    #[test]
    fn an_unknown_field_is_rejected() {
        let text = format!("version = \"0.1.0\"\n{}extra = 1\n", hash_lines());
        let err = Pin::parse(&text, &fixture_train()).unwrap_err();
        assert!(matches!(err, PinError::Parse(_)), "{err:?}");

        for key in ["extra", "sha256-x86_64-pc-windows-msvc"] {
            let text = format!("version = \"0.1.0\"\n{}{key} = \"x\"\n", hash_lines());
            let err = Pin::parse(&text, &fixture_train()).unwrap_err();
            assert!(matches!(&err, PinError::UnknownKey { key: k } if k == key), "{err:?}");
        }
    }
}
