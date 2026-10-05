# Comment style guide

> **Maintenance**: Update this file when a rule, a density limit or a
> comment type changes, or when `cargo xtask comments` gains or drops a
> rule (keep [§Enforcement](#enforcement) in step with
> `xtask/src/comments/rules.rs`).

Comments in muman explain **why**, not **what**. Rust names and types
already say what the code does; a comment earns its place by stating
rationale, trade-offs, measured facts, platform quirks and invariants
the code cannot express.

muman keeps its design notes in the code rather than in separate
documents. The rationale for a module (how a measure was calibrated,
why a threshold sits where it does, what a platform does differently)
lives in that module's `//!` documentation, next to what it explains.
`AGENTS.md` only routes to it.

---

## Comment types

| Type | Syntax | Use for |
|---|---|---|
| Module documentation | `//!` at the top of the file | What the module is for, its design, the invariants it keeps, the measurements behind its constants |
| Item documentation | `///` above an item | What a type, function or constant means, when the name alone does not say it |
| Why-comment | `//` above or beside a line | A reason the line cannot carry: a platform quirk, a non-obvious order, a workaround |
| TODO | `// TODO: <one sentence>` | Work that is known and not yet done |

### Module documentation

Every module under `src/` opens with a `//!` summary. A module whose
design needs explaining keeps that explanation here, in full sentences
and as long as it needs to be. Tables and lists are fine; rustdoc
renders them.

```rust
//! Chromaprint fingerprints and their comparison.
//!
//! Two prints are aligned by the offset most of their exact 16-bit
//! half-words agree on, then compared word by word within 4 bits.
```

### Item documentation

`///` describes what an item means to its caller, not how it works
inside. Skip it when the name and signature already say everything.

### Why-comment

Placed immediately above the line it explains, with no blank line
between them, or inline when it is short. At most three lines; a longer
reason belongs in the item's or the module's rustdoc.

```rust
// Explorer and Finder keep the source's mtime on copy, so a file still
// arriving can look settled; the creation time cannot.
let settled = modified.max(created);
```

### TODO

Starts with `// TODO:` and one complete, actionable sentence.

---

## Rules

1. **No redundant comments.** If removing a comment loses no
   information the line itself carries, remove it.
1. **No comment on the default value.** Say what a setting means and
   what depends on it, never what it is currently set to; the value
   lives in one place.
1. **No commented-out code.** Delete it; history keeps it.
1. **No comments on unchanged code.** When editing, do not add comments
   to code you did not change. Editing an existing comment in place,
   after a rename for example, is fine.
1. **One blank line before a comment block**, none between a comment
   and the line it annotates.
1. **Wrap at 100 columns**, rustfmt's width.
1. **Invented names only.** An example artist, title, album or video ID
   in a comment or a test is made up; see the privacy rule in
   [AGENTS.md](AGENTS.md#rules).

---

## Density limits

| Metric | Limit |
|---|---|
| Plain `//` comment lines per file | ≤ 30 % of non-blank lines |
| Consecutive plain `//` comment lines | ≤ 3 |
| Section headers (`// ---- Title ----`) per file | ≤ 8 |
| Rise in `//` density in one change | ≤ 5 percentage points |

Rustdoc (`///`, `//!`) does not count toward the density limits: it is
the documentation, and its length follows what there is to explain.

---

## AI agent guidance

Treat this file as binding.

- Read it before writing code; every comment you write follows it.
- Do not add comments, docstrings or type annotations to code you did
  not change. That is a separate task, done only when asked.
- Prefer no comment over a redundant one, and match the density of the
  surrounding code.
- Never narrate a change: no `// Added …`, `// Changed from …`,
  `// New: …`, `// Claude: …`. Git records who changed what.
- Add a section header only to a file that already uses them, for a
  genuinely new group.

---

## Enforcement

`cargo xtask comments` checks the mechanical rules over every `.rs`
file; CI runs it on every push, and with `--diff-base` on pull
requests.

```bash
cargo xtask comments
cargo xtask comments --diff-base origin/main
```

| Rule | Checks |
|---|---|
| `ai-narration` | Comments that narrate an edit |
| `comment-density` | Plain `//` lines over 30 % of non-blank lines |
| `commented-out-code` | A plain `//` line shaped like Rust code |
| `consecutive-comments` | More than 3 plain `//` lines in a row |
| `density-delta` | `//` density rising more than 5 points (`--diff-base` only) |
| `line-length` | A comment-bearing line over 100 characters |
| `module-doc` | A module under `src/` without a `//!` summary first |
| `no-comments-on-unchanged-code` | A comment added beside code the change leaves alone (`--diff-base` only) |
| `section-header-count` | More than 8 section headers |
| `section-header-style` | A decorative line not shaped `// ---- Title ----` |
| `todo-format` | A TODO not shaped `// TODO: <sentence>` |

"Why, not what" and "the comment earns its place" are for review.

A `// allow-<rule>: <reason>` comment on or above a line silences that
rule for that line. The reason is required.
