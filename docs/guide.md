# Guide

muman keeps a music library in step with a song list, `songs.toml`,
that you edit by hand or from the command line. Each song lists the
sources it may be made from. muman measures every source, picks the best
audio, cover, lyrics and tags among them, and writes the song once into
the library as a tagged Opus or FLAC file. Where the home and the
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
| `history/` | What the latest changing runs replaced or removed, for `undo` |
| `partial/` | Unfinished downloads, resumed by a later run |

Every `add` and `sync` runs in two phases. The network phase lists what
URLs name, fetches sources and makes the [lookups](#lookups-and-providers)
that are due; each new source is matched to the songs it may belong to
([new sources](#new-sources)). The offline phase, reconcile, holds a
lock on the home:

1. Measure every listed source whose files changed since last measured,
   saving progress as it goes.
1. Compare the audio of every pair of a song's sources.
1. Resolve each song to a plan: the best source for each aspect.
1. Move each song whose plan is unchanged but whose path changed, as a
   new path template makes it, instead of encoding it again.
1. Write each song whose plan changed or whose file is gone; `sync
   --force` writes every song.
1. Delete every file muman wrote that no song makes any more.

muman deletes only files `state.json` records as its own; anything else
in the library folder is left alone. A file whose size or time changed
since muman wrote it, as a tagger rewrites it, is left with a warning
until `sync --force`. Each file is written under a temporary name and
renamed into place, so a player never sees half a track. Library paths
in messages use `/` on every system.

## The song list

```toml
version = 1

[defaults]
lyrics = ["en", "fr"]

[[song]]
sources = ["youtube:vid00000001", "manual:Marlo Venn/Paper Comets.flac"]
cover = "manual:Marlo Venn/Paper Comets.flac"
lyrics_offset_ms = 120

[song.tags]
artist = "Marlo Venn"
album = "Lantern Weather"
genre = ""
```

| Key | Means |
|---|---|
| `version` | The file's format; a newer one is refused |
| `defaults.lyrics` | Subtitle languages lyrics are taken in, most preferred first |
| `sources` | Every source the song may be made from; a key belongs to one song only |
| `album`, `track` | The `[[album]]` the song is on and its place there |
| `audio`, `cover`, `lyrics` | A pin: that aspect from that source, whatever the measures say; `lyrics = false` for none |
| `lyrics_offset_ms` | Moves the lyrics later, on top of the offset measured |
| `[song.tags]` | Tags set over what the sources offer; an empty value sets nothing |
| `[[album]]` | A YouTube Music album added whole, with `[album.tags]` over what its tracks offer |
| `[[removed]]` | A song `remove` took out, kept whole under a `note` naming it |

A song has no ID of its own: it is its sources, and any of its keys
names it. A key is `youtube:<id>` for a video, `lrclib:<id>` for LRCLIB
lyrics, or `manual:<path>` for a file under `sources/manual`.
`[song.tags]` takes `title`, `artist`, `album`, `album_artist`, `genre`,
`date` (or `year`), `track`, `disc`, or any Vorbis comment name; a list
sets a tag several times. `add --artist`, `--album` and the other tag
flags set these on every song an `add` lists. `muman status` shows each
tag's value and the source it came from.

The file also holds the settings tables of
[configuration](configuration.md), `[clean.*]` ([cleaning](#cleaning)),
and the `[providers.*]`, `[[trigger]]` and `[[hook]]` tables below.
muman edits it in place: comments, order and keys it does not know
survive every write, and two runs at once keep each other's songs. A
key that does not parse, a key two songs list, or a pin to a source the
song does not list stops the run before anything is written.

A `[[removed]]` key no song lists is a tombstone: no playlist or dropped
file lists it again. `muman restore`, adding its video alone by URL, or
deleting the table by hand lets it back.

## Sources

A source is a video yt-dlp fetched, a file of your own, or lyrics from
[LRCLIB](https://lrclib.net), a free lyrics database that songs without
timed lyrics [look up](#lookups-and-providers).

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

A listed source whose file is gone is fetched again by the next `sync`.
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
listed as a source of its own.

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

## How the best of each is picked

No rule names a kind of source: a release, an upload and a file of your
own are ranked by what they measure. A pin is the only override, and the
order of `sources` breaks the last tie. Measures are compared in steps,
so noise never decides between near-equals, and a measure that could
not be taken ranks last.

| Aspect | Criteria, in order |
|---|---|
| Audio | Least sound beyond the song; widest real bandwidth; real stereo; least clipping |
| Cover | Square content; effective resolution; fewest block artifacts |
| Lyrics | Timed; in a preferred language; surest to be the chosen audio's recording; spanning most of the song |
| Tags | A dedicated field over one read off a video title; fewest decorations; most sources agreeing |

**Audio.** Sound beyond the song is how long a source plays outside the
stretch it shares with another source of the song: a music video's
intro counts, silence does not. Bandwidth is the highest frequency
really present, so a FLAC transcoded from a lossy file measures as
narrow as that file. Real stereo tells stereo from mono copied into two
channels; clipping is the share of samples stuck at full scale. Opus and
FLAC are copied; another lossless codec is encoded to FLAC, another
lossy one to Opus at the bitrate `[audio]` sets.

**Covers.** A picture that is not square is checked for bars, and a
video frame with square art in the middle is cropped to that art.
Effective resolution is the size up to which a picture keeps its
detail, so an upscale measures the size it was made from.

**Lyrics.** Only subtitles a person wrote count; generated captions
never do. Lyrics from a source other than the chosen audio must be the
same recording, and are shifted by the offset measured between the two.
Cues such as `[Music]`, symbol-only lines and credits are dropped. A
`.lrc` with no audio of its own is taken as timed for the song; one that
states its length, as LRCLIB's do, is trusted as far as that length
agrees with the chosen audio's. Whether lyrics go beside the song,
inside it or both is a `[library]` setting.

**Tags.** The album, album artist, track, disc and date come together
from the one source whose album ranks best, so an album never splits
across folders; a song on no album is a single named for its title.

### Cleaning

Every tag a source offers is cleaned before ranking, so `Paper Comets
(Album Version)` agrees with `Paper Comets`. Tags you set in
`[song.tags]` or `[album.tags]` are never cleaned. Each rule is a switch
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
upload's. The two recordings are then aligned at one offset; they are
one recording when they correlate at 0.8 or more and at least 80% of
their windows agree. The same comparison measures sound beyond the song.

When the two are one recording, both become the song's sources and each
aspect is picked by its measures. Otherwise only the release is listed
and the upload is recorded as replaced, so it is never fetched again.
`add --no-match` keeps each video as uploaded.

YouTube Music names album playlists `OLAK5uy_…`; any other playlist is
a list of separate songs. An album added whole becomes an `[[album]]`
keyed `youtubetab:<id>`, and each track a song with `album` and `track`
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

A trigger makes a song with a source from any provider in `from`, and
none from `find`, look `find` up when `when` holds: `always`,
`no-lyrics` or `no-timed-lyrics`. The built-in triggers:

| From | Finds | When |
|---|---|---|
| `youtube` | `youtube-music` | `always` |
| `youtube-music` | `youtube` | `no-timed-lyrics` |
| `manual`, `youtube`, `youtube-music` | `lrclib` | `no-timed-lyrics` |

Any `[[trigger]]` in the song list replaces all of them. A
`[providers.<name>]` table sets `enabled`, `concurrency` (lookups at
once), `recheck_days` (how long a lookup that found nothing waits),
`per_run` (the most one run makes, `0` for no limit), and for `lrclib`,
`url`, another LRCLIB server:

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
| None, or made by an older method | At once |
| Found nothing | After the provider's `recheck_days` |
| Failed | After an hour, doubling with each failure, up to a week |
| Found a source, an instrumental, or declined | Never |

`sync --rematch` makes every lookup at once. What one round finds can
trigger the next, as an upload's release then finds the release's
lyrics. An LRCLIB record fits when its length is within 2 s of the
song's and its names hold the song's; a timed record wins, then the
closest in length. A song without a title, an artist or a measured
length looks nothing up on LRCLIB.

## Choosing songs and changing them

`list`, `remove`, `restore`, `set` and `edit` take a query, read from
what earlier runs resolved, without fetching or measuring. Its terms
are all required, case ignored:

| Term | Matches |
|---|---|
| `paper comets` | Each word in the title, artist, album, album artist or a key |
| `artist:venn` | A field containing the text |
| `artist:="Marlo Venn"` | A field equal to the text |
| `title::^paper` | A field matching a regular expression |
| `^lyrics:yes` | A song the term does not match |
| `youtube:vid00000001` | The song listing that key |

A field is any tag, or `key`, `path`, `format` (`opus`, `flac`),
`cover` and `lyrics` (`yes`, `none`).

- **`list`** writes each song's key, artist, title and album, separated
  by tabs; `-f` takes a template of `{field}`s, `--removed` lists the
  tombstones.
- **`set`** sets tags, pins and `lyrics_offset_ms`: `genre=Folk` sets,
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
  that changed them, one run further back each time. It refuses when the
  song list changed since. `[history]` sets how many runs and how much
  space are kept; a file past that is written again from its sources.

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

A query of keys alone takes every song it names. `-n` says what would
change and stops. A song changed meanwhile, by hand or by another run,
refuses the save.

## Status, info and check

- **`status`** runs the offline phase without writing: what would be
  written, moved or deleted, and for each song where each aspect comes
  from and why.
- **`info`** counts what the library holds, whether it is in step, the
  lookups due, and what could be better: narrow, mono or clipped audio,
  missing or soft covers, untimed or missing lyrics, missing tags. It
  reads only what earlier runs recorded; `--verbose` names every song.
- **`check`** reports library files missing, empty, changed, left by an
  interrupted run or not muman's, and sources missing or unreadable;
  `--decode` decodes every source in full to find a truncated download.

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
