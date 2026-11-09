//! Asks [`GitHub`] about a repository through a fake `gh` on `PATH`, as
//! `tests/doctor.rs` in sltk-cli fakes fab. The fake `git` names
//! `github.com/o/r` as the remote origin. The fake `gh` answers each endpoint
//! with set output, error line and exit code, the way gh prints them, and
//! records every endpoint it is asked for.

#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use sltk_core::host::{Answer, GitHub, HOST_DEADLINE, HOST_TIMEOUT, Host};

const INSTALLATIONS: &str = "orgs/o/installations";
const SECRETS: &str = "repos/o/r/actions/secrets";
const ORG_SECRETS: &str = "repos/o/r/actions/organization-secrets";
const RULES: &str = "repos/o/r/rules/branches/main";
const CLASSIC: &str = "repos/o/r/branches/main/protection/required_status_checks";

/// What gh prints when a branch has no classic protection.
const NOT_PROTECTED: &str = "gh: Branch not protected (HTTP 404)";

/// What gh prints for a token that may not read an endpoint.
const FORBIDDEN: &str = "gh: Resource not accessible by integration (HTTP 403)";

/// What gh prints when it has no login.
const NO_LOGIN: &str = "To get started with GitHub CLI, please run:  gh auth login";

/// How the fake `gh` answers one endpoint.
#[derive(Clone)]
enum Reply {
    /// Exits 0 printing these lines, as `--jq` leaves them.
    Lines(&'static [&'static str]),
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
    /// test's call on any other.
    fn new(replies: &[(&str, Reply)]) -> FakeGh {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_script(&root.join("git"), "#!/bin/sh\necho https://github.com/o/r.git\n");
        let mut cases = String::new();
        for (n, (endpoint, reply)) in replies.iter().enumerate() {
            let body = match reply {
                Reply::Lines(lines) => {
                    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
                    std::fs::write(root.join(format!("{n}.out")), text).unwrap();
                    format!("cat '{}/{n}.out'; exit 0", root.display())
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
            "#!/bin/sh\nfor arg; do last=$arg; done\necho \"$last\" >> '{log}'\ncase \"$last\" in\n{cases}  *) echo \"gh: unexpected $last\" >&2; exit 99 ;;\nesac\n",
            log = root.join("calls").display(),
        );
        write_script(&root.join("gh"), &script);
        FakeGh { dir }
    }

    /// A fake `gh` for a repository with everything set up: the App on all
    /// repositories, one secret in the repository and one shared by the
    /// organization, and the check required by a ruleset.
    fn set_up() -> Vec<(&'static str, Reply)> {
        vec![
            (
                INSTALLATIONS,
                Reply::Lines(&["other-app all", "stormlight-thunderbot all"]),
            ),
            (SECRETS, Reply::Lines(&["THUNDERBOT_APP_ID"])),
            (ORG_SECRETS, Reply::Lines(&["THUNDERBOT_PRIVATE_KEY"])),
            (RULES, Reply::Lines(&["storm/review"])),
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
    let gh = FakeGh::new(&with(FakeGh::set_up(), INSTALLATIONS, Reply::Lines(&["other-app all"])));
    assert_eq!(gh.github().app_installed("stormlight-thunderbot"), Answer::No);
}

#[test]
fn an_app_on_selected_repositories_is_unknown() {
    let gh = FakeGh::new(&with(
        FakeGh::set_up(),
        INSTALLATIONS,
        Reply::Lines(&["stormlight-thunderbot selected"]),
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
    let gh = FakeGh::new(&with(FakeGh::set_up(), ORG_SECRETS, Reply::Lines(&[])));
    assert_eq!(gh.github().secret_available("THUNDERBOT_PRIVATE_KEY"), Answer::No);
}

#[test]
fn an_unprotected_branch_with_no_rule_does_not_require_the_check() {
    let gh = FakeGh::new(&with(FakeGh::set_up(), RULES, Reply::Lines(&[])));
    assert_eq!(gh.github().check_required("main", "storm/review"), Answer::No);
}

#[test]
fn classic_protection_can_require_the_check() {
    let replies = with(FakeGh::set_up(), RULES, Reply::Lines(&[]));
    let gh = FakeGh::new(&with(replies, CLASSIC, Reply::Lines(&["ci", "storm/review"])));
    assert_eq!(gh.github().check_required("main", "storm/review"), Answer::Yes);
}

#[test]
fn without_a_login_every_answer_is_unknown() {
    let replies: Vec<(&str, Reply)> = [INSTALLATIONS, SECRETS, ORG_SECRETS, RULES, CLASSIC]
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
    assert_eq!(gh.calls(), [SECRETS, ORG_SECRETS]);
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
    assert_eq!(gh.calls(), [INSTALLATIONS]);
}
