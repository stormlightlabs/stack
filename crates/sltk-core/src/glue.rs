//! The glue `sltk init` writes itself: the Tasks section in AGENTS.md, the
//! session hook, the hook's entry in `.claude/settings.json`, and the
//! `[tasks]` table in fab's settings that points `fab run` at arcl.
//!
//! [`plan`] reads the files and works out each change; [`apply`] writes
//! them. Each writer leaves a file alone when it already holds what it would
//! write, so a second run changes no byte. None follows a symlink, whether
//! it is the file or a directory between the repository root and the file,
//! so every write lands inside the repository. The one exception is an
//! AGENTS.md or CLAUDE.md that links to the other: the Tasks section goes in
//! the file it links to. Every file is checked before any is written, so a
//! refusal changes nothing.
//!
//! The Markdown scans follow CommonMark's code fences: a heading or a marker
//! line inside a fence is code, not structure.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::setup::FAB_SETTINGS;

/// Where the Tasks section goes, relative to the repository root.
pub const AGENTS: &str = "AGENTS.md";

/// Where the Tasks section goes in a repository that has this file and no
/// AGENTS.md. Claude Code reads CLAUDE.md and not AGENTS.md, so a new
/// AGENTS.md beside it would hold a section no agent there loads.
pub const CLAUDE: &str = "CLAUDE.md";

/// Where the session hook goes, relative to the repository root.
pub const HOOK: &str = ".claude/hooks/session-start.sh";

/// Where the hook's SessionStart entry goes, relative to the repository root.
pub const SETTINGS: &str = ".claude/settings.json";

/// The command the SessionStart entry runs.
/// The variable is quoted so a project path with a space stays one word.
pub const HOOK_COMMAND: &str = "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/session-start.sh";

/// The line that opens the Tasks section in AGENTS.md.
pub const TASKS_BEGIN: &str = "<!-- sltk:tasks:begin -->";

/// The line that closes the Tasks section in AGENTS.md.
pub const TASKS_END: &str = "<!-- sltk:tasks:end -->";

/// The session hook, the same file in every repository.
pub const HOOK_SCRIPT: &str = include_str!("../templates/session-start.sh");

/// The Tasks section between its markers.
pub const TASKS: &str = include_str!("../templates/tasks.md");

/// `.claude/settings.json` for a repository that has none. Its one
/// SessionStart entry is the one [`settings`] adds to an existing file.
pub const SETTINGS_TEMPLATE: &str = include_str!("../templates/settings.json");

/// The `[tasks]` table [`fab_tasks`] adds to fab's settings: arcl's command
/// for each step of a tracked task, with a blank line and fab's comment
/// before it. The text is what fab 0.9.1 renders for these values, so `fab
/// check` finds no drift in the file and `fab update` keeps the table.
pub const FAB_TASKS: &str = include_str!("../templates/fab-tasks.toml");

/// The mode the hook is written with.
const HOOK_MODE: u32 = 0o755;

/// What the writers do with glue that sltk did not write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existing {
    /// Refuse a hook that differs from [`HOOK_SCRIPT`] and a `## Tasks`
    /// section outside the markers. Fresh setup does this.
    Refuse,
    /// Replace the hook, and the one `## Tasks` section outside the markers
    /// with the marked one. Adoption does this, after showing the diff.
    Replace,
}

/// Why a glue file could not be written.
#[derive(Debug, thiserror::Error)]
pub enum GlueError {
    /// The file could not be read or written.
    #[error("sltk: {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    /// The file, or a directory above it, is a symlink, or is not a regular
    /// file or a directory.
    #[error("sltk: {} {reason}", path.display())]
    NotAFile { path: PathBuf, reason: &'static str },
    /// The file holds something the writer cannot merge into or replace,
    /// such as one Tasks marker without the other, settings that are not a
    /// JSON object, or a hook sltk did not write.
    #[error("sltk: {}: {reason}", path.display())]
    Content { path: PathBuf, reason: String },
}

/// One glue file [`plan`] would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The file, relative to the repository root.
    pub path: &'static str,
    /// Its text now, or `None` when there is no file.
    pub old: Option<String>,
    /// The text it gets.
    pub new: String,
    /// Its Unix permission bits now, when it exists and sltk sets its mode.
    pub old_mode: Option<u32>,
    /// The Unix permission bits it gets, or `None` to leave them to the
    /// umask.
    pub mode: Option<u32>,
}

/// An open code fence: the character its opening line repeats, and how many
/// times. Only a line of the same character, at least as long, closes it.
#[derive(Debug, Clone, Copy)]
struct Fence {
    mark: u8,
    len: usize,
}

/// One line of a Markdown file, as [`scan`] reads it.
#[derive(Debug)]
struct Line<'a> {
    /// The byte offset where the line starts.
    start: usize,
    /// The line, with its ending.
    text: &'a str,
    /// Whether the line opens, closes or lies inside a code fence.
    fenced: bool,
}

/// Writes the Tasks section, the hook, its settings entry and fab's `[tasks]`
/// table under `root`, and returns the paths, relative to `root`, of the
/// files it changed.
///
/// Refuses a hook that differs from [`HOOK_SCRIPT`] rather than replace it.
/// It checks every file before writing any.
pub fn write(root: &Path) -> Result<Vec<&'static str>, GlueError> {
    apply(root, &plan(root, Existing::Refuse)?)
}

