# muman

muman (Music Manager) builds a music library the way a compiler builds a
program. You write `songs.toml`, a declarative list of the songs you
want, and muman works out how to get them. Each song names its sources,
the copies it can be made from: a YouTube upload, its release on YouTube
Music, a CD you ripped, a track in a torrent. `muman sync` is the build,
and the library is its output. Change the list and sync again, and muman
rebuilds only the songs that changed.

Copies of one song are seldom equal, so muman measures what each really
holds: a music video's intro counts against it, a FLAC made from an MP3
measures as narrow as the MP3, and mono copied into two channels counts
as mono. Then it writes each song once, from the best audio, cover,
lyrics and tags among its sources. A collector with three copies of a
record does the same, keeping the cleanest disc, the sharpest sleeve and
the only lyric sheet.

The build is deterministic: one list and its sources make one library on
any machine. It never deletes or writes over a file it did not write.

## What it does

- **Covers and lyrics are measured too.** muman crops a cover out of a
  video frame's bars and ranks it by the detail it holds, so an upscale
  gains nothing. It shifts timed lyrics to fit the audio chosen.
- **It copies before it encodes.** muman copies audio in a codec you
  allow packet for packet. A new tag goes into the file in place, a new
  path template moves files, and loudness goes into ReplayGain tags or
  an Opus file's header, all without encoding the audio again.
- **It knows a recording by its sound.** muman fingerprints each new
  source, which joins the song it is a recording of, whatever its name.
