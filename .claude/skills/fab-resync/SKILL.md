---
name: fab-resync
description: Rebase an open pull request onto the current main, run the gates and push. Use for /fab-resync, or when a pull request has a conflict or its base has moved under it.
argument-hint: <pr-number>
---

# Resync

Move one pull request onto the current head of `main`.

## Read the pull request

```sh
gh pr view <n> --json headRefName,baseRefName,headRefOid,author
```

A cloud session reads the same with `pull_request_read` method `get`. Note the
head before the rebase.

The branch must be an agent's: `claude/`, `codex/` or `storm/`. Rebasing
rewrites it, so leave a person's branch alone and report instead.

## Work in the pull request's tree

Use the worktree `git worktree list` shows on the pull request's branch, or
take one as the `fab-revise` skill does. Never rebase in the user's primary
checkout.

## Rebase

```sh
git fetch origin main <branch>
git diff "$(git merge-base origin/main HEAD)" HEAD > before.diff
git rebase origin/main
git diff origin/main HEAD > after.diff
```

Compare the two diffs as the Review gate does: the same changed lines with
the same surrounding context, wherever they sit in the file. Line numbers in
the `@@` headers and the blob ids on `index` lines may differ; any other
difference, whitespace included, means the change itself differs, as when
`main` already held part of it or changed a line next to it.

A conflict is a decision about the change. Abort with `git rebase --abort`,
name the conflicting files, and stop: a `revise` pass, a `fab-storm` run's
**Finish** step or the operator resolves it.

## Verify and push

Run the gates (the command `gates` names in `.github/fabrial.toml`). A gate that
fails after the rebase is a finding for a `revise` pass: report it and push
nothing.

```sh
git push --force-with-lease origin <branch>
```

When only the base moved, the Review gate keeps `storm/review` green.
Otherwise the check goes back to pending and the change needs another review
sequence, which the driver asks a `fab-storm` run for. Read the check once after
the push and do not wait on it. If it is pending after a push that only moved
the base, the previous head's check was not yet green when the gate read it:
say so, and that the driver can rerun this push's Review gate run once that
check is green, or ask for another sequence.

## Report

Report the pull request, the old and new heads, whether the change is the same
or differs, the gates and their result, and `storm/review`'s state after the
push. A stop names its reason and the files.

## Do not

- Resolve a conflict yourself.
- Rebase a branch a person owns.
- Merge, approve, or push to `main`.
