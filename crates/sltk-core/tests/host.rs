//! Asks [`GitHub`] about a repository through a fake `gh` on `PATH`, as
//! `tests/doctor.rs` in sltk-cli fakes fab. The fake `git` names
//! `github.com/o/r` as the remote origin. The fake `gh` takes only the
//! arguments [`GitHub`] gives a real one, and answers each endpoint the way
//! gh does: it runs the `--jq` filter with `jq` over each recorded JSON page,
//! every page with `--paginate` and the first without, or prints a set error
//! line and exits with a set code. It records every endpoint it is asked for.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sltk_core::host::{Answer, GitHub, HOST_DEADLINE, HOST_TIMEOUT, Host};

const REPO: &str = "repos/o/r";
const INSTALLATIONS: &str = "orgs/o/installations";
const SECRETS: &str = "repos/o/r/actions/secrets";
const ORG_SECRETS: &str = "repos/o/r/actions/organization-secrets";
const RULES: &str = "repos/o/r/rules/branches/main";
const CLASSIC: &str = "repos/o/r/branches/main/protection/required_status_checks";

/// What `repos/o/r` gives for a repository that is not a fork.
const NOT_A_FORK: &str = r#"{"full_name": "o/r", "fork": false, "owner": {"login": "o"}}"#;

/// What gh prints when a branch has no classic protection.
const NOT_PROTECTED: &str = "gh: Branch not protected (HTTP 404)";

/// What gh prints for a token that may not read an endpoint.
const FORBIDDEN: &str = "gh: Resource not accessible by integration (HTTP 403)";

/// What gh prints when it has no login.
const NO_LOGIN: &str = "To get started with GitHub CLI, please run:  gh auth login";

/// How the fake `gh` answers one endpoint.
#[derive(Clone)]
enum Reply {
    /// Exits 0 printing what the `--jq` filter gives for each of these JSON
    /// pages, as GitHub returns them.
    Pages(&'static [&'static str]),
    /// Exits with this code, printing this line to stderr.
    Fails(i32, &'static str),
    /// Runs for this many seconds, then exits 0 printing nothing.
    Hangs(u32),
}

/// A temporary directory holding the fake `git` and `gh`, the replies, and
/// the log of endpoints asked for.
struct FakeGh {
    dir: tempfile::TempDir,
}