/// The changes the glue writers would make under `root`, without writing
/// anything. `existing` says what to do with a hook or a Tasks section sltk
/// did not write.
pub fn plan(root: &Path, existing: Existing) -> Result<Vec<Change>, GlueError> {
    let content = |relative: &str| {
        let path = root.join(relative);
        move |reason| GlueError::Content { path, reason }
    };
    let target = tasks_file(root)?;
    let agents = read(root, target)?;
    let agents_text = tasks_section_with(agents.as_deref(), existing).map_err(content(target))?;
    let hook = read(root, HOOK)?;
    if existing == Existing::Refuse && hook.as_deref().is_some_and(|text| text != HOOK_SCRIPT) {
        return Err(content(HOOK)(
            "differs from the hook sltk writes, and sltk init replaces a hook only when it adopts a \
             repository that has .github/fabrial.toml. Move this one aside and rerun sltk init"
                .to_owned(),
        ));
    }
    let settings_text = read(root, SETTINGS)?;
    let merged = settings(settings_text.as_deref()).map_err(content(SETTINGS))?;
    let fab_text = read(root, FAB_SETTINGS)?;
    let fab = fab_tasks(fab_text.as_deref()).map_err(content(FAB_SETTINGS))?;

    let mut changes = Vec::new();
    if agents.as_deref() != Some(agents_text.as_str()) {
        changes.push(Change { path: target, old: agents, new: agents_text, old_mode: None, mode: None });
    }
    let old_mode = hook.as_ref().and_then(|_| mode_of(&root.join(HOOK)));
    if hook.as_deref() != Some(HOOK_SCRIPT) || !old_mode.is_none_or(|mode| mode & 0o111 == 0o111) {
        changes.push(Change { path: HOOK, old: hook, new: HOOK_SCRIPT.to_owned(), old_mode, mode: Some(HOOK_MODE) });
    }
    if let Some(text) = merged {
        changes.push(Change { path: SETTINGS, old: settings_text, new: text, old_mode: None, mode: None });
    }
    if let Some(text) = fab {
        changes.push(Change { path: FAB_SETTINGS, old: fab_text, new: text, old_mode: None, mode: None });
    }
    Ok(changes)
}

/// Writes each change under `root`, and returns the paths it wrote.
pub fn apply(root: &Path, changes: &[Change]) -> Result<Vec<&'static str>, GlueError> {
    let mut written = Vec::new();
    for change in changes {
        replace(&root.join(change.path), &change.new, change.mode)?;
        written.push(change.path);
    }
    Ok(written)
}

/// The file the Tasks section goes in: the one of [`AGENTS`] and [`CLAUDE`]
/// that already holds a [`TASKS_BEGIN`] line, else [`AGENTS`], or [`CLAUDE`]
/// when the repository has that and no AGENTS.md. Checking for the marker
/// first keeps the target where an earlier run put it, even after `fab
/// update` creates AGENTS.md beside a CLAUDE.md that holds the section.
pub fn tasks_file(root: &Path) -> Result<&'static str, GlueError> {
    for relative in [AGENTS, CLAUDE] {
        if holds_marker(root, relative)? {
            return Ok(relative);
        }
    }
    if kind_or_link(&root.join(AGENTS))? {
        return Ok(follow_link(root, AGENTS));
    }
    if kind_or_link(&root.join(CLAUDE))? {
        return Ok(follow_link(root, CLAUDE));
    }
    Ok(AGENTS)
}

/// AGENTS.md with the Tasks section in it: `existing` with the text between
/// the marker lines replaced, or with the section appended after a blank
/// line when it has neither marker. A missing file becomes the section
/// alone. Fails when a marker line appears without the other, more than
/// once, or out of order, and when a `## Tasks` heading lies outside the
/// markers, since the file would then hold two Tasks sections.
pub fn tasks_section(existing: Option<&str>) -> Result<String, String> {
    tasks_section_with(existing, Existing::Refuse)
}

