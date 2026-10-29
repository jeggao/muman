# Guide

Everything your library is made from lives in one folder, the home: the
song list, which names every song and the sources it may be made from,
and the sources themselves. This page covers the three. [Making the
library](library.md) says how muman picks the best source for each part
of a song and what it looks up online. [Songs](songs.md) covers queries
and changing songs, [configuration](configuration.md) the folders,
programs and settings, and [the command reference](cli.md) every flag.

## The home

The home holds everything muman needs to make the library again. The
library holds only what muman writes from it. `muman info` shows where
both are.

| Path in the home | Holds |
|---|---|
| `songs.toml` | The song list: every song, its sources, tags and pins, and the settings |
| `state.json` | The library files muman wrote and what each was made from, and what it measured of every source |
| `sources/yt-dlp/<site>/` | What yt-dlp fetched, each at a path made from its key: audio, thumbnail, subtitles and metadata in one file |
| `sources/manual/` | Files of your own, at any depth, with the lyrics and pictures beside them |
| `sources/torrent/` | Each torrent's metainfo, `<hash>.torrent`, and its files in the folder `<hash>/` |
| `sources/lrclib/` | Lyrics found on LRCLIB |
| `sources/musicbrainz/` | Tags found on MusicBrainz: a recording and the release picked for it |
| `sources/coverart/` | Album covers from the Cover Art Archive |
| `history/` | What the latest runs replaced or removed, for `undo` |
| `partial/` | Unfinished downloads, which a later run resumes |

A file in `sources/yt-dlp` that is not at its key's path, or in a
torrent's folder that its metainfo does not name, is no source:
`muman purge` deletes it.

## What a sync does