impl FakeGh {
    /// A fake `gh` that answers each endpoint in `replies` and fails the
    /// test's call on any other, or on arguments a real call would not have.
    fn new(replies: &[(&str, Reply)]) -> FakeGh {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_script(&root.join("git"), "#!/bin/sh\necho https://github.com/o/r.git\n");
        let mut cases = String::new();
        for (n, (endpoint, reply)) in replies.iter().enumerate() {
            let body = match reply {
                Reply::Pages(pages) => {
                    for (page, json) in pages.iter().enumerate() {
                        std::fs::write(root.join(format!("{n}.{page}.json")), json).unwrap();
                    }
                    format!(
                        "for page in '{root}'/{n}.*.json; do [ -e \"$page\" ] || break; '{jq}' -r \"$jq\" \"$page\" || exit 1; [ -n \"$paginate\" ] || break; done; exit 0",
                        root = root.display(),
                        jq = jq().display(),
                    )
                }
                Reply::Fails(code, line) => {
                    std::fs::write(root.join(format!("{n}.err")), format!("{line}\n")).unwrap();
                    format!("cat '{}/{n}.err' >&2; exit {code}", root.display())
                }
                Reply::Hangs(seconds) => format!("sleep {seconds}; exit 0"),
            };
            cases.push_str(&format!("  '{endpoint}') {body} ;;\n"));
        }
        let script = format!(
            r#"#!/bin/sh
fail() {{ echo "gh: unexpected arguments: $all" >&2; exit 98; }}
all="$*"
[ "$1" = api ] && [ "$2" = --hostname ] && [ "$3" = github.com ] || fail
shift 3
paginate=
if [ "$1" = --paginate ]; then paginate=1; shift; fi
[ "$#" -eq 3 ] && [ "$1" = --jq ] || fail
jq=$2
endpoint=$3
echo "$endpoint" >> '{log}'
case "$endpoint" in
{cases}  *) echo "gh: unexpected $endpoint" >&2; exit 99 ;;
esac
"#,
            log = root.join("calls").display(),
        );
        write_script(&root.join("gh"), &script);
        FakeGh { dir }
    }

    /// A fake `gh` for a repository with everything set up: the App on all
    /// repositories, one secret in the repository and one shared by the
    /// organization, and the check required by a ruleset. The installations
    /// and the repository's secrets come in two pages.
    fn set_up() -> Vec<(&'static str, Reply)> {
        vec![
            (REPO, Reply::Pages(&[NOT_A_FORK])),
            (
                INSTALLATIONS,
                Reply::Pages(&[
                    r#"{"total_count": 2, "installations": [
                        {"id": 1, "app_slug": "other-app", "repository_selection": "all", "suspended_at": null}]}"#,
                    r#"{"total_count": 2, "installations": [
                        {"id": 2, "app_slug": "stormlight-thunderbot", "repository_selection": "all", "suspended_at": null}]}"#,
                ]),
            ),
            (
                SECRETS,
                Reply::Pages(&[
                    r#"{"total_count": 2, "secrets": [{"name": "OTHER", "created_at": "2026-01-01T00:00:00Z"}]}"#,
                    r#"{"total_count": 2, "secrets": [{"name": "THUNDERBOT_APP_ID", "created_at": "2026-01-01T00:00:00Z"}]}"#,
                ]),
            ),
            (
                ORG_SECRETS,
                Reply::Pages(&[r#"{"total_count": 1, "secrets": [{"name": "THUNDERBOT_PRIVATE_KEY"}]}"#]),
            ),
            (
                RULES,
                Reply::Pages(&[r#"[
                    {"type": "pull_request", "ruleset_id": 1, "parameters": {"required_approving_review_count": 1}},
                    {"type": "required_status_checks", "ruleset_id": 1, "parameters": {
                        "strict_required_status_checks_policy": false,
                        "required_status_checks": [{"context": "storm/review"}]}}]"#]),
            ),
            (CLASSIC, Reply::Fails(1, NOT_PROTECTED)),
        ]
    }

    /// The repository, with the default call limits.
    fn github(&self) -> GitHub {
        self.github_within(HOST_DEADLINE)
    }

    /// The repository, with all calls limited to `total`.
    fn github_within(&self, total: Duration) -> GitHub {
        let path = OsString::from(self.dir.path());
        GitHub::detect(self.dir.path(), Some(&path), HOST_TIMEOUT, total).unwrap()
    }

    /// The endpoints asked for, in order.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// The `jq` the fake `gh` runs, from an absolute entry of the test's `PATH`.
fn jq() -> PathBuf {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("jq"))
        .find(|path| path.is_file())
        .expect("the host tests need jq on PATH")
}

