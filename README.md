# deff 

`deff` is a Rust TUI: interactive, side-by-side file review for git diffs with per-file navigation, vertical and horizontal scrolling, syntax highlighting, and added/deleted line tinting.

## Features

- `upstream-ahead` strategy (default) to compare local branch changes against its upstream
- `range` strategy for explicit `--base` / `--head` comparison
- Optional `--include-uncommitted` mode to include working tree and untracked files
- Side-by-side panes with independent horizontal scroll offsets
- Keyboard and mouse navigation (including wheel + shift-wheel)
- Language-aware syntax highlighting and line-level add/delete tinting

## Usage

```bash
pr-diff
pr-diff --strategy upstream-ahead
pr-diff --strategy range --base origin/main --head HEAD
pr-diff --strategy range --base origin/main --include-uncommitted
pr-diff --theme dark
```

Show help:

```bash
pr-diff --help
```

## Local Build and Usage Flow

Prerequisites:

- Rust toolchain (`cargo`)
- `git`
- Interactive terminal (TTY)

1. Build locally:

   ```bash
   cargo build --release --locked
   ./target/release/pr-diff --help
   ```

2. Optionally install it to your local Cargo bin path:

   ```bash
   cargo install --path .
   pr-diff --help
   ```

3. Run it inside any git repository you want to review:

   ```bash
   cd /path/to/your/repo

   # default: compare local branch commits vs upstream
   pr-diff

   # explicit range
   pr-diff --base origin/main --head HEAD

   # include uncommitted + untracked files
   pr-diff --base origin/main --include-uncommitted
   ```

If your branch has no upstream configured, use the explicit `--base` flow.

Theme selection:

- By default, `pr-diff` prefers a dark syntax theme (better for black/dark terminals).
- Use `--theme auto|dark|light` to control rendering for your terminal.
- `--theme` takes precedence over `PR_DIFF_THEME=dark|light`.

## GitHub Release Workflow

This repo ships with `.github/workflows/release.yml`.

- Trigger: push a tag like `v0.1.0`
- Builds release artifacts for Linux and macOS targets
- Creates a GitHub release and uploads tarballs + SHA256 files
- Optionally updates a Homebrew tap formula automatically

To enable automatic tap updates, configure:

- repo variable `HOMEBREW_TAP_REPO` (example: `<owner>/homebrew-tap`)
- repo secret `HOMEBREW_TAP_TOKEN` with write access to that tap repo

## Homebrew Publishing

`Formula/pr-diff.rb` is included as a formula template and is also what the workflow writes into your tap.

Install from your tap:

```bash
brew tap <owner>/tap
brew install pr-diff
```
