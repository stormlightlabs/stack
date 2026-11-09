//! The git host a repository lives on, as `sltk doctor` asks about it: whether
//! an App is installed, whether a secret is available to the repository, and
//! whether a check is required on a branch.
//!
//! [`Host`] is the interface and [`GitHub`] its one implementation, which asks
//! through `gh api`. Tests use a fake.
//!
//! When `origin` is a fork, the questions are about its parent, since the
//! workflows that need the App, the secrets and the required check run there.

use std::cell::OnceCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::process::run_capped;
use crate::setup::find_program;

/// How long one `gh api` or `git` call may run before doctor stops it.
pub const HOST_TIMEOUT: Duration = Duration::from_secs(20);

/// How long all the calls for one repository may run together. A call still
/// running at this point is stopped, and the calls after it are not made.
pub const HOST_DEADLINE: Duration = Duration::from_secs(30);

/// The most bytes doctor reads from one `gh api` or `git` call.
const MAX_HOST_OUTPUT: u64 = 256 * 1024;

/// The longest error line from `gh` that doctor repeats.
const MAX_REASON: usize = 200;

/// What a host says to one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// It is so.
    Yes,
    /// It is not so.
    No,
    /// It is so, but suspended, and how: an App installation that an owner
    /// or GitHub suspended cannot act, so doctor counts it as a no.
    Suspended(String),
    /// The host could not say, and why, as when the token may not list
    /// secrets.
    Unknown(String),
}

/// How a `gh api` call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ApiError {
    /// GitHub answered 404, with its message.
    NotFound(String),
    /// Anything else, with what `gh` printed or why it could not run.
    Other(String),
}

impl ApiError {
    /// The reason to report.
    fn reason(&self) -> &str {
        match self {
            ApiError::NotFound(reason) | ApiError::Other(reason) => reason,
        }
    }
}

/// A repository on GitHub, asked about through `gh api`.
#[derive(Debug, Clone)]
pub struct GitHub {
    /// The `gh` binary, from an absolute `PATH` entry.
    gh: PathBuf,
    /// The repository `origin` names, as "owner/repo".
    origin: String,
    /// The repository asked about: `origin`'s, or its parent when it is a
    /// fork. Else why neither could be read, which every answer repeats.
    target: Result<Target, String>,
    /// How long each `gh api` call may run.
    timeout: Duration,
    /// How long all the calls may run together, from [`GitHub::detect`].
    total: Duration,
    /// When the calls must stop: `total` after [`GitHub::detect`] started.
    deadline: Instant,
    /// The secret names the repository and organization endpoints list,
    /// read once for every secret asked about.
    secrets: OnceCell<[Result<Vec<String>, String>; 2]>,
}

impl GitHub {
    /// The GitHub repository the remote `origin` of the git repository at
    /// `root` names, or its parent when it is a fork, asked about through the
    /// `gh` on `path`. `gh` and `git` are found in absolute `PATH` entries
    /// only. Each call may run for `timeout`, and all of them, this one's
    /// `git` and `gh api` included, for `total`. The error is why doctor
    /// skips the repository checks: `gh` or `git` is not on `path`, there is
    /// no `origin`, or `origin` is not on GitHub.
    pub fn detect(root: &Path, path: Option<&OsStr>, timeout: Duration, total: Duration) -> Result<GitHub, String> {
        let deadline = Instant::now() + total;
        let gh = find_program("gh", path).ok_or("gh not found")?;
        let git = find_program("git", path).ok_or("git not found")?;
        let mut command = Command::new(git);
        command.args(["remote", "get-url", "origin"]).current_dir(root);
        if let Some(path) = path {
            command.env("PATH", path);
        }
        let output = run_capped(&mut command, timeout.min(total), MAX_HOST_OUTPUT)
            .map_err(|err| format!("could not read the remote origin: git: {err}"))?;
        if !output.status.success() {
            return Err("no remote named origin".to_owned());
        }
        let (owner, repo) = parse_remote(output.stdout.trim()).ok_or("origin is not a GitHub repository")?;
        let mut github = GitHub {
            gh,
            origin: format!("{owner}/{repo}"),
            target: Err(String::new()),
            timeout,
            total,
            deadline,
            secrets: OnceCell::new(),
        };
        github.target = github.resolve();
        Ok(github)
    }

