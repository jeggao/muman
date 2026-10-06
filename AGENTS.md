# AGENTS.md

How to work in the muman repository: what is where, how to build and
check it, the conventions the code keeps, and the rules every change
follows. muman is one Rust command-line program; its design is
documented in the code, in each module's `//!` docs, and this file
routes to them. [README.md](README.md) says what muman does for a user.

Read before changing anything: this file, [COMMENTS.md](COMMENTS.md)
before writing code, and [DOCUMENTATION.md](DOCUMENTATION.md) before
writing Markdown.

## Rules

These bind every change, by a person or an agent.

1. **Invented names only.** Tests, examples, comments and docs use
   made-up artists, titles, albums and channel names ("Marlo Venn",
   "The Glass Orchards") and obviously synthetic IDs (`vid00000001`).
   Never paste data from a real library, real video IDs, or real song
   lyrics: the repository is public, and a test table copied from a
   collection publishes someone's listening history.
1. **No secrets, no personal paths or addresses.** Commits use the
   author's GitHub noreply address.
1. **Every platform.** Code builds and behaves alike on Linux, macOS and
   Windows. A Unix-only or Windows-only API goes behind `cfg` with a
   fallback for the others; paths are joined part by part, never with a
   literal `/`; paths muman records go through `relpath`.
1. **Tests run offline and read nothing from the machine.** Every
   subprocess goes through the `Runner` trait (faked by `testing::Fake`),
   HTTP through `http::HttpTransport`, prompts through `ui::Prompter`;
   folders, settings and waits are parameters, never the real home, the
   environment or the clock.
1. **No pushes to `main`.** Changes go through a pull request, and CI
   must pass on all three platforms.
1. **This file and the style guides change only when asked.** Propose a
   rule change in the pull request instead.

## Layout

```text
src/
  main.rs, lib.rs   entry point; Job, run() and the run_with() test seam
  cli.rs            the command line (clap derive), exit codes
  settings.rs       [library], [audio], [quality], [ytdlp], [history]
  manifest.rs       songs.toml: songs, albums, removed, cleaning, edits
  manifest/new.toml a new home's song list, the reference for every setting
  state.rs          state.json: files written, measures, comparisons
  platform.rs       default folders per OS, path identity, arrival times
  relpath.rs        recorded paths, written with / and in NFC
  runner.rs         the subprocess seam and tool lookup
  acquire.rs        the network phase: yt-dlp listing, fetching, matching
  download.rs       yt-dlp's arguments and the files it reports
  music.rs          YouTube Music search and name matching
  plugins.rs        yt-dlp postprocessors shipped in the binary
  identify.rs       new sources to songs, by fingerprint
  lookup.rs         lookups due by trigger; provider.rs, lrclib.rs,
                    musicbrainz.rs
  reconcile.rs      the offline phase: measure, compare, plan, render, prune
  facts.rs          one source measured; probe.rs, info.rs, ffmpeg.rs
  quality.rs        audio and picture measures
  fingerprint.rs    Chromaprint prints and their comparison
  align.rs          audio comparison by loudness envelopes
  resolve.rs        one song's facts to the plan it is written from
  tags.rs, clean.rs tag offers and the cleaning rules
  lyrics.rs         subtitle languages, LRC cleaning and timing
  naming.rs         safe names; template.rs, the path template
  codec.rs          output codecs: names, containers, ffmpeg arguments
  render.rs         one plan to library files
  fit.rs            songs fitted into a size, least audible loss first
  limit.rs          the library kept under [library] max_size
  export.rs         the song list and the library into one zip
  history.rs        run records and undo
  change.rs, editor.rs, query.rs   list, set, edit, remove, restore
  check.rs, overview.rs            check and info
  duplicates.rs     songs listed apart that are one recording
  hooks.rs, store.rs, source.rs, atomic.rs, ui.rs, http.rs, ...
  */tests.rs        a module's tests, when they outgrow it
testdata/           tiny Opus, FLAC and PNG fixtures
assets/yt-dlp/      the postprocessors plugins.rs embeds
xtask/              cargo xtask comments | docs
docs/               guide.md, configuration.md, cli.md (generated)
```