/// [`tasks_section`], except that with [`Existing::Replace`] a file with no
/// markers and one `## Tasks` heading has that section replaced in place:
/// the heading line up to the next line outside a code fence that is a
/// heading of level one or two or starts an HTML comment, or the end of the
/// file. Adoption also refuses a heading of level one or two outside the
/// markers that reads as Tasks in any case, with or without more text, but
/// is not `## Tasks`, since sltk would leave it beside its own section.
///
/// Both policies refuse a file with a code fence that nothing closes:
/// CommonMark reads the rest of the file as code, so a section sltk writes
/// after it would be code too.
pub fn tasks_section_with(existing: Option<&str>, policy: Existing) -> Result<String, String> {
    let section = format!("{TASKS_BEGIN}\n{TASKS}{TASKS_END}\n");
    let Some(text) = existing.filter(|text| !text.is_empty()) else {
        return Ok(section);
    };
    let (lines, unclosed) = scan(text);
    if let Some(number) = unclosed {
        return Err(format!(
            "opens a code fence on line {number} that nothing closes, so the rest of the file reads as \
             code. Close the fence and rerun sltk init"
        ));
    }
    let begins = marker_lines(&lines, TASKS_BEGIN);
    let ends = marker_lines(&lines, TASKS_END);
    let headings = tasks_headings(&lines);
    if policy == Existing::Replace {
        let (from, to) = match (begins.as_slice(), ends.as_slice()) {
            ([begin], [end]) if begin < end => (*begin, *end),
            _ => (0, 0),
        };
        let near: Vec<String> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.fenced && (line.start < from || line.start >= to) && near_tasks(line.text))
            .map(|(i, line)| format!("line {} ({})", i + 1, line.text.trim()))
            .collect();
        if !near.is_empty() {
            return Err(format!(
                "has a Tasks heading that is not ## Tasks at {}, so sltk would add a second Tasks section. \
                 Rename it to ## Tasks for sltk to replace, or give it another name",
                near.join(", ")
            ));
        }
    }
    let heading_outside = |from: usize, to: usize| headings.iter().any(|&at| at < from || at >= to);
    let two_sections = || {
        format!(
            "has a ## Tasks heading outside the {TASKS_BEGIN} and {TASKS_END} lines, so sltk would add a \
             second Tasks section. Delete that section, or put the marker lines around it for sltk to \
             replace"
        )
    };
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) if policy == Existing::Replace && headings.len() == 1 => {
            let start = headings[0];
            let end = section_end(&lines, start).unwrap_or(text.len());
            let gap = if end < text.len() { "\n" } else { "" };
            Ok(format!("{}{section}{gap}{}", &text[..start], &text[end..]))
        }
        ([], []) if policy == Existing::Replace && headings.len() > 1 => Err(format!(
            "has {} ## Tasks headings and no {TASKS_BEGIN} line, so sltk cannot tell which section to \
             replace. Delete all but one",
            headings.len()
        )),
        ([], []) if heading_outside(0, 0) => Err(two_sections()),
        ([begin], [end]) if begin < end && heading_outside(*begin, *end) => Err(two_sections()),
        ([], []) => {
            let gap = if text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            Ok(format!("{text}{gap}{section}"))
        }
        ([begin], [end]) if begin < end => {
            let after = text[*end..].find('\n').map_or(text.len(), |at| end + at + 1);
            Ok(format!("{}{section}{}", &text[..*begin], &text[after..]))
        }
        _ => Err(format!(
            "needs one {TASKS_BEGIN} line before one {TASKS_END} line, or neither; sltk replaces the text between them"
        )),
    }
}

/// `.claude/settings.json` with the hook's SessionStart entry, or `None`
/// when `existing` already runs [`HOOK_COMMAND`] at SessionStart. A missing
/// file becomes [`SETTINGS_TEMPLATE`]. An existing file keeps its other keys
/// and hooks in their order. When one SessionStart entry already runs
/// [`HOOK`], in any form, such as `${CLAUDE_PROJECT_DIR}/...`, `bash "..."`
/// or with arguments, that entry gets [`HOOK_COMMAND`] in place; otherwise
/// the entry goes at the end of `hooks.SessionStart`. More than one such
/// entry is refused, since the hook would run more than once.
///
/// The new text is serde_json's, not an edit of the old bytes: it is
/// indented with two spaces, a repeated key keeps only its last value, and
/// an integer outside 64 bits comes back as a float. Claude Code reads such
/// a file the same way, so only the bytes change.
pub fn settings(existing: Option<&str>) -> Result<Option<String>, String> {
    let Some(text) = existing else {
        return Ok(Some(SETTINGS_TEMPLATE.to_owned()));
    };
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|err| format!("could not be read as JSON: {err}"))?;
    let template: serde_json::Value = serde_json::from_str(SETTINGS_TEMPLATE).expect("the template is JSON");
    let entry = template["hooks"]["SessionStart"][0].clone();
    let root_map = root.as_object_mut().ok_or("is not a JSON object")?;
    let hooks = root_map
        .entry("hooks")
        .or_insert_with(|| serde_json::Value::Object(Default::default()))
        .as_object_mut()
        .ok_or("hooks is not a JSON object")?;
    let start = hooks
        .entry("SessionStart")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("hooks.SessionStart is not a JSON array")?;
    let mut found: Vec<&mut serde_json::Value> = start
        .iter_mut()
        .filter_map(|group| group.get_mut("hooks").and_then(serde_json::Value::as_array_mut))
        .flatten()
        .filter(|hook| hook["command"].as_str().is_some_and(runs_hook))
        .collect();
    match found.as_mut_slice() {
        [] => {}
        [hook] if hook["command"] == HOOK_COMMAND => return Ok(None),
        [hook] => hook["command"] = serde_json::Value::from(HOOK_COMMAND),
        many => {
            let commands: Vec<String> = many.iter().map(|hook| hook["command"].to_string()).collect();
            return Err(format!(
                "has {} SessionStart entries that run {HOOK}: {}. Delete all but one and rerun sltk init",
                many.len(),
                commands.join(", ")
            ));
        }
    }
    if found.is_empty() {
        start.push(entry);
    }
    let mut out = serde_json::to_string_pretty(&root).expect("a JSON value serializes");
    out.push('\n');
    Ok(Some(out))
}

/// fab's settings with [`FAB_TASKS`] appended, or `None` when there is no
/// file, which `fab init` writes, or the file already has a `tasks` key. A
/// table someone wrote stays as it is. The table goes at the end, where fab
/// renders it, so appending it keeps every other key outside it. Fails when
/// the file is not TOML.
pub fn fab_tasks(existing: Option<&str>) -> Result<Option<String>, String> {
    let Some(text) = existing else {
        return Ok(None);
    };
    let table: toml::Table =
        toml::from_str(text).map_err(|err| format!("could not be read as TOML: {}", err.message()))?;
    if table.contains_key("tasks") {
        return Ok(None);
    }
    let gap = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
    Ok(Some(format!("{text}{gap}{FAB_TASKS}")))
}

