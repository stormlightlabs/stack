//! Adoption: `sltk init` on a repository that fab already set up, with
//! `.github/fabrial.toml` and no pin.
//!
//! [`plan`] works out every change before any is made: the glue files sltk
//! replaces, shown as a diff, the pin, the commands that write the tools'
//! own files, and the files `fab check` reports as drifted, which `fab
//! update` rewrites. Planning first checks that every program it runs is on
//! `PATH`, then runs `fab check` and `arcl skill install --check`, which
//! write nothing, the second to learn whether `arcl skill install` is
//! needed. [`apply`] then
//! writes the glue, runs the commands, and writes the pin last, so a failed
//! adoption leaves no pin and a rerun adopts again.
//!
//! Adoption keeps fab's settings: it runs `fab update`, which keeps them,
//! rather than `fab init`. Settings from before fab 0.9.0 name no App, and
//! `fab update` needs one, so adoption passes `--app` to it then, and
//! `--secret-prefix` when given and the settings name none. It replaces a hand-copied session hook, and
//! rewrites the one `## Tasks` section outside the markers in place.

use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;

use crate::glue::{self, Change, Existing};
use crate::pin::{self, Pin};
use crate::setup::{self, FAB_SETTINGS, Flag, HashSource, Progress, SetupError};
use crate::train::Train;

/// The command that refreshes fab's files and keeps its settings.
pub const FAB_UPDATE: [&str; 2] = ["fab", "update"];

/// The command that reports, writing nothing, each file of fab's that
/// differs from what [`FAB_UPDATE`] would write, one per line.
pub const FAB_CHECK: [&str; 2] = ["fab", "check"];

/// The exit code of [`FAB_CHECK`] when it reports drift.
pub const FAB_DRIFT: i32 = 1;

/// The command that sets up arcl's workspace, run only when the repository
/// has no `.arcl` directory.
pub const ARCL_INIT: [&str; 3] = ["arcl", "init", "--workspace"];

/// The command that reports whether the arcl skill files are missing or
/// differ, writing nothing.
pub const SKILL_CHECK: [&str; 4] = ["arcl", "skill", "install", "--check"];

/// The command that writes the arcl skill files.
pub const SKILL_INSTALL: [&str; 3] = ["arcl", "skill", "install"];

/// The exit code of [`SKILL_CHECK`] when a skill file is missing or differs.
pub const SKILL_DIFFERS: i32 = 4;

/// Lines of unchanged text shown around each change in the diff.
const CONTEXT: usize = 3;

/// The most cells the line diff's table may hold. Past it the diff shows
/// the whole old file removed and the whole new one added.
const MAX_CELLS: usize = 16 * 1024 * 1024;

/// What adoption works on.
pub struct Adopt<'a> {
    /// The repository root, where every command runs.
    pub root: &'a Path,
    /// The fab settings given as flags. Each must match fab's settings file,
    /// which adoption keeps, or set a value the file lacks: `--app` is
    /// required when the file names no App.
    pub given: &'a [(Flag, String)],
    /// The optional tools the pin lists.
    pub with: &'a [String],
    /// Where the `sltk` archive hashes come from.
    pub hashes: &'a dyn HashSource,
    /// The `PATH` to find the commands on and run them with, or `None` for
    /// the one `sltk` runs with.
    pub path: Option<&'a OsStr>,
}

/// Everything adoption will change, worked out before any change.
#[derive(Debug)]
pub struct Adoption {
    /// The glue files sltk writes, with their text now and after.
    pub glue: Vec<Change>,
    /// The pin sltk writes last.
    pub pin: Pin,
    /// The pin's text.
    pub pin_text: String,
    /// The commands run after the glue is written, each as its program and
    /// arguments.
    pub commands: Vec<Vec<String>>,
    /// What [`FAB_CHECK`] reported, which [`FAB_UPDATE`] rewrites: the
    /// paths it named, or its lines as it printed them when one did not
    /// have the form [`drift`] reads.
    pub drift: Vec<String>,
}