Start from a module's `//!` docs: they hold its design and the reasons
for its constants.

## Commands

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo xtask comments
cargo xtask docs
cargo test --test smoke -- --ignored
```

`cargo xtask docs` regenerates [docs/cli.md](docs/cli.md) and checks
links and the changelog; `--check` only checks, as CI does. The smoke
test runs muman end to end against a real ffmpeg on generated audio.

CI (`.github/workflows/ci.yml`) runs clippy, the tests and the smoke
test on Linux, macOS and Windows, plus rustfmt, rustdoc, the minimum
Rust version, `cargo xtask comments` (with `--diff-base` on pull
requests), `cargo xtask docs --check` and cargo-deny. Its clippy is the
latest stable, which can be newer than yours.

## Conventions

- **Command line:** clap derive in `cli.rs`; environment fallbacks
  through `#[arg(env = "MUMAN_…")]`, never `std::env::var` by hand.
- **Output:** messages go to a writer passed in (`&mut impl Write`),
  styled through `ui`; never `println!` or raw escape codes in a module.
  `run` wraps stdout and stderr in `anstream`, which handles terminals,
  `NO_COLOR` and Windows consoles.
- **Errors:** `anyhow` with context naming the file or step.
- **Exit codes:** 0 success or a declined change, 2 a missing tool or no
  home folder, 4 a failure, 5 a refused change. Listed in `cli.rs`,
  `EXIT_CODES_HELP`.
- **Files:** whole-file writes through `atomic::write`; renames and
  deletes in the library through `atomic::rename` and `atomic::remove`,
  which wait out a file Windows reports as held open.
- **Settings:** a new setting goes in `settings.rs`, with its default
  shown commented in `manifest/new.toml`; a test keeps the two equal. A
  setting that changes a written file must change the song's `Plan`.
- **Formats:** `songs.toml` and `state.json` carry a `version`; changing
  what an existing key means is a new version, adding a key is not.
- **Lints:** clippy pedantic and `unsafe_code = "deny"` for the
  workspace; allow one narrowly, with the reason beside it.

## Versioning

One version, in `Cargo.toml`, following Semantic Versioning for what a
user sees: the command line, exit codes, output, the song list's format
and settings, and the library muman writes. While the version is 0.x, a
breaking change raises the minor version and anything else the patch.
Refactors, tests and docs alone are no release.

Each release adds a section to [CHANGELOG.md](CHANGELOG.md), in Keep a
Changelog form, user-facing bullets with a bold lead phrase; changes
since the last release collect under `[Unreleased]`.

## Keeping docs in step

| Change | Update in the same commit |
|---|---|
| A command, flag, help text or exit code | Run `cargo xtask docs`; the README's tables if a command or code changed |
| A setting added, removed or renamed | `settings.rs`, `manifest/new.toml`, [docs/configuration.md](docs/configuration.md) |
| A template variable or filter | `template.rs` docs, [docs/configuration.md](docs/configuration.md) |
| How sources, picking, lookups, editing or hooks behave for a user | [docs/guide.md](docs/guide.md) |
| A module's design or a constant's reason | That module's `//!` docs |
| A module added, removed or renamed | The layout above |
| A release | `Cargo.toml` version, [CHANGELOG.md](CHANGELOG.md) |

## Commits and pull requests

- **Subject:** `<scope>: <imperative summary>`, lowercase, no period,
  at most 72 characters; the scope is the module or area (`naming`,
  `ci`, `docs`).
- **Body:** why before what, wrapped at 72; then `-` bullets led by the
  file or area changed. Do not narrate how the change was made.
- **Pull requests:** the title is the subject; open them ready for
  review, not as drafts.