/// The byte offsets of the lines outside code fences that are a `## Tasks`
/// heading, ignoring up to three spaces of indentation, closing `#`s and a
/// `\r\n` ending.
fn tasks_headings(lines: &[Line]) -> Vec<usize> {
    lines
        .iter()
        .filter(|line| !line.fenced && heading(line.text) == Some((2, "Tasks")))
        .map(|line| line.start)
        .collect()
}

/// The byte offset where the section whose heading starts at `start` ends:
/// the next line, outside a code fence, that is a heading of level one or
/// two or starts an HTML comment, or `None` at the end of the file. A
/// comment line ends it because another tool's marker, such as
/// `<!-- fabrial:begin -->`, can open the block that follows, and replacing
/// the marker would lose it.
fn section_end(lines: &[Line], start: usize) -> Option<usize> {
    lines
        .iter()
        .filter(|line| line.start > start && !line.fenced)
        .find(|line| {
            line.text.trim_start().starts_with("<!--") || heading(line.text).is_some_and(|(level, _)| level <= 2)
        })
        .map(|line| line.start)
}

/// The byte offsets of the lines outside code fences that are exactly
/// `marker`, ignoring a `\n` or `\r\n` ending.
fn marker_lines(lines: &[Line], marker: &str) -> Vec<usize> {
    lines
        .iter()
        .filter(|line| !line.fenced && trim_ending(line.text) == marker)
        .map(|line| line.start)
        .collect()
}

/// The lines of `text`, each marked as fenced or not, and the number,
/// counting from one, of the line that opens a fence nothing closes. As in
/// CommonMark, a fence opens with three or more backticks or tildes after
/// at most three spaces, and an unclosed fence runs to the end of the file.
fn scan(text: &str) -> (Vec<Line<'_>>, Option<usize>) {
    let mut lines = Vec::new();
    let mut open: Option<(Fence, usize)> = None;
    let mut start = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let fenced = match open {
            Some((fence, _)) => {
                if fence_closes(line, fence) {
                    open = None;
                }
                true
            }
            None => {
                open = fence_opens(line).map(|fence| (fence, i + 1));
                open.is_some()
            }
        };
        lines.push(Line { start, text: line, fenced });
        start += line.len();
    }
    (lines, open.map(|(_, number)| number))
}

/// The fence `line` opens: three or more backticks or tildes after at most
/// three spaces. A backtick fence's info string holds no backtick.
fn fence_opens(line: &str) -> Option<Fence> {
    let rest = indented(trim_ending(line))?;
    let mark = *rest.as_bytes().first()?;
    if mark != b'`' && mark != b'~' {
        return None;
    }
    let len = rest.bytes().take_while(|&b| b == mark).count();
    if len < 3 || (mark == b'`' && rest[len..].contains('`')) {
        return None;
    }
    Some(Fence { mark, len })
}

/// Whether `line` closes `fence`: after at most three spaces, at least as
/// many of its character, then nothing but spaces or tabs.
fn fence_closes(line: &str, fence: Fence) -> bool {
    let Some(rest) = indented(trim_ending(line)) else {
        return false;
    };
    let len = rest.bytes().take_while(|&b| b == fence.mark).count();
    len >= fence.len && rest[len..].bytes().all(|b| b == b' ' || b == b'\t')
}

/// The level and text of the ATX heading on `line`: one to six `#`s after
/// at most three spaces, then a space, a tab or the end of the line. The
/// text has its closing `#`s and the spaces around it removed.
fn heading(line: &str) -> Option<(usize, &str)> {
    let rest = indented(trim_ending(line))?;
    let level = rest.bytes().take_while(|&b| b == b'#').count();
    let rest = &rest[level..];
    if !(1..=6).contains(&level) || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let text = rest.trim();
    let open = text.trim_end_matches('#');
    if open.is_empty() || open.ends_with([' ', '\t']) {
        return Some((level, open.trim_end()));
    }
    Some((level, text))
}

/// Whether `line` is a heading of level one or two that reads as Tasks in
/// any case, alone or before more text, other than `## Tasks` itself.
fn near_tasks(line: &str) -> bool {
    let Some((level, text)) = heading(line) else {
        return false;
    };
    let lower = text.to_lowercase();
    let starts = lower
        .strip_prefix("tasks")
        .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric()));
    level <= 2 && starts && (level, text) != (2, "Tasks")
}

/// Whether a SessionStart command runs [`HOOK`]: the path appears in it,
/// not as part of a longer name.
fn runs_hook(command: &str) -> bool {
    let part = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.');
    command.match_indices(HOOK).any(|(at, _)| {
        let before = command[..at].chars().next_back();
        let after = command[at + HOOK.len()..].chars().next();
        !before.is_some_and(part) && !after.is_some_and(|c| part(c) || c == '/')
    })
}

/// `line` after up to three leading spaces, or `None` when it has four or
/// more, which make it indented code.
fn indented(line: &str) -> Option<&str> {
    let spaces = line.bytes().take_while(|&b| b == b' ').count();
    (spaces <= 3).then(|| &line[spaces..])
}

