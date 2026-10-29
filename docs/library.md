# Making the library

muman measures every source a song lists, takes the best audio, cover,
lyrics and tags among them, looks up what the song still lacks, and
writes the song once. This page says how each pick is made, what
muman looks up, what `status`, `info` and `check` report, and how
export and hooks work. The song list and its sources are in [the
guide](guide.md), changing songs in [songs](songs.md), and the settings
in [configuration](configuration.md).

## How the best source wins

No rule names a kind of source. A release, an upload and a file of
your own are ranked by what they measure alone. A pin in the song list
is the only override, and the order of `sources` breaks the last tie.

| Part | Ranked by, in order |
|---|---|
| Audio | Least sound beyond the song or missing from it; real stereo; widest real bandwidth; least clipping |
| Cover | Square content; effective resolution; fewest block artifacts |
| Lyrics | Role and timing, as `[lyrics] prefer` ranks them; in a language you read; surest to be the chosen audio's recording; spanning most of the song |
| Tags | A field kept for the tag over one read off a video title; fewest decorations; most sources agreeing |

Audio and covers are scored. muman counts each measure in whole steps,
so noise never decides between near-equals, and multiplies each step
by its measure's weight. The lowest sum wins. A source missing a
measure ranks after those missing fewer. The weights a song list starts
with rank by each measure in turn, each outweighing all after it.
`[audio.pick]` and `[cover.pick]` change a measure's weight and steps
or switch it off ([how sources are
ranked](configuration.md#how-sources-are-ranked)).

### Audio

- **Sound beyond the song** is how long a source plays outside the
  stretch it shares with another source of the song. A music video's
  intro counts; silence does not. An excerpt lacks the rest of the
  song, and that counts the same, so an excerpt never beats the whole
  song it was cut from. A video's outro past the song's own end is
  sound beyond the song. A source's start counts as cut only when its
  end is cut too, since a release may open on a full chord.
- **Real stereo** tells stereo from mono copied into two channels, even
  at different levels or a few samples apart. Of more than two
  channels, it compares the first two.
- **Bandwidth** is where a lowpass cuts the sound off. A FLAC
  transcoded from a lossy file measures as narrow as that file, while a
  recording whose treble fades on its own measures full.
- **Clipping** is the share of samples stuck at full scale or at the
  audio's own peak, or piled up just under full scale where a lossy
  encoder smeared them. In float lossless audio, which can hold samples
  past full scale, only those at its own peak count.

muman measures every channel as it is, never mixed into two. Of audio
scored alike, lossless beats lossy, then the more bits its samples use,
then the lower sample rate, which an upsample would only waste. A file
with several audio streams is made from its best: lossless before
lossy, then the most channels, then the highest rate.

muman copies the audio chosen when `[audio] codecs` lists its codec.
Otherwise it encodes lossless audio to `[audio] lossless` and lossy
audio to `[audio] lossy`. It always encodes a source cut off, so the
song ends cleanly. [Audio](audio.md#codecs) says what each codec holds.

### Covers

muman checks a picture that is not square for bars, and crops a video
frame with square art in the middle to that art. Effective resolution
is the size up to which a picture keeps its detail, so an upscale
measures no larger than the picture it was made from. Block artifacts
are how much more the pixels jump across JPEG's 8×8 block edges than
between them.

A picture whose file says it is no front cover, by its picture type (a
back, a disc, a booklet) or its name (`back.jpg`, `cd.png`), is taken
only when there is no other picture. A picture is known by its bytes,
never its name, so a PNG named `.jpg` is written as PNG.
[`[[cover.out]]`](configuration.md#where-the-cover-goes) says where
the cover goes.

### Lyrics

Lyrics rank first by [role, timing and
language](configuration.md#which-lyrics-a-song-takes). Only subtitles a
person wrote count; generated captions never do. Lyrics from a source
other than the chosen audio must be of the same recording. muman shifts
them by the offset it measured between the two, and stretches them when
one plays a little faster. A word's time moves with its line's, and an
`[offset:]` the file states is folded into every time.

A song file's own lyrics tag is lyrics of that source, timed when its
text is LRC: `USLT` in MP3, `LYRICS` or `UNSYNCEDLYRICS` in a Vorbis
comment, `©lyr` in MP4, `Lyrics` in APEv2. A `.lrc` with no audio of
its own is taken as timed for the song. One that states its length, as
LRCLIB's do, is trusted as far as that length agrees with the chosen
audio's.

Lyrics are [cleaned](#cleaning) of cues such as `[Music]`, credits and
what a site wrapped around them. A line in brackets of other words is a
backing vocal, and is sung.
[`[[lyrics.out]]`](configuration.md#where-lyrics-go) says where lyrics
go.

### Tags

Each field describes either the recording or the release.

- **The recording's fields**, the title, artist, genre, ISRC and the
  recording's and artists' MusicBrainz IDs, each come from whichever
  source offers it best. The ISRC and the IDs come only from a source
  that titles the recording as the song is titled.
- **The release's fields**, the album, album artist, track, disc, date,
  totals, country and release IDs, all come from the one source whose
  album ranks best. So an album never splits across folders, and one
  release's IDs never mix with another's. Of albums that rank alike,
  the source with more of the release's fields wins.

A song on no album is a single, named for its title. ReplayGain's gains
and peaks are measured from the audio itself ([loudness](audio.md#loudness)).
With `[loudness] mode = "off"`, they come only from the tags of the
source the audio comes from, and the album's only when the release's
fields come from that source too.

muman reads a file's tags by their Vorbis names, or by the names Picard
gives them in MP3 and MP4; a track written `3/12` offers its total too.
In MP3, MP4 and WavPack it writes each field where Picard writes it, so
muman and Picard agree on a library. A value its field cannot hold, as
a date that is none, or a value over 4 KiB, counts as offered by no
source. Control characters, and characters that reorder text, are
dropped from every value.

### Cleaning

muman cleans every tag a source offers before ranking it, so `Paper
Comets (Album Version)` agrees with `Paper Comets`. Lyrics are cleaned as a
song is written. Tags you set in a song's or an album's `tags.<name>`
are never cleaned.

Each rule is a switch under `[tags.clean.<tier>]`, or
`[lyrics.clean.<tier>]` for lyrics; the song list lists every one.
`muman status` names each rule that changed a tag, as `<tier>.<key>`.

| Tier | Its tag rules | Its lyric rules |
|---|---|---|
| `tidy` | Fix spacing and punctuation; never remove a word | Drop markup, `\h`, invisible characters, stray spaces and blank lines; give a look-alike letter, as a Cyrillic `е` in `hеad`, its word's script |
| `packaging` | Remove words naming the release or video: `(Official Video)`, `[Remastered]`, `VEVO` | Drop cues, credits, the title and artist restated around the lyrics, and ID tags in untimed lyrics |
| `structure` | Split or swap a whole value by its shape: artists joined by `;`, a disc number closing an album | Put lines in time order; keep once a subtitle written again for each frame |
| `guesswork` | Read a video title by convention, as `Marlo Venn - Paper Comets (feat. Ada Quill)`; can misread | Keep, as one line, a subtitle line typed out over three lines or more |

Flipping a lyric switch writes every song's lyrics again. A new title
or artist writes again the lyrics of a song cleaned by its old one.
Either way only the lyrics change: a file beside a song is written
again, and the song's own file is retagged, never encoded.

## YouTube Music releases and albums

An upload often has a release on YouTube Music: a track whose metadata
names the song, its artists and album, with the square art the upload
lacks. Before fetching, `add` searches YouTube Music's songs for each
new video. A candidate must carry its track name in the upload's title,
an artist in its channel or title, and a length close to the upload's.
muman then aligns the two recordings by their loudness over time,
allowing one to play up to 1% faster, to decide whether they are one.

When they are one recording, both become the song's sources and each
part is picked by its measures. Otherwise only the release is listed,
and the upload is recorded as replaced, so it is never fetched again.
`add --no-match` keeps each video as uploaded.

YouTube Music names album playlists `OLAK5uy_…`; any other playlist is
a list of separate songs. An album added whole becomes an `[[album]]`
keyed `youtube.com:playlist/<id>`, and each track a song with its
`album` and `track` set by its place in the full listing. A track
already listed only gains its album and place.

## Lookups

A song looks up the sources it lacks. Each source comes from a
provider:

| Provider | Finds | By |
|---|---|---|
| `youtube-music` | A video's release on YouTube Music | Searching YouTube Music's songs, as `add` does |
| `youtube` | An upload of a release, with a person's subtitles | Searching YouTube for the artist and title, the artist's channel first, and comparing the audio |
| `lrclib` | Lyrics from an LRCLIB server | Title, first artist, album and length |
| `musicbrainz` | Tags from a MusicBrainz server: the recording and its release's names, numbers, dates, IDs, type, status, medium, label, catalogue number and barcode | Title, first artist and length, preferring the song's album |
| `acoustid` | The same MusicBrainz records, through an AcoustID server | The fingerprint and length of the song's audio |
| `coverart` | The front cover of the song's album, from the Cover Art Archive | The album's MusicBrainz release group or release, else, while `musicbrainz` is on, its title and album artist searched there |
| `manual` | Files of your own | Never looked up |

The YouTube providers are asked only from a YouTube video, so a file of
your own never searches YouTube. `acoustid` is off while `musicbrainz`
is, since it fetches each recording from MusicBrainz. A song's
`coverart` lookup waits for its `acoustid` and `musicbrainz` lookups
due that run, so it asks for the release they found.

### When a song asks

Each provider has a `[providers.<name>]` table, written in
[new.toml](../src/manifest/new.toml) with every key at its default; set
any key and the rest keep theirs. A song asks a provider until it has
everything the provider's `until` lists:

| Condition | Holds when the song has |
|---|---|
| `lyrics:yes`, `lyrics:timed` | Lyrics, or lyrics whose lines carry times |
| `cover:yes`, `cover:sharp` | A cover, or one with detail enough to look sharp, as `info` counts it |
| `has:<tag>` | The tag from a source or the song list, not one muman fills in itself |
| `from:<provider>` | A source from the provider, so it asks until the provider finds something |

A provider takes only the conditions it can answer, and muman refuses
a song list that gives it any other, naming it. `lrclib` and `youtube`
take the lyrics ones, `coverart` the cover ones, `acoustid` and
`musicbrainz` any tag a MusicBrainz record holds, and each its own
`from:`. No song seeks
lyrics while `[lyrics] out = []`, or when its `lyrics` is pinned or
`false`; none seeks a cover while `[cover] out = []`, or when its
`cover` is pinned.

These leave songs out:

- `skip`, in a provider's table, lists queries as [`list`](songs.md#queries)
  reads them; a song any of them matches asks that provider nothing.
  `[providers] skip` leaves songs out of every provider.
- `lookups = false` on a song asks no provider; `lookups.<provider> =
  false` asks that one nothing.
- `enabled = false` in a provider's table asks its service nothing at
  all, not even for a record it found before.
- `--offline` makes no lookup in that run.

The other keys pace a provider: `concurrency` is how many lookups run
at once, `recheck` how long one waits before it is made again, and
`per_run` the most one run makes, `0` for no limit. `url` points
`lrclib`, `musicbrainz`, `acoustid` or `coverart` at another server,
such as a mirror, and `key` sets the application key AcoustID is asked
with, muman's own unless you set one.

```toml
# Covers only for songs that have none, and none for a hand-tagged album.
[providers.coverart]
until = ["cover:yes"]

[providers]
skip = ['album:="The Glass Orchards"']

# Fingerprints only of what was fetched, never of a file of your own.
[providers.acoustid]
skip = ["from:manual"]
```

### When a lookup is made again

| Last lookup | Made again |
|---|---|
| None | At once |
| Made by an older method | At once, unless it found a source the song lists |
| Found nothing, or an instrumental | After the provider's `recheck` |
| Failed | After an hour, doubling with each failure, up to a week |
| Found a source the song lists that falls short | After `recheck`, doubling each time it comes back unchanged, up to 90 days |
| Found a source, or declined | Never |

A source falls short when it lacks a condition of `until` that the
song lacks too, as an untimed LRCLIB record does while the song has no
timed lyrics from elsewhere. A record that gives what it can is never
asked about again, even when another source wins. LRCLIB is searched
again, since a better record may have joined it. A MusicBrainz record
or a Cover Art Archive cover is fetched again by its ID. A YouTube
video is never looked for again. A record that comes back changed
replaces the one kept, and its songs are built from it anew.

What one round of lookups finds can make another due, as an upload's
release then finds the release's lyrics. A sync makes up to three
rounds, each lookup once. `sync --rematch` makes every lookup a song
still wants, whatever its record says. `sync --until-done` makes every
lookup due in one run: past `per_run`, over as many rounds as finds
start, and asking again at once what failed, each service at its own
pace. `muman status <query>` says for each song why each lookup is
made, waits or is skipped, and `info` counts them.

### LRCLIB

An LRCLIB record fits when its length is within 2 s of the song's and
its names hold the song's, compared by letters and digits in any
width. A record with words wins over one marked instrumental, then one
named exactly as the song over one whose name holds it, as `Rain
(Live)` holds `Rain`, then a timed record, then the closest in length.
A song without a title, an artist or a measured length looks nothing
up on LRCLIB.

### MusicBrainz and AcoustID

A song whose audio has a fingerprint asks AcoustID before it searches
MusicBrainz by name, so a file with wrong tags, or none, still finds
its recording. muman sends AcoustID the fingerprint of the audio's
first two minutes and its length. AcoustID answers with the MusicBrainz
recordings submitted with prints like it. A result counts when its
score is at least 0.5 and the recording's length is within 3 s of the
song's. Of those, one named as the song is wins, then the one with the
most prints submitted, then one on a release that is no compilation,
live album or soundtrack. The recording is then fetched from
MusicBrainz by its ID. To send no fingerprints, set `enabled = false`
under `[providers.acoustid]`.

The search by name runs only when AcoustID finds nothing, and never
for a song that lists a MusicBrainz record. A recording fits when it is
no video, its length is within 3 s of the song's, and its names hold
the song's. One titled exactly as the song wins over one whose title
holds it. Then releases rank:

1. The song's own album, if it has one.
1. A release that is no compilation, live album or soundtrack.
1. An official release.
1. An album, then an EP, then a single.

Of the recordings that fit, the one on the best release wins, then the
one on the most releases, which tells a song's original from its
remixes and live takes. Of its releases, the best ranked wins, then
the earliest. A song without a title, an artist or a measured length
is not searched for by name. The record's tags are ranked with
every other source's: a value it agrees on with another source wins
over one alone. To look every song up once, set `until =
["from:musicbrainz"]` for `acoustid` and `musicbrainz`.

### Cover Art Archive

A cover found joins every song of its album, which asks for it once
between them. It wins over the songs' own pictures only by the measures
any cover is ranked by.

### How fast muman asks

muman asks MusicBrainz at most once a second, as MusicBrainz asks of
every client, AcoustID at most three times a second, as AcoustID asks,
and LRCLIB and the Cover Art Archive at most four times a second,
whatever `concurrency` says. Each request names muman and its
repository in its user agent.

When a service answers that requests come too fast (a 429 or a 503),
muman waits and asks again, up to three times. It waits 2 s for
MusicBrainz and AcoustID and 10 s for LRCLIB and the Cover Art Archive,
doubling, or as long as the service's `Retry-After` asks. A service
that asks for more than two minutes counts as refusing.

A service still refusing, or one muman cannot reach, leaves its
remaining lookups for the next run, recorded as nothing, so none waits
out a failure's backoff. Unreachable means its name does not resolve,
its connection is refused or not open within 10 s, or a proxy refuses
to connect to it; one lookup finds that out for all the rest. A service
that answers with an error, or with what muman cannot read, fails that
lookup alone.

## Status, info and check

- **`status`** runs the offline phase without changing the song list or
  the library, and keeps what it measures for the next sync. It says
  what would be written, moved or deleted, where each part of each song
  comes from and why, and which songs `[library] max_size` writes below
  their best ([library size](audio.md#library-size)). A song a sync
  leaves as it is is only counted, unless `--all` is given. A query
  shows each song it matches, up to date or not. The last line counts
  the songs by what a sync does with each, as `Songs: 2 new, 1 changed,
  1 moved, 1240 up to date`.
- **`info`** counts what the library holds, whether it is in step, the
  lookups due, and what could be better: narrow, mono or clipped audio,
  missing or soft covers, untimed or missing lyrics, missing tags. It
  reads only what earlier runs recorded. `--verbose` names every song.
- **`check`** reports library files missing, empty, changed, left by a
  run that was cut short, or not muman's; and sources missing,
  unreadable, cut off or missing audio. `--decode` decodes every source
  in full again, to find damage in a file changed since it was
  measured.
- **`check --upstream`** asks each site, downloading nothing, whether
  it still serves each fetched source in the format and at the size its
  song records, with the same tags. It names each source served
  otherwise: other audio in that format, the format no longer offered,
  other tags, or the video taken down. The copy in `sources/` is then
  the only one, and worth keeping. A large library asks a site many
  times, as a fetch does.
- **`purge`** deletes the fetched sources and lookup records no song
  uses, which `status` and `info` name: an upload a release replaced, a
  source taken out of its song. A removed song's sources stay for
  `restore`, and files of your own are never touched.
- **`duplicates`** lists songs listed apart that are one recording
  ([duplicates](songs.md#duplicates)).

## Export

`muman export -o FILE` writes the song list and the library, as the
last run left them, into one zip: `songs.toml` at its root and every
song, with its lyrics, under `library/` at its path in the library. A
song not written yet is not in it, so `sync` first. `-o` may name a
folder, which gets `muman.zip`.

With `--folder`, muman writes what the zip would hold into the new or
empty folder `-o` names, for a card or a player to read as it is. It
refuses a folder holding anything, so nothing in it is written over.

The zip's `songs.toml` keeps the song list's commands, which whoever
runs muman on it is [asked to trust](configuration.md#commands-servers-and-secrets).
`--redact` replaces each secret in it with `<redacted>`, as [the
reference](cli.md#muman-export) lists. A secret among a hook's or a
program's own words stays, so read those before you share.

### Other settings

An export may change, for itself alone, any setting of `[library]`,
`[audio]` and `[loudness]`, `lyrics.out`, `cover.out`, and each
cleaning rule, as `tags.clean.tidy.spacing=false`. `--set
table.key=value` sets any of them, and `--codec`, `--template` and
`--lyrics` the commonest. The export then holds the library a sync
would write under those settings. Each song the change leaves alone is
copied, and every other is made again from its sources.

A song you changed since muman wrote it, or whose sources are gone, goes
in as the library holds it; the export says so and exits with code 4.
An export cannot change where the library lives, its `max_size`, how
sources are ranked or which lyrics a song takes, nor loudness when the
song list's `[loudness] mode` measures none.

### A size limit

`--max-size` caps the export, as `--max-size 32GiB` for a card. In a
folder, each file counts in whole blocks of `[library] block_size`,
which `--set` may change. muman encodes the songs that do not fit again
from their sources, at lower bitrates of `[audio] lossy`, the least
audible loss for each byte saved first, as [the library's own
limit](audio.md#library-size) does. It measures each song once encoded
and fits the rest again on what it really took, so the export lands
under the cap. When even the lowest bitrates cannot fit, muman writes
nothing and says what size the export needs. The library itself never
changes.

## Hooks

A `[[hook]]` runs a command as you when the library changes, once you
have [trusted it](configuration.md#commands-servers-and-secrets):

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
| `written` | After each song file is in place, before muman records its size and time, so a hook may tag it | `path`, `rel` (in the library, with `/`, led by `./`), `library` |
| `changed` | Once, after a run that wrote, moved or removed any file; an undo's files put back count as written | `library`, `written`, `moved`, `removed` (counts) |

Each value reaches the command as a `{name}` placeholder in its words
and as a `MUMAN_<NAME>` environment variable, such as `MUMAN_PATH`.
`run` is an argument list, run without a shell. A script handed to a
shell (`sh -c`, `pwsh -Command`, `cmd /c`) must read `"$MUMAN_PATH"`:
a placeholder inside it, where a title could run as code, is refused. A
batch file (`.bat`, `.cmd`) gets the variables only. A hook's output
shows as `hook:`, and a hook that fails is warned of but fails nothing.
`status` runs no hook.
