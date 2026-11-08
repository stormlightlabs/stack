//! Repository setup: `sltk init` runs each tool's own setup commands from the
//! train, passing fab's settings to `fab init`, and then writes the pin.
//!
//! The pin records the sha256 of each platform's `sltk` archive, copied from
//! the `.sha256` files on this release, so `init` reads those before it runs
//! anything. It writes the pin only when every command succeeded.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::install::Fetch;
use crate::pin::{self, Pin, PinError};
use crate::train::{Platform, Tier, Train};

/// Where each `sltk` archive's `.sha256` file is published, with `{version}`
/// and `{target}` to replace.
pub const RELEASE_HASH_URL: &str =
    "https://github.com/stormlightlabs/stack/releases/download/v{version}/sltk-cli-{target}.tar.xz.sha256";

/// Why a download of a `.sha256` file can fail even when the network works.
const UNPUBLISHED: &str =
    "sltk init pins a published sltk release, and stormlightlabs/stack publishes none before 0.1.0";

/// The longest `.sha256` file [`ReleaseHashes`] reads, in bytes. dist writes
/// the digest and the archive name on one line.
const MAX_HASH_FILE: u64 = 4096;

/// A setting `sltk init` passes to `fab init`, by its flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// `--owner`: the GitHub login of the person who approves pull requests.
    Owner,
    /// `--trailer`: the squash commit's Co-authored-by value, `Name <email>`.
    Trailer,
    /// `--gates`: the command agents run before pushing.
    Gates,
}

impl Flag {
    /// Every flag, in the order `fab init` takes them.
    pub const ALL: [Flag; 3] = [Flag::Owner, Flag::Trailer, Flag::Gates];

    /// The flag as typed, such as `--owner`.
    pub fn name(self) -> &'static str {
        match self {
            Flag::Owner => "--owner",
            Flag::Trailer => "--trailer",
            Flag::Gates => "--gates",
        }
    }

    /// Checks a value for this flag. Every value is one line with no control
    /// characters and no space at either end, since fab writes each as one
    /// line of `.github/fabrial.toml`. An owner has no spaces, and a trailer
    /// reads `Name <email>`.
    pub fn check(self, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err("is empty".to_owned());
        }
        if value.chars().any(char::is_control) {
            return Err("holds a control character or line break".to_owned());
        }
        if value.trim() != value {
            return Err("starts or ends with a space".to_owned());
        }
        match self {
            Flag::Owner if value.contains(char::is_whitespace) => Err("holds a space".to_owned()),
            Flag::Trailer if !is_trailer(value) => Err("is not Name <email>".to_owned()),
            _ => Ok(()),
        }
    }
}

/// Why `sltk init` stopped.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// The repository already has a pin. Adopting a set-up repository is not
    /// built yet, and `init` never overwrites the pin.
    #[error(
        "sltk: {} already exists; sltk init sets up a new repository and cannot adopt one yet",
        path.display()
    )]
    PinExists { path: PathBuf },
    /// A fab setting is not valid.
    #[error("sltk: {flag} {reason}")]
    BadFlag { flag: &'static str, reason: String },
    /// The sha256 of this release's archive for a platform could not be read.
    #[error("sltk: could not read the sha256 of the sltk {version} archive for {platform}: {message}")]
    Hash {
        version: String,
        platform: Platform,
        message: String,
    },
    /// The pin could not be checked or written.
    #[error("sltk: {0}")]
    Pin(#[from] PinError),
    /// A setup command could not be started.
    #[error("sltk: could not run {command} in {}: {source}", dir.display())]
    Spawn {
        command: String,
        dir: PathBuf,
        source: std::io::Error,
    },
    /// A setup command exited with a failure.
    #[error("sltk: {command} failed with {status}; sltk did not write the pin")]
    Failed { command: String, status: ExitStatus },
}

/// The three settings `fab init` requires, each checked by [`Flag::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabSettings {
    /// The GitHub login of the person who approves pull requests.
    pub owner: String,
    /// The squash commit's Co-authored-by value, as `Name <email>`.
    pub trailer: String,
    /// The command agents run before pushing.
    pub gates: String,
}

