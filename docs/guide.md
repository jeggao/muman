# Guide

muman keeps a music library in step with a song list, `songs.toml`,
that you edit by hand or from the command line. Each song lists the
sources it may be made from. muman measures every source, picks the best
audio, cover, lyrics and tags among them, and writes the song once into
the library as a tagged file, in its source's codec or one you choose.
Where the home and the
library are, which programs run, and the settings that shape the
library are in [configuration](configuration.md); every command and flag
is in [the command reference](cli.md).

## How muman works

The home holds everything needed to make the library again; the library
holds only what muman renders from it.

| Path in the home | Holds |
|---|---|
| `songs.toml` | The song list: every song, its sources, tags and pins, and the settings |
| `state.json` | Which library files muman wrote and from what, and every source's measures |
| `sources/yt-dlp/<handle>/<title> [<id>].mkv` | What yt-dlp fetched, video, subtitles and metadata in one file |
| `sources/manual/` | Files you dropped in, at any depth, with lyrics and pictures beside them |
| `sources/lrclib/<id>.lrc` | Lyrics found on LRCLIB, with the record they came from |
| `sources/musicbrainz/<id>.json` | Tags found on MusicBrainz: a recording and the release picked for it |
| `sources/coverart/<id>.jpg` | Album covers found on the Cover Art Archive, shared by every song of the album |
| `history/` | What the latest changing runs replaced or removed, for `undo` |
| `partial/` | Unfinished downloads, resumed by a later run |

