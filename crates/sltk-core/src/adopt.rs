//! Adoption: `sltk init` on a repository that fab already set up, with
//! `.github/fabrial.toml` and no pin.
//!
//! [`plan`] works out every change before any is made: the glue files sltk
//! replaces, shown as a diff, the pin, and the commands that write the
//! tools' own files. Planning runs `arcl skill install --check`, which writes
//! nothing, to learn whether `arcl skill install` is needed. [`apply`] then
//! writes the glue, runs the commands, and writes the pin last, so a failed
//! adoption leaves no pin and a rerun adopts again.
//!
//! Adoption keeps fab's settings: it runs `fab update`, which keeps them,
//! rather than `fab init`. It replaces a hand-copied session hook, and
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
    /// which adoption keeps.
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

/// Works out the adoption of `root` without changing it. Reads fab's
/// settings and checks the given flags against them, reads this release's
/// archive hashes for the pin, plans the glue, and runs [`SKILL_CHECK`].
pub fn plan(train: &Train, version: &str, adopt: &Adopt) -> Result<Adoption, SetupError> {
    let pin_path = adopt.root.join(pin::PATH);
    if std::fs::symlink_metadata(&pin_path).is_ok() {
        return Err(SetupError::PinExists { path: pin_path });
    }
    check_settings(adopt)?;
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
    let mut commands = vec![owned(&FAB_UPDATE)];
    if std::fs::symlink_metadata(adopt.root.join(".arcl")).is_err() {
        commands.push(owned(&ARCL_INIT));
    }
    let check = owned(&SKILL_CHECK);
    let line = setup::command_line(&check);
    let status = setup::run(&check, adopt.root, adopt.path).map_err(|source| SetupError::Spawn {
        command: line.clone(),
        dir: adopt.root.to_owned(),
        source,
        progress: Progress { ran: Vec::new(), not_run: Vec::new() },
    })?;
    match status.code() {
        Some(0) => {}
        Some(SKILL_DIFFERS) => commands.push(owned(&SKILL_INSTALL)),
        _ => return Err(SetupError::Check { command: line, status }),
    }
    Ok(Adoption { glue, pin, pin_text, commands })
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

/// Checks that fab's settings file can be read and holds the value of each
/// flag in `adopt.given`.
fn check_settings(adopt: &Adopt) -> Result<(), SetupError> {
    /// The keys of fab's settings file that the flags set.
    #[derive(serde::Deserialize)]
    struct Settings {
        owner: Option<String>,
        trailer: Option<String>,
        gates: Option<String>,
    }
    let path = adopt.root.join(FAB_SETTINGS);
    let refuse = |reason: String| SetupError::Adopt { path: path.clone(), reason };
    let text = setup::read_small(&path)
        .map_err(refuse)?
        .ok_or_else(|| refuse("is missing".to_owned()))?;
    let settings: Settings = toml::from_str(&text)
        .map_err(|err| refuse(format!("could not be read as fab's settings: {}", err.message())))?;
    let differ: Vec<&str> = adopt
        .given
        .iter()
        .filter(|(flag, value)| {
            let held = match flag {
                Flag::Owner => &settings.owner,
                Flag::Trailer => &settings.trailer,
                Flag::Gates => &settings.gates,
            };
            held.as_deref() != Some(value.as_str())
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
    Ok(())
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
    use super::diff;

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