impl FabSettings {
    /// Checks each value and returns the settings.
    pub fn new(owner: String, trailer: String, gates: String) -> Result<FabSettings, SetupError> {
        let settings = FabSettings { owner, trailer, gates };
        for flag in Flag::ALL {
            flag.check(settings.get(flag))
                .map_err(|reason| SetupError::BadFlag { flag: flag.name(), reason })?;
        }
        Ok(settings)
    }

    /// The value for `flag`.
    pub fn get(&self, flag: Flag) -> &str {
        match flag {
            Flag::Owner => &self.owner,
            Flag::Trailer => &self.trailer,
            Flag::Gates => &self.gates,
        }
    }
}

/// What `sltk init` works on.
pub struct Init<'a> {
    /// The repository root, where every setup command runs.
    pub root: &'a Path,
    /// The settings for `fab init`.
    pub settings: &'a FabSettings,
    /// The optional tools the pin lists.
    pub with: &'a [String],
    /// Where the `sltk` archive hashes come from.
    pub hashes: &'a dyn HashSource,
    /// The `PATH` to find and run the setup commands with, or `None` for the
    /// one `sltk` runs with.
    pub path: Option<&'a OsStr>,
}

/// Gives the sha256 of this `sltk` release's archive for a platform.
/// [`ReleaseHashes`] is the real one; tests supply fixed hashes.
pub trait HashSource {
    /// The digest for `platform` in lowercase hexadecimal, or what went wrong.
    fn sha256(&self, platform: Platform) -> Result<String, String>;
}

/// Reads each hash from the `.sha256` file published beside the archive, at
/// [`RELEASE_HASH_URL`].
pub struct ReleaseHashes<'a> {
    /// The release, such as `0.1.0`.
    pub version: &'a str,
    /// How to download the file.
    pub fetch: &'a dyn Fetch,
}

impl HashSource for ReleaseHashes<'_> {
    fn sha256(&self, platform: Platform) -> Result<String, String> {
        let url = RELEASE_HASH_URL
            .replace("{version}", self.version)
            .replace("{target}", platform.as_str());
        let reader = self
            .fetch
            .open(&url)
            .map_err(|err| format!("{url}: {err}; {UNPUBLISHED}"))?;
        let mut text = String::new();
        reader
            .take(MAX_HASH_FILE)
            .read_to_string(&mut text)
            .map_err(|err| format!("{url}: {err}"))?;
        parse_digest(&text).ok_or_else(|| format!("{url} does not start with a 64-digit hexadecimal sha256"))
    }
}

/// The setup commands `init` runs, in train order: those of every essential
/// tool and of each tool in `with`. `fab init` gets fab's three flags.
pub fn commands(train: &Train, settings: &FabSettings, with: &[String]) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for tool in &train.tools {
        if tool.tier != Tier::Essential && !with.contains(&tool.name) {
            continue;
        }
        for command in &tool.setup {
            let mut command = command.clone();
            if tool.name == "fab" && command[1..] == ["init"] {
                for flag in Flag::ALL {
                    command.extend([flag.name().to_owned(), settings.get(flag).to_owned()]);
                }
            }
            commands.push(command);
        }
    }
    commands
}

