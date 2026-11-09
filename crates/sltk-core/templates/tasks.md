## Tasks

The installed skills read this section for the tracker's commands. Tasks live
in arcl, in tracked files under `.arcl/workspace`; `arcl.db` is a local cache.
In a cloud session `.claude/hooks/session-start.sh` installs the `sltk`
release that `.github/sltk.toml` pins, runs `sltk install --yes` to install
fab and arcl, and builds the cache in the session's checkout only. Elsewhere,
run `sltk install`. `sltk init` writes this section, between the
`sltk:tasks` markers, and the hook.

Every task worktree, cloud sessions included, needs `arcl init` before its
first `arcl` command.

| Step              | Command                                                       |
| ----------------- | ------------------------------------------------------------- |
| Read a task       | `arcl task show <id>` and `arcl context <id>`                 |
| Check readiness   | `arcl ready`, and `arcl task explain <id>` for one task       |
| Start             | `arcl init`, then `arcl task start <id>`, in the task's worktree |
| Complete          | `arcl task complete <id> --evidence "Done in #<n> at <sha>: <summary>"` |
| Hand off          | `arcl task handoff <id> --note-file <file>`                   |
| Pick up           | `arcl context <id>`, `arcl task unpark <id>`, `arcl task start <id>` |
| Record new work   | `arcl task create --phase <id> "<title>"`, or `arcl capture create` |
| Record an idea    | `arcl capture create "<title>" --body-file <file>`            |
| Record a spec     | `arcl spec create "<title>" --body-file <file>`               |

`decompose` writes these primitives:

| Primitive  | In arcl                    | Write it with                                                        |
| ---------- | -------------------------- | -------------------------------------------------------------------- |
| Collection | a phase of a plan          | `arcl plan phase create --plan <id> "<title>" --body-file <file>`    |
| Run unit   | a task with child tasks    | `arcl task create --phase <id> "<title>" --body-file <file>`         |
| Task       | a child task               | `arcl task create --parent <id> "<title>" --body-file <file>`        |
| Dependency | a blocker                  | `arcl dependency add <task> --blocked-by <id>`                       |

Work cut from a spec goes in a phase of the spec's plan; a spec with no plan
gets one first, with `arcl plan create --spec <id> "<title>"`. A lone run unit
uses an existing phase, or no `--phase`.

- Every change to a task is a file change: commit it on the branch it
  describes, so the status reaches `main` with the merge. Record new tasks and
  bugs found along the way on the branch in progress, never in a pull request
  of their own.
- `arcl task start` refuses a task that is not ready. Pass `--force` only when
  the owner asks after seeing the reasons `arcl task explain` gives.
- A handoff parks the task. Unpark only when `arcl task explain` gives parking
  as the only reason it is not ready.
- When a merge conflicts on a task file, keep the newer status.