    /// Reads `origin`'s repository: itself when it is not a fork, else its
    /// parent, or why GitHub did not say.
    fn resolve(&self) -> Result<Target, String> {
        let origin = &self.origin;
        let lines = self
            .api(&format!("repos/{origin}"), r#""\(.fork) \(.parent.full_name // "")""#)
            .map_err(|err| err.reason().to_owned())?;
        let (full_name, fork) = match lines.first().and_then(|line| line.split_once(' ')) {
            Some(("false", _)) => (origin.as_str(), false),
            Some(("true", parent)) => (parent, true),
            _ => return Err(format!("GitHub did not say whether {origin} is a fork")),
        };
        let (owner, repo) = split_full_name(full_name)
            .ok_or_else(|| format!("{origin} is a fork, and GitHub did not name its parent"))?;
        Ok(Target { owner, repo, fork })
    }

    /// The repository asked about, or the answer every question gets when it
    /// is not known.
    fn target(&self) -> Result<&Target, Answer> {
        self.target.as_ref().map_err(|reason| Answer::Unknown(reason.clone()))
    }

    /// Runs `gh api` on `endpoint`, every page, filtered by `jq`, and returns
    /// the lines it printed.
    fn api(&self, endpoint: &str, jq: &str) -> Result<Vec<String>, ApiError> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ApiError::Other(format!(
                "not asked, since the repository checks ran past {} s",
                self.total.as_secs_f32()
            )));
        }
        let mut command = Command::new(&self.gh);
        command
            .args(["api", "--hostname", "github.com", "--paginate", "--jq", jq, endpoint])
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1");
        let output = run_capped(&mut command, self.timeout.min(left), MAX_HOST_OUTPUT)
            .map_err(|err| ApiError::Other(format!("gh: {err}")))?;
        if output.status.success() {
            return Ok(output.stdout.lines().map(str::to_owned).collect());
        }
        let line = output
            .stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(clean)
            .unwrap_or_else(|| format!("gh exited with {}", output.status));
        if line.ends_with("(HTTP 404)") {
            Err(ApiError::NotFound(line))
        } else {
            Err(ApiError::Other(line))
        }
    }

    /// The secret names `endpoint` lists, or why it could not list them.
    fn secret_names(&self, endpoint: &str) -> Result<Vec<String>, String> {
        self.api(endpoint, ".secrets[].name")
            .map_err(|err| err.reason().to_owned())
    }

    /// The repository's own secrets and the organization secrets shared
    /// with `target`, listed on the first call and kept for the rest.
    fn secret_lists(&self, target: &Target) -> &[Result<Vec<String>, String>; 2] {
        self.secrets.get_or_init(|| {
            [
                self.secret_names(&format!("repos/{target}/actions/secrets")),
                self.secret_names(&format!("repos/{target}/actions/organization-secrets")),
            ]
        })
    }
}

impl Host for GitHub {
    fn repository(&self) -> String {
        match &self.target {
            Ok(target) if target.fork => format!("{target} on GitHub, the parent of the fork {}", self.origin),
            _ => format!("{} on GitHub", self.origin),
        }
    }

    /// Lists the organization's installations, which a token with
    /// `read:org` may see when its user is an organization owner. GitHub
    /// shows a user account's installations, and the repositories an
    /// installation on selected repositories covers, only to the App, so
    /// those are [`Answer::Unknown`]. A suspended installation is
    /// [`Answer::Suspended`], whatever repositories it covers.
    fn app_installed(&self, app: &str) -> Answer {
        let target = match self.target() {
            Ok(target) => target,
            Err(answer) => return answer,
        };
        let owner = &target.owner;
        match self.api(
            &format!("orgs/{owner}/installations"),
            r#".installations[] | "\(.app_slug) \(.repository_selection) \(.suspended_at // "")""#,
        ) {
            Ok(lines) => {
                let installation = lines.iter().find_map(|line| {
                    let mut parts = line.splitn(3, ' ');
                    let slug = parts.next()?;
                    (slug == app).then(|| (parts.next().unwrap_or(""), parts.next().unwrap_or("")))
                });
                match installation {
                    Some((_, suspended)) if !suspended.is_empty() => Answer::Suspended(format!(
                        "its installation on {owner} was suspended at {}",
                        clean(suspended)
                    )),
                    Some(("all", _)) => Answer::Yes,
                    Some(_) => Answer::Unknown(format!(
                        "it is installed on selected repositories of {owner}, and GitHub shows which only to the App"
                    )),
                    None => Answer::No,
                }
            }
            Err(ApiError::NotFound(reason)) => Answer::Unknown(format!(
                "{reason}; GitHub lists installations only for an organization, to its owners"
            )),
            Err(ApiError::Other(reason)) => Answer::Unknown(reason),
        }
    }

