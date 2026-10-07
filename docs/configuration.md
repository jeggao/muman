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
| `--progress` | `MUMAN_PROGRESS` | How a long step shows how far it has got: `auto`, `plain`, `json` or `none`; see the [guide](guide.md#how-muman-works) |
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

A new home's `songs.toml` starts with every setting written out at its
default: [`src/manifest/new.toml`](../src/manifest/new.toml) is that
file and the reference for every key. Change the values you want. A
song list made by an earlier muman gains each setting it lacks, at its
default, the next time muman writes it. A setting with no default, such
as the library folder, stays a comment until you set it. A key a
settings table does not know is an error that names it, so a
misspelling never goes unnoticed.

A setting that is a quantity takes its unit, written after the number,
the space optional:

| Kind | Such as | Settings |
|---|---|---|
| Size | `"32 GiB"`, `"700 MB"`, `"4K"` | `[library] max_size`, `block_size`, `[history] max_size` |
| Bitrate | `"160 kb/s"`, `"160 kbps"`, `"160k"` | `[audio] *_bitrate` |
| Time | `"14 days"`, `"2 s"`, `"500 ms"`, `"2 weeks"` | `[quality.purity] step`, `[ytdlp] keep_partial`, `[providers.*] recheck`, a song's `lyrics_offset` |
| Frequency | `"500 Hz"`, `"1.5 kHz"` | `[quality.bandwidth] step` |
| Share | `"1 %"`, or `0.01` | `[quality.stereo] incoherence`, `[quality.clipping] cutoffs`, `[quality.resolution] step` |

Every unit is also known by its name, singular or plural, as
`"2 gibibytes"` or `"3 hours"`. `K`, `M`, `G` and `T` alone are binary
sizes, as `KiB`; `KB`, `MB` and `GB` are decimal. A size counts bytes
without a unit, a frequency hertz and a share a fraction; a bitrate or
a time has no bare number.

`edition`, beside `version`, names the settings' names, units and
defaults the file was written to. A song list of an earlier edition
reads as the current one: a setting named the old way, such as
`opus_kbps = 160`, reads as its new name and unit, `opus_bitrate =
"160 kb/s"`, and the next `sync` writes it so, its comments kept;
`status` and `check` name each one meanwhile. Naming a setting both
ways is an error. When a later muman changes a default, `sync`,
`status` and `check` name each setting still at the old one, however
it is spelled, and `muman sync --update-defaults` moves those to the
new default and raises `edition`. A setting you changed stays as you
set it. A list without `edition` counts as edition 1; one of a later
edition than this muman knows reads while it sets nothing it does not
know.

| Table | Sets |
|---|---|
| `[library]` | The library folder, the path template, how names are made safe, length limits, fallback names, where lyrics go, the most the library may take |
| `[audio]` | The codecs copied as they are, what the rest is encoded to, each encoder's bitrate, the lowest bitrate a size limit lowers to |
| `[quality.*]` | How sources are ranked: each measure switched on or off, its weight, and its steps and cutoffs |
| `[ytdlp]` | yt-dlp's format, subtitle languages, fragments, extra arguments, muman's postprocessors, how long partial downloads are kept |
| `[history]` | How many changing runs `undo` keeps, and how much space their replaced files may take together |

The song list's own tables — `[defaults]` (lyrics languages),
`[clean.*]` (tag cleaning), `[providers.*]`, `[[trigger]]` and
`[[hook]]` — are described in the [guide](guide.md).

A setting that changes what a song file holds, such as a bitrate, a
codec, a measure's weight or where lyrics go, makes the next `sync`
write the songs it affects again. A setting that changes only where a song lands, such as the
template, moves the existing files instead of encoding them again.

## Codecs

`[audio] codecs` lists the codecs a song's audio is copied in, packet
for packet: `opus`, `vorbis`, `aac`, `mp3`, `flac` and `alac`. Audio in
any other codec is encoded, lossy audio to `lossy` and lossless audio to
`lossless`, at that codec's bitrate; audio already in the codec it would
be encoded to is copied. Each codec has its own file:

| Codec | File | Tags |
|---|---|---|
| `opus` | `.opus` (Ogg) | Vorbis comments |
| `vorbis` | `.ogg` (Ogg) | Vorbis comments |
| `flac` | `.flac` | Vorbis comments |
| `mp3` | `.mp3` | ID3v2.4 |
| `aac`, `alac` | `.m4a` (MP4) | iTunes atoms |

In MP3 and MP4, a tag those formats name, such as the title, the track
or the lyrics, becomes their own frame or atom; any other is kept under
its name, as a `TXXX` frame or an iTunes freeform atom.

```toml
[audio]
# Keep what YouTube and most stores serve; make the rest AAC and ALAC,
# which Apple devices play.
codecs = ["opus", "aac", "mp3", "alac"]
lossy = "aac"
lossless = "alac"
```

Copying never loses anything, while every lossy encode costs a
generation, so list every codec your players read. Setting `lossless`
to a lossy codec makes a library with no lossless files.

## How sources are ranked

Each measure of [how the best of each is picked](guide.md#how-the-best-of-each-is-picked)
has a table under `[quality]`: `[quality.purity]`, `bandwidth`, `stereo`
and `clipping` rank audio, `[quality.square]`, `resolution` and
`blockiness` rank covers. In each, `enabled` and `weight` say whether and
how much it counts, and the other keys how it is cut into steps. A
source's steps times their weights are summed, and the lowest sum wins.

```toml
# Mono costs as much as 2.5 kHz of bandwidth: 50 against 10 for each
# 500 Hz. Clipping counts for nothing.
[quality.bandwidth]
weight = 10
step = "500 Hz"
[quality.stereo]
weight = 50
[quality.clipping]
enabled = false
```

The default weights, in [`new.toml`](../src/manifest/new.toml), make
each measure outweigh all after it, so their order alone decides; weigh
a measure against the steps of those it should trade with.

## Library size

`[library] max_size` caps what the files muman writes may take, such as
`"32 GiB"` for a card. Each `sync` fits the library under it: a song
that does not fit at its best is written at a lower bitrate of
`[audio] lossy`, never below `[audio] min_bitrate`. The songs lowered are
those that lose the least audible quality for each byte saved: lossless
songs first, which an encoder at a high bitrate loses nothing of, and
lossy songs last, since encoding them again costs a generation.

Which songs are lowered, and how far, follows from the song list, the
sources, the settings and the ffmpeg in use, never from what the library
held before: a library fitted song by song ends exactly as one fitted at
once, and two homes with the same song list write the same files. That
has a cost in time:

- Adding a song can lower others, and removing one or raising the limit
  raises them again.
- A song is rendered to learn exactly what it takes before it counts
  toward the limit. muman keeps what it learns in `state.json`, so a
  `sync` that changes nothing renders nothing, and a song rendered to be
  measured is moved into the library, not rendered twice.

Each file counts as whole `block_size` blocks, so set `block_size` to the
allocation unit of the filesystem the library is copied to: `"32 KiB"` on
a FAT32 card counts every small `.lrc` as 32 KiB, as the card does.

When even the lowest bitrates cannot fit, `sync` says how much the
library needs and leaves songs out, the last listed first, until the
rest fit; their files are removed and the run exits with 4. `status`
shows each lowered song's format and why, and what the library is
projected to take; `info` shows what it takes against `max_size`. A song
you changed since muman wrote it counts as it is and is never encoded
again, and files in the library that muman did not write are not
counted.

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
bytes of UTF-8, which no filesystem counts more strictly, and never
inside a character as a reader sees one, such as a flag or a letter with
its accent; a name keeps
room for what tells it apart, so the least they take is 40 and 16. Windows
programs and some players also stop at 260 characters for a whole path;
`max_path` cuts each title so the full path, library folder included,
stays within it, and a sync warns of the songs it cannot fit.
