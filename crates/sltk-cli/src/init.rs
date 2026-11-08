//! `sltk init`: gathers fab's settings from flags or questions, finds the
//! repository root, and hands both to [`sltk_core::setup::init`].

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use sltk_core::install::HttpFetch;
use sltk_core::setup::{self, FabSettings, Flag, Init, ReleaseHashes};
use sltk_core::train::Train;

use crate::{FAILED, prompt};

/// The error when no `--dir` is given and the current directory is not in a
/// git repository.
const NOT_A_REPOSITORY: &str =
    "sltk: the current directory is not in a git repository; run sltk init in one, or pass --dir";

/// What `sltk init` was given on the command line.
pub struct Flags {
    /// `--owner`, if given.
    pub owner: Option<String>,
    /// `--trailer`, if given.
    pub trailer: Option<String>,
    /// `--gates`, if given.
    pub gates: Option<String>,
    /// The `--with` names.
    pub with: Vec<String>,
    /// `--dir`, if given.
    pub dir: Option<PathBuf>,
}

impl Flags {
    /// The value given for `flag`.
    fn get(&self, flag: Flag) -> Option<&str> {
        match flag {
            Flag::Owner => self.owner.as_deref(),
            Flag::Trailer => self.trailer.as_deref(),
            Flag::Gates => self.gates.as_deref(),
        }
    }
}

/// Runs `sltk init` in the repository root, asking on a `terminal` for each
/// fab setting the flags left out. Returns the exit code, or the exit code
/// and the error to print.
pub fn run(
    train: &Train, flags: &Flags, terminal: bool, input: &mut impl BufRead, output: &mut impl Write,
) -> Result<u8, (u8, String)> {
    let root = repository_root(flags.dir.as_deref()).map_err(|err| (FAILED, err))?;
    let settings = settings(flags, terminal, input, output).map_err(|err| (FAILED, err))?;
    let fetch = HttpFetch::new();
    let hashes = ReleaseHashes { version: sltk_core::VERSION, fetch: &fetch };
    let init = Init { root: &root, settings: &settings, with: &flags.with, hashes: &hashes, path: None };
    setup::init(train, sltk_core::VERSION, init, output).map_err(|err| (FAILED, format!("{err}\n")))?;
    Ok(0)
}

/// The fab settings from `flags`, asking for each missing one on a
/// `terminal`. Off a terminal a missing setting is an error that names each
/// missing flag.
fn settings(
    flags: &Flags, terminal: bool, input: &mut impl BufRead, output: &mut impl Write,
) -> Result<FabSettings, String> {
    let missing: Vec<&str> = Flag::ALL
        .into_iter()
        .filter(|flag| flags.get(*flag).is_none())
        .map(Flag::name)
        .collect();
    if !terminal && !missing.is_empty() {
        return Err(format!(
            "sltk: init needs {}; sltk asks for missing settings only on a terminal\n",
            missing.join(", ")
        ));
    }
    let mut values = Vec::with_capacity(Flag::ALL.len());
    for flag in Flag::ALL {
        let value = match flags.get(flag) {
            Some(value) => value.to_owned(),
            None => prompt::ask(input, output, question(flag), |answer| flag.check(answer))
                .map_err(|err| format!("sltk: could not ask for {}: {err}\n", flag.name()))?
                .ok_or_else(|| format!("sltk: no answer for {}\n", flag.name()))?,
        };
        values.push(value);
    }
    let [owner, trailer, gates] = <[String; 3]>::try_from(values).expect("one value per flag");
    FabSettings::new(owner, trailer, gates).map_err(|err| format!("{err}\n"))
}

/// What `sltk init` asks for `flag` on a terminal.
fn question(flag: Flag) -> &'static str {
    match flag {
        Flag::Owner => "GitHub login of the person who approves pull requests (--owner): ",
        Flag::Trailer => "Co-authored-by trailer for squash commits, as Name <email> (--trailer): ",
        Flag::Gates => "Command agents run before pushing (--gates): ",
    }
}

/// `dir` when given, else the top of the git repository holding the current
/// directory.
fn repository_root(dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(dir) = dir {
        if !dir.is_dir() {
            return Err(format!("sltk: --dir {} is not a directory\n", dir.display()));
        }
        return Ok(dir.to_owned());
    }
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|err| format!("{NOT_A_REPOSITORY} (could not run git: {err})\n"))?;
    let top = String::from_utf8(output.stdout).unwrap_or_default();
    let top = top.trim_end_matches(['\n', '\r']);
    if !output.status.success() || top.is_empty() {
        return Err(format!("{NOT_A_REPOSITORY}\n"));
    }
    Ok(PathBuf::from(top))
}

#[cfg(test)]
mod tests {
    use super::{Flags, question, settings};
    use sltk_core::setup::Flag;

    fn flags(owner: Option<&str>, trailer: Option<&str>, gates: Option<&str>) -> Flags {
        Flags {
            owner: owner.map(String::from),
            trailer: trailer.map(String::from),
            gates: gates.map(String::from),
            with: Vec::new(),
            dir: None,
        }
    }

    #[test]
    fn off_a_terminal_each_missing_flag_is_named() {
        let mut output = Vec::new();
        let err = settings(
            &flags(None, Some("A <a@b.c>"), None),
            false,
            &mut "octo\n".as_bytes(),
            &mut output,
        )
        .unwrap_err();
        assert_eq!(
            err,
            "sltk: init needs --owner, --gates; sltk asks for missing settings only on a terminal\n"
        );
        assert!(output.is_empty());
    }

    #[test]
    fn on_a_terminal_only_the_missing_flags_are_asked_for() {
        let mut output = Vec::new();
        let typed = "not a trailer\nAda <ada@example.com>\n";
        let got = settings(
            &flags(Some("octo"), None, Some("just check")),
            true,
            &mut typed.as_bytes(),
            &mut output,
        )
        .unwrap();
        assert_eq!(
            (got.owner.as_str(), got.trailer.as_str()),
            ("octo", "Ada <ada@example.com>")
        );
        let asked = question(Flag::Trailer);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{asked}that answer is not Name <email>\n{asked}")
        );
    }

    #[test]
    fn a_bad_flag_value_is_an_error_without_a_question() {
        let mut output = Vec::new();
        let err = settings(
            &flags(Some("a b"), Some("A <a@b.c>"), Some("x")),
            true,
            &mut "".as_bytes(),
            &mut output,
        )
        .unwrap_err();
        assert_eq!(err, "sltk: --owner holds a space\n");
        assert!(output.is_empty());
    }

    #[test]
    fn no_answer_is_an_error() {
        let mut output = Vec::new();
        let err = settings(
            &flags(None, Some("A <a@b.c>"), Some("x")),
            true,
            &mut "".as_bytes(),
            &mut output,
        )
        .unwrap_err();
        assert_eq!(err, "sltk: no answer for --owner\n");
    }
}