    /// Looks in the repository's own secrets and in the organization secrets
    /// shared with it, which are listed once for all the secrets asked about.
    /// Listing either needs admin access to the repository.
    fn secret_available(&self, name: &str) -> Answer {
        match self.target() {
            Ok(target) => find_in(self.secret_lists(target).clone(), name),
            Err(answer) => answer,
        }
    }

    /// Looks in the rulesets that apply to `branch` and in its classic branch
    /// protection. A 404 saying the branch is not protected, or has no
    /// required checks, means classic protection requires nothing.
    fn check_required(&self, branch: &str, check: &str) -> Answer {
        let target = match self.target() {
            Ok(target) => target,
            Err(answer) => return answer,
        };
        let rules = self
            .api(
                &format!("repos/{target}/rules/branches/{branch}"),
                r#".[] | select(.type == "required_status_checks") | .parameters.required_status_checks[].context"#,
            )
            .map_err(|err| err.reason().to_owned());
        let classic = match self.api(
            &format!("repos/{target}/branches/{branch}/protection/required_status_checks"),
            "(.contexts // [])[], ((.checks // [])[] | .context)",
        ) {
            Ok(contexts) => Ok(contexts),
            Err(ApiError::NotFound(reason)) if reason.contains("not protected") || reason.contains("not enabled") => {
                Ok(Vec::new())
            }
            Err(err) => Err(err.reason().to_owned()),
        };
        find_in([rules, classic], check)
    }
}

/// A repository on GitHub that [`GitHub`] asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    /// The account that owns the repository.
    owner: String,
    /// The repository's name, without `.git`.
    repo: String,
    /// Whether this is the parent of the fork `origin` names.
    fork: bool,
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

/// The questions `sltk doctor` asks a repository's host. Each answer is
/// [`Answer::Unknown`] when the host could not say, as when the token lacks a
/// permission, so a missing permission is not reported as a missing setting.
pub trait Host {
    /// The repository and its host, as "owner/repo on GitHub".
    fn repository(&self) -> String;
    /// Whether the App with this slug is installed for the repository.
    fn app_installed(&self, app: &str) -> Answer;
    /// Whether the repository's workflows can read the secret `name`.
    fn secret_available(&self, name: &str) -> Answer;
    /// Whether `check` must pass before a change merges into `branch`.
    fn check_required(&self, branch: &str, check: &str) -> Answer;
}

/// The owner and name of the GitHub repository a git remote URL names:
/// `https://github.com/owner/repo`, `git@github.com:owner/repo` or
/// `ssh://git@github.com/owner/repo`, each with an optional `.git`, and a
/// URL with a scheme may give a port after the host. Any other host, or a
/// path that is not two parts, gives `None`.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let (rest, sep) = match url.split_once("://") {
        Some((scheme, rest)) if ["https", "http", "ssh", "git"].contains(&scheme) => (rest, '/'),
        Some(_) => return None,
        None => (url, ':'),
    };
    let (authority, path) = rest.split_once(sep)?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = match host.rsplit_once(':') {
        Some((name, port)) if sep == '/' && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let path = path.trim_end_matches('/');
    split_full_name(path.strip_suffix(".git").unwrap_or(path))
}

/// The owner and name in `owner/repo`, each made of the characters GitHub
/// allows in a name, or `None`.
fn split_full_name(full_name: &str) -> Option<(String, String)> {
    let (owner, repo) = full_name.split_once('/')?;
    let name = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (name(owner) && name(repo)).then(|| (owner.to_owned(), repo.to_owned()))
}