/// Runs `sltk init`: reads this release's archive hashes and checks the pin,
/// runs each of [`commands`] in the repository root, stopping at the first
/// failure, and then writes the pin. Writes the command lines and the pin's
/// path to `output`. Refuses a repository that already has a pin.
pub fn init(train: &Train, version: &str, init: Init, output: &mut dyn Write) -> Result<Pin, SetupError> {
    let path = init.root.join(pin::PATH);
    if path.exists() {
        return Err(SetupError::PinExists { path });
    }
    let mut sha256 = std::collections::BTreeMap::new();
    for platform in pin::TARGETS {
        let hash = init.hashes.sha256(platform).map_err(|message| SetupError::Hash {
            version: version.to_owned(),
            platform,
            message,
        })?;
        sha256.insert(platform, hash);
    }
    let pin = Pin { version: version.to_owned(), sha256, optional: init.with.to_vec() };
    // Checks the pin before any command changes the repository.
    pin.render(train)?;
    for command in commands(train, init.settings, init.with) {
        let line = command_line(&command);
        // The commands still run when stdout is closed.
        let _ = writeln!(output, "running {line}");
        let mut process = Command::new(&command[0]);
        process.args(&command[1..]).current_dir(init.root);
        if let Some(path) = init.path {
            process.env("PATH", path);
        }
        let status = process.status().map_err(|source| SetupError::Spawn {
            command: line.clone(),
            dir: init.root.to_owned(),
            source,
        })?;
        if !status.success() {
            return Err(SetupError::Failed { command: line, status });
        }
    }
    pin.write(init.root, train)?;
    let _ = writeln!(output, "wrote {}", pin::PATH);
    Ok(pin)
}

/// The digest at the start of a `.sha256` file, lowercased, when it is 64
/// hexadecimal digits.
fn parse_digest(text: &str) -> Option<String> {
    let digest = text.split_whitespace().next()?.to_ascii_lowercase();
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digest)
}

/// Whether `value` reads `Name <email>`: a name, a space, and an address with
/// text on both sides of one `@`, in angle brackets that end the value.
fn is_trailer(value: &str) -> bool {
    let Some((name, rest)) = value.split_once(" <") else {
        return false;
    };
    let Some(email) = rest.strip_suffix('>') else {
        return false;
    };
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !name.trim().is_empty()
        && !name.contains(['<', '>'])
        && !local.is_empty()
        && !domain.is_empty()
        && !local.contains(['<', '>', ' '])
        && !domain.contains(['<', '>', ' ', '@'])
}

