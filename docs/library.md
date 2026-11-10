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
| Audio | The song's recording; whole over damaged; fewest measures not taken; least sound beyond the song or missing from it; real stereo; peaks highest over its loudness, where that counts; widest real bandwidth; least clipping; lossless over lossy; most bits used; lowest sample rate; from its album's folder |
| Cover | Square content; effective resolution; fewest block artifacts |
| Lyrics | Role and timing, as `[lyrics] prefer` ranks them; in a language you read; surest to be the chosen audio's recording; spanning most of the song |
| Tags | A field kept for the tag over one read off a video title; fewest decorations; most sources agreeing |

Audio and covers are scored: each measure in whole steps, so noise
never decides between near-equals, times its weight; the lowest sum
wins. A source missing a measure ranks after those missing fewer. The
weights a song list starts with rank by each measure in turn, each
outweighing all after it, but dynamics, off until set.
`[audio.pick]` and `[cover.pick]` change a measure's weight and steps
or switch it off ([how sources are
ranked](configuration.md#how-sources-are-ranked)).

### Audio

- **The song's recording** comes first, ahead of every measure. Sources
  whose audio aligns, or that hold the same audio, are one recording; a
  source under 5 s is too short to compare, of no recording known.
  Where a song holds several, as a live take beside the studio cut, its
  recording is the one whose length fits its MusicBrainz record, else
  the one most of its whole sources hold, then most sources; sources of
  another rank after every source of it. A record counts only for a
  recording that holds none of the audio the lookup asked with and not
  the source it was made from, unless a file names its ID, so a music
  video's own lookup never makes the video's cut the song's. With no
  record and no recording holding the most sources, nothing changes.
  `status` names each source of another recording, and one playing more
  than 0.5% faster or slower than the rest of its own.
  `[audio.pick.recording] enabled = false` turns this off.
- **A whole copy** beats a [damaged](guide.md#damaged-sources) one, however little it lacks.
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
- **Bandwidth** is where a lowpass cuts the sound off. A FLAC or Opus
  made from an MP3 measures as narrow as the MP3, under any fill 20 dB
  down, while a recording whose treble fades on its own measures full.
- **Clipping** is the share of samples stuck at full scale or at the
  audio's own peak, or piled up just under full scale where a lossy
  encoder smeared them. In float lossless audio, which can hold samples
  past full scale, only those at its own peak count.
- **Dynamics** is how far the true peak stands over the loudness, which
  a master squeezed louder lowers. It counts only where
  `[audio.pick.dynamics]` is on, a matter of taste: it puts an older
  master on a lossy upload ahead of a louder one on a CD.

muman measures every channel as it is, never mixed into two. Of audio
scored alike, lossless beats lossy, then the more bits its samples use,
then the lower sample rate, which an upsample would only waste. Lossless
audio whose sound stops under 19 kHz, as lossy audio's does, was made
from lossy audio and counts as lossy. `status` names it, a file above
48 kHz whose sound stops at 24.5 kHz or lower, which was upsampled, and
one whose samples use fewer bits than it declares. Of sources alike in
all that, a song on an album takes the one from the folder most of its
album's audio comes from, a disc's folder counting as its album's, so
the album plays from one rip. A file
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
only when there is no other picture, and one more than twice as wide as
it is tall, or as tall as wide, is a banner and never a cover, unless
pinned. The songs of one album show one design, a pinned cover's, else
the one most of their covers show: a song showing another takes that
design from a source of its own where one holds it, before any picture
that measures better. A picture is known by its bytes, never its name
or the media type a file labels it with, so a PNG named `.jpg` is
written as PNG; one that does not decode is said, and is no cover.
[`[[cover.out]]`](configuration.md#where-the-cover-goes) says where the cover goes.

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
audio's, and lyrics timed past the audio's end are another cut's, taken
only where no lyrics fit; a line starting over 2 s past the end is left
out of any lyrics, and said. A song sung in no language, as MusicBrainz marks one
(`zxx`), or titled as an instrumental or a karaoke take, takes no lyrics.
Lyrics that state no language are the song's own, unless their letters
are Latin alone and the song is sung in another script: then they are
its romanization, ranked as `[lyrics] prefer` ranks one. A pin, as
`lyrics = "<source>"`, takes lyrics whatever these say;
`[lyrics.clean.structure] instrumental` and `fit` and
`[lyrics.clean.guesswork] script` turn each check off.

Lyrics are [cleaned](metadata.md#cleaning) of cues such as `[Music]`, credits and
what a site wrapped around them. A line in brackets of other words is a
backing vocal, and is sung.
[`[[lyrics.out]]`](configuration.md#where-lyrics-go) says where lyrics
go.

### Tags

Each tag comes from the source that offers it best, the songs of one
album agree on one edition and one spelling, and every value is
cleaned before it is ranked: [tags and cleaning](metadata.md) says how.

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
[new.toml](../crates/muman-core/src/new.toml) with every key at its default; set
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
score is at least 0.5 and the recording's length is within 10 s of the
song's, or it states none and two prints name it, unless its title and
first artist both disagree with the song's: a tribute act whose name
holds the artist's is not the artist. Of those, one named
as the song is wins, then one within 3 s, then further, then stating
none, then the one with the most prints submitted, then one on a
release that is no compilation, live album or soundtrack. It is
fetched from MusicBrainz by its ID, passing over one whose comment
names the other mix, as `original stereo album mix` for a mono file.
To send no fingerprints, set `enabled = false` under
`[providers.acoustid]`.

The search by name runs only when AcoustID finds nothing, and never
for a song that lists a MusicBrainz record. A recording fits when it is
no video, its length is within 3 s of the song's, and its names hold
the song's. One titled exactly as the song wins over one whose title
holds it, then one whose comment does not name the other mix than the
song's audio. Then releases rank:

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
over one alone. Artists credited together are written as MusicBrainz
joins them, `Marlo Venn & The Glass Orchards`, and listed one by one in
`ARTISTS`. To look every song up once, set `until =
["from:musicbrainz"]` for `acoustid` and `musicbrainz`.
`sync --rematch` fetches every record again and asks AcoustID again for
its own finds, replacing them: run it once after upgrading.

An AcoustID lookup also keeps every recording that counts, with the
releases, discs and tracks each is on, and a record keeps its artists'
aliases, with their locales, and the genres of its recording, release,
release group and artists, all asked in the requests above. No pick
reads them; a record kept before has them once it is fetched again.

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
  1 moved, 1240 up to date`, then the problems `check` would list.
- **`info`** counts what the library holds, whether it is in step, the
  lookups due, and what could be better: narrow, mono or clipped audio,
  missing or soft covers, untimed or missing lyrics, missing tags. It
  reads only what earlier runs recorded. `--verbose` names every song.
- **`check`** reports library files missing, empty, changed, left by a
  run that was cut short, or not muman's; sources missing, unreadable,
  cut off or missing audio; and songs no sync can build or the last
  could not write. `--decode` decodes every source in full again, to
  find damage in a file changed since it was measured.
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
folder, which gets `muman.zip`; a file there already is written over
only with `--force`, and nothing is written in the home or the library.

With `--folder`, muman writes what the zip would hold into the new or
empty folder `-o` names, for a card or a player to read as it is. It
refuses a folder holding anything, and keeps a link or a mount point.

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
would write under those settings, a song mixed into another layout
levelled on its mix, which the export measures where no sync has. Each
song the change leaves alone is copied, and every other is made again.

A song you changed since muman wrote it, whose sources are gone, or that a sync
leaves as built, as one cut off since, goes in as the library holds it; the
export says so and exits with code 4. An export cannot change where the library
lives, its `max_size`, how sources are ranked or which lyrics a song takes, nor
loudness when the song list's `[loudness] mode` measures none.

### A size limit

`--max-size` caps the export, as `--max-size 32GiB` for a card. In a
folder, each file counts in whole blocks of `[library] block_size`,
which `--set` may change. muman encodes the songs that do not fit again
from their sources, at lower bitrates of `[audio] lossy`, by the rule of
[the library's own limit](audio.md#library-size). The songs get the cap
less the song list and are lowered as a library of that `max_size`
would be, a zip counting its own few bytes for each file too. When even
the lowest bitrates cannot fit, muman writes nothing and says what size
the export needs. The library itself never changes.

## Hooks

A `[[hook]]` runs a command as you once a command has changed the
library, after you have [trusted it](configuration.md#commands-servers-and-secrets):

```toml
[[hook]]
on = "changed"
run = ["mpc", "update"]
```

It runs once, after a run that wrote, moved or removed any file (an
undo's files put back count as written), and after that run lets go of
the home and the library, so it may run muman there; a muman a hook
starts runs no hook. It is given `library` and how many files were
`written`, `moved` and `removed`, each as a `{name}` placeholder in its
words and as a `MUMAN_<NAME>` variable, such as `MUMAN_LIBRARY`; none
holds a title or a file's name. `run` is an argument list, run without a
shell; a batch file (`.bat`, `.cmd`) gets the variables only. Its output
shows as `hook:`, and a hook that fails is warned of but fails nothing.

A hook muman cannot run, as one `on = "written"`, is refused alone:
commands that would run it say so, `check` reports it, and the rest
still work. A tag a program should add goes in through `muman add` as a
[tag file](songs.md#giving-songs-lyrics-pictures-and-tags); ReplayGain
is [`[loudness]`](audio.md#loudness)'s. A muman a hook runs under
another home, on the library the hook's run wrote, stops with exit code
5: pass `--library` to name another.
