This directory contains extra `.sublime-syntax` grammars loaded by deff at startup.

You can add more grammar files here to extend language coverage further.

deff loads custom syntaxes from these locations (if present):

- `assets/syntaxes` (relative to the current working directory)
- `.deff/syntaxes` (relative to the current working directory)
- `DEFF_SYNTAX_DIR`
- `DEFF_SYNTAX_PATHS` (OS path list, e.g. colon-separated on macOS/Linux)

Syntactic detection still uses syntect APIs first, then first-line/shebang fallback.