/// `replies` with the reply for `endpoint` replaced.
fn with(mut replies: Vec<(&'static str, Reply)>, endpoint: &str, reply: Reply) -> Vec<(&'static str, Reply)> {
    let slot = replies.iter_mut().find(|(e, _)| *e == endpoint).unwrap();
    slot.1 = reply;
    replies
}

/// The reason of an [`Answer::Unknown`], or a panic naming the answer.
fn unknown(answer: Answer) -> String {
    match answer {
        Answer::Unknown(reason) => reason,
        other => panic!("expected Unknown, got {other:?}"),
    }
}

fn write_script(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_repository_with_everything_set_up_answers_yes() {
    let gh = FakeGh::new(&FakeGh::set_up());
    let github = gh.github();
    assert_eq!(github.repository(), "o/r on GitHub");
    assert_eq!(github.app_installed("stormlight-thunderbot"), Answer::Yes);
    assert_eq!(github.secret_available("THUNDERBOT_APP_ID"), Answer::Yes);
    assert_eq!(github.secret_available("THUNDERBOT_PRIVATE_KEY"), Answer::Yes);
    assert_eq!(github.check_required("main", "storm/review"), Answer::Yes);
}

#[test]
fn an_app_missing_from_the_organization_is_no() {
    let gh = FakeGh::new(&with(
        FakeGh::set_up(),
        INSTALLATIONS,
        Reply::Pages(&[
            r#"{"total_count": 1, "installations": [{"app_slug": "other-app", "repository_selection": "all", "suspended_at": null}]}"#,
        ]),
    ));
    assert_eq!(gh.github().app_installed("stormlight-thunderbot"), Answer::No);
}

#[test]
fn an_app_on_selected_repositories_is_unknown() {
    let gh = FakeGh::new(&with(
        FakeGh::set_up(),
        INSTALLATIONS,
        Reply::Pages(&[r#"{"total_count": 1, "installations": [
            {"app_slug": "stormlight-thunderbot", "repository_selection": "selected", "suspended_at": null}]}"#]),
    ));
    let reason = unknown(gh.github().app_installed("stormlight-thunderbot"));
    assert!(reason.contains("selected repositories of o"), "{reason}");
}

#[test]
fn installations_of_a_user_account_are_unknown() {
    let gh = FakeGh::new(&with(
        FakeGh::set_up(),
        INSTALLATIONS,
        Reply::Fails(1, "gh: Not Found (HTTP 404)"),
    ));
    let reason = unknown(gh.github().app_installed("stormlight-thunderbot"));
    assert_eq!(
        reason,
        "gh: Not Found (HTTP 404); GitHub lists installations only for an organization, to its owners"
    );
}

#[test]
fn a_403_is_unknown_with_what_gh_printed() {
    let replies = with(FakeGh::set_up(), INSTALLATIONS, Reply::Fails(1, FORBIDDEN));
    let replies = with(replies, SECRETS, Reply::Fails(1, FORBIDDEN));
    let replies = with(replies, ORG_SECRETS, Reply::Fails(1, FORBIDDEN));
    let replies = with(replies, RULES, Reply::Fails(1, FORBIDDEN));
    let gh = FakeGh::new(&replies);
    let github = gh.github();
    assert_eq!(unknown(github.app_installed("stormlight-thunderbot")), FORBIDDEN);
    assert_eq!(unknown(github.secret_available("THUNDERBOT_APP_ID")), FORBIDDEN);
    // Classic protection read nothing, but the rulesets could not be read.
    assert_eq!(unknown(github.check_required("main", "storm/review")), FORBIDDEN);
}

#[test]
fn a_secret_neither_endpoint_lists_is_no() {
    let gh = FakeGh::new(&with(
        FakeGh::set_up(),
        ORG_SECRETS,
        Reply::Pages(&[r#"{"total_count": 0, "secrets": []}"#]),
    ));
    assert_eq!(gh.github().secret_available("THUNDERBOT_PRIVATE_KEY"), Answer::No);
}

#[test]
fn an_unprotected_branch_with_no_rule_does_not_require_the_check() {
    let gh = FakeGh::new(&with(FakeGh::set_up(), RULES, Reply::Pages(&["[]"])));
    assert_eq!(gh.github().check_required("main", "storm/review"), Answer::No);
}

#[test]
fn classic_protection_can_require_the_check() {
    let replies = with(FakeGh::set_up(), RULES, Reply::Pages(&["[]"]));
    let gh = FakeGh::new(&with(
        replies,
        CLASSIC,
        Reply::Pages(&[r#"{"strict": true, "contexts": ["ci"], "checks": [
            {"context": "ci", "app_id": null}, {"context": "storm/review", "app_id": 15368}]}"#]),
    ));
    assert_eq!(gh.github().check_required("main", "storm/review"), Answer::Yes);
}

#[test]
fn without_a_login_every_answer_is_unknown() {
    let replies: Vec<(&str, Reply)> = [REPO, INSTALLATIONS, SECRETS, ORG_SECRETS, RULES, CLASSIC]
        .into_iter()
        .map(|endpoint| (endpoint, Reply::Fails(4, NO_LOGIN)))
        .collect();
    let gh = FakeGh::new(&replies);
    let github = gh.github();
    assert_eq!(unknown(github.app_installed("stormlight-thunderbot")), NO_LOGIN);
    assert_eq!(unknown(github.secret_available("THUNDERBOT_APP_ID")), NO_LOGIN);
    assert_eq!(unknown(github.check_required("main", "storm/review")), NO_LOGIN);
}

#[test]
fn the_secrets_are_listed_once_for_every_secret() {
    let gh = FakeGh::new(&FakeGh::set_up());
    let github = gh.github();
    for name in ["THUNDERBOT_APP_ID", "THUNDERBOT_PRIVATE_KEY", "OTHER"] {
        github.secret_available(name);
    }
    assert_eq!(gh.calls(), [REPO, SECRETS, ORG_SECRETS]);
}

#[test]
fn the_calls_stop_at_one_deadline() {
    let replies = with(FakeGh::set_up(), INSTALLATIONS, Reply::Hangs(30));
    let gh = FakeGh::new(&replies);
    let started = Instant::now();
    let github = gh.github_within(Duration::from_secs(1));
    let app = unknown(github.app_installed("stormlight-thunderbot"));
    assert!(app.contains("did not exit within"), "{app}");
    let secret = unknown(github.secret_available("THUNDERBOT_APP_ID"));
    assert_eq!(secret, "not asked, since the repository checks ran past 1 s");
    unknown(github.check_required("main", "storm/review"));
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(gh.calls(), [REPO, INSTALLATIONS]);
}

#[test]
fn a_suspended_installation_is_suspended() {
    let gh = FakeGh::new(&with(
        FakeGh::set_up(),
        INSTALLATIONS,
        Reply::Pages(&[
            r#"{"total_count": 1, "installations": [{"app_slug": "stormlight-thunderbot",
            "repository_selection": "all", "suspended_at": "2026-10-01T12:00:00Z"}]}"#,
        ]),
    ));
    assert_eq!(
        gh.github().app_installed("stormlight-thunderbot"),
        Answer::Suspended("its installation on o was suspended at 2026-10-01T12:00:00Z".into())
    );
}

#[test]
fn a_fork_is_checked_through_its_parent() {
    let replies = vec![
        (
            REPO,
            Reply::Pages(&[r#"{"full_name": "o/r", "fork": true, "parent": {"full_name": "up/stream"}}"#]),
        ),
        (
            "orgs/up/installations",
            Reply::Pages(&[r#"{"total_count": 1, "installations": [
                {"app_slug": "stormlight-thunderbot", "repository_selection": "all", "suspended_at": null}]}"#]),
        ),
        ("repos/up/stream/actions/secrets", Reply::Fails(1, FORBIDDEN)),
        (
            "repos/up/stream/actions/organization-secrets",
            Reply::Fails(1, FORBIDDEN),
        ),
        (
            "repos/up/stream/rules/branches/main",
            Reply::Pages(&[r#"[{"type": "required_status_checks", "parameters": {
                "required_status_checks": [{"context": "storm/review"}]}}]"#]),
        ),
        (
            "repos/up/stream/branches/main/protection/required_status_checks",
            Reply::Fails(1, NOT_PROTECTED),
        ),
    ];
    let gh = FakeGh::new(&replies);
    let github = gh.github();
    assert_eq!(github.repository(), "up/stream on GitHub, the parent of the fork o/r");
    assert_eq!(github.app_installed("stormlight-thunderbot"), Answer::Yes);
    // A fork's owner may not list the parent's secrets: unknown, not missing.
    assert_eq!(unknown(github.secret_available("THUNDERBOT_APP_ID")), FORBIDDEN);
    assert_eq!(github.check_required("main", "storm/review"), Answer::Yes);
    assert!(
        !gh.calls().iter().any(|call| call.starts_with("repos/o/r/")),
        "{:?}",
        gh.calls()
    );
}

#[test]
fn a_fork_whose_parent_cannot_be_read_is_unknown() {
    let gh = FakeGh::new(&[(REPO, Reply::Pages(&[r#"{"full_name": "o/r", "fork": true}"#]))]);
    let github = gh.github();
    assert_eq!(github.repository(), "o/r on GitHub");
    let reason = "o/r is a fork, and GitHub did not name its parent";
    assert_eq!(unknown(github.app_installed("stormlight-thunderbot")), reason);
    assert_eq!(unknown(github.secret_available("THUNDERBOT_APP_ID")), reason);
    assert_eq!(unknown(github.check_required("main", "storm/review")), reason);
    assert_eq!(gh.calls(), [REPO]);
}