impl Adoption {
    /// What adoption will do, for a person to read before saying yes: the
    /// commands, then a unified diff of each file sltk writes.
    pub fn describe(&self) -> String {
        let mut out = format!(
            "sltk init adopts this repository and keeps the settings in {FAB_SETTINGS}.\n\
             It runs these commands, which write their own files:\n"
        );
        for command in &self.commands {
            out.push_str(&format!("  {}\n", setup::command_line(command)));
        }
        if !self.drift.is_empty() {
            out.push_str("fab update will rewrite or remove these files, which fab check reports as drifted:\n");
            for line in &self.drift {
                out.push_str(&format!("  {line}\n"));
            }
        }
        out.push_str("It writes these files:\n");
        for change in &self.glue {
            out.push_str(&diff(change.path, change.old.as_deref(), &change.new));
            if let (Some(old), Some(new)) = (change.old_mode, change.mode)
                && old & 0o777 != new
            {
                out.push_str(&format!("mode {:o} -> {new:o}\n", old & 0o777));
            }
        }
        out.push_str(&diff(pin::PATH, None, &self.pin_text));
        out
    }
}

/// Whether `sltk init` in `root` adopts the repository: it has fab's
/// settings file and no pin, and git holds no record of an unfinished fresh
/// `sltk init`, which resumes instead.
pub fn adopting(root: &Path) -> Result<bool, SetupError> {
    let exists = |path: &Path| std::fs::symlink_metadata(path).is_ok();
    if exists(&root.join(pin::PATH)) || !exists(&root.join(FAB_SETTINGS)) {
        return Ok(false);
    }
    Ok(!exists(&setup::record_path(root)?))
}

/// Whether adopting `root` needs `--app`: fab's settings file names no App,
/// as a file from fab 0.9.0 or earlier does, and `fab update` needs one.
pub fn needs_app(root: &Path) -> Result<bool, SetupError> {
    Ok(read_settings(root)?.app.is_none())
}

/// Works out the adoption of `root` without changing it. Reads fab's
/// settings and checks the given flags against them, checks that fab and
/// arcl are on `PATH`, reads this release's archive hashes for the pin,
/// plans the glue, and runs [`FAB_CHECK`] and [`SKILL_CHECK`].
pub fn plan(train: &Train, version: &str, adopt: &Adopt) -> Result<Adoption, SetupError> {
    let pin_path = adopt.root.join(pin::PATH);
    if std::fs::symlink_metadata(&pin_path).is_ok() {
        return Err(SetupError::PinExists { path: pin_path });
    }
    let settings = check_settings(adopt)?;
    // Every command below and in apply runs fab or arcl.
    let search = adopt.path.map(OsStr::to_owned).or_else(|| std::env::var_os("PATH"));
    for program in ["fab", "arcl"] {
        if setup::find_program(program, search.as_deref()).is_none() {
            return Err(SetupError::NoProgram { program: program.to_owned() });
        }
    }
    let mut sha256 = std::collections::BTreeMap::new();
    for platform in pin::TARGETS {
        let hash = adopt.hashes.sha256(platform).map_err(|message| SetupError::Hash {
            version: version.to_owned(),
            platform,
            message,
        })?;
        sha256.insert(platform, hash);
    }
    let pin = Pin { version: version.to_owned(), sha256, optional: adopt.with.to_vec() };
    let pin_text = pin.render(train)?;
    let glue = glue::plan(adopt.root, Existing::Replace)?;

    let owned = |command: &[&str]| command.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    let mut update = owned(&FAB_UPDATE);
    // A value the file lacks goes to fab update, which writes it; one the
    // file has matched the flag in check_settings.
    for (flag, value) in adopt.given {
        if matches!(flag, Flag::App | Flag::SecretPrefix) && settings.get(*flag).is_none() {
            update.extend([flag.name().to_owned(), value.clone()]);
        }
    }
    let mut commands = vec![update];
    if std::fs::symlink_metadata(adopt.root.join(".arcl")).is_err() {
        commands.push(owned(&ARCL_INIT));
    }
    let spawn = |line: &str, source| SetupError::Spawn {
        command: line.to_owned(),
        dir: adopt.root.to_owned(),
        source,
        progress: Progress { ran: Vec::new(), not_run: Vec::new() },
    };

    let check = owned(&FAB_CHECK);
    let line = setup::command_line(&check);
    let output = setup::run_captured(&check, adopt.root, adopt.path).map_err(|source| spawn(&line, source))?;
    let drift = match output.status.code() {
        Some(0) => Vec::new(),
        Some(FAB_DRIFT) => drift(&String::from_utf8_lossy(&output.stdout)),
        _ => return Err(SetupError::Check { command: line, status: output.status }),
    };

    let check = owned(&SKILL_CHECK);
    let line = setup::command_line(&check);
    let status = setup::run(&check, adopt.root, adopt.path).map_err(|source| spawn(&line, source))?;
    match status.code() {
        Some(0) => {}
        Some(SKILL_DIFFERS) => commands.push(owned(&SKILL_INSTALL)),
        _ => return Err(SetupError::Check { command: line, status }),
    }
    Ok(Adoption { glue, pin, pin_text, commands, drift })
}

