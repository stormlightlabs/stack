//! `sltk init`: finds the repository root, then adopts a repository set up
//! by hand through [`sltk_core::adopt`], or gathers fab's settings from flags
//! or questions and hands them to [`sltk_core::setup::init`].

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use sltk_core::adopt::{self, Adopt};
use sltk_core::install::HttpFetch;
use sltk_core::setup::{self, FabSettings, Flag, Init, ReleaseHashes};
use sltk_core::train::Train;

use crate::{DECLINED, FAILED, prompt};

/// The question adoption asks on a terminal without `--yes`. Enter means no.
const ADOPT_QUESTION: &str = "Adopt this repository? [y/N] ";

/// The last line adoption prints when the person answers no.
const NOT_ADOPTED: &str = "sltk: nothing was written\n";

/// The error adoption prints off a terminal without `--yes`.
const NO_TERMINAL: &str = "sltk: nothing was written; pass --yes to adopt the repository off a terminal\n";

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
    /// `--yes`: adopt without asking.
    pub yes: bool,
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

/// Runs `sltk init` in the repository root. Adopts a repository set up by
/// hand, or else asks on a `terminal` for each fab setting the flags left
/// out. Returns the exit code, or the exit code and the error to print.
pub fn run(
    train: &Train, flags: &Flags, terminal: bool, input: &mut impl BufRead, output: &mut impl Write,
) -> Result<u8, (u8, String)> {
    let root = repository_root(flags.dir.as_deref()).map_err(|err| (FAILED, err))?;
    let fetch = HttpFetch::new();
    let hashes = ReleaseHashes { version: sltk_core::VERSION, fetch: &fetch };
    if adopt::adopting(&root).map_err(|err| (FAILED, format!("{err}\n")))? {
        let given: Vec<(Flag, String)> = Flag::ALL
            .into_iter()
            .filter_map(|flag| flags.get(flag).map(|value| (flag, value.to_owned())))
            .collect();
        let adopt = Adopt { root: &root, given: &given, with: &flags.with, hashes: &hashes, path: None };
        return run_adopt(train, &adopt, flags.yes, terminal, input, output);
    }
    let settings = settings(flags, terminal, input, output).map_err(|err| (FAILED, err))?;
    let init = Init { root: &root, settings: &settings, with: &flags.with, hashes: &hashes, path: None };
    setup::init(train, sltk_core::VERSION, init, output).map_err(|err| (FAILED, format!("{err}\n")))?;
    Ok(0)
}

/// Adopts the repository `adopt` names: plans the adoption and prints it,
/// then carries it out with `yes`, or on a `terminal` when the person says
/// yes. Off a terminal without `yes` it exits [`DECLINED`] without asking.
/// Returns the exit code, or the exit code and the error to print.
fn run_adopt(
    train: &Train, adopt: &Adopt, yes: bool, terminal: bool, input: &mut impl BufRead, output: &mut impl Write,
) -> Result<u8, (u8, String)> {
    let failed = |err: setup::SetupError| (FAILED, format!("{err}\n"));
    let io_failed = |err: std::io::Error| (FAILED, format!("sltk: could not ask before adopting: {err}\n"));
    let adoption = adopt::plan(train, sltk_core::VERSION, adopt).map_err(failed)?;
    output.write_all(adoption.describe().as_bytes()).map_err(io_failed)?;
    if !yes {
        if !terminal {
            return Err((DECLINED, NO_TERMINAL.to_owned()));
        }
        if !prompt::confirm(input, output, ADOPT_QUESTION, false).map_err(io_failed)? {
            output.write_all(NOT_ADOPTED.as_bytes()).map_err(io_failed)?;
            return Ok(DECLINED);
        }
    }
    adopt::apply(train, adopt, &adoption, output).map_err(failed)?;
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
    git_toplevel(None)
}

/// The top of the git repository holding `cwd`, or the current directory
/// when `cwd` is `None`. git is found on the absolute entries of `PATH`.
fn git_toplevel(cwd: Option<&Path>) -> Result<PathBuf, String> {
    let git = setup::find_program("git", std::env::var_os("PATH").as_deref())
        .ok_or_else(|| format!("{NOT_A_REPOSITORY} (no executable file named git in an absolute PATH directory)\n"))?;
    let mut command = Command::new(git);
    command.args(["rev-parse", "--show-toplevel"]);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command
        .output()
        .map_err(|err| format!("{NOT_A_REPOSITORY} (could not run git: {err})\n"))?;
    if !output.status.success() {
        return Err(format!("{NOT_A_REPOSITORY}\n"));
    }
    setup::path_from_git(output.stdout).map_err(|err| format!("sltk: could not find the repository root: {err}\n"))
}

#[cfg(test)]
mod tests {
    use super::{ADOPT_QUESTION, Flags, NO_TERMINAL, NOT_ADOPTED, git_toplevel, question, run_adopt, settings};
    use crate::DECLINED;
    use sltk_core::adopt::Adopt;
    use sltk_core::setup::{FAB_SETTINGS, Flag, HashSource};
    use sltk_core::train::{Platform, Train};