`add` and `sync` work in two phases. Online, muman lists what URLs
name, fetches sources, and makes the [lookups](library.md#lookups)
that are due; each new source is compared with the songs already
listed ([new sources](#new-sources)). Offline, muman takes a lock on
the home and brings the library in line with the song list:

1. Measure each source whose files changed since it was last measured,
   saving progress as it goes.
1. Compare the audio of each pair of a song's sources.
1. Resolve each song to a plan: the best source for each of its parts.
1. Find each song's file: the one made from most of its sources, one
   file to a song, wherever it is now.
1. Move each song whose path changed, if its file differs from its
   plan at most in its tags. Nothing is encoded again.
1. Write new tags into each file whose plan changed only in its tags.
   Write a song anew when its plan changed otherwise, or its file is
   gone. `sync --force` writes every song.
1. Delete each file muman wrote that no song makes any more.

muman knows each part a song takes from a source, its audio, cover or
lyrics, by a digest of its content. A source fetched again, or copied
without its file times, rewrites no song while it holds the same.

`muman status` makes the same decisions and changes nothing. A song
may move into a path another song leaves in the same sync; two songs
that swap paths are written again.

### Files muman owns

muman deletes or writes over only the files `state.json` records as
its own. Anything else in the library folder stays as it is:

- A file of yours at a song's path, or where a file beside the song
  goes, holds that song back until `sync --force`.
- A file muman wrote that you changed since, as a tagger changes a
  song or an editor its `.lrc`, stays with a warning. A new path
  template still moves it, your change with it, but a change to the
  song leaves it alone. `sync --force` writes the song again over your
  change, and `undo` brings your change back.
- muman knows its files by their size and modification time, else by
  their bytes, so a restored backup or a copy without its times is
  still muman's. A file whose bytes changed under the same size and
  time, as some taggers leave one, is caught as muman is about to
  write over it or remove it.

muman writes every file under a temporary name and renames it into
place, so a player never sees half a track. Library paths in messages
use `/` on every system.

### Progress and output

A long step shows its progress on stderr. On a terminal, one status
line sits under the messages:

```text
[5/5] Writing [====>   ] 812/1546  53%  41 s left  Rooms of Salt · Paper Comets
```

It gives the step, the count, the time left at the last minute's rate,
and what is under way. A narrow terminal drops the percentage, then the
bar, then the time left, so the line never wraps. Terminals that show a
task's progress in their tab or taskbar show it there too. `--progress`
chooses another form:

| Mode | Writes |
|---|---|
| `auto` | The status line on a terminal; elsewhere, as in a log, what `plain` writes |
| `plain` | A line every tenth of the way or every 10 s, and one when the step ends |
| `json` | One JSON object a line: `start`, `progress` and `finish` events, and each message as a `message` event with its `text` |
| `none` | Nothing |

What a command does goes to stderr. What `status`, `list`, `info`,
`check` and `duplicates` report goes to stdout.

## The song list

`songs.toml` lists every song and the sources it may be made from:

```toml
version = 1

[lyrics]
read = ["en", "fr"]

[[song]]
sources = ["youtube.com:vid00000001", "manual:Marlo Venn/Paper Comets.flac"]
cover = "manual:Marlo Venn/Paper Comets.flac"
lyrics_offset = "120 ms"
tags.artist = "Marlo Venn"
tags.album = "Lantern Weather"
tags.genre = ""
```

| Key | Means |
|---|---|
| `version` | The file's format; muman refuses a newer one |
| `sources` | Every source the song may be made from. A file belongs to one song; a lookup's record may be listed by every song of its recording |
| `album`, `track` | The `[[album]]` the song is on, and its place there |
| `audio`, `cover`, `lyrics` | A pin: that part from that source, whatever the measures say. `lyrics = false` takes none |
| `lyrics_offset` | Moves the lyrics later by this time, on top of the offset measured; a negative time moves them earlier |
| `lookups` | `false` asks no provider anything for the song; `lookups.<provider> = false` asks that one nothing ([lookups](library.md#lookups)) |
| `tags.<name>` | A tag set over what the sources offer; an empty value sets nothing |
| `held."<key>"` | What a source held when the song was built, written by muman ([a source that changes](#a-source-that-changes)) |
| `[[album]]` | A YouTube Music album added whole, with `tags.<name>` over what its tracks offer |
| `[torrents."<hash>"]` | A torrent whose files songs list: its `name`, `trackers` and `v2` hash, from which it is fetched again ([torrents](#torrents)) |
| `[[removed]]` | A song `remove` took out, kept whole under a `note` naming it ([removed songs](#removed-songs)) |

The file also holds the [settings](configuration.md#settings-in-the-song-list),
the cleaning switches under `[tags.clean.*]` and `[lyrics.clean.*]`
([cleaning](library.md#cleaning)), the providers under `[providers.*]`
([lookups](library.md#lookups)) and the [`[[hook]]`](library.md#hooks)
tables.

### Source keys

A song has no ID of its own. It is its sources, and any of their keys
names it:

| Key | Names |
|---|---|
| `<site>:<id>` | What yt-dlp fetched, as `youtube.com:vid00000001` |
| `manual:<path>` | A file under `sources/manual`, as `manual:Marlo Venn/Paper Comets.flac` |
| `torrent:<hash>/<path>` | A file of a torrent |
| `lrclib:<id>` | Lyrics kept from LRCLIB |
| `musicbrainz:<id>` | A recording kept from MusicBrainz |
| `coverart:<id>` | An album cover kept from the Cover Art Archive |

A fetched source is named by the site it came from and the ID yt-dlp
gives it there. A site reached at several addresses has one name: a
video from YouTube Music, `youtu.be` or `m.youtube.com` is
`youtube.com:<id>`, and a track on `<artist>.bandcamp.com` is
`bandcamp.com:<id>`. A file in an archive.org item of several is
`archive.org:<item>/<file>`, its name kept as it is, as
`archive.org:<item>/01 Lantern #2.mp3`, and is fetched again alone.
The song list's `[sites]` says which sites muman knows, the IDs each
takes and where a source is fetched again from
([sites](configuration.md#sites)). A source of any other site is named
by its page's domain, without `www.`.

A torrent's file is named by the torrent's v1 hash, or its v2 hash if
it has no v1, then its path inside the torrent without the torrent's
own folder: `torrent:<hash>/CD1/01 Paper Comets.flac`. A torrent of one
file names it by its name. The hash covers every byte, so a torrent's
files never change.

An earlier muman named sources by yt-dlp's extractor, as
`youtube:<id>`. muman reads such a key as its site's where the
built-in `[sites]` knows the extractor, and the next write renames it
without writing any song again. Any other keeps its name until a sync
renames it by the page its source came from, except a file yt-dlp's
generic extractor fetched from its address, which keeps it for good.

### Tags

`tags.<name>` takes any Vorbis comment name. muman knows `title`,
`artist`, `artists`, `album`, `album_artist`, `genre`, `date` (or
`year`), `originaldate`, `track`, `disc`, `track_total`, `disc_total`,
`discsubtitle`, `composer`, `lyricist`, `conductor`, `remixer`,
`label`, `catalognumber`, `barcode`, `asin`, `isrc`, `release_country`,
`releasetype`, `releasestatus`, `media`, `script`, `compilation`,
`bpm`, `initialkey`, `mood`, `grouping`, `work`, `language`,
`copyright`, `comment`, the sort names (`artistsort`, `titlesort` and
the like) and the MusicBrainz IDs under Picard's names (as
`musicbrainz_trackid`), and writes each where Picard writes it in MP3,
MP4 and WavPack. A list sets a tag several times, and `true` or `false`
is written as `1` or `0`. These tags take a value in a set form:

| Tag | Takes |
|---|---|
| `track`, `disc` | `3`, or `3/12`, which sets the total too unless the song sets it |
| `date` | `YYYY-MM-DD`, `YYYY-MM` or a year; `2019/07/04` and `20190704` read too, and so does an unquoted TOML date |
| `isrc` | An ISRC, kept without the dashes it is often shown with |
| `bpm` | A number from 1 to 9999 with up to two decimals, rounded where a format holds whole ones (`TBPM`, `tmpo`) |
| `barcode` | Its digits |
| `language` | Any tag for a language, as `ja`, written as its ISO 639-2 code, `jpn` |
| `initialkey` | Written as `KEY` and `INITIALKEY` in Vorbis comments; in MP3 as `TKEY` when it is a key that frame holds (`Abm`, `F#`), else as a user text frame |

A song's `language` is the one it is sung in, which decides what its
[lyrics](configuration.md#which-lyrics-a-song-takes) are. A source's
own `comment` is written only with `[library] source_comments`. `add
--artist`, `--album` and the other tag flags set tags on every song an
`add` lists, and `muman status` shows each tag's value and the source
it came from.

muman refuses a song list with a tag its field cannot hold, naming the
song by its number and first source: a date of `July 2019` or
`2019-02-30`, a tempo outside 1 to 9999, a table, or a ReplayGain gain
or peak, which muman measures itself. It also refuses a tag name a
Vorbis comment cannot carry: one with `=`, or anything but printable
ASCII.

### Chapters

A song keeps the chapters of the source its audio comes from, as an
M4B audiobook or a video in parts has them. The song stays one file,
and a player skips from chapter to chapter within it. MP4 holds them as
MP4 chapters, FLAC, Ogg and Opus as `CHAPTER000` and `CHAPTER000NAME`
comments, and MP3 as `CTOC` and `CHAP` frames. WavPack holds none.

### How muman edits the song list

muman edits `songs.toml` in place. Comments, order and keys it does not
know survive every write, and two runs at once keep each other's
songs. Each write fills in the common `tags.<name>` keys a song lacks,
empty, so you can see what to set.

These stop a run before anything is written: a key that does not
parse, a file two songs list, and a pin to a source the song does not
list. muman writes each song list in the oldest format that holds its
songs, and an older muman refuses a newer format rather than misread
it. Songs sharing a lookup's record need `version = 2`, songs sharing a
picture of yours `version = 3`, and a song listing a torrent's file
`version = 4`.

### Removed songs

`muman remove` moves a song to `[[removed]]`. A key there that no song
lists is a tombstone: no playlist or dropped file adds it again.
`muman restore`, adding its video alone by URL, or deleting the table
by hand lets it back in.

A song you delete from the list by hand is kept out the same way. The
next `sync` puts its files in `sources/manual` under `[[removed]]`
rather than listing them again as files dropped in, and `status` says
so first.

## Sources

A source is a video yt-dlp fetched, a file of a torrent, a file of your
own, or a record a [lookup](library.md#lookups) kept.

### Videos and audio from the web

`muman add <URL>` asks yt-dlp what the URL names, a video, a playlist,
an album or a channel, and fetches each video's audio into
`sources/yt-dlp`, with its thumbnail, the subtitles people wrote and
its captions in the original language. muman never reads the picture
of a video. A Mix, the radio YouTube plays on from a video, is fetched
as its one video.

yt-dlp runs with `--ignore-config`, so your own yt-dlp config never
changes what a source holds; `[ytdlp]` in the song list sets the
format, subtitle languages and extra arguments instead. muman ships
three yt-dlp plugins, written to its cache folder on the first fetch.
One keeps out machine-translated captions, which trip YouTube's rate
limit. One makes a YouTube Music track's square album art its
thumbnail. One gives each file of an archive.org item the item's
creator and dates, as yt-dlp gives a file fetched alone. A yt-dlp that
cannot load them runs once more without them, with a warning.

muman strips each fetched file's metadata down to the fields it reads,
so no cookie, header or signed address from yt-dlp's requests stays in
it. A file it cannot clean is not kept.

YouTube changes what it serves every few months, and an older yt-dlp
then fails with a sign-in check, a format that is not available, or an
HTTP 403. A fetch that fails that way says to update yt-dlp and names
the version that ran.

The next `sync` fetches again a listed source whose file is gone. A
YouTube video is asked for in the format its song records, then as its
audio alone, then as `[ytdlp] format` says. A video of another site is
fetched from the address its `[sites]` table gives, else from the page
its song records. A fetch that fails waits an hour before the next
try, doubling each time up to a week; `sync --retry` tries at once. A
lookup's record gone from the store is fetched again by its ID, and
waits the same way when that fails. Only runs that fetch need yt-dlp.

### Torrents

`muman add <magnet link|FILE.torrent>` fetches a torrent's song files
with aria2c, each with the `.lrc` and pictures named as it is and its
folder's cover, by the rules for [files of your
own](#files-of-your-own). Scans, logs and the rest stay with the
torrent's peers. Each song file is listed as a song, or joins the song
it is the same recording as, and is tagged and looked up as a file of
your own is.

The metainfo is kept in the store, and the torrent's name, trackers and
v2 hash in the song list's `[torrents."<hash>"]`. A private torrent's
trackers are left out, since their addresses hold a key of yours.
Peers see your IP address and get pieces from you: [what aria2c
shares](configuration.md#what-aria2c-shares) says how much and how to
limit it.

A fetch with nothing arriving for `[torrent] stall` fails. It keeps
each file whose pieces all check, and the rest waits in
`partial/torrent`. aria2c cannot fetch a torrent of v2 alone, so muman
refuses one.

`--from <FOLDER>` takes up a torrent another client downloaded there.
Its files are copied in and checked piece by piece, and only what is
missing or wrong is fetched, so a complete download needs no peer.

A listed file gone from the store is fetched again by the next `sync`,
from the kept metainfo, else from the torrent's hash and recorded
trackers. A file no song lists is never fetched again. A failed fetch
waits as a video's does. A file found [damaged](#damaged-sources), or
holding other audio than its song was built from, is fetched again
too, and only its pieces that fail their check are downloaded.

Each file is kept in the torrent's folder at its path in the torrent,
with what Windows refuses in a name replaced by a lookalike. Where two
files land at one place that way, or differ only by case, the first
the torrent lists is kept and the other is left with a warning. While a
listed torrent's metainfo is gone, its folder is kept and its song
files are read without the lyrics and pictures they bring.

### Files of your own

`muman add <FILE|FOLDER>` copies audio into `sources/manual`. You can
also drop files there yourself; the next `sync` lists them. muman reads
a dropped file only 10 s after it arrived, whatever modification time
the copy kept, so it never reads a file still being copied. A listed
file you move within the folder is followed, not listed again.

A song file brings what sits beside it:

- the `.lrc` named as it is, in any case: `Paper Comets.flac` brings
  `Paper Comets.lrc`;
- the JPEG, PNG or WebP pictures named as it is, as `paper comets.jpg`;
- its folder's `cover`, `folder`, `front` or `album` picture.

A `.lrc` may be in UTF-8 or UTF-16, or in an older encoding such as
GBK, Shift-JIS or Windows-1252, which muman tells by its bytes.

A folder given to `add` is copied except for what is no song, lyrics or
picture, as a booklet `.pdf`, which is left out and named. A file
yt-dlp fetched elsewhere, named `<title> [<id>]` with yt-dlp's metadata
inside, goes into `sources/yt-dlp` under its key instead. If the store
holds that key already, the file joins its song and is not copied
again. A lyrics file, picture or tag file given alone goes to the songs
it belongs to: see [giving songs lyrics, pictures and
tags](songs.md#giving-songs-lyrics-pictures-and-tags).

Links in `sources/manual` are read as what they lead to, except a link
back to a folder it is inside, which is passed over. In a folder given
to `add`, or the folder `--from` names, a link is followed only to a
place inside that folder. One leading out, as a `Song.lrc` linked to a
key of yours, is left out and named.

### A source that changes

muman remembers what each source gave each song, so it can tell when a
source changes. Each sync records, in the song, a digest of each part a
source gave it: its audio, covers, lyrics and tags. For what yt-dlp
fetched it also records the format the site served and its size:

```toml
held."youtube.com:vid00000001" = { audio = "7c1cfa040b3b82c0", cover = "5be1d2a03f4c6e71", lyrics = "e04f1c7d2b9a6385", tags = "9a3c0e5f7d1b2468", format = "251", size = 3456789 }
```

A part the source does not offer is left out. A video of a site not in
`[sites]` also records the page it came from, so a home made again from
the song list alone can fetch it.

Each digest covers one part alone: audio and subtitles by their
packets, which a remux keeps, a picture by its bytes, and tags by the
metadata fields muman reads them from, before cleaning. So a source
changes only when its content does.

What muman fetched changes only when it is fetched again, after its
file is lost, and the site may then serve something else: a re-encoded
video, edited audio, another format, a new thumbnail, new subtitles or
an edited title. The song keeps the file built from what was recorded,
with a warning naming the source and each part that changed, until
`sync --accept` takes what the source holds now. Deleting the `held`
line does the same. A song with no file left, as in a home made again
from the song list, is built from what its sources hold now, and the
warning stays until you accept.

A file of your own in `sources/manual`, or a picture or `.lrc` beside
it, is yours to change. Its song follows the change, says so in a line,
and records it anew. `muman check --upstream` asks each site whether a
fetch would bring other audio or tags, before any copy is lost
([status, info and check](library.md#status-info-and-check)).

### Damaged sources

muman decodes every source whole once, and that decode finds two kinds
of damage:

- **Cut off:** the audio's packets end before its stream says, as a
  copy or download stopped short leaves them. The length a file states
  counts every stream in it, so a subtitle running past the audio is no
  cut.
- **Missing:** less audio decodes than the packets span, as from holes
  a download that lost pieces leaves, or packets too corrupt to decode.

A cut-off source is encoded rather than copied, so its song ends
cleanly. A source missing audio ranks below any whole copy of the
recording. A damaged download is fetched again, at once and then less
often, up to five times or until `sync --retry`. The whole copy it
brings is taken without `--accept`, since nobody chose the damage. A
damaged file of your own is named and left for you to replace.

When a source's audio changed and the new audio is damaged, muman does
not follow it. The song keeps its file and fails, with exit code 4,
until you put the whole file back or `sync --force` writes the song
from what the source holds.

### New sources

muman fingerprints each new source and compares it with every listed
song. It counts the stretches where two recordings agree and ignores
silence and held notes, which any two songs share:

| The two agree on | Span of the longer | Verdict |
|---|---|---|
| 30% or more, over at least 6 s | 80% or more | The same recording: the source joins the song |
| 50% or more, over 1 to 6 s | 80% or more | The same recording, too short for 6 s, as an interlude |
| 30% to 50%, over 1 to 6 s | 80% or more | Unsure of a short track: asked |
| 30% or more, over at least 6 s | Less | An excerpt, or a video with a skit: asked |
| 10% to 30% | 80% or more | Another master or mix: asked |
| Less | Any | A different song |

Asked means shown beside the song on a terminal, for you to join or
keep apart. Without a terminal the source becomes a song of its own,
with a warning. `-y` joins every unsure match, `--new` compares
nothing, and `--verbose` shows how close each song came.