Every `add` and `sync` runs in two phases. The network phase lists what
URLs name, fetches sources and makes the [lookups](#lookups-and-providers)
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
`finish` events, for a program to read; `none` writes nothing. What a
command reports, as `status` does, stays on stdout.

muman deletes only files `state.json` records as its own; anything else
in the library folder is left alone. A file whose size or time changed
since muman wrote it, as a tagger rewrites it, is left with a warning
until `sync --force`: a new path template still moves it, your change
with it, but a change to the song itself leaves it where it is, and
`sync --force` writes the song again and deletes it, `undo` putting it
back. A file whose time changed but whose bytes are still the ones
written, as a backup put back or a copy made without its times, is
muman's still. Each file is written under a temporary name and
renamed into place, so a player never sees half a track. Library paths
in messages use `/` on every system.

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
| `[[removed]]` | A song `remove` took out, kept whole under a `note` naming it |

A song has no ID of its own: it is its sources, and any of its keys
names it. A key is `<site>:<id>` for what yt-dlp fetched, `lrclib:<id>`
for LRCLIB lyrics, `musicbrainz:<id>` for a MusicBrainz recording, or
`manual:<path>` for a file under `sources/manual`.

A fetched source's site is the domain it came from and its ID the one
yt-dlp gives it there, as `youtube.com:vid00000001` or
`archive.org:<id>`; a file in an archive.org item of several is
`archive.org:<item>/<file>`, and is fetched again alone. A site yt-dlp
reaches by several addresses has one domain: a video from YouTube
Music, `youtu.be` or `m.youtube.com` is `youtube.com:<id>`, the one
source however it was found, and a track on any
`<artist>.bandcamp.com` is `bandcamp.com:<id>`. Which sites have one
domain, what IDs they take and where their sources are fetched again
from is the song list's `[sites]`
([configuration](configuration.md#settings-in-the-song-list)); any other
site is the domain of the page yt-dlp fetched, without `www.`.

An earlier muman named a source by yt-dlp's extractor, as
`youtube:<id>`, `youtubetab:<id>` or `archiveorg:<id>`. Such a key
still reads, in the song list and in a query, as its site's, and the
next command that writes the song list writes it so; the library is
not written again for it. A key of an extractor whose site muman does
not know reads as it is until a sync renames it by the page its source
was fetched from; one of yt-dlp's generic extractor, a file fetched by
its address, keeps its name, since its source recorded only the mirror
it was served from.
`tags.<name>` takes `title`, `artist`, `album`, `album_artist`, `genre`,
`date` (or `year`), `track`, `disc`, `track_total`, `disc_total`, `isrc`,
`release_country`, the MusicBrainz IDs under Picard's names
(`musicbrainz_trackid`, `musicbrainz_albumid` and the like),
`replaygain_track_gain` and its `_peak` and `album_` kin, or any
other Vorbis comment name; a list sets a tag several times. `add
--artist`, `--album` and the other tag flags set these on every song an
`add` lists. `muman status` shows each tag's value and the source it
came from.

The file also holds the settings tables of
[configuration](configuration.md), `[clean.*]` ([cleaning](#cleaning)),
and the `[providers.*]`, `[[trigger]]` and `[[hook]]` tables below.
muman edits it in place: comments, order and keys it does not know
survive every write, and two runs at once keep each other's songs. A
key that does not parse, a file two songs list, or a pin to a source the
song does not list stops the run before anything is written. A song list
in which songs share a record is written as `version = 2`, which muman
before 0.2 refuses rather than misreads.

A `[[removed]]` key no song lists is a tombstone: no playlist or dropped
file lists it again. `muman restore`, adding its video alone by URL, or
deleting the table by hand lets it back.

A song deleted from the list by hand is kept out the same way: the next
`sync` puts each of its files in `sources/manual` under `[[removed]]`,
rather than listing them again as files dropped in, and `status` says
so first.

## Sources

A source is a video yt-dlp fetched, a file of your own, lyrics from
[LRCLIB](https://lrclib.net), a free lyrics database that songs without
timed lyrics [look up](#lookups-and-providers), or tags from
[MusicBrainz](https://musicbrainz.org), the open music encyclopedia,
which songs on no album look up.

### yt-dlp

`muman add <URL>` asks yt-dlp what the URL names, a video, a playlist,
an album or a channel, and fetches each video whole into
`sources/yt-dlp`, with the subtitles people wrote and the
original-language captions. A Mix, the radio YouTube plays on from a
video, is fetched as its one video. yt-dlp runs with `--ignore-config`,
so your own yt-dlp config never changes what a source holds; `[ytdlp]`
sets the format, subtitle languages and extra arguments instead
([configuration](configuration.md)). Two yt-dlp plugins ship inside
muman and are written to its cache folder on the first fetch: one keeps
out the machine-translated captions that trip YouTube's rate limit, one
makes a YouTube Music track's square album art its thumbnail.

A listed source whose file is gone is fetched again by the next `sync`:
a YouTube video in the format its song records first, its audio alone
next, then as `[ytdlp] format` says; a video of another site from the
address its `[sites]` table gives, else the page its song records, as
that site's formats are numbered by their place in its list.
One that fails waits an hour before the next try, doubling each time;
`sync --retry` tries at once. yt-dlp is needed only by runs that fetch.

### Manual files and sidecars

`muman add <FILE|FOLDER>` copies audio into `sources/manual`, or you
drop it there yourself and the next `sync` lists it. A file that
yt-dlp fetched elsewhere goes into `sources/yt-dlp` under its key
instead. A dropped file waits until it is ten seconds old, so one still
being copied is not read half written, and a listed file moved within
the folder is followed rather than listed again. A song file brings the
`.lrc` and the JPEG, PNG or WebP pictures named as it is, as
`Paper Comets.flac` brings `Paper Comets.lrc`, and its folder's `cover`,
`folder`, `front` or `album` picture. A `.lrc` or a picture can also be
listed as a source of its own, and one given to `add` alone goes to the
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
`check --upstream` asks each site whether a fetch again would bring
other audio or tags before any is lost
([status, info and check](#status-info-and-check)).

### New sources

A new source is fingerprinted and compared with every listed song,
counting the stretches where the two agree and ignoring silence and held
notes that any two songs share:

| Agreement | Covering the longer recording | Verdict |
|---|---|---|
| 30% or more, over at least 6 s | 80% or more | The same recording: joins that song |
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
read, as an empty `.lrc`, a picture that does not decode, a song file
in which ffprobe finds no audio or a cue sheet splitting one song's
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

## How the best of each is picked

No rule names a kind of source: a release, an upload and a file of your
own are ranked by what they measure. A pin is the only override, and the
order of `sources` breaks the last tie. Measures are counted in steps,
so noise never decides between near-equals; each step is multiplied by
its measure's weight and the sums compared, lowest best. A source with a
measure that could not be taken ranks after those with fewer such. Of
audio scored alike, lossless beats lossy, then the more bits its samples
use, as 24 against 16 padded to 24, then the lower sample rate, which an
upsample would only waste.

The weights `songs.toml` starts with rank by each measure in turn, as
the table lists them: each outweighs everything after it. `[quality]`
in the song list switches a measure off, changes its weight, or changes
the steps and cutoffs it is judged by; lower one weight and the
measures after it can make up for it. See
[configuration](configuration.md#how-sources-are-ranked).

| Aspect | Criteria, by default in order |
|---|---|
| Audio | Least sound beyond the song; widest real bandwidth; real stereo; least clipping |
| Cover | Square content; effective resolution; fewest block artifacts |
| Lyrics | Timed; in a preferred language; surest to be the chosen audio's recording; spanning most of the song |
| Tags | A dedicated field over one read off a video title; fewest decorations; most sources agreeing |

**Audio.** Sound beyond the song is how long a source plays outside the
stretch it shares with another source of the song: a music video's
intro counts, silence does not. Bandwidth is where a lowpass cuts the
sound off, so a FLAC transcoded from a lossy file measures as narrow as
that file, while a recording whose treble fades on its own measures
full. Real stereo tells stereo from mono copied into two channels, even
at different levels or a few samples apart, by the front two of more;
clipping is the share of samples stuck at full scale or at the audio's
own peak, or piled up just under full scale where a lossy encoder
smeared them. Float lossless audio can hold samples past full scale, so
in it only those at its own peak count. Every channel is measured as it
is, never mixed into two. The chosen audio is
copied when `[audio] codecs` lists its codec; any other is encoded,
lossless audio to `[audio] lossless` and lossy audio to `[audio] lossy`,
at the bitrate `[audio]` sets. A file with several audio streams is made
from the best: lossless before lossy, then the most channels, then the
highest rate. Lossless audio a codec cannot keep whole, as 32-bit, float
or more than eight channels in FLAC, is written in one that can, as
[Codecs](configuration.md#codecs) says.

**Covers.** A picture that is not square is checked for bars, and a
video frame with square art in the middle is cropped to that art.
Effective resolution is the size up to which a picture keeps its
detail, so an upscale measures the size it was made from.

**Lyrics.** Only subtitles a person wrote count; generated captions
never do. Lyrics from a source other than the chosen audio must be the
same recording, and are shifted by the offset measured between the two,
and stretched when one plays a little faster than the other.
Cues such as `[Music]`, symbol-only lines and credits are dropped. A
`.lrc` with no audio of its own is taken as timed for the song; one that
states its length, as LRCLIB's do, is trusted as far as that length
agrees with the chosen audio's. Whether lyrics go beside the song,
inside it or both is a `[library]` setting.

**Tags.** Each field describes the recording or the release. The
recording's, title, artist, genre, ISRC and the recording's and artists'
MusicBrainz IDs, each come from whichever source offers it best, the
ISRC and IDs only from a source that titles the recording as the song
is titled. The release's, album, album artist, track, disc, date, totals, country and
the release's MusicBrainz IDs, come together from the one source whose
album ranks best, so an album never splits across folders and one
release's IDs never mix with another's; a song on no album is a single
named for its title. ReplayGain's track gain and peak are measured from
the audio the song is, and its album gain and peak over every song of
its album, as [Loudness](configuration.md#loudness) says; with
`[loudness] mode = "off"`, they come only from the tags of the source
whose audio the song is, the album's only when the release's fields come
from it too, since another source's loudness is not this audio's. An
Opus file holds the gains as `R128_TRACK_GAIN` and `R128_ALBUM_GAIN` and
no peaks, as Opus players read them, and an Opus source's R128 gains are
read as ReplayGain's. A file's own tags offer every field by its Vorbis
name or the name Picard gives it in MP3 and MP4, and a track written
`3/12` offers its total too. Fields with no tag of their own in MP3 or
MP4 are written as Picard writes them there. A value over 4 KiB is
offered by no source, and control characters and those that reorder
text are dropped from every value.

### Cleaning

Every tag a source offers is cleaned before ranking, so `Paper Comets
(Album Version)` agrees with `Paper Comets`. Tags you set in
a song's or an album's `tags.<name>` are never cleaned. Each rule is a switch
under `[clean.<tier>]`, and every write lists them all, so the file
names every rule there is; `muman status` names each rule that changed
a tag as `<tier>.<key>`.

| Tier | Its rules |
|---|---|
| `tidy` | Fix spacing and punctuation only; never remove a word |
| `packaging` | Remove words naming the release or video: `(Official Video)`, `[Remastered]`, `VEVO` |
| `structure` | Split or swap a whole value by its shape: artists joined by `;`, a disc number closing an album |
| `guesswork` | Read a video title by convention, `Marlo Venn - Paper Comets (feat. Ada Quill)`; can misread |

## YouTube Music matching and albums

An upload often has a release on YouTube Music: a track whose metadata
names the song, its artists and album, with the square art the upload
lacks. Before fetching, `add` searches YouTube Music's songs for each
new video. A candidate must carry its track name in the upload's title,
an artist in the upload's channel or title, and a length close to the
upload's. The two recordings are then aligned at one offset, or along a
line when one plays up to 1% faster; they are one recording when they
correlate at 0.8 or more and at least 80% of their windows agree. The
same comparison measures sound beyond the song.

When the two are one recording, both become the song's sources and each
aspect is picked by its measures. Otherwise only the release is listed
and the upload is recorded as replaced, so it is never fetched again.
`add --no-match` keeps each video as uploaded.

YouTube Music names album playlists `OLAK5uy_…`; any other playlist is
a list of separate songs. An album added whole becomes an `[[album]]`
keyed `youtube.com:playlist/<id>`, and each track a song with `album` and `track`
set from its place in the full listing. A track already listed only
gains its album and place.

## Lookups and providers

A song looks for sources it lacks. Each source comes from a provider:

| Provider | Its sources | Looked up by |
|---|---|---|
| `manual` | Files you dropped in | Never looked up |
| `youtube` | A video that is not a release | Searching YouTube for the artist and title: the first video with a person's subtitles that is the same recording, the artist's channel first |
| `youtube-music` | A release | Searching YouTube Music's songs, as matching does |
| `lrclib` | Lyrics from an LRCLIB server | Title, first artist, album and length |
| `musicbrainz` | Tags from a MusicBrainz server: the recording's and its release's names, numbers, date, ISRCs and IDs | Title, first artist and length, preferring the song's album |
| `acoustid` | The same MusicBrainz records, found on an AcoustID server | The fingerprint and length of the song's audio, preferring a recording named as the song is |
| `coverart` | The front cover of the song's album, from the Cover Art Archive | The album's MusicBrainz release group or release ID, else its title and album artist searched on MusicBrainz |

A trigger makes a song with a source from any provider in `from`, and
none from `find`, look `find` up when `when` holds: `always`,
`no-lyrics`, `no-timed-lyrics`, `no-album` (neither a source nor the
song list names an album) or `small-cover` (no cover, or one with too
little detail to look sharp, as `info` counts). The built-in triggers:

| From | Finds | When |
|---|---|---|
| `youtube` | `youtube-music` | `always` |
| `youtube-music` | `youtube` | `no-timed-lyrics` |
| `manual`, `youtube`, `youtube-music` | `lrclib` | `no-timed-lyrics` |
| `manual`, `youtube`, `youtube-music` | `acoustid` | `no-album` |
| `manual`, `youtube`, `youtube-music` | `musicbrainz` | `no-album` |
| `manual`, `youtube`, `youtube-music` | `coverart` | `small-cover` |

`--offline`, or `MUMAN_OFFLINE=1`, makes a run look nothing up and
fetch nothing, and refuses URLs. Any `[[trigger]]` in the song list
replaces all of them. A
`[providers.<name>]` table sets `enabled`, `concurrency` (lookups at
once), `recheck` (how long a lookup that found nothing waits, as
`"30 days"`),
`per_run` (the most one run makes, `0` for no limit), for `lrclib`,
`musicbrainz`, `acoustid` and `coverart`, `url`, another server, such as
a mirror, and for `acoustid`, `key`, the application key it is asked
with, muman's own unless you set one:

```toml
[[trigger]]
from = ["manual"]
find = "lrclib"
when = "no-lyrics"

[providers.youtube]
enabled = false
```

| Last lookup | Made again |
|---|---|
| None | At once |
| Made by an older method | At once, unless it found a source the song lists |
| Found nothing, or an instrumental | After the provider's `recheck` |
| Failed | After an hour, doubling with each failure, up to a week |
| Found a source, or declined | Never |

`sync --rematch` makes every lookup at once, whatever its record says,
but a song with a source from a provider still asks that provider
nothing: one whose found source was taken out of it asks again. What
one round finds can trigger the next, as an upload's release then finds
the release's lyrics. An LRCLIB record fits when its length is within 2 s of the
song's and its names hold the song's, compared by letters and digits in
any width; a record with words wins over one marked instrumental, then
one named exactly the song's title over one holding it, as `Rain (Live)`
holds `Rain`, then a timed record, then the closest in length. A song without a title, an artist or a measured
length looks nothing up on LRCLIB, nor searches MusicBrainz by name.

A MusicBrainz recording fits when it is no video, its length is within
3 s of the song's and its names hold the song's; one titled exactly the
song's title wins over one holding it. Releases rank by the
song's own album if it has one, then a release that is no compilation,
live album or soundtrack, an official one, an album before an EP before
a single. Of the recordings that fit, the one on the best release wins,
then the one on the most releases, which tells a famous song's original
from its remixes and live takes; of its releases, the best ranked, then
the earliest. Its tags
are ranked with every other source's, as [tags](#how-the-best-of-each-is-picked)
are: a value it agrees on with another source wins over one alone. To
look every song up, write the built-in triggers out with `when =
"always"` for `acoustid` and `musicbrainz`.

A song whose audio has a fingerprint asks AcoustID before it searches
MusicBrainz by name, so a file with wrong tags, or none, still finds its
recording. muman sends the fingerprint of the audio's first two minutes
and its length; AcoustID answers with the MusicBrainz recordings
submitted with prints like it. A result counts when its score is at
least 0.5 and the recording's length is within 3 s of the song's; of
those, one named as the song is wins, then the one with the most prints
submitted, then one on a release that is no compilation, live album or
soundtrack. The recording is then fetched from MusicBrainz by its ID,
its release picked as a search picks one. The song's search by name
waits for AcoustID and runs in the same `sync` only when AcoustID finds
nothing. To send no fingerprints, set `enabled = false` under
`[providers.acoustid]`.

A cover found joins every song of its album, which asks for it once
between them, and is picked over the songs' own pictures only by the
measures any cover is. To take covers only where a song has none, write
the built-in triggers out with `when = "small-cover"` changed for
`coverart`.

muman asks MusicBrainz at most once a second, as MusicBrainz asks of
every client, AcoustID at most three times a second, as AcoustID asks,
and LRCLIB at most four times a second, whatever `concurrency` says.
When a service answers that requests come too fast (a 429 or a 503),
muman waits 2 s for MusicBrainz and AcoustID and 10 s for LRCLIB,
doubling, or longer where the service's `Retry-After` asks, in seconds
or by a date, and asks again up to three times; one asking for more
than two minutes is refusing at once. A
service still refusing after that leaves its remaining lookups for the
next run, recorded as nothing, so none waits out a failure's backoff.
Each request names muman and its repository in its user agent.

## Choosing songs and changing them

`list`, `remove`, `restore`, `set` and `edit` take a query, read from
what earlier runs resolved, without fetching or measuring. Its terms
are all required, case ignored:

| Term | Matches |
|---|---|
| `paper comets` | Each word in the title, artist, album or album artist, or a source's whole ID or file name |
| `artist:venn` | A field containing the text |
| `artist:="Marlo Venn"` | A field equal to the text |
| `title::^paper` | A field matching a regular expression |
| `^lyrics:yes` | A song the term does not match |
| `youtube.com:vid00000001` | The song listing that key |

A field is any tag, or `key`, `path`, `format` (the file's extension:
`opus`, `ogg`, `flac`, `mp3`, `m4a`),
`cover` and `lyrics` (`yes`, `none`). A field that is no tag muman knows
and that no song has is refused, so a misspelled one never matches
nothing, or, negated, every song.

- **`list`** writes each song's key, artist, title and album, separated
  by tabs; `-f` takes a template of `{field}`s, `--removed` lists the
  tombstones.
- **`set`** sets tags, pins and `lyrics_offset`: `genre=Folk` sets,
  `genre!` clears so the sources' value shows, `audio=<key>` pins,
  `audio!` unpins, `lyrics=false` takes none.
- **`edit`** opens the songs' entries in `$VISUAL` or `$EDITOR`. Delete
  a table to remove its song, move a key to another table to join the
  two, or into a new table without an `id` to split it off. A file with
  problems reopens with each as an `# ERROR:` line.
- **`remove`** moves songs to `[[removed]]` and deletes their library
  files; `--purge` also deletes fetched sources and trashes your own.
- **`restore`** lists removed songs again, fetching any purged source.
- **`undo`** puts the song list and library back as before the last run
  that changed them, one run further back each time: files it wrote over
  or removed come back, files it moved move back, and an edit you made by
  hand before that run is undone with it. A run stopped partway, by a
  crash or Ctrl-C, can be undone too, and so can an undo stopped partway.
  It refuses when the song list changed since, and after `remove
  --purge` until you put the files of your own it trashed back from the
  trash, where it names them. `[history]` sets how many runs are kept
  and how much space they take together; each run that records holds
  the history to it, letting go of the oldest runs' files first, and a
  file not kept is written again from its sources.

```bash
muman list | fzf -m -d '\t' --with-nth 2.. --accept-nth 1 | xargs muman remove -y
muman set artist:="Marlo Venn" album="Lantern Weather"
```

Every change says what it does first, each tag as `old → new`:

| Situation | On a terminal | Without one |
|---|---|---|
| The query matches nothing | Refused, exit 5 | Refused, exit 5 |
| It matches several songs | Picked from a list | Refused unless `--all`, exit 5 |
| The change itself | Asked, unless `-y` | Made only with `-y`, else exit 5 |

`edit` takes no `-y`: it needs a terminal for its editor, `$VISUAL`,
else `$EDITOR`, else `vi`, or Notepad on Windows. A query of keys alone
takes every song it names. `-n` says what would change and stops. A song changed meanwhile, by hand or by another run,
refuses the save.

## Status, info and check

- **`status`** runs the offline phase without changing the song list or
  the library, keeping what it measures for the next sync: what would be
  written, moved or deleted, and for each song where each aspect comes
  from and why, and, under `[library] max_size`, which songs are
  written below their best format to fit; see
  [configuration](configuration.md#library-size). A song a sync leaves
  as it is is only counted, unless `--all` is given. A query, as `list`
  reads it, shows each song it matches, up to date or not, and leaves
  out the files to remove or left unused, which are no song's. A
  last line counts the songs shown by what a sync does with each, such
  as `Songs: 2 new, 1 changed, 1 moved, 1240 up to date`.
- **`info`** counts what the library holds, whether it is in step, the
  lookups due, and what could be better: narrow, mono or clipped audio,
  missing or soft covers, untimed or missing lyrics, missing tags. It
  reads only what earlier runs recorded; `--verbose` names every song.
- **`check`** reports library files missing, empty, changed, left by an
  interrupted run or not muman's, and sources missing or unreadable;
  `--decode` decodes every source in full to find a truncated download.
  `--upstream` asks each site, downloading nothing, whether it still
  serves each fetched source in the format and at the size its song
  records, with the same tags, and names each it serves otherwise:
  other audio in that format, the format no longer offered, other tags,
  or the video taken down. The
  copy in `sources/` is then the only one of what was fetched, and worth
  keeping; a large library asks a site many times, as a fetch does.
- **`purge`** deletes fetched sources and lookup records no song uses,
  which `status` and `info` name: an upload a release took the place of,
  a source taken out of its song. A removed song's sources stay for
  `restore`, and files of your own are never touched.
- **`duplicates`** lists songs listed apart that are one recording, by
  their audio fingerprints: a file of an album there twice first, then a
  track and its copies on other albums, which a library of whole albums
  keeps on purpose. It changes nothing; to merge a group, move the
  others' sources into one `[[song]]` with `edit`, or `remove` them. A
  query names the groups shown.

## Export

`muman export -o FILE` writes the song list and the library, as the
last run left them, into one zip: `songs.toml` at its root and every
song with its lyrics under `library/`, at its path in the library. A
song not written yet is not in it, so `sync` first. `-o` may name a
folder, which gets `muman.zip`.

`--max-size` caps the zip, such as `--max-size 32GiB` for a card, as
`[library] max_size` caps the library.
Songs that do not fit are encoded again from their sources at lower
bitrates of `[audio] lossy`, the least audible loss for each byte saved
first: lossless songs before lossy ones, which would lose a generation,
and a few songs lowered far before many a little; once they fit, each
song lowered is raised back as far as the room left allows, as the
library is. Each song is measured once encoded and the rest fitted
again on what it really took, so the zip lands under the cap. When even the lowest bitrates cannot fit,
nothing is written and the size needed is said. The library itself is
never changed.

## Hooks

A `[[hook]]` runs a command as the library changes:

```toml
[[hook]]
on = "written"
run = ["rsgain", "custom", "-s", "i", "{path}"]

[[hook]]
on = "changed"
run = ["mpc", "update"]
```

| Event | Runs | Values |
|---|---|---|
| `written` | After each song file is in place, before muman records its size and time, so a hook may tag it | `path`, `rel` (in the library, with `/`), `library` |
| `changed` | Once, after a run that wrote or removed any file | `library`, `written`, `removed` (counts) |

Each value reaches the command as a `{name}` placeholder in its words
and as a `MUMAN_<NAME>` environment variable, such as `MUMAN_PATH`.
`run` is an argument list, run without a shell. A Windows batch file
(`.bat`, `.cmd`) gets the variables only, since cmd.exe could run a
title passed as an argument; a batch hook with placeholders is skipped
with a warning. A hook's output is shown prefixed `hook:`, and a hook
that fails is warned of but fails nothing. `status` runs no hooks.