- **Lookups fill the gaps.** muman matches a YouTube upload to its
  release on YouTube Music by aligning their audio. Songs look up
  lyrics on [LRCLIB](https://lrclib.net), tags on
  [MusicBrainz](https://musicbrainz.org), by name or by fingerprint
  through [AcoustID](https://acoustid.org), and covers on the [Cover
  Art Archive](https://coverartarchive.org), each only until a song
  has what you ask for.
- **It fits a library into a size.** Under `[library] max_size`, muman
  encodes smaller first the songs that lose the least audible quality
  per byte. `muman export` writes a zip or a folder under other
  settings, such as every song in MP3 for a car stereo.

## What it promises

- **Your files stay yours.** muman deletes or writes over only the
  files it recorded writing. A song file or `.lrc` you changed since
  stays as you left it, with a warning, until `sync --force`.
- **Nothing is half written.** muman writes each file under a temporary
  name and renames it into place, so a player never sees half a track,
  and a crash leaves the old song list or the new one.
- **Runs can be undone.** `muman undo` puts the song list and the
  library back as they were before the last run that changed them,
  one run further back each time, even a run cut short by a crash.
- **A song keeps what it was built from.** muman records what each
  source gave each song. If a site serves a video re-encoded or
  retitled when muman fetches it again, the song stays as built until
  you run `sync --accept`. `muman check --upstream` asks each site
  whether it still serves what you fetched, so you know which copies
  are the last.
- **Damage is found, not copied.** A download cut short, or with holes,
  ranks below a whole copy, and muman fetches it again.
- **A library copies between systems unchanged.** Its names are safe on
  Linux, macOS, Windows and phones.
- **A song list runs nothing until you trust it.** Settings that run a
  program or reach another server, as hooks and yt-dlp arguments do,
  wait for your yes, so a list someone hands you runs nothing unseen.

## Install

muman needs [ffmpeg](https://ffmpeg.org) 5.1 or newer, with ffprobe.
Fetching from the web also needs [yt-dlp](https://github.com/yt-dlp/yt-dlp)
2025.03.21 or newer, and fetching torrents [aria2](https://aria2.github.io)
1.35 or newer; an older one stops the run with exit code 2. YouTube
breaks a yt-dlp a few months old, and Debian's and Ubuntu's packages
lag behind, so install it there with pipx and keep it current with
`pipx upgrade yt-dlp`.

| Platform | Install the tools |
|---|---|
| Debian, Ubuntu | `sudo apt install ffmpeg aria2 pipx && pipx install yt-dlp` |
| Fedora | `sudo dnf install yt-dlp aria2`, and ffmpeg from [RPM Fusion](https://rpmfusion.org) |
| Arch | `sudo pacman -S ffmpeg yt-dlp aria2` |
| macOS | `brew install ffmpeg yt-dlp aria2` |
| Windows | `winget install Gyan.FFmpeg yt-dlp.yt-dlp aria2.aria2`, or `scoop install ffmpeg yt-dlp aria2` |

Then install muman from [crates.io](https://crates.io/crates/muman)
with a [Rust toolchain](https://rustup.rs). The same command updates it:

```bash
cargo install --locked muman
```

Each [release](https://github.com/jeggao/muman/releases) also has a
built `muman` for Linux, macOS and Windows. Its `SHA256SUMS` lists each
archive's checksum, and `gh attestation verify <archive> --repo
jeggao/muman` checks that the release workflow built it. An ffmpeg,
ffprobe or yt-dlp placed beside `muman` runs before one on `PATH`.

With [Nix](https://nixos.org), `nix profile add github:jeggao/muman`
installs muman with ffmpeg, yt-dlp and aria2 on Linux and Apple silicon
Macs. It builds `main`; `github:jeggao/muman/v<version>` builds a
release. NixOS and Home Manager take the flake's `overlays.default`.

`muman completions --help` says how to load completions for your shell.

## First run

```bash
muman sync
muman add "https://www.youtube.com/watch?v=vid00000001"
muman add ~/rips/lantern-weather/
muman status
```

The first `sync` writes `songs.toml` with every setting at its default,
for you to read before adding songs. `add` fetches what a URL names, or
copies your files into muman's sources, lists each as a song or as a
source of a listed one, and syncs. `status` says what a sync
would write and where each song's audio, cover and lyrics come from.
`muman info` shows where the song list and the library are.

`add` and `sync` look songs up on YouTube, YouTube Music, LRCLIB,
MusicBrainz, AcoustID and the Cover Art Archive, and send AcoustID a
fingerprint of your files' audio. `--offline` sends nothing, and
[lookups](docs/library.md#lookups) says how to turn one service off.

Change songs from the command line, and undo any change:

```bash
muman list artist:"Marlo Venn"
muman set artist:"Marlo Venn" genre=Ambient
muman undo
```

## Commands

| Command | Does |
|---|---|
| `add` | Fetch URLs, torrents or files, list each as a song or a copy of one, give lyrics, pictures and tag files to their songs, then sync |
| `sync` | Fetch what is missing, make the lookups due, and bring the library in line with the song list |
| `status` | Say what a sync would do and why, changing nothing |
| `list` | List the songs a query matches |
| `set` | Set tags, pins or the lyrics offset of songs, or step through them field by field |
| `edit` | Edit songs' entries in your editor |
| `remove`, `restore` | Take songs out of the list, keeping a record so nothing adds them back; list them again |
| `undo` | Put back the song list and the library as they were before the last changing run |
| `duplicates` | Find songs listed apart that are one recording, and merge them |
| `info` | Count what the library holds and what could be better |
| `check` | Compare the library and the sources with what muman recorded |
| `export` | Write the song list and the library into a zip or a folder, under other settings or a size if given |
| `purge` | Delete fetched sources and lookup records no song uses |
| `completions` | Write a shell's completion script |

| Exit code | Means |
|---|---|
| 0 | Done, nothing to do, or a change you declined |
| 2 | A program missing, too old or not running; no home folder; a command line or setting that does not read |
| 4 | A failure: a download, a song, the song list or state, a song an export left out, or a problem `check` found |
| 5 | A refusal: a query that matches nothing, a change with no terminal and no `-y`, a file `add` cannot place, an untrusted song list |
| 130 | Stopped by Ctrl-C or a signal, with every program muman started; the next run resumes the downloads |

The [command reference](docs/cli.md) lists every flag and exit case.

## Documentation

- [Guide](docs/guide.md): the home, the song list and its sources.
- [Making the library](docs/library.md): how the best copy wins,
  lookups, status, export and hooks.
- [Songs](docs/songs.md): queries, changing songs, duplicates.
- [Configuration](docs/configuration.md): folders, programs, settings,
  the path template and safe names.
- [Audio](docs/audio.md): codecs, loudness and the size limit.
- [Changelog](CHANGELOG.md).

## License

[GNU Affero General Public License v3.0](LICENSE).