/// `line` without its `\n` or `\r\n` ending.
fn trim_ending(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// `relative`, one of [`AGENTS`] and [`CLAUDE`], or the other of the two
/// when `relative` is a symlink to it and it is a regular file in `root`.
/// Any other symlink stays as it is, and [`read`] refuses it.
fn follow_link(root: &Path, relative: &'static str) -> &'static str {
    let other = if relative == AGENTS { CLAUDE } else { AGENTS };
    let Ok(target) = std::fs::read_link(root.join(relative)) else {
        return relative;
    };
    let other_path = root.join(other);
    let same = match (root.join(target).canonicalize(), other_path.canonicalize()) {
        (Ok(target), Ok(other)) => target == other,
        _ => false,
    };
    let regular = std::fs::symlink_metadata(&other_path).is_ok_and(|meta| meta.is_file());
    if same && regular { other } else { relative }
}

/// The text of the file at `relative` under `root`, or `None` when there is
/// none. Refuses a symlink and anything but a regular file, in the file and
/// in each directory between `root` and it.
fn read(root: &Path, relative: &str) -> Result<Option<String>, GlueError> {
    let path = root.join(relative);
    let mut dir = root.to_path_buf();
    for part in Path::new(relative).parent().into_iter().flat_map(Path::components) {
        dir.push(part);
        match kind(&dir)? {
            None => return Ok(None),
            Some(meta) if meta.is_dir() => {}
            Some(_) => {
                return Err(GlueError::NotAFile { path: dir, reason: "is not a directory" });
            }
        }
    }
    match kind(&path)? {
        None => return Ok(None),
        Some(meta) if !meta.is_file() => {
            return Err(GlueError::NotAFile { path, reason: "is not a regular file" });
        }
        Some(_) => {}
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|source| GlueError::Io { path, source })
}

/// The metadata of `path` itself, or `None` when there is nothing there.
/// Refuses a symlink.
fn kind(path: &Path) -> Result<Option<std::fs::Metadata>, GlueError> {
    match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(GlueError::Io { path: path.to_owned(), source }),
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(GlueError::NotAFile { path: path.to_owned(), reason: "is a symlink, which sltk does not follow" })
        }
        Ok(meta) => Ok(Some(meta)),
    }
}

/// Writes `text` to a temporary file beside `path`, with `mode` on Unix when
/// given, and renames it over `path`, creating the parent directories.
fn replace(path: &Path, text: &str, mode: Option<u32>) -> Result<(), GlueError> {
    let io = |source| GlueError::Io { path: path.to_owned(), source };
    let dir = path.parent().expect("a glue path has a parent");
    std::fs::create_dir_all(dir).map_err(io)?;
    let name = path.file_name().expect("a glue path has a file name").to_string_lossy();
    let temp = dir.join(format!(".{name}.sltk-{}.tmp", std::process::id()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode.unwrap_or(0o644));
        }
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            // The umask may have cleared bits the mode asks for.
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(io)
}