/// Carries out `adoption`, which [`plan`] made for `adopt`: checks that no
/// glue file changed since, writes the glue, runs each command in the root,
/// stopping at the first failure, and writes the pin. Writes the paths it
/// wrote and the commands it ran to `output`.
pub fn apply(train: &Train, adopt: &Adopt, adoption: &Adoption, output: &mut dyn Write) -> Result<Pin, SetupError> {
    let pin_path = adopt.root.join(pin::PATH);
    if std::fs::symlink_metadata(&pin_path).is_ok() {
        return Err(SetupError::PinExists { path: pin_path });
    }
    let now = glue::plan(adopt.root, Existing::Replace)?;
    if now != adoption.glue {
        let find = |list: &[Change], path: &str| list.iter().find(|c| c.path == path).cloned();
        let path = adoption
            .glue
            .iter()
            .chain(&now)
            .map(|c| c.path)
            .find(|path| find(&now, path) != find(&adoption.glue, path))
            .unwrap_or(glue::AGENTS);
        return Err(SetupError::Changed { path: adopt.root.join(path) });
    }
    // The files are written, so a closed stdout must not stop the run.
    for path in glue::apply(adopt.root, &adoption.glue)? {
        let _ = writeln!(output, "wrote {path}");
    }
    let lines: Vec<String> = adoption.commands.iter().map(|c| setup::command_line(c)).collect();
    let mut ran = Vec::new();
    for (i, command) in adoption.commands.iter().enumerate() {
        let line = &lines[i];
        let _ = writeln!(output, "running {line}");
        let progress = || Progress { ran: ran.clone(), not_run: lines[i + 1..].to_vec() };
        let status = setup::run(command, adopt.root, adopt.path).map_err(|source| SetupError::Spawn {
            command: line.clone(),
            dir: adopt.root.to_owned(),
            source,
            progress: progress(),
        })?;
        if !status.success() {
            return Err(SetupError::Failed { command: line.clone(), status, progress: progress() });
        }
        ran.push(line.clone());
    }
    adoption.pin.write(adopt.root, train)?;
    let _ = writeln!(output, "wrote {}", pin::PATH);
    Ok(adoption.pin.clone())
}

