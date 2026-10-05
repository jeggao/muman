# Configuration

muman is configured in two places, split by what a setting is about:

- **How the library is made** — its layout and file names, encoding,
  how yt-dlp fetches, how much `undo` keeps — is set in the song list,
  `songs.toml`, so one song list renders the same library on every
  machine that runs it.
- **Where things are on this machine** — the home folder, the library
  folder, and the programs muman runs — comes from command-line flags
  and environment variables.

## Folders and programs

| Flag | Variable | Names |
|---|---|---|
| `--home` | `MUMAN_HOME` | The home: `songs.toml`, `state.json`, the sources and the run history |
| `--library` | `MUMAN_LIBRARY` | The library folder, over the song list's `[library] path` |
| | `MUMAN_FFMPEG` | The ffmpeg to run |
| | `MUMAN_FFPROBE` | The ffprobe to run |
| | `MUMAN_YT_DLP` | The yt-dlp to run; may be a whole command, such as `python -m yt_dlp` |

Without them, each folder is the platform's own:

| Platform | Home | Library |
|---|---|---|
| Linux | `$XDG_DATA_HOME/muman`, usually `~/.local/share/muman` | `$XDG_MUSIC_DIR/muman`, usually `~/Music/muman` |
| macOS | `~/Library/Application Support/muman` | `~/Music/muman` |
| Windows | `%LOCALAPPDATA%\muman\data` | `%USERPROFILE%\Music\muman` |

A program not named in its variable is looked for beside the muman
executable, then on `PATH` (with `PATHEXT` on Windows, so a `.cmd`
wrapper is found). ffmpeg and ffprobe are needed by every command that
reads audio; yt-dlp only by a run that fetches, so a library made only
from your own files never needs it. `muman info` prints the folders in
use.

## Settings in the song list

A new home's `songs.toml` starts with every setting commented out at
its default: [`src/manifest/new.toml`](../src/manifest/new.toml) is
that file and the reference for every key. Uncomment a table and the
keys you change. A key a settings table does not know is an error that
names it, so a misspelling never goes unnoticed.

| Table | Sets |
|---|---|
| `[library]` | The library folder, the path template, how names are made safe, length limits, fallback names, where lyrics go |
| `[audio]` | The Opus bitrates for audio that is encoded rather than copied |
| `[ytdlp]` | yt-dlp's format, subtitle languages, fragments, extra arguments, muman's postprocessors, how long partial downloads are kept |
| `[history]` | How many changing runs `undo` keeps, and how much space their replaced files may take |

The song list's own tables — `[defaults]` (lyrics languages),
`[clean.*]` (tag cleaning), `[providers.*]`, `[[trigger]]` and
`[[hook]]` — are described in the [guide](guide.md).

A setting that changes what a song file holds, such as a bitrate or
where lyrics go, makes the next `sync` write the songs it affects
again. A setting that changes only where a song lands, such as the
template, moves the existing files instead of encoding them again.

## The path template

`[library] template` is a [MiniJinja](https://docs.rs/minijinja)
template: Jinja2 syntax, rendered once per song into its path without
the extension. `/` in the template separates folders.

| Variable | Holds |
|---|---|
| `album`, `album_artist` | The album and its artist, or the fallback names when missing |
| `artist`, `artists` | The first artist, and every artist as a list |
| `date`, `year` | The date as tagged, and its first four digits |
| `disc`, `track` | The disc and track numbers, or none |
| `disc_track` | `03 ` on a first disc, `2-03 ` on a later one, nothing without a track number |
| `genre` | The genre |
| `id` | The ID or file name of the song's audio source |
| `title` | The title, or the fallback name when missing |

Beside MiniJinja's built-in filters (`default`, `lower`, `upper`,
`replace`, `first`, `join`, and more), muman adds:

| Filter | Does |
|---|---|
| `asciify` | Transliterates to ASCII: `Ça` becomes `Ca` |
| `pad(n)` | Pads a number with zeros: `track \| pad(3)` gives `003` |
| `the_suffix` | Moves a leading "The": `The Orchards` becomes `Orchards, The` |
| `truncate(n)` | Keeps the first `n` characters |

Some layouts:

```toml
[library]
# Artist folders, then "2024 - Album", then "03 Title".
template = "{{ album_artist }}/{% if year %}{{ year }} - {% endif %}{{ album }}/{{ disc_track }}{{ title }}"
```

```toml
[library]
# One folder per genre, songs named "Artist - Title".
template = "{{ genre | default('Unsorted', true) }}/{{ artist }} - {{ title }}"
```

Every tag value is made safe before the template sees it, so a `/` in a
title never makes a folder; only the template's own `/` does. A path
that two songs both resolve to is told apart by the second song's
source ID: `Title [vid00000001]`. `muman status` shows each song's path
before anything is written.

## Safe names on every system

Names are made safe the same way on every platform, so a library copies
from one system to another, or to a phone, unchanged. `[library]
restrict` sets how far that goes:

| `restrict` | Replaces |
|---|---|
| `none` | Only `/` and control characters, which no filesystem allows |
| `windows` | Also the characters Windows, FAT and most phones refuse — `\ : * ? " < > \|` — trailing dots and spaces, and reserved device names such as `CON` or `nul.txt` |
| `ascii` | As `windows`, and transliterates everything else to ASCII |

The refused characters become their full-width lookalikes (`：`, `？`,
`⧸`, …), as yt-dlp names its files, so a title stays readable. `[library]
replace` maps characters or strings to replacements of your own,
applied first and over these: `{ ":" = " -" }` turns `A: B` into
`A - B`. A reserved device name gets `_` after it: `CON_.opus`.

File and folder names are cut to `max_name_bytes` and `max_folder_bytes`
bytes of UTF-8, which no filesystem counts more strictly. Windows
programs and some players also stop at 260 characters for a whole path;
`max_path` cuts each title so the full path, library folder included,
stays within it.