/// The permission bits of the file at `path`, after symlinks, or `None` off
/// Unix or when it cannot be read.
fn mode_of(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|meta| meta.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Whether `relative` under `root` is a regular file with a [`TASKS_BEGIN`]
/// line. A symlink or anything but a regular file holds none, so
/// [`tasks_file`] falls back to its usual choice and [`read`] refuses it.
fn holds_marker(root: &Path, relative: &str) -> Result<bool, GlueError> {
    let path = root.join(relative);
    match std::fs::symlink_metadata(&path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(GlueError::Io { path, source }),
        Ok(meta) if !meta.is_file() => Ok(false),
        Ok(_) => {
            let bytes = std::fs::read(&path).map_err(|source| GlueError::Io { path, source })?;
            let text = String::from_utf8_lossy(&bytes);
            Ok(!marker_lines(&scan(&text).0, TASKS_BEGIN).is_empty())
        }
    }
}

/// Whether anything, a symlink included, is at `path`.
fn kind_or_link(path: &Path) -> Result<bool, GlueError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(GlueError::Io { path: path.to_owned(), source }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AGENTS, CLAUDE, Existing, FAB_TASKS, HOOK_COMMAND, SETTINGS_TEMPLATE, TASKS, TASKS_BEGIN, TASKS_END, fab_tasks,
        settings, tasks_file, tasks_section, tasks_section_with,
    };

    #[test]
    fn fab_settings_without_tasks_get_arcls_table_at_the_end() {
        let text = "owner = \"octo\"\nplanning = [\".arcl/workspace/\"]\nversion = \"0.9.1\"\n";
        let got = fab_tasks(Some(text)).unwrap().unwrap();
        assert_eq!(got, format!("{text}{FAB_TASKS}"));
        let table: toml::Table = toml::from_str(&got).unwrap();
        assert_eq!(table["tasks"]["start"].as_str(), Some("arcl task start {id}"));
        assert_eq!(table["tasks"]["skill"].as_str(), Some("arclightning"));
        assert_eq!(table["owner"].as_str(), Some("octo"));
        assert_eq!(fab_tasks(Some(&got)).unwrap(), None);
        assert_eq!(
            fab_tasks(Some("version = \"0.9.1\"")).unwrap().unwrap(),
            format!("version = \"0.9.1\"\n{FAB_TASKS}")
        );
    }

    #[test]
    fn fab_settings_with_their_own_tasks_or_none_at_all_are_left_alone() {
        assert_eq!(fab_tasks(None).unwrap(), None);
        let theirs = "owner = \"octo\"\n[tasks]\nshow = \"gh issue view {id}\"\n";
        assert_eq!(fab_tasks(Some(theirs)).unwrap(), None);
        let err = fab_tasks(Some("owner = ")).unwrap_err();
        assert!(err.starts_with("could not be read as TOML"), "{err}");
    }

    /// Settings whose one SessionStart group runs `commands`.
    fn settings_running(commands: &[&str]) -> String {
        let hooks: Vec<serde_json::Value> = commands
            .iter()
            .map(|command| serde_json::json!({"type": "command", "command": command, "timeout": 60}))
            .collect();
        serde_json::json!({"hooks": {"SessionStart": [{"hooks": hooks}]}}).to_string()
    }

    fn section() -> String {
        format!("{TASKS_BEGIN}\n{TASKS}{TASKS_END}\n")
    }

    #[test]
    fn a_missing_agents_file_becomes_the_section() {
        assert_eq!(tasks_section(None).unwrap(), section());
        assert_eq!(tasks_section(Some("")).unwrap(), section());
    }

    #[test]
    fn the_section_is_appended_after_a_blank_line() {
        let want = format!("# Notes\n\n{}", section());
        for text in ["# Notes", "# Notes\n", "# Notes\n\n"] {
            assert_eq!(tasks_section(Some(text)).unwrap(), want, "{text:?}");
        }
    }

    #[test]
    fn the_section_is_replaced_in_place_and_nothing_else_moves() {
        let text = format!("# A\n\n{TASKS_BEGIN}\nold\n{TASKS_END}\n\n## B\ntext `{TASKS_BEGIN}` inline\n");
        let got = tasks_section(Some(&text)).unwrap();
        assert_eq!(
            got,
            format!("# A\n\n{}\n## B\ntext `{TASKS_BEGIN}` inline\n", section())
        );
        assert_eq!(tasks_section(Some(&got)).unwrap(), got);
    }

    #[test]
    fn markers_on_crlf_lines_are_found_and_replaced() {
        let text = format!("# A\r\n\r\n{TASKS_BEGIN}\r\nold\r\n{TASKS_END}\r\n\r\n## B\r\n");
        let got = tasks_section(Some(&text)).unwrap();
        assert_eq!(got, format!("# A\r\n\r\n{}\r\n## B\r\n", section()));
        assert_eq!(tasks_section(Some(&got)).unwrap(), got);
    }

    #[test]
    fn a_lone_or_repeated_marker_is_refused() {
        for text in [
            format!("{TASKS_BEGIN}\n"),
            format!("{TASKS_END}\n"),
            format!("{TASKS_END}\n{TASKS_BEGIN}\n"),
            format!("{TASKS_BEGIN}\n{TASKS_END}\n{TASKS_BEGIN}\n{TASKS_END}\n"),
        ] {
            let err = tasks_section(Some(&text)).unwrap_err();
            assert!(err.starts_with("needs one <!-- sltk:tasks:begin --> line"), "{err}");
        }
    }

    #[test]
    fn a_tasks_heading_outside_the_markers_is_refused() {
        for text in [
            "# A\n\n## Tasks\n\nOther rules.\n".to_owned(),
            "# A\r\n\r\n  ## Tasks ##\r\n".to_owned(),
            format!("## Tasks\n\n{TASKS_BEGIN}\nold\n{TASKS_END}\n"),
            format!("{TASKS_BEGIN}\nold\n{TASKS_END}\n## Tasks\n"),
        ] {
            let err = tasks_section(Some(&text)).unwrap_err();
            assert!(err.starts_with("has a ## Tasks heading outside"), "{text:?}: {err}");
        }
        for text in [
            "# A\n\n## Tasks and bugs\n\n### Tasks\n".to_owned(),
            format!("# A\n\n{TASKS_BEGIN}\n## Tasks\nold\n{TASKS_END}\n"),
        ] {
            assert!(tasks_section(Some(&text)).is_ok(), "{text:?}");
        }
    }

    #[test]
    fn adoption_replaces_an_unmarked_section_up_to_the_next_heading() {
        let replace = |text: &str| tasks_section_with(Some(text), Existing::Replace).unwrap();
        let old = "## Tasks\n\nold\n\n```md\n# not a heading\n## nor this\n```\n\n### Sub\n\nmore\n\n";
        assert_eq!(
            replace(&format!("# A\n\n{old}## B\n\nkept\n")),
            format!("# A\n\n{}\n## B\n\nkept\n", section())
        );
        assert_eq!(
            replace(&format!("# A\n\n{old}# C\n")),
            format!("# A\n\n{}\n# C\n", section())
        );
        assert_eq!(replace(&format!("# A\n\n{old}")), format!("# A\n\n{}", section()));
        let got = replace(&format!("# A\n\n{old}## B\n"));
        assert_eq!(replace(&got), got);
        // Fresh setup still refuses the same file.
        assert!(tasks_section(Some(&format!("# A\n\n{old}"))).is_err());
    }

    #[test]
    fn adoption_stops_the_unmarked_section_at_an_html_comment() {
        let replace = |text: &str| tasks_section_with(Some(text), Existing::Replace).unwrap();
        // Shaped like stack's own AGENTS.md: fab's block follows Tasks with
        // its begin marker before its first heading.
        let fab = "<!-- fabrial:begin -->\n## Branches and pull requests\n\nfab's\n<!-- fabrial:end -->\n";
        let text = format!("# AGENTS.md\n\n## Tasks\n\nold\n\n{fab}");
        let got = replace(&text);
        assert_eq!(got, format!("# AGENTS.md\n\n{}\n{fab}", section()));
        assert_eq!(got.matches("<!-- fabrial:begin -->").count(), 1);
        assert_eq!(replace(&got), got);
        // Any comment line ends it, indented or not, but not one in a fence.
        let text = "## Tasks\n\n```md\n<!-- in a fence -->\n```\n  <!-- note -->\nkept\n";
        assert_eq!(replace(text), format!("{}\n  <!-- note -->\nkept\n", section()));
    }

    #[test]
    fn adoption_refuses_two_unmarked_sections_or_one_beside_the_markers() {
        let replace = |text: &str| tasks_section_with(Some(text), Existing::Replace);
        let err = replace("## Tasks\n\na\n\n## Tasks\n\nb\n").unwrap_err();
        assert!(err.starts_with("has 2 ## Tasks headings"), "{err}");
        let err = replace(&format!("## Tasks\n\n{TASKS_BEGIN}\nold\n{TASKS_END}\n")).unwrap_err();
        assert!(err.starts_with("has a ## Tasks heading outside"), "{err}");
    }

    #[test]
    fn any_entry_for_the_hook_is_replaced_in_place() {
        for command in [
            "$CLAUDE_PROJECT_DIR/.claude/hooks/session-start.sh",
            "${CLAUDE_PROJECT_DIR}/.claude/hooks/session-start.sh",
            "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/session-start.sh\"",
            "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/session-start.sh --quiet",
            ".claude/hooks/session-start.sh",
        ] {
            let got = settings(Some(&settings_running(&["a.sh", command]))).unwrap().unwrap();
            let value: serde_json::Value = serde_json::from_str(&got).unwrap();
            let start = value["hooks"]["SessionStart"].as_array().unwrap();
            assert_eq!(start.len(), 1, "{command}");
            assert_eq!(start[0]["hooks"].as_array().unwrap().len(), 2, "{command}");
            assert_eq!(start[0]["hooks"][0]["command"], "a.sh");
            assert_eq!(start[0]["hooks"][1]["command"], HOOK_COMMAND, "{command}");
            assert_eq!(start[0]["hooks"][1]["timeout"], 60, "{command}");
            assert_eq!(settings(Some(&got)).unwrap(), None, "{command}");
        }
    }

    #[test]
    fn a_command_naming_another_file_is_not_the_hook() {
        for command in [
            "$CLAUDE_PROJECT_DIR/.claude/hooks/session-start.sh.bak",
            "$CLAUDE_PROJECT_DIR/x.claude/hooks/session-start.sh",
            "$CLAUDE_PROJECT_DIR/.claude/hooks/session-start.sh/run",
        ] {
            let got = settings(Some(&settings_running(&[command]))).unwrap().unwrap();
            let value: serde_json::Value = serde_json::from_str(&got).unwrap();
            let start = value["hooks"]["SessionStart"].as_array().unwrap();
            assert_eq!(start.len(), 2, "{command}");
            assert_eq!(start[0]["hooks"][0]["command"], command);
            assert_eq!(start[1]["hooks"][0]["command"], HOOK_COMMAND);
        }
    }

    #[test]
    fn two_entries_for_the_hook_are_refused_and_named() {
        let text = settings_running(&[HOOK_COMMAND, "bash .claude/hooks/session-start.sh"]);
        let err = settings(Some(&text)).unwrap_err();
        assert_eq!(
            err,
            "has 2 SessionStart entries that run .claude/hooks/session-start.sh: \
             \"\\\"$CLAUDE_PROJECT_DIR\\\"/.claude/hooks/session-start.sh\", \
             \"bash .claude/hooks/session-start.sh\". Delete all but one and rerun sltk init"
        );
    }

    #[test]
    fn a_fence_closes_only_on_its_own_character_at_its_length_or_longer() {
        let replace = |text: &str| tasks_section_with(Some(text), Existing::Replace);
        let after = "<!-- fabrial:begin -->\n## B\n<!-- fabrial:end -->\n";
        // A tilde line inside a backtick fence, a shorter backtick line, and
        // a backtick line with text after it close nothing; the longer
        // closing fence does.
        let body = "````md\n~~~\n## not a heading\n```\n```` not closed\n## nor this\n`````  \n\n";
        let got = replace(&format!("## Tasks\n\n{body}{after}")).unwrap();
        assert_eq!(got, format!("{}\n{after}", section()));
        let body = "~~~\n```\n<!-- in a fence -->\n~~~\n\n";
        let got = replace(&format!("## Tasks\n\n{body}{after}")).unwrap();
        assert_eq!(got, format!("{}\n{after}", section()));
    }

    #[test]
    fn an_unclosed_fence_is_refused() {
        for policy in [Existing::Refuse, Existing::Replace] {
            let text = "# A\n\n## Tasks\n\n```sh\nold\n~~~\n## B\n";
            let err = tasks_section_with(Some(text), policy).unwrap_err();
            assert_eq!(
                err,
                "opens a code fence on line 5 that nothing closes, so the rest of the file reads as code. \
                 Close the fence and rerun sltk init"
            );
        }
    }

    #[test]
    fn a_tasks_heading_or_marker_inside_a_fence_is_code() {
        let text = format!("# A\n\nExample:\n\n```md\n## Tasks\n{TASKS_BEGIN}\n```\n\n## B\n");
        let want = format!("{text}\n{}", section());
        assert_eq!(tasks_section(Some(&text)).unwrap(), want);
        assert_eq!(tasks_section_with(Some(&text), Existing::Replace).unwrap(), want);
        // Four spaces make indented code, not a heading.
        let text = "# A\n\n    ## Tasks\n";
        assert!(tasks_section(Some(text)).is_ok());
    }

    #[test]
    fn adoption_refuses_a_tasks_heading_that_is_not_exactly_level_two() {
        for (text, line) in [
            ("# A\n\n## Tasks (arcl)\n\nold\n", "line 3 (## Tasks (arcl))"),
            ("# A\n\n## tasks\n", "line 3 (## tasks)"),
            ("# Tasks\n", "line 1 (# Tasks)"),
            ("# A\n\n## TASKS ##\n", "line 3 (## TASKS ##)"),
            ("## Tasks\n\n## Tasks and bugs\n", "line 3 (## Tasks and bugs)"),
        ] {
            let err = tasks_section_with(Some(text), Existing::Replace).unwrap_err();
            assert_eq!(
                err,
                format!(
                    "has a Tasks heading that is not ## Tasks at {line}, so sltk would add a second Tasks \
                     section. Rename it to ## Tasks for sltk to replace, or give it another name"
                ),
                "{text:?}"
            );
        }
        let marked = format!("# A\n\n{TASKS_BEGIN}\n## Tasks (old)\n{TASKS_END}\n");
        for text in [
            "# A\n\n### Tasks\n",
            "# A\n\n## Taskset\n",
            "# A\n\n```\n## tasks\n```\n",
            marked.as_str(),
        ] {
            assert!(tasks_section_with(Some(text), Existing::Replace).is_ok(), "{text:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_between_agents_and_claude_is_followed_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(CLAUDE), "# A\n").unwrap();
        std::os::unix::fs::symlink(CLAUDE, root.join(AGENTS)).unwrap();
        assert_eq!(tasks_file(root).unwrap(), CLAUDE);
        std::fs::remove_file(root.join(AGENTS)).unwrap();
        std::fs::rename(root.join(CLAUDE), root.join(AGENTS)).unwrap();
        std::os::unix::fs::symlink(root.join(AGENTS), root.join(CLAUDE)).unwrap();
        assert_eq!(tasks_file(root).unwrap(), AGENTS);
        // A link to any other file stays, for the reader to refuse.
        std::fs::remove_file(root.join(CLAUDE)).unwrap();
        std::fs::rename(root.join(AGENTS), root.join("NOTES.md")).unwrap();
        std::os::unix::fs::symlink("NOTES.md", root.join(AGENTS)).unwrap();
        assert_eq!(tasks_file(root).unwrap(), AGENTS);
    }

    #[test]
    fn the_section_names_no_tool_version() {
        assert!(!TASKS.contains("0.3.0"));
        assert!(TASKS.starts_with("## Tasks\n"));
    }

    #[test]
    fn missing_settings_become_the_template() {
        assert_eq!(settings(None).unwrap().as_deref(), Some(SETTINGS_TEMPLATE));
        let value: serde_json::Value = serde_json::from_str(SETTINGS_TEMPLATE).unwrap();
        assert_eq!(value["hooks"]["SessionStart"][0]["hooks"][0]["command"], HOOK_COMMAND);
    }

    #[test]
    fn the_entry_is_added_once_and_other_keys_and_hooks_stay() {
        let text = r#"{"permissions": {"allow": ["Bash(ls)"]}, "hooks": {"Stop": [], "SessionStart": [{"hooks": [{"type": "command", "command": "other.sh"}]}]}}"#;
        let got = settings(Some(text)).unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&got).unwrap();
        assert_eq!(value["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(value["hooks"]["Stop"], serde_json::json!([]));
        let start = value["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 2);
        assert_eq!(start[0]["hooks"][0]["command"], "other.sh");
        assert_eq!(start[1]["hooks"][0]["command"], HOOK_COMMAND);
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["permissions", "hooks"]);
        assert!(got.ends_with("}\n"));
        assert_eq!(settings(Some(&got)).unwrap(), None);
        assert_eq!(settings(Some(SETTINGS_TEMPLATE)).unwrap(), None);
    }

    #[test]
    fn settings_that_cannot_take_the_entry_are_refused() {
        assert!(settings(Some("[]")).unwrap_err().contains("not a JSON object"));
        assert!(settings(Some(r#"{"hooks": []}"#)).unwrap_err().contains("hooks is not"));
        assert!(
            settings(Some(r#"{"hooks": {"SessionStart": {}}}"#))
                .unwrap_err()
                .contains("hooks.SessionStart is not")
        );
        assert!(
            settings(Some("{"))
                .unwrap_err()
                .starts_with("could not be read as JSON")
        );
    }
}
