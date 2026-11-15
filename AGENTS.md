# Agent guidelines

## Worktrees

Do every change in a git worktree under `.worktree/` inside this repository, without being
asked. Don't edit the main checkout directly and don't clone the repository elsewhere.

```sh
git fetch origin
git worktree add -b <branch> .worktree/<name> origin/main
```

`.worktree/` is ignored by Git. Remove a worktree with `git worktree remove .worktree/<name>`
once its branch is merged.

## Checks

Before opening a PR, run `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`
and `cargo test`. Merge only after `CI / ready` passes. Releases follow
[docs/RELEASING.md](docs/RELEASING.md).
