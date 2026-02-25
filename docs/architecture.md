# deff Architecture

`deff` is organized into small modules so git logic, diff modeling, rendering, and terminal input can evolve independently.

## Module map

- `src/lib.rs`: top-level orchestration (`run`) and dependency wiring.
- `src/main.rs`: binary entrypoint and error exit handling.
- `src/cli.rs`: clap definitions and argument validation into `CliOptions`.
- `src/model.rs`: shared enums/structs for comparison metadata and file views.
- `src/git.rs`: git command execution plus comparison strategy resolution.
- `src/diff.rs`: file descriptor discovery, hunk highlight parsing, and view construction.
- `src/render.rs`: layout calculations and frame rendering with syntax highlighting.
- `src/app.rs`: state transitions for keyboard/mouse navigation.
- `src/terminal.rs`: TUI lifecycle and event loop plumbing.
- `src/text.rs`: pure string-width and formatting helpers.

## Extension points

- Add a new comparison strategy:
  1. Add enum variant in `src/model.rs`.
  2. Parse/validate in `src/cli.rs`.
  3. Resolve refs and metadata in `src/git.rs`.

- Add a new file status behavior:
  1. Extend parser logic in `src/diff.rs`.
  2. Add view metadata fields in `src/model.rs` if needed.
  3. Render that metadata in `src/render.rs`.

- Add keybindings or mouse interactions:
  1. Map input in `src/app.rs`.
  2. Update footer help in `src/render.rs`.

- Add rendering features:
  1. Extend `render_frame` in `src/render.rs`.
  2. Keep `DiffFileView` stable where possible so non-render modules do not change.

## Recommended next refactors

- Introduce an `Action` enum in `src/app.rs` to decouple event decoding from state mutation.
- Add unit tests for `src/cli.rs`, `src/diff.rs`, and `src/render.rs` pure helpers.
- Add lightweight integration tests using temporary git repos for strategy and diff parsing behavior.
