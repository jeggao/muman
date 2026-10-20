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
1. **The rules here and the style guides change only when asked.**
   Propose a rule change in the pull request instead. The layout and
   the checklist below describe the code and change with it, in the
   same commit; `cargo xtask docs` fails on a module the layout lacks.

## Layout

```text
src/
  main.rs, lib.rs   entry point; Job, run() and the run_with() test seam
  add.rs, sync.rs   the add and sync commands, and the network steps they share
  cli.rs            the command line (clap derive), exit codes
  settings.rs       [library], [audio], [loudness], [quality], [ytdlp], [tools], [history]
  migrate.rs        older song lists brought up to this edition
  units.rs          sizes, bitrates, times, frequencies, shares (uom)
  manifest.rs       songs.toml: songs, albums, removed, cleaning, edits
  manifest/new.toml a new home's song list, the reference for every setting
  state.rs          state.json: files written, measures, comparisons
  platform.rs       default folders per OS, path identity, arrival times
  dirs.rs           the home and the library folders, neither inside the other
  relpath.rs        recorded paths, written with / and in NFC
  runner.rs         the subprocess seam
  tools.rs          ffmpeg, ffprobe and yt-dlp: where found, version, floor
  acquire.rs        the network phase: yt-dlp listing, fetching, matching
  sites.rs          the song list's [sites]: how each site names and fetches
  torrent.rs        torrent metainfo and magnet links: what a torrent: key names
  download.rs       yt-dlp's arguments and the files it reports
  ytdlp_log.rs      yt-dlp's output, line by line: its errors, warnings and progress
  music.rs          YouTube Music search and name matching
  plugins.rs        yt-dlp postprocessors shipped in the binary
  identify.rs       new sources to songs, by fingerprint
  attach.rs         lyrics, pictures and tag files given to `add`, to their songs
  similar.rs        names and texts compared in any script
  picture.rs        perceptual hashes of covers
  tagfile.rs, cue.rs   tag files and cue sheets read into hand tags
  lookup.rs         lookups due by trigger; provider.rs, lrclib.rs,
                    musicbrainz.rs, acoustid.rs, coverart.rs
  reconcile.rs      the offline phase: measure, compare, plan, render, prune
  facts.rs          one source measured; probe.rs, info.rs, ffmpeg.rs
  analysis.rs       the whole of a source's audio, one decode, every analyzer
  integrity.rs      whether a source's audio is all there: cut off or missing
  loudness.rs       EBU R128 loudness, album pooling and the gains of a song
  held.rs           what each source held when its song was built
  quality.rs        audio and picture measures
  fingerprint.rs    Chromaprint prints and their comparison
  align.rs          audio comparison by loudness envelopes
  resolve.rs        one song's facts to the plan it is written from
  tags.rs, clean.rs tag offers and the cleaning rules
  lyrics.rs         subtitle languages, LRC cleaning and timing
  chapters.rs       a source's chapters, in each container's own form
  naming.rs         safe names; template.rs, the path template
  codec.rs          output codecs: names, containers, ffmpeg arguments
  ogg.rs            the output gain an Opus file's header holds
  render.rs         one plan to library files
  library.rs        the ledger: every move and removal in the library
  fit.rs            songs fitted into a size, least audible loss first
  limit.rs          the library kept under [library] max_size
  export.rs         the song list and the library into one zip
  history.rs        run records and undo
  change.rs, editor.rs, query.rs   list, set, edit, remove, restore
  check.rs, overview.rs            check and info
  duplicates.rs     songs listed apart that are one recording
  purge.rs          fetched sources and records no song uses, deleted
  progress.rs       how far a long step has got, on stderr
  parallel.rs       a fixed number of threads over a list, for subprocess-bound steps
  hooks.rs, store.rs, source.rs, atomic.rs, ui.rs, http.rs, ...
  testing.rs        the fake runner and the fixtures every test shares
  tests.rs, tests/attaching.rs   the crate's end-to-end tests, over the fakes
  */tests.rs        a module's tests, when they outgrow it
tests/              the smoke test, yt-dlp given muman's command line, and
                    the architecture's rules over src/
testdata/           tiny Opus, FLAC and PNG fixtures
assets/yt-dlp/      the postprocessors plugins.rs embeds
xtask/              cargo xtask comments | docs
packaging/          the Homebrew, Scoop and AUR entries each release fills in
docs/               guide.md, library.md, configuration.md, cli.md (generated)
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
cargo xtask commits --diff-base origin/main
cargo xtask docs
cargo test --test smoke -- --ignored
```

`cargo xtask docs` regenerates [docs/cli.md](docs/cli.md) and checks
links and the changelog; `--check` only checks, as CI does. The smoke
test runs muman end to end against a real ffmpeg on generated audio;
`cargo test --test ytdlp -- --ignored` hands a real yt-dlp the command
line muman builds, against a file served from the test itself. Runs
against real recordings and sites are made by hand, as
[tests/stress.md](tests/stress.md) records; a run's findings go there
and into the pull request that fixes them.

