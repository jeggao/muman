# muman

muman (Music Manager) keeps a music library in step with a song list
you can read and edit by hand.

Each song in the list may have several sources: what
[yt-dlp](https://github.com/yt-dlp/yt-dlp) fetched from YouTube or
YouTube Music, files you dropped into a folder, and lyrics from
[LRCLIB](https://lrclib.net). muman measures every source and picks
the best audio, cover, lyrics and tags by what it measures — real
bandwidth and clipping, how much of a cover is picture and how sharp,
timed lyrics over untimed — never by where a source came from. Each song
is then written once into your library as a tagged Opus or FLAC file
with its cover and lyrics, at a path you choose with a template.

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

muman needs [ffmpeg](https://ffmpeg.org) (with ffprobe), and
[yt-dlp](https://github.com/yt-dlp/yt-dlp) to fetch from the web.

| Platform | Install the tools |
|---|---|
| Debian, Ubuntu | `sudo apt install ffmpeg yt-dlp` |
| Fedora | `sudo dnf install ffmpeg yt-dlp` |
| Arch | `sudo pacman -S ffmpeg yt-dlp` |
| macOS | `brew install ffmpeg yt-dlp` |
| Windows | `winget install Gyan.FFmpeg yt-dlp.yt-dlp`, or `scoop install ffmpeg yt-dlp` |

Then build muman with a [Rust toolchain](https://rustup.rs):

```bash
cargo install --locked --git https://github.com/jeggao/muman
```

## Quick start

```bash
muman add "https://www.youtube.com/watch?v=vid00000001"
muman add ~/Downloads/some-album/
muman status
muman sync
```

`add` fetches what a URL names, or copies your files into muman's
sources, lists each as a song, and syncs. `status` says what a sync
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
| `add` | Fetch what URLs name or copy in files, list each as a song or as a source of a listed one, then sync |
| `check` | Compare the library and the sources with what muman recorded |
| `edit` | Edit the songs a query matches in your editor |
| `info` | Count what the library holds and what could be better, from what earlier runs measured |
| `list` | List the songs a query matches |
| `remove`, `restore` | Take songs out of the list, keeping a record so nothing adds them back; and list them again |
| `set` | Set tags, pins or the lyrics offset on the songs a query matches |
| `status` | Say what a sync would do, changing nothing |
| `sync` | Fetch what is missing, look up what is due, and bring the library in line with the list |
| `undo` | Put the song list and the library back as they were before the last run that changed them |

Every flag is in the [command reference](docs/cli.md).

| Exit code | Means |
|---|---|
| 0 | Done, or nothing needed doing, or a change was declined |
| 2 | ffmpeg, ffprobe or yt-dlp is missing, or the system names no home folder |
| 4 | Something failed: a download, a song, reading or writing the song list or state, or a problem `check` found |
| 5 | A query matched no song, or a change needs a terminal, `-y` or `--all` |

## Documentation

- [Guide](docs/guide.md): the song list, sources, how the best of each is
  picked, lookups, queries and editing, hooks.
- [Configuration](docs/configuration.md): folders, programs, the
  settings in `songs.toml`, the path template and safe names.
- [Command reference](docs/cli.md): every command and flag.
- [Changelog](CHANGELOG.md).

## License

[GNU Affero General Public License v3.0](LICENSE).