/// A unified diff of `path` from `old`, or no file, to `new`, with
/// [`CONTEXT`] lines around each change.
pub fn diff(path: &str, old: Option<&str>, new: &str) -> String {
    let from = if old.is_some() { format!("a/{path}") } else { "/dev/null".to_owned() };
    let mut out = format!("--- {from}\n+++ b/{path}\n");
    let old_lines: Vec<&str> = old.map(|text| text.split_inclusive('\n').collect()).unwrap_or_default();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let ops = line_ops(&old_lines, &new_lines);
    let changed: Vec<usize> = (0..ops.len()).filter(|&i| ops[i].0 != ' ').collect();
    let mut i = 0;
    while i < changed.len() {
        // A hunk spans changes less than two contexts apart.
        let start = changed[i].saturating_sub(CONTEXT);
        let mut last = changed[i];
        while i + 1 < changed.len() && changed[i + 1] - last <= 2 * CONTEXT {
            i += 1;
            last = changed[i];
        }
        let end = (last + CONTEXT + 1).min(ops.len());
        let before = |kind: char| ops[..start].iter().filter(|op| op.0 != kind).count();
        let count = |kind: char| ops[start..end].iter().filter(|op| op.0 != kind).count();
        let (old_count, new_count) = (count('+'), count('-'));
        let old_start = before('+') + usize::from(old_count > 0);
        let new_start = before('-') + usize::from(new_count > 0);
        out.push_str(&format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"));
        for (kind, line) in &ops[start..end] {
            out.push(*kind);
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
        i += 1;
    }
    out
}

/// The drift `report`, the output of [`FAB_CHECK`], names. fab 0.9.1 prints
/// one line per drifted file: `missing: <path>` or `differs: <path>`,
/// `retired: <path>` for a file this release no longer installs, which `fab
/// update` removes, and two lines about [`FAB_SETTINGS`], which `fab update`
/// rewrites: `version: ...` when the settings name another fab, and
/// ``missing: `app` in ...`` when they name no App. When every line has one
/// of these forms the list holds the paths, each once; otherwise it holds
/// the report's lines as they are, so nothing fab printed is lost.
pub fn drift(report: &str) -> Vec<String> {
    let lines: Vec<&str> = report
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    let mut paths: Vec<String> = Vec::new();
    for line in &lines {
        let path = if line.starts_with("version: ") || line.starts_with("missing: `app` in ") {
            FAB_SETTINGS
        } else if let Some(path) = line
            .strip_prefix("missing: ")
            .or_else(|| line.strip_prefix("differs: "))
            .or_else(|| line.strip_prefix("retired: "))
        {
            path
        } else {
            return lines.iter().map(|line| line.to_string()).collect();
        };
        if !paths.iter().any(|seen| seen == path) {
            paths.push(path.to_owned());
        }
    }
    paths
}

/// The keys of fab's settings file that the flags set. The file holds other
/// keys, lists and a `[tasks]` table, which adoption leaves to fab.
#[derive(serde::Deserialize)]
struct Settings {
    owner: Option<String>,
    app: Option<String>,
    secret_prefix: Option<String>,
    trailer: Option<String>,
    gates: Option<String>,
}

impl Settings {
    /// The value the file holds for `flag`.
    fn get(&self, flag: Flag) -> Option<&str> {
        match flag {
            Flag::Owner => self.owner.as_deref(),
            Flag::App => self.app.as_deref(),
            Flag::SecretPrefix => self.secret_prefix.as_deref(),
            Flag::Trailer => self.trailer.as_deref(),
            Flag::Gates => self.gates.as_deref(),
        }
    }
}

/// Reads fab's settings file under `root` as TOML.
fn read_settings(root: &Path) -> Result<Settings, SetupError> {
    let path = root.join(FAB_SETTINGS);
    let refuse = |reason: String| SetupError::Adopt { path: path.clone(), reason };
    let text = setup::read_small(&path)
        .map_err(refuse)?
        .ok_or_else(|| refuse("is missing".to_owned()))?;
    toml::from_str(&text).map_err(|err| refuse(format!("could not be read as fab's settings: {}", err.message())))
}

/// Reads fab's settings file and checks the flags in `adopt.given` against
/// it: each value must match the file's, except that `--app` and
/// `--secret-prefix` may give a value the file lacks. The file must name an
/// App or `--app` must give one.
fn check_settings(adopt: &Adopt) -> Result<Settings, SetupError> {
    let settings = read_settings(adopt.root)?;
    let refuse = |reason: String| SetupError::Adopt { path: adopt.root.join(FAB_SETTINGS), reason };
    let differ: Vec<&str> = adopt
        .given
        .iter()
        .filter(|(flag, value)| match settings.get(*flag) {
            Some(held) => held != value,
            // fab update writes only these two when the file lacks them.
            None => !matches!(flag, Flag::App | Flag::SecretPrefix),
        })
        .map(|(flag, _)| flag.name())
        .collect();
    if !differ.is_empty() {
        let flags = differ.join(", ");
        return Err(refuse(format!(
            "holds other values for {flags}. Adoption keeps the settings in this file; leave out {flags}, \
             or change the file with fab update first"
        )));
    }
    if settings.app.is_none() && !adopt.given.iter().any(|(flag, _)| *flag == Flag::App) {
        return Err(refuse(
            "names no app, and fab update needs one. Pass --app with the slug of the repository's GitHub App"
                .to_owned(),
        ));
    }
    Ok(settings)
}

/// The lines of `old` and `new` as a list of kept (` `), removed (`-`) and
/// added (`+`) lines, from a longest common subsequence.
fn line_ops<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(char, &'a str)> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (a, b) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);
    let mut ops: Vec<(char, &str)> = old[..prefix].iter().map(|line| (' ', *line)).collect();
    let (n, m) = (a.len(), b.len());
    if (n + 1).saturating_mul(m + 1) > MAX_CELLS {
        ops.extend(a.iter().map(|line| ('-', *line)));
        ops.extend(b.iter().map(|line| ('+', *line)));
    } else {
        // table[i][j]: the longest common subsequence of a[i..] and b[j..].
        let width = m + 1;
        let mut table = vec![0u32; (n + 1) * width];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i * width + j] = if a[i] == b[j] {
                    table[(i + 1) * width + j + 1] + 1
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && a[i] == b[j] {
                ops.push((' ', a[i]));
                i += 1;
                j += 1;
            } else if i < n && (j == m || table[(i + 1) * width + j] >= table[i * width + j + 1]) {
                ops.push(('-', a[i]));
                i += 1;
            } else {
                ops.push(('+', b[j]));
                j += 1;
            }
        }
    }
    ops.extend(old[old.len() - suffix..].iter().map(|line| (' ', *line)));
    ops
}

#[cfg(test)]
mod tests {
    use super::{diff, drift};

    #[test]
    fn drift_lists_each_path_fab_check_names_once() {
        let report = "differs: .github/workflows/branch-pr.yml\nmissing: .claude/agents/reviser.md\n\
                      differs: AGENTS.md's fabrial section\nversion: installed 0.7.0, this build 0.8.0\n\
                      differs: .github/workflows/branch-pr.yml\n";
        assert_eq!(
            drift(report),
            [
                ".github/workflows/branch-pr.yml",
                ".claude/agents/reviser.md",
                "AGENTS.md's fabrial section",
                ".github/fabrial.toml"
            ]
        );
        assert!(drift("").is_empty());
    }

    #[test]
    fn drift_reads_retired_files_and_a_missing_app() {
        // As fab 0.9.1 prints them, from crates/fab-core/src/install.rs.
        let report = "retired: .claude/skills/implement/SKILL.md\n\
                      missing: `app` in .github/fabrial.toml; run fab update --app <slug> with the repository's GitHub App\n\
                      version: installed 0.8.0, this build 0.9.1\n";
        assert_eq!(
            drift(report),
            [".claude/skills/implement/SKILL.md", ".github/fabrial.toml"]
        );
    }

    #[test]
    fn drift_keeps_a_report_it_cannot_read_as_printed() {
        let report = "differs: a.md\nsomething new: b.md\n";
        assert_eq!(drift(report), ["differs: a.md", "something new: b.md"]);
    }

    #[test]
    fn a_new_file_is_all_added_lines() {
        assert_eq!(
            diff("x", None, "a\nb\n"),
            "--- /dev/null\n+++ b/x\n@@ -0,0 +1,2 @@\n+a\n+b\n"
        );
    }

    #[test]
    fn a_change_shows_three_lines_of_context_and_line_numbers() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n9\n";
        assert_eq!(
            diff("f", Some(old), new),
            "--- a/f\n+++ b/f\n@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+five\n 6\n 7\n 8\n"
        );
    }

    #[test]
    fn distant_changes_make_separate_hunks() {
        let old: String = (1..=20).map(|n| format!("{n}\n")).collect();
        let new: String = (1..=20)
            .map(|n| match n {
                2 => "two\n".to_owned(),
                19 => "nineteen\n".to_owned(),
                n => format!("{n}\n"),
            })
            .collect();
        let got = diff("f", Some(&old), &new);
        assert_eq!(got.matches("\n@@ ").count(), 2, "{got}");
        assert!(got.contains("@@ -1,5 +1,5 @@\n 1\n-2\n+two\n 3\n"), "{got}");
        assert!(
            got.contains("@@ -16,5 +16,5 @@\n 16\n 17\n 18\n-19\n+nineteen\n 20\n"),
            "{got}"
        );
    }

    #[test]
    fn identical_text_has_no_hunk_and_a_missing_newline_is_marked() {
        assert_eq!(diff("f", Some("a\n"), "a\n"), "--- a/f\n+++ b/f\n");
        assert_eq!(
            diff("f", Some("a"), "b"),
            "--- a/f\n+++ b/f\n@@ -1,1 +1,1 @@\n-a\n\\ No newline at end of file\n+b\n\\ No newline at end of file\n"
        );
    }
}
