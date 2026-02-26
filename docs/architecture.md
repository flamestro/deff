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

