# AGENTS.md

How to work in the muman repository: what is where, how to build and
check it, the conventions the code keeps, and the rules every change
follows. muman is one Rust command-line program built from six crates,
one a layer, as the layout shows; its design is documented in the
code, in each module's `//!` docs, and this file routes to them.
[README.md](README.md) says what muman does for a user.

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
  cli.rs            the command line (clap derive), exit codes
  attach.rs         lyrics, pictures and tag files given to `add`, to their songs
  export.rs, redact.rs   the song list and the library into one zip; its secrets replaced, for --redact
  change.rs, editor.rs   list, set, edit, remove, restore
  guided.rs, review.rs   set with a query alone: songs changed field by field; songs in doubt walked
  check.rs, overview.rs            check and info
  duplicates.rs     songs listed apart that are one recording; duplicates/merge.rs, merged
  purge.rs, trust.rs   sources and records no song uses, deleted; commands and servers run once trusted
  tests.rs, tests/attaching.rs   the crate's end-to-end tests, over the fakes
  tests/convergence.rs   random runs, each held to the library one sync builds
  */tests.rs        a module's tests, when they outgrow it
crates/muman-engine/src/   the engine: the state, plans, the library written, runs undone
  album.rs, release.rs   the songs of one album made to agree: one release, spelling, cover
  names.rs          one spelling for each artist across the library
  state.rs          the state: files written, measures, comparisons; state/db.rs, state.db, the
                    store it moves into: schema, transactions; state/json.rs, an old state.json, read once
  acquire.rs        the network phase: yt-dlp listing, fetching, matching; due.rs, what a sync fetches
  identify.rs       new sources to songs, by fingerprint
  lookup.rs         lookups due, each provider asked until a song has what it names
  reconcile.rs      the offline phase, a module a step, in order: measure.rs, keys.rs,
                    damage.rs, plan.rs, decide.rs, owned.rs, write.rs, prune.rs, report.rs
  resolve.rs, vet.rs, levels.rs   one song's facts to its plan; its sources vetted, and its
                    audio's recordings in vet/recordings.rs; songs levelled
  artifact.rs       the part of a plan each library file is made from, and its name
  render.rs         one plan to library files; render/lyrics.rs, render/cover.rs
  outputs.rs, folder.rs   the files beside a song, named and moved; the cover file a folder holds once
  library.rs        the ledger: every move and removal in the library
  fit.rs, limit.rs  songs fitted into a size, least audible loss first; [library] max_size
  history.rs, query.rs   run records and undo; a query held against the songs
  diagnose.rs, rows.rs   what `check` reports and `status` counts; songs shown as rows
crates/muman-config/src/   the config layer: the song list, its settings, editions and hooks
  settings.rs       [library], [lyrics], [audio], [loudness], [cover], [ytdlp],
                    [torrent], [tools], [history], [sites]; naming_tests.rs, its names
  manifest.rs       songs.toml: songs, albums, removed, cleaning, edits; migrate.rs, older editions
  held.rs, hooks.rs   what each source held when its song was built; the [[hook]] commands
  providers.rs, terms.rs   each provider's [providers.*] table; a query's terms
crates/muman-net/src/   the network layer: yt-dlp and aria2c fetches, and lookups
  download.rs, ytdlp_log.rs   yt-dlp's arguments and the files it reports; its output
  aria2.rs          aria2c fetching a torrent's files: its arguments and its lines
  music.rs, plugins.rs   YouTube Music search; yt-dlp plugins shipped in the binary, from assets/yt-dlp/
  lrclib.rs, musicbrainz.rs, acoustid.rs, coverart.rs   the providers looked up
