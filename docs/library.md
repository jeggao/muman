# Making the library

muman measures every source a song lists, picks the best audio, cover,
lyrics and tags among them, looks up what the song still lacks, and
writes the song once. This page says how each pick is made, how songs
are chosen and changed from the command line, what `status`, `info` and
`check` report, how a library is exported and what hooks run. The song
list and its sources are in [the guide](guide.md); the settings in
[configuration](configuration.md); every command and flag in [the
command reference](cli.md).

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
| Audio | Least sound beyond the song or missing from it; real stereo; widest real bandwidth; least clipping |
| Cover | Square content; effective resolution; fewest block artifacts |
| Lyrics | Timed; in a preferred language; surest to be the chosen audio's recording; spanning most of the song |
| Tags | A dedicated field over one read off a video title; fewest decorations; most sources agreeing |

**Audio.** Sound beyond the song is how long a source plays outside the
stretch it shares with another source of the song: a music video's intro
counts, silence does not. A source that ends where another plays the
song on, as an excerpt cut off mid-sound, in a short rest or faded out
does, lacks the rest of the song, which counts as sound beyond the song
does, so an excerpt never beats the whole song it was cut from. A song's
own end is in every source of it, so a video's outro past it is sound
beyond the song; a release may start on a full chord at its first
sample, so a source's start counts as cut only when its end is too.
Bandwidth is where a lowpass cuts the sound off, so a FLAC transcoded
from a lossy file measures as narrow as that file, while a recording
whose treble fades on its own measures full, as does one whose notes end
low over a noise floor that runs on to the top. Real stereo tells stereo
from mono copied into two channels, even at different levels or a few
samples apart, by the front two of more; clipping is the share of
samples stuck at full scale or at the audio's own peak, or piled up just
under full scale where a lossy encoder smeared them. Float lossless
audio can hold samples past full scale, so in it only those at its own
peak count. Every channel is measured as it is, never mixed into two.
The chosen audio is copied when `[audio] codecs` lists its codec; any
other is encoded, lossless audio to `[audio] lossless` and lossy audio
to `[audio] lossy`, at the bitrate `[audio]` sets. A file with several
audio streams is made from the best: lossless before lossy, then the
most channels, then the highest rate. Lossless audio a codec cannot keep
whole, as 32-bit, float or more than eight channels in FLAC, is written
in one that can, as [Codecs](configuration.md#codecs) says.

**Covers.** A picture that is not square is checked for bars, and a
video frame with square art in the middle is cropped to that art.
Effective resolution is the size up to which a picture keeps its
detail, so an upscale measures the size it was made from. A picture
whose file says it shows no front cover, by its picture type (a back, a
disc, a booklet) or by its name (`back.jpg`, `cd.png`), is taken only
when no other picture is. A picture is known by its bytes, never by its
name: a PNG named `.jpg` is written as PNG, and a WebP converted.

**Lyrics.** Only subtitles a person wrote count; generated captions
never do. Lyrics from a source other than the chosen audio must be the
same recording, and are shifted by the offset measured between the two,
and stretched when one plays a little faster than the other; a word's
time moves with its line's, and an `[offset:]` the file states is folded
into every time it holds. A song file's own lyrics tag, `USLT` in an
MP3, `LYRICS` or `UNSYNCEDLYRICS` in a Vorbis comment, `©lyr` in an M4A
or `Lyrics` in APEv2, is lyrics of that source, timed when its text is
LRC; one a `USLT` frame marks in a language `defaults.lyrics` does not
list ranks after those in a language it does. Cues such as `[Music]` or
`(Guitar solo)`, symbol-only lines and the credits before the first line
sung or after the last are dropped; a line in brackets of other words is
sung, as a backing vocal is. A `.lrc` with no audio of its own is taken
as timed for the song; one that states its length, as LRCLIB's do, is
trusted as far as that length agrees with the chosen audio's. Whether
lyrics go beside the song, inside it or both is a `[library]` setting.

**Tags.** Each field describes the recording or the release. The
recording's, title, artist, genre, ISRC and the recording's and artists'
MusicBrainz IDs, each come from whichever source offers it best, the
ISRC and IDs only from a source that titles the recording as the song
is titled. The release's, album, album artist, track, disc, date, totals, country and
the release's MusicBrainz IDs, come together from the one source whose
album ranks best, so an album never splits across folders and one
release's IDs never mix with another's; of albums that rank alike, the
source holding more of the release's fields wins before the order of
`sources`. A song on no album is a single named for its title.
ReplayGain's track gain and peak are measured from the audio the song
is, and its album gain and peak over every song of its album, as [Loudness](configuration.md#loudness) says; with
`[loudness] mode = "off"`, they come only from the tags of the source
whose audio the song is, the album's only when the release's fields come
from it too, since another source's loudness is not this audio's. An
Opus file holds the gains as `R128_TRACK_GAIN` and `R128_ALBUM_GAIN` and
no peaks, as Opus players read them, and an Opus source's R128 gains are
read as ReplayGain's. A file's own tags offer every field by its Vorbis
name or the name Picard gives it in MP3 and MP4, and a track written
`3/12` offers its total too. In MP3, MP4 and WavPack every field is
written into the frame, atom or item Picard writes it in, and read back
from it, so a library tagged by muman and by Picard agrees. A value its field cannot
hold, as a date that is none, and a value over 4 KiB are offered by no
source, and control characters and those that reorder
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
less a version or translation in brackets or after a dash that names no
other recording, as `(Live)` does; an artist in its channel or title;
and a length close to its own. The two are aligned at one offset, or
along a line when one plays up to 1% faster, and are one recording when
they correlate at 0.8 or more and at least 80% of their windows agree.

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
| `musicbrainz` | Tags from a MusicBrainz server: the recording's and its release's names, numbers, dates, ISRCs and IDs, the release's type, status and medium, and its label, catalogue number and barcode from one more request a release | Title, first artist and length, preferring the song's album |
| `acoustid` | The same MusicBrainz records, found on an AcoustID server | The fingerprint and length of the song's audio, preferring a recording named as the song is |
| `coverart` | The front cover of the song's album, from the Cover Art Archive | The album's MusicBrainz release group or release ID, else its title and album artist searched on MusicBrainz; made after the song's `acoustid` and `musicbrainz` lookups due that run, so it asks for the release they found |

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

Any `[[trigger]]` in the song list replaces all of them, and `--offline`
makes no lookup. A `[providers.<name>]` table sets `enabled`,
`concurrency` (lookups at once), `recheck` (how long a lookup that found
nothing waits, as `"30 days"`), `per_run` (the most one run makes, `0`
for no limit), for `lrclib`, `musicbrainz`, `acoustid` and `coverart`,
`url`, another server, such as a mirror, and for `acoustid`, `key`, the
application key it is asked with, muman's own unless you set one:

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
So does a service muman cannot reach, as when the network is down: its
name not resolving, its connection refused or not open within 10 s, or
a proxy refusing to connect to it. One lookup finds that out, rather
than each waiting out its timeout. A service that answers with an
error, or with what muman cannot read, fails that lookup alone.
Each request names muman and its repository in its user agent.

## Choosing songs and changing them

`list`, `remove`, `restore`, `set` and `edit` take a query, read from
what earlier runs resolved, without fetching or measuring. Its terms
are all required, case and accents ignored, so `cafe` matches "Café":

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
  `audio!` unpins, `lyrics=false` takes none. A track or disc number
  that is no number, a date that is none, and the gains muman measures
  are refused, as by `add --tag`.
- **`edit`** opens the songs' entries in `$VISUAL` or `$EDITOR`. Delete
  a table to remove its song, move a key to another table to join the
  two, or into a new table without an `id` to split it off. A file with
  problems reopens with each as an `# ERROR:` line.
- **`remove`** moves songs to `[[removed]]` and deletes their library
  files; `--purge` also deletes fetched sources and trashes your own.
- **`restore`** lists removed songs again, fetching any purged source.
  It refuses until you put back from the trash the files of your own
  `--purge` moved there, naming each.
- **`undo`** puts the song list and library back as before the last run
  that changed them, one run further back each time: files it wrote over
  or removed come back, a file you had changed still yours, files it
  moved move back, a file `add` copied into `sources/` goes, deleted
  while it holds what was copied and moved to the trash once changed,
  and an edit you made by hand before that run is undone with it. A run stopped partway, by a
  crash or Ctrl-C, can be undone too, and so can an undo stopped partway:
  until the sync it ends with is done, it stays the latest run, which
  the next `undo` finishes, going no further back, and `check` names.
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
- **`info`** counts what the library holds, whether it is in step, by
  the decisions `status` shows, the lookups due, and what could be
  better: narrow, mono or clipped audio,
  missing or soft covers, untimed or missing lyrics, missing tags. It
  reads only what earlier runs recorded; `--verbose` names every song.
- **`check`** reports library files missing, empty, changed, left by an
  interrupted run or not muman's, and sources missing, unreadable, cut
  off or missing audio; `info` counts the songs made from damaged audio;
  `--decode` decodes every source in full again, to find damage in a
  file changed since it was measured. `--upstream` asks each site,
  downloading nothing, whether it still serves each fetched source in
  the format and at the size its song records, with the same tags, and
  names each it serves otherwise: other audio in that format, the format
  no longer offered, other tags, or the video taken down. The copy in
  `sources/` is then the only one of what was fetched, and worth
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
folder, which gets `muman.zip`. With `--folder`, `-o` names a folder to
make, or an empty one, and what the zip would hold is written into it,
for a card or a player to read as it is; a folder holding anything is
refused, so nothing in it is written over.

The zip's `songs.toml` keeps the song list's commands, which whoever runs
muman on it is [asked to trust](configuration.md#commands-servers-and-secrets).

The library's settings may be changed for the export alone: any setting
of `[library]`, `[audio]` and `[loudness]` with `--set table.key=value`,
and the commonest with `--codec`, `--template` and `--lyrics`. The
export then holds the library a sync under those settings would write:
each song the change leaves alone is copied, and every other is made
again from its sources. A song changed since muman wrote it, or whose
sources are gone, goes in as the library holds it, and the export says
so and exits with 4. Where the library lives and its `max_size` are not
the export's to change, nor `[quality]`, nor loudness when the song
list's `[loudness] mode` measures none.

`--max-size` caps the export, such as `--max-size 32GiB` for a card, as
`[library] max_size` caps the library; in a folder each file counts in
whole blocks of `[library] block_size`, which `--set` may change. Songs
that do not fit are encoded again from their sources at lower bitrates
of `[audio] lossy`, the least audible loss for each byte saved first:
lossless songs before lossy ones, which would lose a generation, and a
few songs lowered far before many a little; once they fit, each song
lowered is raised back as far as the room left allows, as the library
is. Each song is measured once encoded and the rest fitted again on what
it really took, so the export lands under the cap. When even the lowest
bitrates cannot fit, nothing is written and the size needed is said. The
library itself is never changed.

`--redact` writes the zip's `songs.toml` with each secret replaced by
`<redacted>`, as [the reference](cli.md#muman-export) lists; one among a
hook's or a program's own words stays, so read those before you share.

## Hooks

A `[[hook]]` runs a command as you as the library changes, once you have
[trusted it](configuration.md#commands-servers-and-secrets):

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
| `changed` | Once, after a run that wrote, moved or removed any file, an undo's files put back counting as written | `library`, `written`, `moved`, `removed` (counts) |

Each value reaches the command as a `{name}` placeholder in its words
and as a `MUMAN_<NAME>` environment variable, such as `MUMAN_PATH`.
`run` is an argument list, run without a shell; a script it hands a
shell (`sh -c`, `pwsh -Command`, `cmd /c`) reads `"$MUMAN_PATH"`, as a
placeholder in it, where a title could run, is refused. A batch file
(`.bat`, `.cmd`) gets the variables only. A hook's output shows as
`hook:`; one that fails is warned of but fails nothing. `status` runs none.
