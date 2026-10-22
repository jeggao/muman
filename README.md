# muman

muman (Music Manager) keeps a music library in step with a song list
you can read and edit by hand.

Each song in the list may have several sources: what
[yt-dlp](https://github.com/yt-dlp/yt-dlp) fetched from YouTube or
YouTube Music, files you dropped into a folder, lyrics from
[LRCLIB](https://lrclib.net) and tags from
[MusicBrainz](https://musicbrainz.org), found by name or by fingerprint
on [AcoustID](https://acoustid.org). muman measures every source and picks
the best audio, cover, lyrics and tags by what it measures — real
bandwidth and clipping, how much of a cover is picture and how sharp,
timed lyrics over untimed — never by where a source came from. Each song
is then written once into your library as a tagged file in the codecs
you choose, with its cover and lyrics, at a path you choose with a
template.

- **The song list is the library's source of truth.** Edit it, run
  `muman sync`, and the library follows: songs added, rewritten where
  their sources changed, moved where their path changed, and removed
  when they leave the list.
- **muman only touches what it wrote.** Anything else in the library
  folder is left alone, a file you changed since muman wrote it is not
  overwritten, and `muman undo` puts back what the last run changed.
- **Duplicates find each other.** A new source that is the same
  recording as a listed song, by its audio fingerprint, joins that song
  rather than becoming a second one.
- **It runs on Linux, macOS and Windows**, and names files so a library
  copies between them unchanged.

## Install

muman needs [ffmpeg](https://ffmpeg.org) 5.1 or newer (with ffprobe),
[yt-dlp](https://github.com/yt-dlp/yt-dlp) 2025.03.21 or newer to
fetch from the web, and [aria2](https://aria2.github.io) 1.35 or newer
to fetch torrents. A run stops with exit code 2 on an older one, naming
it. Debian's and Ubuntu's own yt-dlp packages are older, and YouTube
breaks a yt-dlp more than a few months old, so install it from
[PyPI](https://pypi.org/project/yt-dlp/) with pipx and keep it updated
with `pipx upgrade yt-dlp`.

| Platform | Install the tools |
|---|---|
| Debian, Ubuntu | `sudo apt install ffmpeg aria2 pipx && pipx install yt-dlp` |
| Fedora | `sudo dnf install yt-dlp aria2`, and ffmpeg from [RPM Fusion](https://rpmfusion.org) |
| Arch | `sudo pacman -S ffmpeg yt-dlp aria2` |
| macOS | `brew install ffmpeg yt-dlp aria2` |
| Windows | `winget install Gyan.FFmpeg yt-dlp.yt-dlp aria2.aria2`, or `scoop install ffmpeg yt-dlp aria2` |

`muman info` prints the version and path of each it found.

Then install muman from [crates.io](https://crates.io/crates/muman)
with a [Rust toolchain](https://rustup.rs):

```bash
cargo install --locked muman
```

The same command updates it to the latest release. Each release also
has a built `muman` for Linux, macOS and Windows on its [releases
page](https://github.com/jeggao/muman/releases); a `ffmpeg`, `ffprobe`
or `yt-dlp` placed beside it is used before one on `PATH`. The
release's `SHA256SUMS` lists each archive's checksum, and `gh attestation verify
<archive> --repo jeggao/muman` checks that the release workflow built
it. To build what is on `main` since the last release instead:

```bash
cargo install --locked --git https://github.com/jeggao/muman
```

`muman completions <SHELL>` writes a script that completes muman's
commands, flags and values in bash, zsh, fish, PowerShell or elvish;
`muman completions --help` says how to load it.

### Nix

With [Nix](https://nixos.org), the flake builds muman with ffmpeg,
yt-dlp and aria2 on Linux and on Macs with Apple silicon, so none needs
installing:

```bash
nix run github:jeggao/muman -- info
nix profile add github:jeggao/muman
```

These build `main`; name a release's tag, as in
`github:jeggao/muman/v<version>`, to build that release.

It names its ffmpeg, ffprobe, yt-dlp and aria2c in `MUMAN_FFMPEG`,
`MUMAN_FFPROBE`, `MUMAN_YT_DLP` and `MUMAN_ARIA2C` only where those are unset, so a
program you name there still wins. For NixOS or Home Manager, add the
flake's `overlays.default` and install `pkgs.muman`. Without flakes,
`nix-build` in a checkout builds the same package, from the nixpkgs
`flake.lock` pins. The package installs completion for bash, zsh and
fish.

## Quick start

```bash
muman add "https://www.youtube.com/watch?v=vid00000001"
muman add ~/Downloads/some-album/
muman status
muman sync
```

`add` fetches what a URL names, or copies your files into muman's
sources, lists each as a song, and syncs. It also looks songs up on
YouTube, YouTube Music, LRCLIB, MusicBrainz, AcoustID and the Cover Art
Archive, sending AcoustID your files' fingerprints; `--offline` makes no request, and
`enabled = false` under a `[providers.<name>]` turns one off for good
([making the library](docs/library.md#lookups-and-providers) has the names).
`muman sync` on a new home writes the song list, every setting in it,
to edit before the first `add`. `status` says what a sync
would write and why each song's audio, cover and lyrics come from where
they do; `sync` writes it. `muman info` shows where the song list and
the library are on this machine.

The song list, `songs.toml`, lists every song and its sources. Open it
in an editor, or change songs from the command line:

```bash
muman list artist:"Marlo Venn"
muman set artist:"Marlo Venn" genre=Ambient
muman edit album:"Lantern Weather"
muman remove title:"Rough Demo"
muman undo
```

## Commands

| Command | Does |
|---|---|
| `add` | Fetch what URLs name or copy in files, list each as a song or as a source of a listed one, give lyrics, pictures and tag files to their songs, then sync |
| `check` | Compare the library and the sources with what muman recorded |
| `completions` | Write a script that completes muman's commands and flags in a shell |
| `duplicates` | List songs listed apart that are one recording, by fingerprint |
| `edit` | Edit the songs a query matches in your editor |
| `export` | Write the song list and the library into a zip or a folder, under other settings or fitted to a size if given |
| `info` | Count what the library holds and what could be better, from what earlier runs measured |
| `list` | List the songs a query matches |
| `purge` | Delete fetched sources and lookup records no song uses |
| `remove`, `restore` | Take songs out of the list, keeping a record so nothing adds them back; and list them again |
| `set` | Set tags, pins or the lyrics offset on the songs a query matches |
| `status` | Say what a sync would do, changing nothing |
| `sync` | Fetch what is missing, look up what is due, and bring the library in line with the list |
| `undo` | Put the song list and the library back as they were before the last run that changed them |

Every flag is in the [command reference](docs/cli.md).

| Exit code | Means |
|---|---|
| 0 | Done, or nothing needed doing, or a change was declined |
| 2 | ffmpeg, ffprobe, yt-dlp or aria2c is missing, does not run or is older than muman runs, the system names no home folder, or the command line does not parse or gives a setting that does not read |
| 4 | Something failed: a download, a song, reading or writing the song list or state, a song an export left out, or a problem `check` found |
| 5 | A query for a change matched no song or did not read, a change needs a terminal, `-y` or `--all`, a file `add` was given matched no song for sure, a change or `undo` was refused, `--offline` rules out what was asked, or the song list sets commands or servers not trusted, with no terminal to ask on and no `--trust` |
| 130 | Stopped by Ctrl-C or a signal to stop, with every program muman started; a later run resumes what was fetched |

## Documentation

- [Guide](docs/guide.md): the home, the song list and its sources.
- [Making the library](docs/library.md): how the best of each is picked,
  lookups, queries and editing, status, export, hooks.
- [Configuration](docs/configuration.md): folders, programs, the
  settings in `songs.toml`, the path template and safe names.
- [Audio](docs/audio.md): codecs, encoding and loudness.
- [Command reference](docs/cli.md): every command and flag.
- [Changelog](CHANGELOG.md).

## License

[GNU Affero General Public License v3.0](LICENSE).
