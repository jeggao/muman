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
   Propose a rule change in the pull request instead. The layout, the
   checklist and the files and formats the conventions name describe
   the code and change with it, in the same commit; what a convention
   asks does not. `cargo xtask docs` fails on a module the layout lacks.

## Layout

```text
src/
  main.rs, lib.rs   entry point; Job, run() and the run_with() test seam
  add.rs, sync.rs   the add and sync commands, and the network steps they share
  album.rs          the songs of one album made to agree: one edition, spelling, release
  cli.rs            the command line (clap derive), exit codes
  settings.rs       [library], [lyrics], [audio], [loudness], [cover], [ytdlp],
                    [torrent], [tools], [history], [sites]
  migrate.rs        older song lists brought up to this edition
  units.rs          sizes, bitrates, times, frequencies, shares (uom)
  manifest.rs       songs.toml: songs, albums, removed, cleaning, edits
  manifest/new.toml a new home's song list, the reference for every setting
  state.rs          the state: files written, measures, comparisons
  state/json.rs     state.json, as an earlier muman wrote it, read once
  state/db.rs       state.db, the store the state moves into: schema, transactions
  platform.rs       default folders per OS, path identity, arrival times
  dirs.rs           the home and the library folders, neither inside the other
  relpath.rs        recorded paths, written with / and in NFC
  runner.rs         the subprocess seam
  tools.rs          ffmpeg, ffprobe, yt-dlp and aria2c: where found, version, floor
  acquire.rs        the network phase: yt-dlp listing, fetching, matching
  sites.rs          the song list's [sites]: how each site names and fetches
  torrent.rs        torrent metainfo and magnet links: what a torrent: key names
  download.rs       yt-dlp's arguments and the files it reports
  aria2.rs          aria2c fetching a torrent's files: its arguments and its lines
  ytdlp_log.rs      yt-dlp's output, line by line: its errors, warnings and progress
  music.rs          YouTube Music search and name matching
  plugins.rs        yt-dlp plugins shipped in the binary
  identify.rs       new sources to songs, by fingerprint
  attach.rs         lyrics, pictures and tag files given to `add`, to their songs
  given.rs          folders given to `add`: walked and copied, links followed inside
  similar.rs        names and texts compared in any script
  picture.rs        perceptual hashes of covers
  tagfile.rs, cue.rs   tag files and cue sheets read into hand tags
  lookup.rs         lookups due, each provider asked until a song has what it names;
                    provider.rs, providers.rs, lrclib.rs, musicbrainz.rs, acoustid.rs, coverart.rs
  reconcile.rs      the offline phase, its steps in order, each a module of its own:
                    measure.rs, keys.rs, damage.rs, plan.rs, decide.rs, owned.rs,
                    write.rs, prune.rs, report.rs
  facts.rs          one source measured; probe.rs, info.rs, ffmpeg.rs
  analysis.rs       the whole of a source's audio, one decode, every analyzer
  integrity.rs      whether a source's audio is all there: cut off or missing
  loudness.rs       EBU R128 loudness, album pooling and the gains of a song
  held.rs           what each source held when its song was built
  quality.rs        audio and picture measures
  fingerprint.rs    Chromaprint prints and their comparison
  align.rs          audio comparison by loudness envelopes
  resolve.rs        one song's facts to the plan it is written from
  artifact.rs       the part of a plan each library file is made from, and its name
  tags.rs, clean.rs tag offers and the cleaning rules; clean/lyrics.rs, the lyrics'
  lang.rs           language tags: their spellings, ISO 639 codes, preferences, roles
  lyrics.rs         subtitle languages, LRC cleaning and timing
  cues.rs           lyrics written as LRC, WebVTT, SubRip or plain text
  chapters.rs       a source's chapters, in each container's own form
  naming.rs         safe names; template.rs, the path template
  codec.rs          output codecs: names, containers, ffmpeg arguments
  ogg.rs            the output gain an Opus file's header holds
  render.rs         one plan to library files; render/lyrics.rs, render/cover.rs
  outputs.rs        the files beside a song: their names, and where a move takes them
  folder.rs         the cover file a folder holds once, and the song that holds it
  library.rs        the ledger: every move and removal in the library
  fit.rs            songs fitted into a size, least audible loss first
  limit.rs          the library kept under [library] max_size
  export.rs         the song list and the library into one zip
  redact.rs         the song list with its secrets replaced, for export --redact
  history.rs        run records and undo
  change.rs, editor.rs, query.rs, terms.rs   list, set, edit, remove, restore
  guided.rs         set with a query alone: songs changed field by field
  rows.rs           songs shown as aligned rows that tell them apart
  check.rs, overview.rs            check and info
  duplicates.rs     songs listed apart that are one recording; duplicates/merge.rs, merged
  purge.rs          fetched sources and records no song uses, deleted
  progress.rs       how far a long step has got, on stderr
  parallel.rs       a fixed number of threads over a list, for subprocess-bound steps
  trust.rs          the song list's commands and servers, run only once trusted
  hooks.rs, store.rs, source.rs, atomic.rs, hash.rs, defaults.rs, errors.rs, ui.rs, http.rs, ...
  testing.rs        the fake runner and the fixtures every test shares
  tests.rs, tests/attaching.rs   the crate's end-to-end tests, over the fakes
  tests/convergence.rs   random runs, each held to the library one sync builds
  */tests.rs        a module's tests, when they outgrow it
tests/              the smoke test, yt-dlp and aria2c given muman's command
                    lines, and the architecture's rules over src/
testdata/           tiny audio and picture fixtures; editions/, the song list
                    each earlier edition wrote for a new home
assets/yt-dlp/      the plugins plugins.rs embeds
xtask/              cargo xtask comments | commits | docs | layers
packaging/          the Homebrew, Scoop and AUR entries each release fills in
docs/               guide.md, library.md, songs.md, configuration.md, audio.md, cli.md (generated)
```

