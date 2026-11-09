+++
id = "arcl-c-01M70VXWA58XNN13J6X73E6VWP"
title = "Set arcl's disposable and planning paths in fabrial.toml"
status = "captured"
created-at = "2026-11-09T19:17:58.725Z"
+++

fab 0.9.1 reads two lists in `.github/fabrial.toml` that hold arcl's paths
in fabrial's own file:

- `disposable`: ignored paths `fab cleanup` may delete with a merged
  worktree, such as `.arcl/arcl.db` and its `-wal`, `-shm` and
  `-sync.lock` files.
- `planning`: directories whose files `fab review-depth` reviews as
  documentation, such as `.arcl/workspace/`.

`fab init` takes no flag for either. `sltk init` writes the `[tasks]` table
for arcl and leaves these unset, so a repository set up by sltk keeps
arcl's cache through `fab cleanup` and gets the full review pass for a
task-only change. Decide whether sltk sets them, as it sets `[tasks]`, and
whether `sltk check` reports them missing.
