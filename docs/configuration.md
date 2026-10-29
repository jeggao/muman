# Configuration

muman takes its configuration from two places, split by what a setting
is about:

- **How the library is made** is set in the song list, `songs.toml`:
  its layout and file names, encoding, how yt-dlp and aria2c fetch,
  which programs run, how much `undo` keeps. So one song list makes the
  same library on every machine.
- **Where things are on this machine**, the home, the library folder
  and the programs, comes from flags and environment variables, which
  override the song list.

## Folders and programs

| Flag | Variable | Sets |
|---|---|---|
| `--home` | `MUMAN_HOME` | The home: `songs.toml`, `state.json`, the sources and the run history |
| `--library` | `MUMAN_LIBRARY` | The library folder, over the song list's `[library] path` |
| `--offline` | `MUMAN_OFFLINE` | Reach no network: no lookup, no fetch, and URLs refused |
| `--progress` | `MUMAN_PROGRESS` | How a long step shows its progress: `auto`, `plain`, `json` or `none` ([progress](guide.md#progress-and-output)) |
| `--trust` | `MUMAN_TRUST` | Trust the commands and servers the song list sets, as it sets them now, without asking ([commands, servers and secrets](#commands-servers-and-secrets)) |
| | `MUMAN_FFMPEG` | The ffmpeg to run, over `[tools.ffmpeg] command` |
| | `MUMAN_FFPROBE` | The ffprobe to run, over `[tools.ffprobe] command` |
| | `MUMAN_YT_DLP` | The yt-dlp to run, over `[tools.yt-dlp] command`; may be a whole command, as `python -m yt_dlp` |
| | `MUMAN_ARIA2C` | The aria2c to run, over `[tools.aria2c] command` |

Without them, each folder is the platform's own:

| Platform | Home | Library |
|---|---|---|
| Linux | `$XDG_DATA_HOME/muman`, usually `~/.local/share/muman` | `$XDG_MUSIC_DIR/muman`, usually `~/Music/muman` |
| macOS | `~/Library/Application Support/muman` | `~/Music/muman` |
| Windows | `%LOCALAPPDATA%\muman\data` | `%USERPROFILE%\Music\muman` |

Neither folder may hold the other, except that the library may sit
beside `sources/` in the home. A command that writes refuses a library
inside `sources/`, or a home inside the library.

When you point muman at another library folder, it writes the library
there. Files it finds there exactly as it wrote them, as after you
moved or copied the library, stay muman's. Files in the old folder are
left alone, and are muman's again only while they are unchanged.

The song list may hold logins and tokens, and the run history keeps
copies of it. So on Linux and macOS a home muman makes is yours alone
(`0700`), as are the history's folders and records. A home made before
keeps its permissions; `chmod -R go= <home>` makes it yours alone. On
Windows a new home takes its parent folder's permissions, your own by
default.

### Programs

A program not named in its variable runs as the song list's
`[tools.*] command` names it. A name is looked for beside the muman
executable, then on `PATH`, with `PATHEXT` on Windows so a `.cmd`
wrapper is found. A path is taken as it is: `~/` starts in your home
directory, and a relative path in the home folder. A fork, or a wrapper
that takes the same options, can run in a tool's place.

muman runs each program once for its version when it first finds it.
One older than muman needs, as the [README](../README.md#install)
lists, stops the run with exit code 2, naming it and where a newer one
comes from. A version muman cannot read, as a git build's, counts as
new enough. `version_check = false` in a `[tools.*]` table, as for a
fork numbered its own way, lets any version run.

Every command that reads audio needs ffmpeg and ffprobe. Only a run
that fetches needs yt-dlp, and only one that fetches a torrent aria2c,
so a library made of your own files never needs either. ffmpeg and
ffprobe read a file only as one of the audio, picture and subtitle
formats muman takes, whatever its name, so a playlist named `.flac` is
refused rather than fetched. `muman info` prints the folders in use and
each program's version and path.

## Settings in the song list

The first `add` or `sync` in a new home writes `songs.toml` with every
setting at its default. [`src/manifest/new.toml`](../src/manifest/new.toml)
is that file, and the reference for every key; change the values you
want. A song list made by an earlier muman gains each setting it lacks,
at its default, the next time muman writes it. A setting with no
default, such as the library folder, stays a comment until you set it.

muman refuses a key a settings table does not know, naming it, so a
misspelling never goes unnoticed. A table muman does not know is kept,
for a newer muman, but `sync` and `check` warn of one a letter or two
from a known name, as `[audoi]`. One a letter or two from `[[song]]`,
`[[album]]` or `[[removed]]` is an error instead, since its songs would
leave the library.

A quantity takes its unit after the number; the space is optional:

| Kind | Written as | Settings |
|---|---|---|
| Size | `"32 GiB"`, `"700 MB"`, `"4K"` | `[library] max_size`, `block_size`, `[history] max_size` |
| Bitrate | `"160 kb/s"`, `"160 kbps"`, `"160k"` | `[audio] *_bitrate` |
| Time | `"14 days"`, `"2 s"`, `"500 ms"`, `"2 weeks"` | `[audio.pick.purity] step`, `[ytdlp] keep_partial`, `[torrent] stall`, `[providers.*] recheck`, a song's `lyrics_offset` |
| Frequency | `"500 Hz"`, `"1.5 kHz"` | `[audio.pick.bandwidth] step` |
| Share | `"1 %"`, or `0.01` | `[audio.pick.stereo] incoherence`, `[audio.pick.clipping] cutoffs`, `[cover.pick.resolution] step` |
| Loudness | `"-18 LUFS"`, `"-23 LKFS"` | `[loudness] target` |
| Level | `"-1 dB"`, `"-1 dBTP"`, `"-0.5 dBFS"` | `[loudness] ceiling` |

Every unit may also be spelled out, singular or plural, as `"2
gibibytes"` or `"3 hours"`. `K`, `M`, `G` and `T` alone are binary
sizes, as `KiB`; `KB`, `MB` and `GB` are decimal. A bare number is
bytes for a size, hertz for a frequency and a fraction for a share. A
bitrate, a time, a loudness or a level needs its unit.

| Table | Sets |
|---|---|
| `[library]` | The library folder, the [path template](#the-path-template), [safe names](#safe-names-on-every-system), fallback names, whether sources' comments are written, and [the most the library may take](audio.md#library-size) |
| `[lyrics]` | The languages you read, [which lyrics a song takes](#which-lyrics-a-song-takes) and [where they go](#where-lyrics-go) |
| `[audio]` | [The codecs](audio.md#codecs) copied as they are, what the rest is encoded to, each encoder's bitrate, and the speaker layouts kept |
| `[audio.pick.*]` | [How audio sources are ranked](#how-sources-are-ranked) |
| `[loudness]` | [How songs are levelled](audio.md#loudness) |
| `[cover]` | How covers are ranked, and [where the cover goes](#where-the-cover-goes) |
| `[ytdlp]` | yt-dlp's format, subtitle languages and fragments, muman's yt-dlp plugins, extra arguments, and how long partial downloads are kept |
| `[torrent]` | aria2c's extra arguments, how long a fetch waits with nothing arriving, how long a torrent is seeded, and how long partial fetches are kept |
| `[tools.*]` | The [programs](#programs) muman runs, and whether each one's version is checked |
| `[history]` | How many runs `undo` keeps, and how much space their files may take together |
| `[sites.*]` | The [sites](#sites) yt-dlp fetches from |

The song list's other tables are described where they are used:
`[tags.clean.*]` and `[lyrics.clean.*]` in [cleaning](library.md#cleaning),
`[providers.*]` in [lookups](library.md#lookups), and `[[hook]]` in
[hooks](library.md#hooks).

A setting that changes what a song file holds, such as a bitrate, a
codec, a measure's weight or where lyrics go, makes the next `sync`
write the songs it affects again. A setting that changes only where a
song lands, such as the template, moves the files instead.

### Editions

`edition`, beside `version`, names the settings' names, units and
defaults the file was written to; a file without one is of edition 1.
muman reads a song list of an earlier edition as the current one:
`opus_kbps = 160` reads as `opus_bitrate = "160 kb/s"`, and a table
moved whole, as `[quality.purity]` to `[audio.pick.purity]`, keeps its
settings, comments and place. `status` and `check` name each renamed
setting, and the next `sync` writes it under its new name, comments
kept. Naming a setting both ways is an error.

When a later muman changes a default, `sync`, `status` and `check`
name each setting still at the old one, however it is spelled. `muman
sync --update-defaults` moves those to the new default and raises the
edition; a setting you changed stays as you set it. A song list of a
later edition than this muman knows reads as long as it sets nothing
this muman does not know.

### yt-dlp and aria2c arguments

Any user of the machine can see a program's command line. So `[ytdlp]
args` and `extractor_args` reach yt-dlp in a config file of the run's
own that only you can read. yt-dlp reads that file before its command
line, so where an argument sets what one of muman's own options sets,
muman's wins; `format`, `sub_langs` and `concurrent_fragments` are
where those are set. Of `[torrent] args`, a login, password, header,
proxy or tracker (`--header`, `--http-passwd`, `--bt-tracker` and their
like) reaches aria2c the same way, and the rest on its command line.
`--verbose` shows each program's command with these values hidden.

One aria2c run fetches a step's torrents, eight at a time unless
`[torrent] args` sets `--max-concurrent-downloads`, so a torrent no
peer seeds stalls beside the others rather than ahead of them.

### Sites

Each `[sites."<domain>"]` table describes a site yt-dlp fetches from:

- `extractors` maps each of yt-dlp's extractors, in lower case, to the
  prefix its IDs take in a key. YouTube's tab extractor fetches
  playlists, so a playlist is `youtube.com:playlist/<id>`.
- `kinds` lists the kinds of source the site holds, each with its
  `prefix`, an `id` regular expression its IDs match, and a `fetch`
  address with `{id}` where the ID goes, percent-encoded except for
  `/`. A source gone from the store is fetched again from that address.
  A song list naming a key that no kind of its site takes is refused.
- `aliases` lists other domains the site's sources were named by before
  it had a table. A fetch from one is named by the site, and a sync
  renames a listed key of one, moving its file without fetching again.

A source of a site with no table is named by its page's domain and
fetched again from the page its song records. A site added to
`new.toml` reaches every song list the next time muman writes it. List
an extractor only for a site whose pages are its own, since a source it
fetched before was named by its page.

### What aria2c shares

While aria2c fetches a torrent it is a peer like any other. Each peer
and tracker it reaches sees your IP address, and it uploads pieces to
peers as it downloads, at no speed limit. muman leaves aria2c's own
defaults for finding peers:

- it asks the IPv4 DHT (`--enable-dht`) and other peers
  (`--enable-peer-exchange`) for more peers than the trackers name,
  which tells the DHT's nodes and those peers what you fetch; it asks
  the DHT nothing for a torrent marked private;
- it listens for peers on a TCP port, and for the DHT on a UDP port,
  each between 6881 and 6999 (`--listen-port`, `--dht-listen-port`).

Local peer discovery and the IPv6 DHT are off. Once a torrent's files
are fetched, aria2c seeds them for `[torrent] seed_time` at most, or
until it has sent as much as it fetched. `[torrent] args` changes any
of this: `["--enable-dht=false", "--enable-peer-exchange=false"]` keeps
to a torrent's trackers, and `["--max-overall-upload-limit=50K"]` caps
the upload.

### Commands, servers and secrets

Some settings run programs or send requests as you:

| Setting | Runs or reaches |
|---|---|
| `[[hook]]` `run` | Its command, as the library changes ([hooks](library.md#hooks)) |
| `[tools.*] command` | A program in place of ffmpeg, ffprobe, yt-dlp or aria2c; `info` runs each for its version |
| `[ytdlp] args`, `extractor_args` | Whatever yt-dlp's options run or load, as `--exec`, `--netrc-cmd` or `--plugin-dirs`, and the headers and logins it sends |
| `[torrent] args` | Whatever aria2c's options run, as `--on-download-complete` |
| `[providers.*] url`, `key` | The server a provider's lookups go to, and the key sent to AcoustID |
| `[sites.*]` `fetch` | The address a source is fetched again from |

They live in the song list so that one list makes the same library on
every machine. That means a song list you are given, or one in a zip
`muman export` wrote, runs its commands as you. Its songs name
addresses too, which a sync fetches from. Read a song list as you would
a script before you run muman on it.

muman runs none of these until you trust them. It keeps what you
trusted in `state.json`, by each setting's value, not its spelling or
comments. When a command could run a program, a hook or a request, and
the song list holds such a setting you have not trusted as it is now,
muman lists every one, marks the new and the changed, and asks once. A
no runs nothing. Without a terminal muman stops with exit code 5,
unless `--trust` or `MUMAN_TRUST` is given; `-y` trusts nothing.

A setting at its default needs no trust, so a new home's song list
never asks. Nor does a setting taken out, which is asked about again
when put back, by hand or by `undo`. `list`, `duplicates`, `purge`,
`completions`, `check` without `--decode` or `--upstream`, and an
export that encodes nothing run none of these, and never ask.

## Which lyrics a song takes

Lyrics have one of three roles, set by their language against the one
the song is sung in:

- **Original:** in the language sung.
- **Romanization:** in that language's Latin letters, as `ja-Latn` for
  a song sung in `ja`.
- **Translation:** in a language `[lyrics] read` names. A translation
  into any other language is never taken.

Lyrics that say nothing of their language, as LRCLIB's, count as the
original. The language sung is the song's `language` tag: one set by
hand, else a MusicBrainz work's lyrics language, a file's own tag, the
language YouTube gives a video's audio, or that of the audio file's own
lyrics tag; never a subtitle's, which may be a translation. YouTube's
language is the one its uploader set, so it counts only where the
song's title and its release's script could be in it: a title in kana
is Japanese, never Korean or Chinese.

`[lyrics] prefer` lists the roles, and `"timed"` or `"untimed"`, best
first. Lyrics rank by the first entry that tells them apart, and a role
left out is never taken. Its default is in
[new.toml](../src/manifest/new.toml). This takes a song's own words,
else their romanization, timed lyrics before untimed ones of the same
role, and never a translation:

```toml
[lyrics]
read = ["en"]
prefer = ["original", "romanization", "timed"]
```

Put `"timed"` first to take timed lyrics of any role before untimed
ones. A translation is for lyrics you can read, so the original counts
as one too when it is in a language `read` names. With
`"translation"` first and `"original"` after, a song sung in a language
you read takes its own lyrics, and one sung in another a translation.

Of lyrics `prefer` ranks alike, those in the language `read` names
first win, then those surest to be the chosen audio's recording. Where
the language sung is unknown, no lyrics have a role, so `prefer` ranks
them all alike, and only lyrics in a language `read` names are taken; a
song file's own lyrics in another language rank after those.

Languages match as a browser matches them: `zh` takes `zh-Hant`,
`zh-TW` takes `zh-Hant` but never `zh-Hans`, `pt-BR` takes `pt-PT`
after any `pt-BR`, and `ger`, `deu` and `de` are one language.

## Where lyrics go

Each `[[lyrics.out]]` is one place the lyrics a song takes are written:

| Key | Takes |
|---|---|
| `to` | `"file"`, beside the song, named as the song is, then its `suffix`, then the format's extension; or `"tag"`, inside the song |
| `format` | `"lrc"`; `"vtt"` or `"srt"`, subtitle cues of timed lyrics alone; or `"txt"`, plain lines. A tag holds `"lrc"` or `"txt"` |
| `when` | `"timed"` or `"untimed"` writes only lyrics whose lines carry times, or only those whose lines do not |
| `words` | `false` drops each word's time |

A cue lasts until the next line, and the last until the song ends; a
line sung twice is a cue each time. `out = []` under `[lyrics]` writes
no lyrics. This writes timed lyrics beside the song and inside it, and
untimed ones beside it as `Song.plain.lrc`:

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

Each `[[cover.out]]` places the cover a song takes, as a
`[[lyrics.out]]` places lyrics: `to = "tag"` or `"file"`, with a
`suffix`. `format = "keep"` writes a JPEG or PNG without borders as its
own bytes, and any other picture as PNG. `"jpeg"`, `"png"` and
`"webp"` encode it, JPEG and WebP at `quality`, from 1 to 100. A
picture longer than `max_side` pixels is scaled down, in its own format
if kept. A tag holds `keep`, `jpeg` or `png`. `out = []` under
`[cover]` writes no cover. WebP needs an ffmpeg built with libwebp, and
its bytes may differ between processors. This adds a small WebP beside
the cover inside the song:

```toml
[[cover.out]]
to = "tag"
[[cover.out]]
to = "file"
suffix = ".small"
format = "webp"
max_side = 600
```

`scope = "folder"` writes one file in the song's folder, named by
`name` and the format's extension: the cover most of the folder's songs
take. Songs share an album when they share an album artist and album;
a single is an album of its own. A folder holding songs of several
albums gets no file, and `sync` says so, unless `mixed = "majority"`
takes the album most of its songs are on. A file of yours at that name
is left alone, even by `sync --force`. This writes the cover inside
each song and as `cover.jpg` in its folder:

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
removes its files. A change to the files beside a song alone writes
them again and leaves the song's own file as it is.

## How sources are ranked

Each measure of [how the best source wins](library.md#how-the-best-source-wins)
has a table under its part's `pick`: `[audio.pick.purity]`, `stereo`,
`bandwidth` and `clipping` rank audio, and `[cover.pick.square]`,
`resolution` and `blockiness` rank covers. In each, `enabled` and
`weight` say whether and how much it counts, and the other keys how it
is cut into steps. muman sums each source's steps times their weights,
and the lowest sum wins.

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

The default weights, in [new.toml](../src/manifest/new.toml), make each
measure outweigh all after it, so their order alone decides. To let
measures trade, weigh each against the steps of those it should trade
with.

## The path template

`[library] template` is a [MiniJinja](https://docs.rs/minijinja)
template in Jinja2 syntax, rendered once for each song into its path
without the extension. `/` in the template separates folders.

| Variable | Holds |
|---|---|
| `title` | The title, or the fallback name |
| `artist`, `artists` | The first artist, and every artist as a list; empty when missing, with no fallback, so `{{ artist or album_artist }}` gives one |
| `album`, `album_artist` | The album and its artist, or the fallback names |
| `genre`, `composer` | The genre and the composer, or empty |
| `date`, `year` | The date as tagged, and its first four digits |
| `track`, `disc` | The track and disc numbers, or none |
| `disc_track` | `03 ` on a first disc, `2-03 ` on a later one, nothing without a track number |
| `bpm`, `key` | The tempo and the musical key, or empty |
| `id` | The ID or file name of the song's audio source |

Beside MiniJinja's own filters (`default`, `lower`, `upper`, `replace`,
`first`, `join` and more), muman adds:

| Filter | Does |
|---|---|
| `asciify` | Transliterates to ASCII: `Ça` becomes `Ca` |
| `pad(n)` | Pads a number with zeros: `track \| pad(3)` gives `003` |
| `the_suffix` | Moves a leading "The": `The Orchards` becomes `Orchards, The` |
| `truncate(n)` | Keeps the first `n` characters |

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

muman makes every tag value safe before the template sees it, so a `/`
in a title never makes a folder; only the template's own `/` does. When two
songs resolve to one path, the second song's source ID tells them
apart: `Title [vid00000001]`. `muman status` shows each song's path
before anything is written.

## Safe names on every system

muman makes names safe the same way on every platform, so a library
copies from one system to another, or to a phone, unchanged. `[library]
restrict` sets how far that goes:

| `restrict` | Replaces |
|---|---|
| `none` | Only `/`, `\` and control characters, which no filesystem allows or Windows reads as a folder |
| `windows` | Also what Windows, FAT and most phones refuse: `\ : * ? " < > \|`, trailing dots and spaces, and device names such as `CON` or `nul.txt` |
| `ascii` | As `windows`, and transliterates everything else to ASCII |

Refused characters become their full-width lookalikes (`：`, `？`, `⧸`
and so on), as yt-dlp names its files, so a title stays readable. A
device name gets `_` after it: `CON_.opus`. On Windows itself, `none`
replaces what `windows` does, since Windows can hold no such name.
`[library] replace` maps characters or strings to replacements of your
own, applied first: `{ ":" = " -" }` turns `A: B` into `A - B`.

File and folder names are cut to `max_name_bytes` and
`max_folder_bytes` bytes of UTF-8, once in Unicode NFC, which no
filesystem counts more strictly. A cut never splits a character as a
reader sees one, such as a flag or a letter with its accent, unless
that character alone is too long, and never ends on a joiner. A name
keeps room for what tells it apart, so `max_name_bytes` takes at least
40, more when a file beside a song has a longer tail than `.flac`, and
`max_folder_bytes` at least 16. Neither takes more than 255, as much as
any filesystem holds.

Windows programs and some players stop at 260 characters for a whole
path. `max_path` cuts each title so the full path, library folder
included, stays within it, and a sync warns of any song it cannot fit.
It counts characters as Windows does, in UTF-16 units, so an emoji
counts two.

Two names a case-insensitive filesystem holds as one, as `Song` and
`SONG`, or `ΟΔΟΣ` and `οδοσ`, are told apart by a suffix naming the
song's source, so a library copied to Windows or macOS loses neither.