/// Whether any list in `lists` holds `want`: [`Answer::Yes`] if one does,
/// else [`Answer::No`] when every list was read, else [`Answer::Unknown`] with
/// each distinct reason a list could not be read.
fn find_in(lists: [Result<Vec<String>, String>; 2], want: &str) -> Answer {
    let mut reasons: Vec<String> = Vec::new();
    for list in lists {
        match list {
            Ok(items) if items.iter().any(|item| item == want) => return Answer::Yes,
            Ok(_) => {}
            Err(reason) if !reasons.contains(&reason) => reasons.push(reason),
            Err(_) => {}
        }
    }
    if reasons.is_empty() { Answer::No } else { Answer::Unknown(reasons.join("; ")) }
}

/// `line` without control characters, cut to [`MAX_REASON`] characters, so
/// what `gh` relays from the network cannot move the terminal's cursor.
fn clean(line: &str) -> String {
    let line: String = line.chars().filter(|c| !c.is_control()).collect();
    match line.char_indices().nth(MAX_REASON) {
        Some((end, _)) => format!("{}...", &line[..end]),
        None => line,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;

    use super::{Answer, GitHub, HOST_DEADLINE, HOST_TIMEOUT, find_in, parse_remote};

    #[test]
    fn a_remote_url_names_a_github_repository_or_none() {
        let pair = |owner: &str, repo: &str| Some((owner.to_owned(), repo.to_owned()));
        for url in [
            "https://github.com/stormlightlabs/stack",
            "https://github.com/stormlightlabs/stack.git",
            "https://github.com/stormlightlabs/stack/",
            "https://token@github.com/stormlightlabs/stack.git",
            "git@github.com:stormlightlabs/stack.git",
            "git@github.com:stormlightlabs/stack",
            "ssh://git@github.com/stormlightlabs/stack.git",
            "https://GitHub.com/stormlightlabs/stack",
            "ssh://git@github.com:22/stormlightlabs/stack.git",
            "https://github.com:443/stormlightlabs/stack",
        ] {
            assert_eq!(parse_remote(url), pair("stormlightlabs", "stack"), "{url}");
        }
        assert_eq!(parse_remote("git@github.com:a/b.c.git"), pair("a", "b.c"));
        for url in [
            "https://gitlab.com/stormlightlabs/stack",
            "https://github.com.evil.test/stormlightlabs/stack",
            "https://github.com:/stormlightlabs/stack",
            "https://github.com:x22/stormlightlabs/stack",
            "ssh://git@github.com:stormlightlabs/stack",
            "https://github.com/stormlightlabs",
            "https://github.com/stormlightlabs/stack/tree/main",
            "https://github.com//stack",
            "https://github.com/../stack",
            "file:///github.com/stormlightlabs/stack",
            "/srv/git/stack.git",
            "github.com/stormlightlabs/stack",
            "",
        ] {
            assert_eq!(parse_remote(url), None, "{url}");
        }
    }

    #[test]
    fn without_gh_on_path_the_repository_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        // A PATH whose one directory holds no gh, and no PATH at all.
        let path = OsString::from(dir.path());
        let err = GitHub::detect(dir.path(), Some(&path), HOST_TIMEOUT, HOST_DEADLINE).unwrap_err();
        assert_eq!(err, "gh not found");
        assert_eq!(
            GitHub::detect(dir.path(), None, HOST_TIMEOUT, HOST_DEADLINE).unwrap_err(),
            "gh not found"
        );
    }

    #[test]
    fn a_name_in_either_list_is_found_and_an_unread_list_is_unknown() {
        let list = |items: &[&str]| Ok(items.iter().map(|s| s.to_string()).collect());
        let denied = || Err("HTTP 403".to_owned());
        assert_eq!(find_in([list(&["A"]), list(&["B"])], "B"), Answer::Yes);
        assert_eq!(find_in([denied(), list(&["B"])], "B"), Answer::Yes);
        assert_eq!(find_in([list(&["A"]), list(&[])], "B"), Answer::No);
        assert_eq!(
            find_in([list(&["A"]), denied()], "B"),
            Answer::Unknown("HTTP 403".into())
        );
        assert_eq!(find_in([denied(), denied()], "B"), Answer::Unknown("HTTP 403".into()));
        assert_eq!(
            find_in([denied(), Err("timed out".into())], "B"),
            Answer::Unknown("HTTP 403; timed out".into())
        );
    }
}