    fn flags(owner: Option<&str>, trailer: Option<&str>, gates: Option<&str>) -> Flags {
        Flags {
            owner: owner.map(String::from),
            trailer: trailer.map(String::from),
            gates: gates.map(String::from),
            with: Vec::new(),
            dir: None,
            yes: false,
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
        assert_eq!(
            err,
            "sltk: --owner is not a GitHub login: letters, digits and single hyphens, not at either end, up to 39 characters\n"
        );
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

    #[cfg(unix)]
    #[test]
    fn a_root_that_is_not_utf8_is_kept_as_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join(std::ffi::OsStr::from_bytes(b"repo-\xff"));
        std::fs::create_dir(&repo).unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&repo)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(git_toplevel(Some(&repo)).unwrap(), repo.canonicalize().unwrap());
    }

    /// The same hash for every platform, in place of a published release.
    #[cfg(unix)]
    struct Fixed;

    #[cfg(unix)]
    impl HashSource for Fixed {
        fn sha256(&self, _: Platform) -> Result<String, String> {
            Ok("ab".repeat(32))
        }
    }

    /// A repository set up by hand, with a hand-copied hook, and a bin
    /// directory whose `fab` and `arcl` exit 0 and append each call to
    /// `calls`.
    #[cfg(unix)]
    fn hand_set_up() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (root, bin) = (dir.path().join("repo"), dir.path().join("bin"));
        std::fs::create_dir_all(root.join(".github")).unwrap();
        std::fs::create_dir_all(root.join(".claude/hooks")).unwrap();
        std::fs::create_dir_all(root.join(".arcl")).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let settings = "owner = \"octo\"\ntrailer = \"Ada <ada@example.com>\"\ngates = \"make\"\n";
        std::fs::write(root.join(FAB_SETTINGS), settings).unwrap();
        std::fs::write(root.join("AGENTS.md"), "# A\n\n## Tasks\n\nold\n").unwrap();
        std::fs::write(
            root.join(".claude/hooks/session-start.sh"),
            "#!/bin/sh\nversion=0.3.0\n",
        )
        .unwrap();
        for tool in ["fab", "arcl"] {
            let script = bin.join(tool);
            let text = format!(
                "#!/bin/sh\necho \"{tool} $*\" >> '{}'\n",
                dir.path().join("calls").display()
            );
            std::fs::write(&script, text).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (dir, root, bin)
    }

    /// Each file under `root` with its bytes, in order.
    #[cfg(unix)]
    fn files(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                stack.extend(std::fs::read_dir(&path).unwrap().map(|entry| entry.unwrap().path()));
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
        out.sort();
        out
    }

    /// Runs adoption on a repository set up by hand, with `yes`, `terminal`
    /// and `typed` as the answer. Returns the result, what it wrote, and
    /// whether the tree changed.
    #[cfg(unix)]
    fn adopt(yes: bool, terminal: bool, typed: &str) -> (Result<u8, (u8, String)>, String, bool) {
        let (dir, root, bin) = hand_set_up();
        let before = files(&root);
        let path = std::ffi::OsString::from(&bin);
        let adopt = Adopt { root: &root, given: &[], with: &[], hashes: &Fixed, path: Some(&path) };
        let train = Train::embedded().unwrap();
        let mut output = Vec::new();
        let result = run_adopt(&train, &adopt, yes, terminal, &mut typed.as_bytes(), &mut output);
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        if !yes && (result != Ok(0)) {
            assert_eq!(calls, "arcl skill install --check\n");
        }
        (result, String::from_utf8(output).unwrap(), files(&root) != before)
    }

    #[cfg(unix)]
    #[test]
    fn off_a_terminal_without_yes_adoption_prints_the_diff_and_writes_nothing() {
        let (result, output, changed) = adopt(false, false, "y\n");
        assert_eq!(result, Err((DECLINED, NO_TERMINAL.to_owned())));
        assert!(output.contains("\n--- a/.claude/hooks/session-start.sh\n"), "{output}");
        assert!(output.ends_with("\n+optional = []\n"), "{output}");
        assert!(!changed);
    }

    #[cfg(unix)]
    #[test]
    fn on_a_terminal_no_or_enter_writes_nothing() {
        for typed in ["n\n", "\n", ""] {
            let (result, output, changed) = adopt(false, true, typed);
            assert_eq!(result, Ok(DECLINED), "{typed:?}");
            assert!(output.contains(ADOPT_QUESTION), "{output}");
            assert!(output.ends_with(NOT_ADOPTED), "{output}");
            assert!(!changed, "{typed:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn yes_at_the_prompt_or_as_a_flag_adopts() {
        let (result, output, changed) = adopt(false, true, "y\n");
        assert_eq!(result, Ok(0));
        assert!(
            output.contains(&format!("{ADOPT_QUESTION}wrote AGENTS.md\n")),
            "{output}"
        );
        assert!(
            output.ends_with("running fab update\nwrote .github/sltk.toml\n"),
            "{output}"
        );
        assert!(changed);
        let (result, output, changed) = adopt(true, false, "");
        assert_eq!(result, Ok(0));
        assert!(!output.contains(ADOPT_QUESTION), "{output}");
        assert!(changed);
    }
}
