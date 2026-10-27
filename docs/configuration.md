# Configuration

muman is configured in two places, split by what a setting is about:

- **How the library is made** — its layout and file names, encoding,
  how yt-dlp and aria2c fetch, which programs muman runs, how much `undo` keeps —
  is set in the song list, `songs.toml`, so one song list renders the
  same library on every machine that runs it.
- **Where things are on this machine** — the home folder, the library
  folder, and a program in a place of its own — comes from command-line
  flags and environment variables, which override the song list.

## Folders and programs

| Flag | Variable | Names |
|---|---|---|
| `--home` | `MUMAN_HOME` | The home: `songs.toml`, `state.json`, the sources and the run history |
| `--library` | `MUMAN_LIBRARY` | The library folder, over the song list's `[library] path`. Named anew, each file found at its place there as muman wrote it, the library moved or copied, stays muman's, and the rest is written there; the files in the folder before are left alone, muman's again only as far as they are as written when you go back to it |
| `--offline` | `MUMAN_OFFLINE` | Reach no network: no lookups, no fetching, URLs refused |
| `--progress` | `MUMAN_PROGRESS` | How a long step shows how far it has got: `auto`, `plain`, `json` or `none`; see the [guide](guide.md#how-muman-works) |
| `--trust` | `MUMAN_TRUST` | Trust the commands and servers the song list sets, as it sets them now, without asking: see [commands, servers and secrets](#commands-servers-and-secrets) |
| | `MUMAN_FFMPEG` | The ffmpeg to run, over `[tools.ffmpeg] command` |
| | `MUMAN_FFPROBE` | The ffprobe to run, over `[tools.ffprobe] command` |
| | `MUMAN_YT_DLP` | The yt-dlp to run, over `[tools.yt-dlp] command`; may be a whole command, such as `python -m yt_dlp` |
| | `MUMAN_ARIA2C` | The aria2c to run, over `[tools.aria2c] command` |

Without them, each folder is the platform's own:

| Platform | Home | Library |
|---|---|---|
| Linux | `$XDG_DATA_HOME/muman`, usually `~/.local/share/muman` | `$XDG_MUSIC_DIR/muman`, usually `~/Music/muman` |
| macOS | `~/Library/Application Support/muman` | `~/Music/muman` |
| Windows | `%LOCALAPPDATA%\muman\data` | `%USERPROFILE%\Music\muman` |

Neither folder may hold the other, but for a library beside `sources/`
in the home: a command that writes refuses a library in `sources/` or a
home in the library.

The song list may hold logins and tokens, and the run history keeps its
text, so on Linux and macOS a home muman makes is yours alone (`0700`),
as are the history's folders and records. A home made before keeps its
permissions: `chmod -R go= <home>` makes it yours alone. On Windows a
new home takes its parent folder's permissions, your own under the
default.

A program not named in its variable is run as the song list's
`[tools.*] command` names it: a name is looked for beside the muman
executable, then on `PATH` (with `PATHEXT` on Windows, so a `.cmd`
wrapper is found), and a path is taken as it is, `~/` in your home and
a relative one under the home folder. A fork, or a wrapper that takes
the same options, runs in a tool's place when named there. Each program
is run once for its version when it is first found, and one older than
the oldest release muman runs, named in the
[README](../README.md#install), stops the run with exit code 2 and a
message naming it and where a newer one comes from; a version muman
cannot read, as a git build's, is taken as new enough. With `[tools.*]
version_check = false`, as for a fork numbered its own way, neither an
old version nor one that does not print stops a run. ffmpeg and
ffprobe are needed by every command that reads audio; yt-dlp only by a
run that fetches, so a library made only from your own files never needs
it. ffmpeg and ffprobe read a file only as one of the audio, picture and
subtitle formats muman takes, whatever its name, so a playlist named
`.flac` is refused rather than fetched. `muman info` prints the folders
in use and each program's version and path.

## Settings in the song list

A new home's `songs.toml` starts with every setting written out at its
default, written by the first `add` or `sync`:
[`src/manifest/new.toml`](../src/manifest/new.toml) is that file and
the reference for every key. Change the values you want. A
song list made by an earlier muman gains each setting it lacks, at its
default, the next time muman writes it. A setting with no default, such
as the library folder, stays a comment until you set it. A key a
settings table does not know is an error that names it, so a
misspelling never goes unnoticed. A table muman does not know is kept,
for a newer muman, but one a letter or two from a known name, as
`[audoi]`, is warned of by `sync` and `check`. One a letter or two from
`[[song]]`, `[[album]]` or `[[removed]]` is an error instead, since what
it lists would leave the library.

A setting that is a quantity takes its unit, written after the number,
the space optional:

| Kind | Such as | Settings |
|---|---|---|
| Size | `"32 GiB"`, `"700 MB"`, `"4K"` | `[library] max_size`, `block_size`, `[history] max_size` |
| Bitrate | `"160 kb/s"`, `"160 kbps"`, `"160k"` | `[audio] *_bitrate` |
| Time | `"14 days"`, `"2 s"`, `"500 ms"`, `"2 weeks"` | `[audio.pick.purity] step`, `[ytdlp] keep_partial`, `[torrent] stall`, `[providers.*] recheck`, a song's `lyrics_offset` |
| Frequency | `"500 Hz"`, `"1.5 kHz"` | `[audio.pick.bandwidth] step` |
| Share | `"1 %"`, or `0.01` | `[audio.pick.stereo] incoherence`, `[audio.pick.clipping] cutoffs`, `[cover.pick.resolution] step` |
| Loudness | `"-18 LUFS"`, `"-23 LKFS"` | `[loudness] target` |
| Level | `"-1 dB"`, `"-1 dBTP"`, `"-0.5 dBFS"` | `[loudness] ceiling` |

Every unit is also known by its name, singular or plural, as
`"2 gibibytes"` or `"3 hours"`. `K`, `M`, `G` and `T` alone are binary
sizes, as `KiB`; `KB`, `MB` and `GB` are decimal. A size counts bytes
without a unit, a frequency hertz and a share a fraction; a bitrate, a
time, a loudness or a level has no bare number.

`edition`, beside `version`, names the settings' names, units and
defaults the file was written to. A song list of an earlier edition
reads as the current one: a setting named the old way, such as
`opus_kbps = 160`, reads as its new name and unit, `opus_bitrate =
"160 kb/s"`, and the next `sync` writes it so, its comments kept;
`status` and `check` name each one meanwhile; a table moved whole, as
`[quality.purity]` to `[audio.pick.purity]`, keeps its settings, its
comments and its place in the file. Naming a setting both
ways is an error. When a later muman changes a default, `sync`,
`status` and `check` name each setting still at the old one, however
it is spelled, and `muman sync --update-defaults` moves those to the
new default and raises `edition`. A setting you changed stays as you
set it. A list without `edition` counts as edition 1; one of a later
edition than this muman knows reads while it sets nothing it does not
know.

| Table | Sets |
|---|---|
| `[library]` | The library folder, the path template, how names are made safe, length limits, fallback names, whether sources' comments are written, [the most the library may take](audio.md#library-size) |
| [`[lyrics]`](#which-lyrics-a-song-takes) | The languages you read, which lyrics a song takes first, the original, a romanization or a translation, timed or untimed, and [where they go](#where-lyrics-go) |
| [`[audio]`](audio.md#codecs) | The codecs copied as they are, what the rest is encoded to, each encoder's bitrate, the lowest bitrate a size limit lowers to |
| [`[audio.pick.*]`](#how-sources-are-ranked) | How audio sources are ranked: each measure switched on or off, its weight, and its steps and cutoffs |
| [`[loudness]`](audio.md#loudness) | How songs are levelled: where the gain goes, the loudness aimed at, which peaks count, album or track gain, the highest peak a gain may raise a song to |
| [`[cover]`](#how-sources-are-ranked) | How covers are ranked, as `[audio.pick.*]` ranks audio, and [where the cover goes](#where-the-cover-goes) |
| `[ytdlp]` | yt-dlp's format, subtitle languages, fragments, muman's yt-dlp plugins, the arguments every run takes, each extractor's arguments, how long partial downloads are kept |
| `[torrent]` | The arguments every aria2c run takes, how long a fetch waits with nothing arriving, how long a fetched torrent is seeded, how long partial fetches are kept |
| `[tools.*]` | The ffmpeg, ffprobe, yt-dlp and aria2c muman runs, and whether each one's version is checked |
| `[history]` | How many changing runs `undo` keeps, and how much space their replaced files may take together |
| `[sites.*]` | The sites yt-dlp fetches from: which extractors each names its sources by, the IDs each kind of source takes, and where one is fetched again from |

A step's torrents are fetched by one aria2c run, eight at a time unless
`[torrent] args` sets `--max-concurrent-downloads`, so one no peer
seeds stalls beside the others rather than before them.

Any user of the machine can see a program's command line, so `[ytdlp]
args` and `extractor_args` reach yt-dlp in a config file of the run's
own that only you can read. yt-dlp reads such a file before its command
line, so where an argument sets what an option of muman's own sets,
muman's wins: `format`, `sub_langs` and `concurrent_fragments` set three
of those. Of `[torrent] args`, a login, password, header, proxy or tracker
(`--header`, `--http-passwd`, `--bt-tracker` and their like) reaches
aria2c the same way, the rest on its command line. `--verbose` names
each program's command with these values hidden.

Each `[sites."<domain>"]` table describes one site. `extractors` maps
each of yt-dlp's extractors, lower-cased, to the prefix its IDs take in
a key, so a playlist YouTube's tab extractor fetched is
`youtube.com:playlist/<id>`. Each entry of `kinds` is a kind of source
the site holds: its `prefix`, an `id` regular expression the rest of an
ID matches, and a `fetch` address with `{id}` where it goes,
percent-encoded but for `/`, which a missing source is fetched again
from. A song list naming a key whose ID
no kind of its site takes is refused when read. `aliases` lists other
domains the site's sources were named by, as a page's domain named them
before the site had a table: a fetch from one is named by the site, and
a sync renames a listed key of one, its file moved, nothing fetched
again. A source of a site with no table is named by its page's domain
and fetched again from the page its song records. Adding a site to `new.toml` adds it to every song list
the next time muman writes one; list an extractor only for a site whose
pages are its own, since a source it fetched before is named by its page.

The song list's own tables — `[tags.clean.*]` and `[lyrics.clean.*]` (cleaning),
`[providers.*]`, `[[trigger]]` and `[[hook]]` — are described in the [guide](guide.md) and in [making the
library](library.md).

A setting that changes what a song file holds, such as a bitrate, a
codec, a measure's weight or where lyrics go, makes the next `sync`
write the songs it affects again. A setting that changes only where a song lands, such as the
template, moves the existing files instead of encoding them again.

### What aria2c shares

While aria2c fetches a torrent it is a peer like any other: each peer
and tracker it reaches sees your IP address, and it uploads the pieces
it has to peers as it downloads, at no speed limit. muman leaves
aria2c's own defaults for how it finds peers: it asks the IPv4 DHT
(`--enable-dht`) and other peers (`--enable-peer-exchange`) for more
peers than the trackers name, which tells the DHT's nodes and those
peers what you fetch, though it asks the DHT nothing for a torrent
marked private; and it listens for peers on a TCP port and for the DHT
on a UDP port, each between 6881 and 6999 (`--listen-port`,
`--dht-listen-port`). Local peer discovery and the IPv6 DHT are off.
Once a torrent's files are fetched, it seeds them for `[torrent]
seed_time` at most, or until it has sent as much as it fetched.
`[torrent] args` changes any of these:
`["--enable-dht=false", "--enable-peer-exchange=false"]` keeps to a
torrent's trackers, and `["--max-overall-upload-limit=50K"]` caps the
upload.

### Commands, servers and secrets

Some settings run programs or send requests as you:

| Setting | Runs or reaches |
|---|---|
| `[[hook]]` `run` | Its command, as the library changes ([hooks](library.md#hooks)) |
| `[tools.*] command` | A program in place of ffmpeg, ffprobe, yt-dlp or aria2c; `info` runs each for its version |
| `[ytdlp] args`, `extractor_args` | Whatever yt-dlp's options run or load, as `--exec`, `--netrc-cmd` or `--plugin-dirs`, and the headers and logins it sends |
| `[torrent] args` | Whatever aria2c's options run, as `--on-download-complete` |
| `[providers.*] url`, `key` | The server a provider's lookups go to, and AcoustID's key sent with them |
| `[sites.*]` `fetch` | The address a source is fetched again from |

The song list holds them so that it renders one library alike on every
machine, so a song list you are given, or one in a zip `muman export`
wrote, runs its commands as you; its songs name addresses too, which a
sync fetches from. Read one as you would a script before you run muman
on it.

muman runs none of them until you trust them. It keeps what you
trusted in `state.json`, by each setting's value, not its spelling or
comments. When a command could run a program, a hook or a request and
the song list holds one you have not trusted as it is now, muman first
lists every such setting, marking the new and the changed, and asks
once whether to trust them; a no runs nothing. Without a terminal it
stops with exit code 5, unless `--trust` or `MUMAN_TRUST` is given;
`-y` trusts nothing. A setting at its default needs no trust, so a new
home's list never asks; one taken out needs none either, and is asked
about again when put back, by hand or by `undo`. `list`, `duplicates`,
`purge`, `completions`, `check` without `--decode` or `--upstream`,
and an export that encodes nothing run none of them and never ask.

## Which lyrics a song takes

Lyrics have one of three roles, by their language and the one the song
is sung in, its `language` tag: one set by hand, else a MusicBrainz
work's lyrics language, a file's own tag, the language YouTube says a
video's audio is in, or that of the audio file's own lyrics tag; never
a subtitle's, which may be a translation. Lyrics in that language are
the original; in its Latin letters, as `ja-Latn` for a song sung in
`ja`, a romanization; in a language `[lyrics] read` names, a
translation. A translation into a language `read` does not name is
never taken. Lyrics that say nothing of their language, as LRCLIB's,
are taken as the original.

`[lyrics] prefer` lists the roles, and `"timed"` or `"untimed"`, best
first. Lyrics rank by the first entry that tells them apart; a role
left out is never taken. Its default is in
[`new.toml`](../src/manifest/new.toml). This takes a song's own words,
else their romanization, timed lyrics before untimed of the same role,
and never a translation:

```toml
[lyrics]
read = ["en"]
prefer = ["original", "romanization", "timed"]
```

Put `"timed"` first to take timed lyrics of any role before untimed
ones. Of lyrics `prefer` ranks alike, those in the language `read` names
first win, then the surest to be the chosen audio's recording. Where the
language sung is not known, no lyrics' role is, so the roles rank all
lyrics alike, and only lyrics in a language `read` names are taken; a
song file's own lyrics in another rank after those.

Languages match as a browser matches them: `zh` takes `zh-Hant`,
`zh-TW` takes `zh-Hant` but never `zh-Hans`, `pt-BR` takes `pt-PT`
after any `pt-BR`, and `ger`, `deu` and `de` are one language.

## Where lyrics go

Each `[[lyrics.out]]` is one place the lyrics a song takes are written:
`to = "file"`, a file beside the song named as the song is, then its
`suffix`, then the format's extension; or `to = "tag"`, inside the
song. `format` is `lrc`, `vtt` or `srt`, subtitle cues of timed lyrics
alone, or `txt`, plain lines; a tag holds `lrc` or `txt`. A cue lasts
until the next line, the last until the song ends; a line sung twice is
a cue each time. `words = false` drops each word's time. `when =
"timed"` or `"untimed"` writes only lyrics whose lines carry times, or
only those whose lines do not. `out = []` under `[lyrics]` writes none.
This writes timed lyrics beside the song and inside it, and untimed
ones beside it as `Song.plain.lrc`:

```toml
[[lyrics.out]]
to = "file"
when = "timed"
[[lyrics.out]]
to = "tag"
when = "timed"
[[lyrics.out]]
to = "file"
suffix = ".plain"
when = "untimed"
```

## Where the cover goes

Each `[[cover.out]]` places the cover a song takes as a `[[lyrics.out]]`
places lyrics: `to = "tag"` or `"file"`, with a `suffix`. `format =
"keep"` writes a JPEG or PNG without borders as its own bytes and any
other picture as PNG; `"jpeg"`, `"png"` and `"webp"` encode it, JPEG and
WebP at `quality`, from 1 to 100. A picture over `max_side` pixels is
scaled down, in its own format if kept. A tag holds `keep`, `jpeg` or
`png`; `out = []` under `[cover]` writes none. WebP needs an ffmpeg
built with libwebp, and is not held to the same bytes on every
processor. This adds a small WebP beside the cover inside the song:

```toml
[[cover.out]]
to = "tag"
[[cover.out]]
to = "file"
suffix = ".small"
format = "webp"
max_side = 600
```

`scope = "folder"` makes a file the song's folder holds once, named by
`name` and the format's extension: the cover most of the folder's songs
take. Songs share an album when they share an album artist and album; a
single is an album of its own. A folder holding songs of several albums
takes none, and `sync` says so, unless `mixed = "majority"` takes the
album most of its songs are on; where no album has the most, none. A
file of your own at that name is left alone, even by `sync --force`.
This writes the cover inside each song and as `cover.jpg` in its folder:

```toml
[[cover.out]]
to = "tag"
[[cover.out]]
to = "file"
scope = "folder"
name = "cover"
format = "jpeg"
```

A suffix is a dot and up to 24 bytes a name may hold on every system,
and every song's name keeps room for the longest. Taking an output out
removes its files; a change to the files beside a song alone writes
them again and leaves the song's own file as it is.

## How sources are ranked

Each measure of [how the best of each is picked](library.md#how-the-best-of-each-is-picked)
has a table under its aspect's `pick`: `[audio.pick.purity]`, `stereo`,
`bandwidth` and `clipping` rank audio, `[cover.pick.square]`,
`resolution` and `blockiness` rank covers. In each, `enabled` and `weight` say whether and
how much it counts, and the other keys how it is cut into steps. A
source's steps times their weights are summed, and the lowest sum wins.

```toml
# Mono costs as much as 2.5 kHz of bandwidth: 50 against 10 for each
# 500 Hz. Clipping counts for nothing.
[audio.pick.bandwidth]
weight = 10
step = "500 Hz"
[audio.pick.stereo]
weight = 50
[audio.pick.clipping]
enabled = false
```

The default weights, in [`new.toml`](../src/manifest/new.toml), make
each measure outweigh all after it, so their order alone decides; weigh
a measure against the steps of those it should trade with.

## The path template

`[library] template` is a [MiniJinja](https://docs.rs/minijinja)
template: Jinja2 syntax, rendered once per song into its path without
the extension. `/` in the template separates folders.

| Variable | Holds |
|---|---|
| `album`, `album_artist` | The album and its artist, or the fallback names when missing |
| `bpm`, `key` | The tempo and the musical key, or empty |
| `composer` | The composer, or empty |
| `artist`, `artists` | The first artist, and every artist as a list; empty when missing, with no fallback name, so `{{ artist or album_artist }}` gives one |
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
| `none` | Only `/`, `\` and control characters, which no filesystem allows or Windows reads as a folder |
| `windows` | Also the characters Windows, FAT and most phones refuse — `\ : * ? " < > \|` — trailing dots and spaces, and reserved device names such as `CON` or `nul.txt` |
| `ascii` | As `windows`, and transliterates everything else to ASCII |

The refused characters become their full-width lookalikes (`：`, `？`,
`⧸`, …), as yt-dlp names its files, so a title stays readable. `[library]
replace` maps characters or strings to replacements of your own,
applied first and over these: `{ ":" = " -" }` turns `A: B` into
`A - B`. A reserved device name gets `_` after it: `CON_.opus`. On
Windows itself, `none` replaces what `windows` does all the same, since
Windows holds no such name.

File and folder names are cut to `max_name_bytes` and `max_folder_bytes`
bytes of UTF-8, which no filesystem counts more strictly, once they are
in Unicode NFC, and never inside a character as a reader sees one, such
as a flag or a letter with its accent, unless one character alone is
longer; such a cut ends on no joiner or variation selector. A name
keeps room for
what tells it apart, so the least they take is 40 and 16; the most is
255, as much as any filesystem holds. Windows
programs and some players also stop at 260 characters for a whole path;
`max_path` cuts each title so the full path, library folder included,
stays within it, and a sync warns of the songs it cannot fit. Its
characters are counted as Windows counts them, in UTF-16 units, so an
emoji or a character past the Basic Multilingual Plane counts two.

Two names a case-insensitive filesystem holds as one, as `Song` and
`SONG`, or `ΟΔΟΣ` and `οδοσ`, are told apart by a suffix naming the
song's source, so a library copied to Windows or macOS loses neither.