CI (`.github/workflows/ci.yml`) runs clippy, the tests and the smoke
and yt-dlp tests on Linux, macOS and Windows, plus rustfmt, rustdoc, the minimum
Rust version, `cargo xtask comments` (with `--diff-base` on pull
requests), `cargo xtask commits`, `cargo xtask docs --check` and
cargo-deny, all on the Rust `RUST_TOOLCHAIN` in `ci.yml` names. Each
week `toolchain-preview.yml` runs the same checks on the newest stable;
when it fails, one pull request raises the pin, in `ci.yml` and
`release.yml`, and meets the new lints. Dependabot opens the week's
minor and patch bumps as one pull request and each major alone;
`flake-update.yml` moves `flake.lock` the same way.

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
  written out in `manifest/new.toml`; a test keeps the two equal. A
  renamed setting, or one given a unit, and a changed default each add
  a step to a new edition in `migrate::HISTORY`, raising
  `migrate::EDITION`. A setting that changes a written file must change the
  song's `Plan`.
- **Formats:** `songs.toml` and `state.json` carry a `version`; changing
  what an existing key means is a new version, adding a key is not.
- **Lints:** clippy pedantic and `unsafe_code = "deny"` for the
  workspace, the three cast lints off, as `Cargo.toml` says why.
  `#[expect]` one lint narrowly, with the reason in a `//` comment
  above it; `#[allow]` only where the lint fires on some platforms
  alone.

## Versioning

One version, in `Cargo.toml`, following Semantic Versioning for what a
user sees: the command line, exit codes, output, the song list's format
and settings, and the library muman writes. While the version is 0.x, a
breaking change raises the minor version and anything else the patch.
Refactors, tests and docs alone are no release.

Each release adds a section to [CHANGELOG.md](CHANGELOG.md), in Keep a
Changelog form, user-facing bullets with a bold lead phrase; changes
since the last release collect under `[Unreleased]`.

A release is a pull request of its own, `release: muman <version>`,
changing the version in `Cargo.toml` and `Cargo.lock` and dating the
changelog section. Merging it tags `main` as `v<version>`, builds the
binaries for each platform and publishes them with that section as the
notes (`.github/workflows/release.yml`); run that workflow by hand to
build a tag made before it existed. The crate goes to crates.io by hand
until Trusted Publishing is set up there and the repository variable
`PUBLISH_CRATE` is `true`.

The release also reaches each package manager set up for it below,
through the repository variable and secret its row names. Until then
that package manager is skipped.

| Package manager | Set up once | Variable | Secret |
|---|---|---|---|
| Homebrew | A tap repository, such as `jeggao/homebrew-tap` | `HOMEBREW_TAP`, its `owner/name` | `PACKAGES_TOKEN`, a fine-grained token that may write the tap's and the bucket's contents |
| Scoop | A bucket repository, which may be the tap | `SCOOP_BUCKET`, its `owner/name` | `PACKAGES_TOKEN` |
| AUR | An AUR account with an SSH key | `PUBLISH_AUR`, `true` | `AUR_SSH_KEY`, the key's private half |
| winget | A fork of `microsoft/winget-pkgs`, and the first version submitted by hand with `komac new` | `WINGET_PACKAGE`, the package identifier | `WINGET_TOKEN`, a classic token with `public_repo` |

## Keeping docs in step

| Change | Update in the same commit |
|---|---|
| A command, flag, help text or exit code | Run `cargo xtask docs`; the README's tables if a command or code changed |
| A setting added, removed or renamed, or a default changed | `settings.rs`, `manifest/new.toml`, [docs/configuration.md](docs/configuration.md); a rename or a changed default, an edition in `migrate.rs` |
| A template variable or filter | `template.rs` docs, [docs/configuration.md](docs/configuration.md) |
| How the song list or sources behave for a user | [docs/guide.md](docs/guide.md) |
| How picking, lookups, editing, status, export or hooks behave for a user | [docs/library.md](docs/library.md) |
| A module's design or a constant's reason | That module's `//!` docs |
| A module added, removed or renamed | The layout above |
| A release | `Cargo.toml` version, [CHANGELOG.md](CHANGELOG.md), in a pull request of their own; the merge tags and publishes it |

## Commits and pull requests

- **Subject:** `<scope>: <imperative summary>`, lowercase, no period,
  at most 72 characters; the scope is the module or area (`naming`,
  `ci`, `docs`). The ` (#123)` a squash merge appends is not counted.
- **Body:** why before what, wrapped at 72; then `-` bullets led by the
  file or area changed. Do not narrate how the change was made, and do
  not paste a branch's commit subjects as the body.
- **Pull requests:** the title is the subject and the description the
  body; a squash merge writes them to `main` as they are, so both follow
  the rules above. Open them ready for review, not as drafts. One pull
  request is one change, the one a changelog bullet or a few describe;
  one over about a thousand changed lines is split before review, a
  release pull request apart.
- **Merging:** squash and merge, every time. A rebase merge puts a
  branch's authors and half-made commits on `main`.
- `cargo xtask commits` checks the subject and body rules; CI runs it
  on a pull request's commits and on its title and description, and on
  each push to `main`.
