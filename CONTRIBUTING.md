# Contributing to deff

Thanks for contributing.

## Local setup

Prerequisites:

- Rust toolchain (`cargo`)
- `git`
- Interactive terminal (for running `deff`)

From a fresh clone:

```bash
cargo build --release --locked
cargo check --locked
```

## Commit message conventions

This repository uses commit prefixes to drive post-release version bumping.

- `feat:` user-facing feature work (next version bumps minor)
- `chore:` maintenance, refactors, tooling, and internal cleanups (next version bumps patch)
- `docs:` documentation-only changes (next version bumps patch)

Examples:

```text
feat: add keyboard shortcut help overlay
chore: simplify diff cache invalidation
docs: clarify install instructions
```

## Release and version bump flow

- Publish a GitHub release with a tag like `v0.1.0` to trigger `.github/workflows/bump-version.yml`
- The bump workflow analyzes commit subjects between the previous release tag and the published tag
- If any commit starts with `feat:`, the next version is a minor bump (`X.Y+1.0`)
- Otherwise, `chore:` and `docs:` changes produce a patch bump (`X.Y.Z+1`)
- The workflow commits updated `Cargo.toml` and `Cargo.lock` back to the default branch

## Pull requests

- Keep PRs focused and describe the user-facing impact
- Run `cargo check --locked` before opening/updating a PR
- Update docs when behavior changes
