# Guide

muman keeps a music library in step with a song list, `songs.toml`,
that you edit by hand or from the command line. Each song lists the
sources it may be made from. muman measures every source, picks the best
audio, cover, lyrics and tags among them, and writes the song once into
the library as a tagged file, in its source's codec or one you choose.
This page is the home, the song list and its sources; how the best of
each is picked, the lookups, and the commands that change, report on and
export the library are in [making the library](library.md). Where the
home and the library are, which programs run, and the settings that
shape the library are in [configuration](configuration.md); every
command and flag is in [the command reference](cli.md).

## How muman works

The home holds everything needed to make the library again; the library
holds only what muman renders from it.

| Path in the home | Holds |
|---|---|
| `songs.toml` | The song list: every song, its sources, tags and pins, and the settings |
| `state.json` | Which library files muman wrote and from what, and every source's measures |
| `sources/yt-dlp/<site>/<id>.mkv` | What yt-dlp fetched, video, subtitles and metadata in one file, at a path of its key alone, one too long for a name cut into folders; anything else there is no source, and `purge` deletes it |
| `sources/manual/` | Files you dropped in, at any depth, with lyrics and pictures beside them |
| `sources/lrclib/<id>.lrc` | Lyrics found on LRCLIB, with the record they came from |
| `sources/musicbrainz/<id>.json` | Tags found on MusicBrainz: a recording and the release picked for it |
| `sources/coverart/<id>.jpg` | Album covers found on the Cover Art Archive, shared by every song of the album |
| `sources/torrent/<hash>.torrent` | A torrent's metainfo, which names its files |
| `sources/torrent/<hash>/` | A torrent's files, each at its path in the torrent with what Windows refuses in a name replaced by a lookalike; of two files kept at one place so, or apart only by case, the first the torrent lists, the other left with a warning; a file the metainfo does not name is no source, and `purge` deletes it, but while a listed torrent's metainfo is gone its folder is kept and its song files read without what they bring |
| `history/` | What the latest changing runs replaced or removed, for `undo` |
| `partial/` | Unfinished downloads, resumed by a later run |