A module's `//!` docs hold its design and the reasons for its constants.

## Commands

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo xtask comments
cargo xtask commits --diff-base origin/main
cargo xtask docs
cargo xtask layers
cargo test --test smoke -- --ignored
```

`cargo xtask docs` regenerates [docs/cli.md](docs/cli.md) and checks
links and the changelog; `--check` only checks, as CI does. `cargo
xtask layers` checks that no module names one of a higher layer than
its own but as `xtask/src/layers.rs` lists; `--report` prints the
upward references and the cycles. The smoke
test runs muman end to end against a real ffmpeg on generated audio;
`cargo test --test ytdlp -- --ignored` hands a real yt-dlp the command
line muman builds, against a file served from the test itself, and
`cargo test --test torrent -- --ignored` a real aria2c its own, against
a tracker and a seeder of the test's. Runs
against real recordings and sites are made by hand.

CI (`.github/workflows/ci.yml`) runs clippy, the tests and the smoke,
yt-dlp and torrent tests on Linux, macOS and Windows, plus rustfmt,
rustdoc, the minimum Rust version, `cargo xtask comments` (with `--diff-base` on pull
requests), `cargo xtask commits`, `cargo xtask docs --check`,
`cargo xtask layers` and cargo-deny, all on the Rust `RUST_TOOLCHAIN` in
`ci.yml` names, and `nix flake check` on Linux and macOS. A pull request
that changes only Markdown and `docs/` builds and tests nothing, and one
runs `nix flake check` only when it changes the flake, `nix/`,
`Cargo.toml`, `Cargo.lock` or `ci.yml`; every push to main runs every
check, and only main saves the Rust caches. Each push to main also runs
the random runs of `src/tests/convergence.rs` at 64 cases. Each
week `toolchain-preview.yml` runs the same checks on the newest stable;
when it fails, one pull request raises the pin, in `ci.yml` and
`release.yml`, and meets the new lints. Dependabot opens the week's
minor and patch bumps as one pull request, those of the crates the
measures and lyrics are made with as another, and each major alone;
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
  home folder, 4 a failure, 5 a refused change, 130 stopped by a signal.
  Listed in `cli.rs`, `EXIT_CODES_HELP`.
- **Files:** whole-file writes through `atomic::write`; renames and
  deletes in the library through `atomic::rename` and `atomic::remove`,
  which wait out a file Windows reports as held open.
- **Settings:** a new setting goes in `settings.rs`, or in the lower
  module that applies it, re-exported from `settings.rs`. Its default is
  written out in `manifest/new.toml`; a test keeps the two equal. A
  renamed setting, or one given a unit, and a changed default each add a
  step to a new edition in `migrate::HISTORY`, raising `migrate::EDITION`.
  A setting that changes a written file must change the song's `Plan`.
- **Formats:** `songs.toml` carries a `version`, `state.db` a schema in
  `user_version`; a changed meaning is a new version, a new key is not.
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
| A setting added, removed or renamed, or a default changed | `settings.rs`, `manifest/new.toml`, [docs/configuration.md](docs/configuration.md), or [docs/audio.md](docs/audio.md) for `[audio]` and `[loudness]`; a rename or a changed default, an edition in `migrate.rs` and the `new.toml` it replaces as `testdata/editions/<N>.toml` |
| A template variable or filter | `template.rs` docs, [docs/configuration.md](docs/configuration.md) |
| How the song list or sources behave for a user | [docs/guide.md](docs/guide.md) |
| How picking, lookups, status, export or hooks behave for a user | [docs/library.md](docs/library.md) |
| How queries, changing songs or duplicates behave for a user | [docs/songs.md](docs/songs.md) |
| A module's design or a constant's reason | That module's `//!` docs |
| A measure, renderer or writer changes what it makes, or a dependency bump changes a digest | Raise its version constant and record the digest its test prints, or, for a bump that leaves saved results valid, re-record the digest and say why |
| A module added, removed or renamed | The layout above, and its layer in `xtask/src/layers.rs` |
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