crates/muman-media/src/   the media layer: sources measured, tags, lyrics, names
  similar.rs, picture.rs   names and texts compared in any script; perceptual hashes of covers
  tagfile.rs, cue.rs   tag files and cue sheets read into hand tags
  facts.rs          one source measured; probe.rs, info.rs, ffmpeg.rs
  analysis.rs       the whole of a source's audio, one decode, every analyzer
  integrity.rs      whether a source's audio is all there: cut off or missing
  loudness.rs       EBU R128 loudness and a song's gains
  tags.rs, clean.rs tag offers and the cleaning rules; clean/lyrics.rs, the lyrics'
  detect.rs         values that say nothing of a song: placeholders and watermarks
  genres.rs         the genres a MusicBrainz record gives, spelled as genres/spelled.txt says
  overrides.rs      [[tags.from]]: what one torrent's or channel's sources are not believed for
  parts.rs          a value read as its parts: a title, a credit, an album, a date
  style.rs          the house style: featured artists, versions' brackets, context notes
  lyrics.rs, cues.rs   LRC cleaning and timing; lyrics written as LRC, WebVTT, SubRip, text
  chapters.rs       a source's chapters, in each container's own form
  naming.rs         safe names, through the path template
  quality.rs, align.rs   audio and picture measures; audio compared by loudness envelopes
  codec.rs, ogg.rs  output codecs: names, containers, ffmpeg arguments; an Opus header's gain
  record.rs         the MusicBrainz record kept, and the answer it is read from
crates/muman-core/src/   the lowest layer, which every other module builds on
  units.rs          sizes, bitrates, times, frequencies, shares (uom)
  new.toml          a new home's song list, the reference for every setting
  platform.rs, dirs.rs   default folders per OS, path identity; a run's folders
  relpath.rs, runner.rs   recorded paths, written with / and in NFC; the subprocess seam
  tools.rs          ffmpeg, ffprobe, yt-dlp and aria2c: where found, version, floor
  sites.rs          the song list's [sites]: how each site names and fetches
  torrent.rs        torrent metainfo and magnet links: what a torrent: key names
  given.rs          folders given to `add`: walked and copied, links followed inside
  fingerprint.rs    Chromaprint prints and their comparison
  lang.rs           language tags: their spellings, ISO 639 codes, preferences, roles
  progress.rs, parallel.rs   a long step's progress on stderr; threads over a list
  testing.rs        the fake runner and the fixtures tests share; kill.rs, the kill hooks
  template.rs       the path template; portable.rs, what no system refuses in a name
  store.rs, source.rs, provider.rs, atomic.rs, hash.rs, defaults.rs, errors.rs, ui.rs, http.rs
tests/              the smoke test, yt-dlp and aria2c given muman's command
                    lines, and the architecture's rules over src/
testdata/           tiny audio and picture fixtures; editions/, the song list
                    each earlier edition wrote for a new home
xtask/              cargo xtask comments | commits | docs | layers
packaging/          the Homebrew, Scoop and AUR entries each release fills in
docs/               guide.md, library.md, metadata.md, songs.md, configuration.md, audio.md,
                    cli.md (generated)
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
its own, each layer a crate; `--report` prints each module's fan-in
and the cycles among a crate's modules. The smoke
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
  written out in `new.toml`; a test keeps the two equal. A
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
changing the version and the crates' pins on it in `Cargo.toml`, then
`Cargo.lock`, and dating the changelog section. Merging it tags `main`
as `v<version>`, builds the binaries for each platform and publishes
them with that section as the notes (`.github/workflows/release.yml`);
run that workflow by hand to build a tag made before it existed. Each
crate goes to crates.io by hand until Trusted Publishing is set up there
and the repository variable `PUBLISH_CRATE` is `true`.

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
| A setting added, removed or renamed, or a default changed | `settings.rs`, `new.toml`, [docs/configuration.md](docs/configuration.md), or [docs/audio.md](docs/audio.md) for `[audio]` and `[loudness]`; a rename or a changed default, an edition in `migrate.rs` and the `new.toml` it replaces as `testdata/editions/<N>.toml` |
| A template variable or filter | `template.rs` docs, [docs/configuration.md](docs/configuration.md) |
| How the song list or sources behave for a user | [docs/guide.md](docs/guide.md) |
| How picking, lookups, status, export or hooks behave for a user | [docs/library.md](docs/library.md), or [docs/metadata.md](docs/metadata.md) for tags and cleaning |
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
- `cargo xtask commits` checks these rules on a pull request's commits,
  title and description, and each push to `main`; only a broken title
  fails, and the rest warns.
