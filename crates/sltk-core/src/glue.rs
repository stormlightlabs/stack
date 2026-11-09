//! The glue `sltk init` writes itself: the Tasks section in AGENTS.md, the
//! session hook, and the hook's entry in `.claude/settings.json`.
//!
//! [`plan`] reads the files and works out each change; [`apply`] writes
//! them. Each writer leaves a file alone when it already holds what it would
//! write, so a second run changes no byte. None follows a symlink, whether
//! it is the file or a directory between the repository root and the file,
//! so every write lands inside the repository. Every file is checked before
//! any is written, so a refusal changes nothing.

use std::io::Write;
use std::path::{Path, PathBuf};

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

/// The command a hand-copied SessionStart entry runs for the same hook,
/// without the quotes. The settings writer rewrites it to [`HOOK_COMMAND`]
/// rather than add a second entry that would run the hook twice.
pub const UNQUOTED_HOOK_COMMAND: &str = "$CLAUDE_PROJECT_DIR/.claude/hooks/session-start.sh";

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

/// Writes the Tasks section, the hook and its settings entry under `root`,
/// and returns the paths, relative to `root`, of the files it changed.
///
/// Refuses a hook that differs from [`HOOK_SCRIPT`] rather than replace it.
/// It checks all three files before writing any.
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

/// The file the Tasks section goes in: [`AGENTS`], or [`CLAUDE`] when the
/// repository has that and no AGENTS.md.
pub fn tasks_file(root: &Path) -> Result<&'static str, GlueError> {
    if kind_or_link(&root.join(AGENTS))? {
        return Ok(AGENTS);
    }
    if kind_or_link(&root.join(CLAUDE))? {
        return Ok(CLAUDE);
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
/// the heading line up to the next heading of level one or two outside a
/// code fence, or the end of the file.
pub fn tasks_section_with(existing: Option<&str>, policy: Existing) -> Result<String, String> {
    let section = format!("{TASKS_BEGIN}\n{TASKS}{TASKS_END}\n");
    let Some(text) = existing.filter(|text| !text.is_empty()) else {
        return Ok(section);
    };
    let begins = marker_lines(text, TASKS_BEGIN);
    let ends = marker_lines(text, TASKS_END);
    let headings = tasks_headings(text);
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
            let end = section_end(text, start);
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
/// and hooks in their order and gains the entry at the end of
/// `hooks.SessionStart`, unless an entry there runs
/// [`UNQUOTED_HOOK_COMMAND`]: that entry gets [`HOOK_COMMAND`] in place.
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
    let mut rewrote = false;
    for hook in start
        .iter_mut()
        .filter_map(|group| group.get_mut("hooks").and_then(serde_json::Value::as_array_mut))
        .flatten()
    {
        if hook["command"] == UNQUOTED_HOOK_COMMAND {
            hook["command"] = serde_json::Value::from(HOOK_COMMAND);
            rewrote = true;
        }
    }
    let runs_hook = start.iter().any(|group| {
        group["hooks"]
            .as_array()
            .is_some_and(|list| list.iter().any(|hook| hook["command"] == HOOK_COMMAND))
    });
    if runs_hook && !rewrote {
        return Ok(None);
    }
    if !runs_hook {
        start.push(entry);
    }
    let mut out = serde_json::to_string_pretty(&root).expect("a JSON value serializes");
    out.push('\n');
    Ok(Some(out))
}

/// The byte offsets of the lines in `text` that are a `## Tasks` heading,
/// ignoring indentation, closing `#`s and a `\r\n` ending.
fn tasks_headings(text: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let heading = line
            .trim()
            .strip_prefix("## ")
            .map(|rest| rest.trim_end_matches('#').trim());
        if heading == Some("Tasks") {
            offsets.push(start);
        }
        start += line.len();
    }
    offsets
}

/// The byte offset where the section whose heading starts at `start` ends:
/// the next line, outside a code fence, that is a heading of level one or
/// two, or the end of `text`.
fn section_end(text: &str, start: usize) -> usize {
    let mut at = start;
    let mut fenced = false;
    for (i, line) in text[start..].split_inclusive('\n').enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        } else if i > 0 && !fenced {
            let level = trimmed.bytes().take_while(|&b| b == b'#').count();
            let rest = &trimmed[level..];
            if (level == 1 || level == 2) && (rest.is_empty() || rest.starts_with(char::is_whitespace)) {
                return at;
            }
        }
        at += line.len();
    }
    text.len()
}

/// The byte offsets of the lines in `text` that are exactly `marker`, ignoring
/// a `\n` or `\r\n` ending.
fn marker_lines(text: &str, marker: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let line_text = line.strip_suffix('\n').unwrap_or(line);
        if line_text.strip_suffix('\r').unwrap_or(line_text) == marker {
            offsets.push(start);
        }
        start += line.len();
    }
    offsets
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
        Existing, HOOK_COMMAND, SETTINGS_TEMPLATE, TASKS, TASKS_BEGIN, TASKS_END, UNQUOTED_HOOK_COMMAND, settings,
        tasks_section, tasks_section_with,
    };

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
    fn adoption_refuses_two_unmarked_sections_or_one_beside_the_markers() {
        let replace = |text: &str| tasks_section_with(Some(text), Existing::Replace);
        let err = replace("## Tasks\n\na\n\n## Tasks\n\nb\n").unwrap_err();
        assert!(err.starts_with("has 2 ## Tasks headings"), "{err}");
        let err = replace(&format!("## Tasks\n\n{TASKS_BEGIN}\nold\n{TASKS_END}\n")).unwrap_err();
        assert!(err.starts_with("has a ## Tasks heading outside"), "{err}");
    }

    #[test]
    fn an_unquoted_entry_for_the_hook_is_quoted_in_place() {
        let text = format!(
            r#"{{"hooks": {{"SessionStart": [{{"hooks": [{{"type": "command", "command": "a.sh"}}, {{"type": "command", "command": "{}"}}]}}]}}}}"#,
            UNQUOTED_HOOK_COMMAND
        );
        let got = settings(Some(&text)).unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&got).unwrap();
        let start = value["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 1);
        assert_eq!(start[0]["hooks"][0]["command"], "a.sh");
        assert_eq!(start[0]["hooks"][1]["command"], HOOK_COMMAND);
        assert_eq!(settings(Some(&got)).unwrap(), None);
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