/// A command as a person would type it, with each argument that holds
/// anything but letters, digits and `-_./=:@` in single quotes.
fn command_line(command: &[String]) -> String {
    let plain = |arg: &str| !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@".contains(c));
    command
        .iter()
        .map(
            |arg| {
                if plain(arg) { arg.clone() } else { format!("'{}'", arg.replace('\'', r"'\''")) }
            },
        )
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::{FabSettings, Flag, HashSource, ReleaseHashes, command_line, commands, parse_digest};
    use crate::install::Fetch;
    use crate::train::{Platform, Train};

    /// Serves `body` for one URL and fails for any other.
    struct OneFile {
        url: &'static str,
        body: String,
    }

    impl Fetch for OneFile {
        fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
            if url == self.url {
                Ok(Box::new(std::io::Cursor::new(self.body.clone().into_bytes())))
            } else {
                Err("404 Not Found".to_owned())
            }
        }
    }

    #[test]
    fn a_release_hash_comes_from_the_sha256_file_beside_the_archive() {
        let url = "https://github.com/stormlightlabs/stack/releases/download/v0.1.0/sltk-cli-x86_64-unknown-linux-gnu.tar.xz.sha256";
        let fetch = OneFile { url, body: format!("{DIGEST}  sltk-cli-x86_64-unknown-linux-gnu.tar.xz\n") };
        let hashes = ReleaseHashes { version: "0.1.0", fetch: &fetch };
        assert_eq!(hashes.sha256(Platform::LinuxX86_64).as_deref(), Ok(DIGEST));
        let err = hashes.sha256(Platform::AppleSilicon).unwrap_err();
        assert!(
            err.contains("v0.1.0/sltk-cli-aarch64-apple-darwin.tar.xz.sha256: 404 Not Found"),
            "{err}"
        );
        assert!(err.ends_with("publishes none before 0.1.0"), "{err}");
        let garbled = OneFile { url, body: "<html>".to_owned() };
        let err = ReleaseHashes { version: "0.1.0", fetch: &garbled }
            .sha256(Platform::LinuxX86_64)
            .unwrap_err();
        assert!(
            err.ends_with("does not start with a 64-digit hexadecimal sha256"),
            "{err}"
        );
    }

    const DIGEST: &str = "bd8c707af7c6088c1afdf57ff5b9e9f1c4f4830d0dabf1432c4b5ebaced33fe0";

    fn settings() -> FabSettings {
        FabSettings::new(
            "octo".into(),
            "Ada Lovelace <ada@example.com>".into(),
            "just check".into(),
        )
        .unwrap()
    }

    #[test]
    fn a_trailer_must_read_name_and_email() {
        for good in ["Ada <ada@example.com>", "Ada Lovelace <a.b+c@mail.example>"] {
            assert_eq!(Flag::Trailer.check(good), Ok(()), "{good}");
        }
        for bad in [
            "ada@example.com",
            "<ada@example.com>",
            "Ada <ada@example.com",
            "Ada <adaexample.com>",
            "Ada <@example.com>",
            "Ada <ada@>",
            "Ada <ada@x@y>",
            "Ada <ada@example.com> extra",
            "Ada <a b@example.com>",
        ] {
            assert_eq!(Flag::Trailer.check(bad), Err("is not Name <email>".to_owned()), "{bad}");
        }
    }

    #[test]
    fn every_flag_takes_one_trimmed_line() {
        for flag in Flag::ALL {
            assert_eq!(flag.check(""), Err("is empty".to_owned()));
            assert!(flag.check("a\nb").is_err());
            assert_eq!(flag.check(" a"), Err("starts or ends with a space".to_owned()));
        }
        assert_eq!(Flag::Owner.check("a b"), Err("holds a space".to_owned()));
        assert_eq!(Flag::Gates.check("cargo test --locked"), Ok(()));
    }

    #[test]
    fn a_bad_setting_names_its_flag() {
        let err = FabSettings::new("octo".into(), "nobody".into(), "just check".into()).unwrap_err();
        assert_eq!(err.to_string(), "sltk: --trailer is not Name <email>");
    }

    #[test]
    fn a_digest_is_the_first_word_of_the_file() {
        let upper = DIGEST.to_ascii_uppercase();
        assert_eq!(
            parse_digest(&format!("{DIGEST}  sltk-cli-x.tar.xz\n")).as_deref(),
            Some(DIGEST)
        );
        assert_eq!(parse_digest(&format!("{upper}\n")).as_deref(), Some(DIGEST));
        assert_eq!(parse_digest(""), None);
        assert_eq!(parse_digest("Not Found"), None);
        assert_eq!(parse_digest(&DIGEST[1..]), None);
    }

    #[test]
    fn a_command_line_quotes_what_a_shell_would_split() {
        let command = ["fab", "--trailer", "Ada <a@b.c>", "it's", "--owner=octo"].map(String::from);
        assert_eq!(
            command_line(&command),
            r"fab --trailer 'Ada <a@b.c>' 'it'\''s' --owner=octo"
        );
    }

    #[test]
    fn fab_init_gets_the_three_flags_and_arcl_its_own_commands() {
        let train = Train::embedded().unwrap();
        let got = commands(&train, &settings(), &[]);
        let want: Vec<Vec<String>> = [
            &[
                "fab",
                "init",
                "--owner",
                "octo",
                "--trailer",
                "Ada Lovelace <ada@example.com>",
                "--gates",
                "just check",
            ][..],
            &["arcl", "init", "--workspace"],
            &["arcl", "skill", "install"],
        ]
        .iter()
        .map(|c| c.iter().map(|s| s.to_string()).collect())
        .collect();
        assert_eq!(got, want);
    }
}