Every `add` and `sync` runs in two phases. The network phase lists what
URLs name, fetches sources and makes the [lookups](library.md#lookups-and-providers)
that are due; each new source is matched to the songs it may belong to
([new sources](#new-sources)). The offline phase, reconcile, holds a
lock on the home:

1. Measure every listed source whose files changed since last measured,
   saving progress as it goes. Each part a song takes from a source,
   its audio, cover or lyrics, is known by a digest of its content, so
   a source fetched again or copied without its file times writes no
   song again while it holds the same.
1. Compare the audio of every pair of a song's sources.
1. Resolve each song to a plan: the best source for each aspect.
1. Find each song's file: the one made from most of its own sources,
   one file to one song, wherever it is now.
1. Move each song whose file differs from its plan at most in its tags
   but whose path changed, as a new path template makes it, instead of
   encoding it again.
1. Write the tags of each song whose plan changed only in its tags into
   its file, encoding nothing; write each song whose plan changed
   otherwise, or whose file is gone; `sync --force` writes every song.
1. Delete every file muman wrote that no song makes any more.

`status` says what these steps would do by the same decisions. A song
moves into a path another song leaves, as `Untitled [vid00000001]`
becomes `Untitled` once the other is renamed; two songs that trade
paths are written again.

A long step shows how far it has got on stderr, as `--progress` says.
On a terminal, `auto` pins one status line under the messages, such as
`[5/5] Writing [====>   ] 812/1546  53%  41 s left  Rooms of Salt · Paper Comets`:
the step of the command's steps, the count, the time left at the rate
so far, and the songs or files under way; terminals that show a task's
progress in their tab or taskbar show it there too. On a narrower
terminal the line drops the percentage, then the bar, then the time
left, and stays one line. A step inside another, as a fetch of what a
lookup found, takes the line until it ends. Elsewhere, as in a
log, `auto` and `plain` write a line every tenth of the way or ten
seconds; `json` writes one JSON object a line, `start`, `progress` and
`finish` events, and each message, as `Added: …`, as a `message` event
with its `text`, so that a program reads every line; `none` writes
nothing. What a command does goes to stderr with its progress; what
`status`, `list`, `info`, `check` and `duplicates` report goes to stdout.

muman deletes only files `state.json` records as its own; anything else
in the library folder is left alone. A file whose size or time changed
since muman wrote it, as a tagger rewrites it, is left with a warning
until `sync --force`: a new path template still moves it, your change
with it, but a change to the song itself leaves it where it is, and
`sync --force` writes the song again and deletes it, `undo` putting it
back. A file whose time changed but whose bytes are still the ones
written, as a backup put back or a copy made without its times, is
muman's still; one whose size and time are as written but whose bytes
are not, as a tagger set to keep files' times leaves it, is changed
too, found as the file is about to be written over or removed. Each
file is written under a temporary name and renamed into place, so a
player never sees half a track. Library paths in messages use `/` on every system.

## The song list

```toml
version = 1

[defaults]
lyrics = ["en", "fr"]

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
| `version` | The file's format; a newer one is refused |
| `defaults.lyrics` | Subtitle languages lyrics are taken in, most preferred first |
| `sources` | Every source the song may be made from; a file belongs to one song only, while an `lrclib:` or `musicbrainz:` record may be listed by every song of its recording |
| `album`, `track` | The `[[album]]` the song is on and its place there |
| `audio`, `cover`, `lyrics` | A pin: that aspect from that source, whatever the measures say; `lyrics = false` for none |
| `lyrics_offset` | Moves the lyrics later, on top of the offset measured, as `"120 ms"`; a negative time moves them earlier |
| `tags.<name>` | Tags set over what the sources offer; an empty value sets nothing |
| `held."<key>"` | What a source held when the song was built, written by muman: [a source that changes](#a-source-that-changes) |
| `[[album]]` | A YouTube Music album added whole, with its `tags.<name>` over what its tracks offer |
| `[torrents."<hash>"]` | A torrent songs list files of: its `name`, `trackers` and `v2` hash, which fetch it again ([torrents](#torrents)) |
| `[[removed]]` | A song `remove` took out, kept whole under a `note` naming it |

A song has no ID of its own: it is its sources, and any of its keys
names it. A key is `<site>:<id>` for what yt-dlp fetched, `lrclib:<id>`
for LRCLIB lyrics, `musicbrainz:<id>` for a MusicBrainz recording,
`manual:<path>` for a file under `sources/manual`, or
`torrent:<hash>/<path>` for a file of a torrent.

A torrent's file is named by the torrent's hash, its v1 hash where it
has one, else its v2 hash, and the file's path inside the torrent
without the torrent's own folder, as `torrent:<hash>/CD1/01 Paper Comets.flac`;
a torrent of one file names it by its name. The hash names every byte
of the torrent, so its files never change.

A fetched source's site is the domain it came from and its ID the one
yt-dlp gives it there, as `youtube.com:vid00000001` or
`archive.org:<id>`; a file in an archive.org item of several is
`archive.org:<item>/<file>`, its name as it is, as
`archive.org:<item>/01 Lantern #2.mp3`, and is fetched again alone. A
site yt-dlp reaches by several addresses has one domain: a video from YouTube
Music, `youtu.be` or `m.youtube.com` is `youtube.com:<id>`, the one
source however it was found, and a track on any
`<artist>.bandcamp.com` is `bandcamp.com:<id>`. Which sites have one
domain, what IDs they take and where their sources are fetched again
from is the song list's `[sites]`
([configuration](configuration.md#settings-in-the-song-list)); any other
site is the domain of the page yt-dlp fetched, without `www.`.

An earlier muman named a source by yt-dlp's extractor, as `youtube:<id>`
or `archiveorg:<id>`. Such a key of an extractor the built-in `[sites]`
lists reads, in the song list and in a query, as its site's, and the
next command that writes the song list writes it so; the library is
not written again for it. A key of an extractor whose site muman does
not know reads as it is until a sync renames it by the page its source
was fetched from; one of yt-dlp's generic extractor, a file fetched by
its address, keeps its name, since its source recorded only the mirror
it was served from.
`tags.<name>` takes `title`, `artist`, `album`, `album_artist`, `genre`,
`date` (or `year`), `track`, `disc`, `track_total`, `disc_total`, `isrc`,
`release_country`, the MusicBrainz IDs under Picard's names
(`musicbrainz_trackid`, `musicbrainz_albumid` and the like); `bpm`,
`initialkey`, `compilation`, `composer`, `lyricist`, `conductor`,
`remixer`, `label`, `catalognumber`, `barcode`, `asin`, `artists`,
`artistsort`, `albumartistsort`, `titlesort`, `albumsort`,
`composersort`, `originaldate`, `releasetype`, `releasestatus`, `media`,
`script`, `discsubtitle`, `mood`, `grouping`, `work`, `language`,
`copyright` and `comment`, each written where Picard writes it in each
format; or any other Vorbis comment name; a list sets a tag several times, and `true`
or `false` is written `1` or `0`. Each value is read as its field holds
it: a track or disc as `3`, or `3/12`, which sets the total too unless
the song names it; a date as `YYYY-MM-DD`, `YYYY-MM` or a year, read
from `2019/07/04` or `20190704` too; an ISRC without the dashes it is
shown with; a tempo as a number of up to two decimals, rounded where a
format holds whole ones (`TBPM`, `tmpo`); a barcode as its digits; a
language as its ISO 639-2 code, as `jpn`. A key is written as `KEY` and
`INITIALKEY` in Vorbis comments, as Picard and DJ tools name it, and in
`TKEY` where it is a key as that frame holds one (`Abm`, `F#`), else as
a user text frame. Without a `language` of its own, a song's is its
lyrics' language. A value its field cannot hold, as a date of `July 2019`
or `2019-02-30`, a tempo that is not from 1 to 9999 once rounded, a
ReplayGain gain or peak, which muman measures, or a table, refuses the
song list, naming the song by its number and first source; so does a tag
name a Vorbis comment cannot carry, of other than printable ASCII or with
`=`. A TOML date, `date = 2019-07-04` unquoted, is read as its text. `add
--artist`, `--album` and the other tag flags set these on every song an
`add` lists. `muman status` shows each tag's value and the source it
came from.

A song's audio keeps the chapters of the source it is taken from, as an
M4B audiobook or a video in parts has them: the song stays one file, and
a player skips from chapter to chapter within it. They are written as
MP4 chapters, as `CHAPTER000` and `CHAPTER000NAME` comments in FLAC, Ogg
and Opus, and as `CTOC` and `CHAP` frames in MP3; WavPack holds none. A
source of one chapter has none to keep.

The file also holds the settings of [configuration](configuration.md),
`[clean.*]` ([cleaning](library.md#cleaning)) and the `[providers.*]`,
`[[trigger]]` and `[[hook]]` tables below. Some run programs or send
requests as you, which muman asks you to [trust](configuration.md#commands-servers-and-secrets)
first: read a song list you are given as you would a script.
muman edits it in place: comments, order and keys it does not know
survive every write, and two runs at once keep each other's songs. A
key that does not parse, a file two songs list, or a pin to a source the
song does not list stops the run before anything is written. A song list
in which songs share a record is written as `version = 2`, which muman
before 0.2 refuses rather than misreads; one in which a song lists a
torrent's file is written as `version = 4`, which an older muman refuses.

A `[[removed]]` key no song lists is a tombstone: no playlist or dropped
file lists it again. `muman restore`, adding its video alone by URL, or
deleting the table by hand lets it back.

A song deleted from the list by hand is kept out the same way: the next
`sync` puts each of its files in `sources/manual` under `[[removed]]`,
rather than listing them again as files dropped in, and `status` says
so first.

## Sources

A source is a video yt-dlp fetched, a file of your own, a file of a
torrent, lyrics from
[LRCLIB](https://lrclib.net), a free lyrics database that songs without
timed lyrics [look up](library.md#lookups-and-providers), or tags from
[MusicBrainz](https://musicbrainz.org), the open music encyclopedia,
which songs on no album look up.

### yt-dlp

`muman add <URL>` asks yt-dlp what the URL names, a video, a playlist,
an album or a channel, and fetches the audio of each video into
`sources/yt-dlp`, with its thumbnail, the subtitles people wrote and the
original-language captions; muman never reads the video itself, and the
default `[ytdlp] format` keeps none. A Mix, the radio YouTube plays on
from a video, is fetched as its one video. yt-dlp runs with `--ignore-config`,
so your own yt-dlp config never changes what a source holds; `[ytdlp]`
sets the format, subtitle languages and extra arguments instead
([configuration](configuration.md)). Three yt-dlp plugins ship inside
muman and are written to its cache folder on the first fetch: one keeps
out the machine-translated captions that trip YouTube's rate limit, one
makes a YouTube Music track's square album art its thumbnail, one gives
each file of an archive.org item the item's creator and dates, as yt-dlp
does a file fetched alone. A yt-dlp that cannot load them is run once
more without them, with a warning, rather than fetch nothing. Each file
fetched, or given to `add` as one yt-dlp fetched, keeps the info JSON
yt-dlp embeds without the cookies and HTTP headers its requests sent,
so a login passed in `[ytdlp] args` stays out of the store.

YouTube changes what it serves every few months, and a yt-dlp behind it
fails with a sign-in check, a format that is not available or a 403
where the next release reads the page again. A fetch that fails so says
to update yt-dlp, naming the version that ran.

A listed source whose file is gone is fetched again by the next `sync`:
a YouTube video in the format its song records first, its audio alone
next, then as `[ytdlp] format` says; a video of another site from the
address its `[sites]` table gives, else the page its song records, as
that site's formats are numbered by their place in its list.
One that fails waits an hour before the next try, doubling each time;
`sync --retry` tries at once. A record a [lookup](library.md#lookups-and-providers)
kept, gone from the store, is fetched again by its ID, and waits alike
when that fails or its service no longer has it. yt-dlp is needed only
by runs that fetch.

### Torrents

`muman add <magnet link|FILE.torrent>` fetches a torrent's song files
with aria2c, each with the `.lrc` and pictures named as it is and its
folder's cover, by the same rules as [manual files](#manual-files-and-sidecars);
scans, logs and the rest stay on its peers. Each song file is listed as
a song or joins the song it is the same recording as, its tags read
from the file as a file of your own is, and lookups are made for it as
for one. The torrent's metainfo is kept in the store, and its name,
trackers and v2 hash in the song list's `[torrents."<hash>"]`; a private
torrent's trackers are left out, since their addresses hold a key of
your own. Peers see your IP address while it fetches. A fetch with
nothing arriving for `[torrent] stall` fails, keeping each file whose
pieces check; the rest waits in `partial/torrent` ([configuration](configuration.md)). A torrent of BitTorrent v2 alone,
which aria2c cannot fetch, is refused.
`--from <FOLDER>` takes up a torrent another client downloaded there:
its files are copied in and checked piece by piece, and only what is
missing or wrong is fetched, so a complete download needs no peer.
A listed file gone from the store is fetched again by the next `sync`,
with what it brings, from the kept metainfo, else from the torrent's
hash and recorded trackers; a file no song lists is not, and one that
fails waits as a video does. A file found [damaged](#a-source-that-changes),
or holding other audio than its song was built from, is fetched again
as a damaged download is, only its pieces that do not check fetched.

### Manual files and sidecars

`muman add <FILE|FOLDER>` copies audio into `sources/manual`, or you
drop it there yourself and the next `sync` lists it. A file that
yt-dlp fetched elsewhere, named `<title> [<id>]` and holding the info
yt-dlp embeds, goes into `sources/yt-dlp` under its key instead, given
alone or in a folder; one of a key the store holds already joins its
song and is not copied again. A dropped file waits until ten seconds after it arrived,
whatever modification time a copy kept for it, so one still being
copied is not read half written, and a listed file moved within
the folder is followed rather than listed again. A song file brings the
`.lrc` and the JPEG, PNG or WebP pictures named as it is, in any case,
as `Paper Comets.flac` brings `Paper Comets.lrc` and `paper comets.jpg`,
and its folder's `cover`, `folder`, `front` or `album` picture. A
folder given to `add` is copied but for what is no song, lyrics or
picture, as a booklet `.pdf`, which is left out and named. A `.lrc` or
a picture can also be listed as a source of its own, and one given to `add` alone goes to the
songs it belongs to ([lyrics, pictures and tags of their
own](#lyrics-pictures-and-tags-of-their-own)). A `.lrc` reads in UTF-8 or UTF-16, or in
an older encoding such as GBK, Shift-JIS or Windows-1252, which muman
tells by its bytes. A folder linked into the manual folder is read as
the folder it links to, except a link back to a folder it is inside,
which is passed over.

### A source that changes

Each sync records in the song what each of its sources holds of every
part of a built file, the audio, covers, lyrics and tags, as a digest of
each, with the format a site served and its size for what yt-dlp
fetched:

```toml
held."youtube.com:vid00000001" = { audio = "7c1cfa040b3b82c0", cover = "5be1d2a03f4c6e71", lyrics = "e04f1c7d2b9a6385", tags = "9a3c0e5f7d1b2468", format = "251", size = 3456789 }
```

A part a source does not offer is left out. A video of another site
also records the page it came from, which its key does not name, so a
home made again from the song list alone can fetch it.

Each digest is of that part alone: audio and subtitles by their packets,
which a remux leaves, a picture by its bytes, and tags by the fields of
the info JSON they are read from, or a file's own tags, before muman
cleans them. A fetched source changes only when it is fetched again,
its file gone, and a site may then serve otherwise: the video
re-encoded, its audio edited, another format offered or preferred, a new
thumbnail, new subtitles, a title edited. Its song keeps the file it
was built from, with a warning naming the source and each part that
changed, until `sync --accept` takes what it holds now and records it;
deleting the line does the same. A song with no file left, as in a new
home made from the song list alone, is built from what its sources hold
now, and the warning stays until accepted. A file of your own in
`sources/manual`, or a picture or `.lrc` beside it, is yours to change:
its song follows it, said in a line, and its record is written anew.
A source whose audio changed and is now damaged is not followed: its
song keeps the file it was built from and fails, exit 4, until you put
the whole file back or `sync --force` writes the song from what it
holds. Damage is found in the decode every source has: audio **cut
off**, its packets ending before its audio stream says, as a copy or a
download stopped short leaves it; or audio **missing**, less decoding
than its packets span, as holes a download that lost pieces leaves, or
packets too corrupt to decode. The length a file states counts every
stream in it, so a subtitle running past the audio is no cut. A cut-off
source is encoded rather than copied, so its song ends cleanly; one
missing audio ranks below any copy of the recording that is whole. A
damaged download is fetched again, at once and then less often, up to
five times or until `sync --retry`; the whole copy it brings is taken,
without `--accept`, as nobody chose the damage. A damaged file of your
own is named, and left for you to replace. `check --upstream` asks each site whether a fetch again would bring
other audio or tags before any is lost
([status, info and check](library.md#status-info-and-check)).

### New sources

A new source is fingerprinted and compared with every listed song,
counting the stretches where the two agree and ignoring silence and held
notes that any two songs share:

| Agreement | Covering the longer recording | Verdict |
|---|---|---|
| 30% or more, over at least 6 s | 80% or more | The same recording: joins that song |
| 50% or more, over 1 to 6 s | 80% or more | The same recording: a track too short for 6 s, as an interlude |
| 30% to 50%, over 1 to 6 s | 80% or more | Unsure of a short track: asked |
| 30% or more, over at least 6 s | Less | An excerpt, or a video with a skit: asked |
| 10% to 30% | 80% or more | Another master or mix: asked |
| Less | Any | A different song |

Asked means on a terminal; without one, the source becomes a song of its
own, with a warning. `-y` joins every unsure match, `--new` compares
nothing, and `--verbose` shows how close the nearest songs came.

### Lyrics, pictures and tags of their own

`muman add` also takes a file that is no song and gives it to the songs
it belongs to:

| File | Is |
|---|---|
| `.lrc`, a `.txt` of plain lyrics, `.srt`, `.vtt` | Lyrics, copied in as `.lrc`, listed among the song's sources |
| JPEG, PNG or WebP | A cover, listed by every song whose cover looks like it |
| `NAME=value` lines (`.txt`, `.tags`, `.vc`), `;FFMETADATA1` (`.ffmeta`), `.json` from beets or MusicBrainz | Tags, set on the song by hand and not kept |
| `.cue` | Tags of the album and each track, each track's set on its song |

A lyrics file or picture is ranked like any other source, so a song may
keep lyrics that are timed where the new ones are not; pin it with
`muman set` to choose it. Gains, peaks, embedded pictures and lyrics,
and encoder names in a tag file describe another file and are skipped.
A beets field sets the Vorbis comment Picard writes it as, `label` as
`LABEL`; one Picard has no name for is skipped, and what beets records
of its own file, as its path and bitrate, is no tag. `--verbose` names
each tag skipped.

Each file is matched by the strongest of what it shows: an ID it shares
with the song (a video ID in its name, an ISRC, a MusicBrainz recording
ID); its name, as the song's file is named; its fields, from an `.lrc`'s
`[ti:]`, `[ar:]`, `[al:]` and `[length:]`, a tag file, or its own name
read as `03 - Artist - Title`, `Artist - Title` or `Title`; and what it
holds, lyrics against the words the song's lyrics sing, a picture
against the song's covers by how they look, unchanged by scale,
recompression or a frame of bars. Names are compared in their own
script: accents, case, width and Traditional or Simplified forms count
for little, but kanji and romaji, or Hangul and Latin, cannot be told
apart by their letters and decide nothing.

| Evidence | Verdict |
|---|---|
| An ID, its file's name, words or a cover alike | The song's: given to it |
| Title, artist, album, track and length all but equal, and nearer than any other song | The song's |
| A title alone, two songs as near, another version (live, instrumental, remix…), or a near match | Asked |
| Lyrics ending after the song does, or stating another length | Not the song's |

Asked means on a terminal: you pick among the likeliest songs, each
shown with why, as `Lantern Weather — Marlo Venn · The Glass Orchards ·
3:20   close name, length agrees`, where `same name`, `close name` or `other name` says how the
names compare and the rest what else agrees or not. You may also search
the song list with a query as `muman list` reads one, or leave the file
out, which copies nothing. `-y` takes the likeliest when it is clearly
the nearest. Without a terminal, a file not surely a song's is left
out, and `add` exits with 5 once the rest is done; a file that does not
read, as a `.pdf`, an empty `.lrc`, a picture that does not decode whole,
a song file in which ffprobe finds no audio or a cue sheet splitting one song's
file into tracks, is not added and `add` exits
with 4, the songs given with it written all the same. `--to` names the song
with a query instead, quoted as one word when it has several, and
matches nothing; for a picture it names every
song to give it to, for a cue sheet the songs its tracks go among:

```bash
muman add "Lantern Weather.lrc"
muman add front.jpg --to 'album:="The Glass Orchards"'
muman add words.txt --to "artist:venn lantern"
muman add album.cue rips/*.flac --new
```

A picture alike to the covers of more than three songs of more than one
album is asked about rather than given to all, and a cover given with
songs from its folder goes to those songs. Several songs may list one
picture of your own; a song list where they do is format version 3,
which an older muman refuses. Each song still lists a file of its own:
one whose every source another song lists too is refused. Lyrics are
one song's: the same lyrics given to a second song are copied for it.
A picture or `.lrc` moved within `sources/manual` is followed, as a
song file is.
