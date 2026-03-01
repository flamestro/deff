These `.sublime-syntax` grammars are bundled into the deff binary at compile time.

Any `*.sublime-syntax` file added here is automatically bundled; no manual `src/syntax.rs` update is required.

You can also add more grammar files here (or in the locations below) to extend language coverage further.

deff loads custom syntaxes from these locations (if present):

- `assets/syntaxes` (relative to the current working directory)
- `.deff/syntaxes` (relative to the current working directory)

Syntactic detection still uses syntect APIs first, then first-line/shebang fallback.
