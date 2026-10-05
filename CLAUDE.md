# Claude instructions

## Before any change

1. Read [AGENTS.md](AGENTS.md): the layout, commands, conventions and
   the rules every change follows.
1. Read [COMMENTS.md](COMMENTS.md) before writing code.
1. Read [DOCUMENTATION.md](DOCUMENTATION.md) before writing Markdown.
1. Read the `//!` docs of each module you change: they hold its design.

## While editing

- **Invented names only.** Never put a real artist, title, video ID or
  lyric in a test, example, comment or doc.
- **Every platform.** No Unix-only or Windows-only API without `cfg` and
  a fallback; join paths part by part.
- **No comment on self-documenting code.** If removing a comment loses
  nothing the line itself says, remove it. Rationale goes in rustdoc.
- **Never copy a default into docs.** Link `src/manifest/new.toml`.

## After editing

- Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, `cargo xtask comments` and
  `cargo xtask docs`.
- Update every file the matching row of the AGENTS.md update checklist
  names, in the same commit.
