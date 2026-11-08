//! The repository pin: the `sltk` version a repository uses and the optional
//! tools it chose, kept in `.github/sltk.toml`. The essentials are implied.
//!
//! The session hook reads the version with
//! `sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' .github/sltk.toml`, so the
//! file is written by hand rather than by a TOML serializer, and reading it
//! requires that exact line.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::train::{Tier, Train};

/// Where the pin lives, relative to the repository root.
pub const PATH: &str = ".github/sltk.toml";

/// The comment at the top of a written pin.
const HEADER: &str = "# The sltk release this repository pins, and the optional tools it uses.\n\
# The session hook reads the version line; sltk writes it, so keep it as is.\n";

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
    /// the line the session hook reads, and each optional tool must be an
    /// optional tool in `train`.
    pub fn parse(text: &str, train: &Train) -> Result<Pin, PinError> {
        let line_version = hook_version(text).ok_or(PinError::MissingVersionLine)?;
        let raw: RawPin = toml::from_str(text)?;
        if raw.version != line_version {
            return Err(PinError::MissingVersionLine);
        }
        let pin = Pin { version: raw.version, optional: raw.optional };
        pin.check(train)?;
        Ok(pin)
    }

    /// Returns the text of `.github/sltk.toml` for this pin. The version is
    /// the line `version = "x.y.z"`, with no other spacing or comment.
    pub fn render(&self, train: &Train) -> Result<String, PinError> {
        self.check(train)?;
        let optional: Vec<String> = self
            .optional
            .iter()
            .map(|tool| toml::Value::String(tool.clone()).to_string())
            .collect();
        Ok(format!(
            "{HEADER}version = \"{}\"\noptional = [{}]\n",
            self.version,
            optional.join(", ")
        ))
    }

    /// Checks that the session hook can read the version and that each
    /// optional tool is an optional tool in `train`, listed once.
    fn check(&self, train: &Train) -> Result<(), PinError> {
        if !is_hook_version(&self.version) {
            return Err(PinError::BadVersion { version: self.version.clone() });
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

/// `.github/sltk.toml` as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPin {
    version: String,
    #[serde(default)]
    optional: Vec<String>,
}

/// Returns the version the session hook's `sed` one-liner would print first:
/// from a line that is exactly `version = "` then a digit, then digits and
/// dots, then `"`. Lines split on `\n` alone, as `sed` splits them.
fn hook_version(text: &str) -> Option<&str> {
    text.split('\n').find_map(|line| {
        let version = line.strip_prefix("version = \"")?.strip_suffix('"')?;
        is_hook_version(version).then_some(version)
    })
}

/// Whether `version` matches the one-liner's `[0-9][0-9.]*`.
fn is_hook_version(version: &str) -> bool {
    version.starts_with(|c: char| c.is_ascii_digit()) && version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

#[cfg(test)]
mod tests {
    use super::{PATH, Pin, PinError};
    use crate::train::{Tier, Train};

    const HASH: &str = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0";

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
        Pin { version: "0.1.0".to_owned(), optional: optional.iter().map(|s| (*s).to_owned()).collect() }
    }

    /// The session hook's one-liner,
    /// `sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p'`, written out by hand
    /// apart from the module's reader so a change to either shows here.
    fn sed_one_liner(text: &str) -> Vec<&str> {
        text.split('\n')
            .filter_map(|line| {
                let rest = line.strip_prefix("version = \"")?;
                let version = rest.strip_suffix('"')?;
                let mut chars = version.chars();
                let first_is_digit = chars.next()?.is_ascii_digit();
                (first_is_digit && chars.all(|c| c.is_ascii_digit() || c == '.')).then_some(version)
            })
            .collect()
    }

    #[test]
    fn the_written_version_is_the_exact_line_the_hook_reads() {
        let text = pin(&["mire"]).render(&fixture_train()).unwrap();
        assert!(text.split('\n').any(|line| line == r#"version = "0.1.0""#), "{text}");
        assert_eq!(sed_one_liner(&text), ["0.1.0"]);
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

        let text = "version = \"0.1.0\"\noptional = [\"nimbus\"]\n";
        let err = Pin::parse(text, &train).unwrap_err();
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
            let written = Pin { version: version.to_owned(), optional: vec![] };
            let err = written.render(&train).unwrap_err();
            assert!(matches!(err, PinError::BadVersion { .. }), "{version:?}: {err:?}");
        }
    }

    #[test]
    fn a_file_without_the_exact_version_line_is_rejected() {
        let train = fixture_train();
        for text in [
            "version=\"0.1.0\"\n",
            "version = \"0.1.0\" # pinned\n",
            "version  = \"0.1.0\"\n",
            " version = \"0.1.0\"\n",
            "version = '0.1.0'\n",
            "version = \"0.1.0\"\r\n",
            "optional = []\n",
        ] {
            let err = Pin::parse(text, &train).unwrap_err();
            assert!(matches!(err, PinError::MissingVersionLine), "{text:?}: {err:?}");
            assert_eq!(
                err.to_string(),
                ".github/sltk.toml: no line reads exactly version = \"x.y.z\", which the session hook needs"
            );
        }
    }

    #[test]
    fn a_version_line_the_toml_does_not_hold_is_rejected() {
        // The hook would read 0.1.0 from inside the string; the TOML version is 0.2.0.
        let text = "version = '0.2.0'\noptional = [\"\"\"\nversion = \"0.1.0\"\n\"\"\"]\n";
        let err = Pin::parse(text, &fixture_train()).unwrap_err();
        assert!(matches!(err, PinError::MissingVersionLine), "{err:?}");
    }

    #[test]
    fn an_unknown_field_is_rejected() {
        let err = Pin::parse("version = \"0.1.0\"\nextra = 1\n", &fixture_train()).unwrap_err();
        assert!(matches!(err, PinError::Parse(_)), "{err:?}");
    }
}
