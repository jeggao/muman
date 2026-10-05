# Documentation style guide

> **Maintenance**: Update this file when a rule for Markdown in this
> repository changes: voice, structure, length, cross-references, or
> what documentation may not record.

Documentation in muman says what code cannot: how to use the tool, what
a setting means, and why a design is as it is. It is read by people
using muman, by contributors, and by AI agents arriving with no context;
all three want the one canonical answer, stated once.

Design rationale belongs in the code, in the `//!` documentation of the
module that owns it ([COMMENTS.md](COMMENTS.md)). Markdown files hold
what is not about one module.

---

## Where things go

| Content | Lives in |
|---|---|
| What muman is, installing it, a first run | [README.md](README.md) |
| Using it: the song list, sources, picking, editing, hooks | [docs/guide.md](docs/guide.md) |
| Folders, programs, settings, templates, safe names | [docs/configuration.md](docs/configuration.md) |
| Every command and flag | [docs/cli.md](docs/cli.md), generated from `src/cli.rs` |
| Each setting's default | [src/manifest/new.toml](src/manifest/new.toml) |
| Why a module works as it does | That module's `//!` docs |
| How to work in this repository | [AGENTS.md](AGENTS.md) |
| What changed in each release | [CHANGELOG.md](CHANGELOG.md) |

Do not create other Markdown files unless asked.

---

## Voice

- **Declarative, present tense.** "`sync` writes each song once", not
  "`sync` will write".
- **Second person for instructions**, third person for facts.
- **No narration of changes:** never "now", "new", "recently", "as of
  this change". Git and the changelog record history.
- **No marketing words:** "robust", "powerful", "seamless", "leverages".
- **Invented names only** in examples: artists, titles, albums and video
  IDs are made up (see [AGENTS.md](AGENTS.md#rules)).

---

## Structure

- One `# Title` per file, in sentence case. Then `##` and `###`; a
  section that needs `####` is too big.
- Style guides (`COMMENTS.md`, this file) open with a
  `> **Maintenance**:` line naming exactly what change requires an
  update, and separate `##` sections with `---`. Other files do not.
- Length caps, roughly: `AGENTS.md` and style guides 250 non-blank
  lines, the README 150, each file under `docs/` 400.

---

## One source per fact

1. **State a fact once, and link to it elsewhere.** A paraphrase drifts.
1. **Never copy a default value or a list the code holds.** Say what a
   setting means and link [new.toml](src/manifest/new.toml); a test keeps
   that file equal to the code's defaults.
1. **Link in-repo files by relative path**, never by GitHub URL; the
   repository may move. `cargo xtask docs --check` fails on a link to a
   file that does not exist.
1. **Backtick a path when it names something, link it when the reader
   will open it.** Never link a symbol; the reader searches for it.

---

## Formatting

- **Tables** for three or more parallel facts: the first column is the
  lookup key, separators are `|---|---|` without alignment colons, and a
  cell over ~120 characters becomes a paragraph below.
- **Lists** use `-`; numbered lists use `1.` for every item.
- **Code blocks** are always fenced and language-tagged. `bash` blocks
  hold commands with no `$` prompt.
- **`docs/cli.md` is generated**: run `cargo xtask docs` after changing
  `src/cli.rs`, never edit it by hand.

---

## AI agent guidance

1. Read the canonical source before writing; link it rather than
   restating it.
1. When a change matches a row of the update checklist in
   [AGENTS.md](AGENTS.md#keeping-docs-in-step), update every file the row
   names in the same commit.
1. Do not rewrite documentation unrelated to the change.
1. Match the voice and density of the file you are editing.
1. After renaming a heading or a file, fix every link to it.
