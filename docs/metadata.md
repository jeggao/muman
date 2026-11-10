# Tags and cleaning

Which source each of a song's tags comes from, how the songs of one
album come to agree, and how muman cleans what its sources offer, tags
and lyrics alike. How audio, covers and lyrics are picked is in
[making the library](library.md#how-the-best-source-wins), and the
settings in [configuration](configuration.md).

## Tags

Each field describes either the recording or the release.

- **The recording's fields**, the title, artist, genre, ISRC and the
  recording's and artists' MusicBrainz IDs, each come from whichever
  source offers it best. A source [set aside](#sources-set-aside), even
  in part, gives no ISRC or ID. A MusicBrainz record offers the genres
  MusicBrainz's users voted for, the recording's, else its release
  group's, its release's or its artists', the most voted, three at most
  and none with under half the votes of the first, each word
  capitalized, as `J-Pop`, but for those
  [genres/spelled.txt](../crates/muman-media/src/genres/spelled.txt)
  spells otherwise, as `R&B`.
- **The release's fields**, the album, album artist, track, disc, date,
  totals, country and release IDs, all come from the one source whose
  album ranks best. So one release's IDs never mix with another's. Of
  albums that rank alike, the source with more of the release's fields
  wins.

Then the songs of one album agree: a MusicBrainz release group, else
an album artist and album alike but for case and punctuation, in one
year. A song with no date joins the songs of its name dated in one
year; of two, as an original and its reissue, it stays an album of its
own. The songs of one album are levelled as one too. Songs on several releases of it take the one most can be on, as
[releases rank](library.md#musicbrainz-and-acoustid); a mono mix on a mono edition keeps its own. An
album spelled two ways, as `On The Border` and `On the Border`, is
spelled as most of its songs spell it; of spellings as common, the one
with more words opening with a capital, then fewer capitals, whatever
order the songs were added in. Songs on one release share its date,
label and IDs. A tag set by hand never changes nor counts.
`original_year` puts every edition in one folder
([configuration](configuration.md#the-path-template)).

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

## Which source is believed

Offers that mean one value count as one: alike in their letters, or
names alike but for a spelling or a decoration that name the same
versions, so `Lanterns Ovr the Harbor` agrees with `Lanterns Over the
Harbor` while `Night Ferry (Live 1975)` keeps apart from `Night Ferry`.
`[tags] believe` says which kind of source is believed first:
`musicbrainz-first`, the default, puts a MusicBrainz record that
vetting believes before a file's own tags, a site's release such as a
YouTube Music track, and an upload; `files-first` puts files first;
`sources` takes the order of each song's `sources`, as muman did before.
The kind believed first decides:

- how a value its sources agree on is spelled, so a file's typo gives
  way to MusicBrainz's spelling. Of sources believed alike that spell
  it alike but for case and punctuation, the spelling most of them
  write wins, then as an album's tie goes, whatever their order;
- which artists a song credits when its sources credit others, so a
  concert's record crediting each singer wins over a file crediting its
  star; a record vetting is unsure of never does, nor one crediting no
  one the others do whose album is not the files' own, as a cover act's.

Other fields rank as before: a field kept for it over one read off a
video's title, plain over decorated, agreed on by more sources; and of
two titles that differ only by a version one names, as `(Live)`, the one
naming it, whatever their order. A title or artist still won by the
order of `sources` alone is contested: `muman status` and `muman check`
name it, so you can set it by hand.

## One name for each artist

Sources spell one artist their own ways: a record as its release
credits it, a file as its tagger wrote it, an upload by its channel's
name. Once every song is resolved, the names crediting one artist alone
are grouped across the library by MusicBrainz's artist ID, and by the
name folded, as `The Marlo Venn` folds as `MARLO VENN` does, never by
names merely alike; a name two artists share joins neither. Each group
holding an ID is written one way, as `[tags] script` says: `original`,
the artist's own name in its own script; `latin`, a name of theirs in
Latin letters where MusicBrainz knows one, an artist name before a
legal name; a search hint, a misspelling MusicBrainz keeps for search,
neither spells an artist nor joins a name to it. With neither known, the
spelling most of the group's records credit in that script is written,
one in another script only where none spells it so, and a song already
spelled as another as common keeps it. A group with no
ID, a credit naming several artists and a tag you set stay as they are.
`muman status` names the spelling's source, as `ARTIST マルロ・ヴェン
(musicbrainz artist …'s own name)`. Records keep artists' own names and
aliases once fetched again, with `muman sync --rematch`.

## Sources set aside

A MusicBrainz record comes to a song by a lookup, and a fingerprint
submitted with another song names that song. So a record's tags count
only once it agrees with what the song's other sources say: their own
tags, never a title read off a video, and the tags you set by hand,
never what the record itself gave the song. muman compares:

- its title, in any two scripts a rule bridges, so a title in kanji
  against one in Latin letters says nothing either way, read as files
  write theirs, as `Kiri Hoshino / Night Ferry`; a take or a date in
  brackets tells nothing apart, and a recording whose title the record
  names live is not the files' studio one;
- its artists, by one whole name in common or a MusicBrainz artist ID:
  `Marlo Venn Tribute Band` holds `Marlo Venn` but is not that artist;
- an ISRC or MusicBrainz recording ID another source holds too, which
  outweighs every name;
- the length it states, against the audio's;
- its album and album artist, against those the song's files name;
- its release, against the one its album is on: songs that came in one
  folder under one album name, a disc's folder and a closing `CD 2`
  taken as the album's, are on the release more than half of them are
  on; those of one `[[album]]` on the one most of them are on.

A record whose title and artists both disagree names another song: it
gives the song nothing, and neither does the Cover Art Archive's cover
of its album. So does a record titled as an instrumental or a karaoke
take, `(Instrumental)`, `(Off Vocal)`, when no other source's title
says so: a single holds its instrumental beside the song, by the same
artist and as long, so only the title tells them apart. One that may
name another, as a cover act's record whose
title agrees but whose artist and album do not, gives no release field
and no ID, and so does one on another release than its album, unless
its album is the files' own, as a concert's video beside its album, or
the files name none. A record on its album's release is believed
though its title is spelled otherwise, as `3 Lanterns` for `Three
Lanterns`. One whose artists share no name with the song's gives no
ID, though it may give its album, as a concert's record crediting each
singer. A source with audio of its own, a pinned source and the tags
you set by hand are never set aside. An AcoustID lookup refuses such a
recording from the start: one whose title and first artist both
disagree with those the song's own sources give, never a record's, is
no match.

`muman status` names each source set aside and why, as `set aside
musicbrainz:…: titled "Night Ferry" by Kiri Hoshino: no title or credit
in common`, and `muman check` counts the songs holding one, names each
song whose audio plays 30 s or more from what a record it believes
says, and each name spelled two ways that no artist ID tells apart.
`[tags.clean.structure] vetting = false` turns this off.

The points vetting gives each kind of evidence were set by hand, but an
album's and its unit's, which `check --calibrate` fitted together on one
library: a record on another release than its album's counts against
it, as one in twenty of its songs' own records were and two in three of
other songs'. `muman check --calibrate` counts, on your library, how
often each agreed,
disagreed or said nothing for records of a song's recording and for
records of another, and prints the points those counts suggest beside
the ones vetting gives; it changes nothing. A record is the song's where
your files carry its recording ID, or its ISRC and no ID, or where an
AcoustID lookup of the song's own audio found it; another where your
files carry other IDs, or where another song lists it and shares a title
or a credit with the song. Each counts only for what never chose it: the
lookup ranks by title, credits and length, so its find counts for the
album and album unit alone, or for all but length where it was the only
candidate, which only lookups made since this version keep (`muman sync
--rematch` asks again); a record chosen for its title never counts for
titles. Points are counted against saying nothing, which vetting counts
as nought, and the kinds every label counts are fitted together too,
printed as `with`, so two that tend to agree at once, as an album and
its unit, are not counted twice: those are the points to take.

## Which release

A MusicBrainz record holds one release of its recording, the one
MusicBrainz ranks first, so songs that came together land on the
album, a compilation and a box set side by side. An AcoustID lookup
keeps every recording a song's print names with the releases each is
on, and `[release] prefer` chooses among them:

- `album` puts the songs of one album, those that came in one folder
  under one album name or in one `[[album]]`, on the one release that
  holds most of them, three in five at least: of those, the one holding
  the most, then whose title is the album's, whose track numbers are the
  files' own, that lacks the fewest tracks, an album before an EP, a
  single or a compilation, then the earliest. Each song takes the track
  it is on there, two songs never one track.
- `original` puts each song on the first album its recording came out
  on.

A release of a kind `[release] avoid` names, as `"compilation"`, is
chosen only where no other would be, and a song's `release` key pins
it to one release of its recording, as `muman review` writes it. An
album three in five of whose songs are on one release already stays on
it, a stray joining it where its recording is on it, unless another
release holds more of its songs or `avoid` names the kind it is on. A song moved takes
the album, date, country, types, track and disc numbers and IDs its
place on the release has, and the label, catalog number and barcode the
album's other songs on it have; its record's own release fields are
left out, but for the album artist and the first release's date where
the release is of the record's own group. A song whose album you set,
or whose record vetting set aside, is left where it is. Songs looked up before
this muman kept no such recordings until `muman sync --rematch` asks
again. `muman status` names the release a song was moved to and why,
as `release  musicbrainz:…: on "Rooms of Salt", the release 11 of its
album's 12 songs are on`.

A song on a reissue is dated as the reissue came out, and its path's
`year` with it. `[release] date = "original"` dates it as its album
first came out, by MusicBrainz's `ORIGINALDATE`, so every edition of an
album shares one year and one folder; a song with no such date keeps
its own. [new.toml](../crates/muman-core/src/new.toml) holds the
defaults.

## Values that say nothing

Some values a source's tags carry name nothing about the song, and
muman drops them before ranking, so they neither win a field nor count
against another source:

- **Placeholders**, which stand for no value: MusicBrainz's `[unknown]`,
  `[no label]` and `[non-album tracks]`, a tagger's `Unknown Artist`, a
  video site's category given as a genre, an artist that is a
  soundtrack's short name, as `Night Ferry OST 2`, and a release country
  that is no two-letter code. Each is told whatever its case, spacing,
  quotes or hyphens, so typography never keeps one.
- **Watermarks**, the name of whoever shared the files, written into
  the work, label, copyright, comment or grouping: a value that stands as
  a tag in its torrent's name, as `GRP` in `Marlo Venn - Discography
  [GRP]`; one a file writes in two of those fields, but for a work it
  writes as its grouping too or a label as its copyright; or a work
  shared by ten titles or more across three albums or more of one
  torrent, an album's discs counting as one, which no piece of music is.
  A label a discography's albums share stays.

`muman status` names each value dropped, as `dropped  WORK “GRP” from
torrent:…: a watermark of torrent:…`. `[tags.clean.packaging]
placeholder` and `watermark` turn each off.

A video's upload date is when it went up, so it dates a song only when
the channel that uploaded it is the song's artist, as another source or
a tag you set names it, one releasing its own music; a date from anyone
else's upload, or from an upload only its own channel credits, is left
out.

## Titles read off a video

An upload names its song in one line, `Marlo Venn - Night Ferry`,
`Night Ferry / Marlo Venn` or `「夜の船」Marlo Venn`. Cleaning splits it
by convention; beside that, every reading its separators allow is
weighed by what the song's other sources and the tags you set say: a
side they credit, the channel's name, a quoted side, a side naming a
version or a title they give. Where no reading leads the next clearly,
`muman check` names the song, as `A title read more than one way`, and
`muman review` walks it; a title or artist you set, or a
`[[tags.from]]` title pattern for the channel, settles it.

## House style

Sources write a featured artist, a version and where a song is from
each their own way: `Night Ferry (feat. Kiri Hoshino)` beside
`Marlo Venn feat. Kiri Hoshino`, `[Live]` beside `(Live)`. Three
`[tags]` keys write each one way once a song's values are chosen:

- `featured`: `title` moves a featured artist the credit names into the
  title, as `(feat. Kiri Hoshino)`, out of the album artist muman fills
  in from the artist too; `artist` moves one the title names
  into the credit, after `feat.`. A name already there is not written
  twice, one featured list takes them all, and `(With You)` is a
  subtitle, featuring no one.
- `versions`: `parentheses` or `brackets` writes each version, as
  `(Live)` or `(Radio Edit)`, in the one kind.
- `context`: `drop` takes out a note on where the song is from, as
  `(From "Harbor Lights")` or `(from the film Harbor Lights)`, though
  not a subtitle as `(From Me to You)`.

`keep` leaves values as their sources wrote them, and a title nothing
changes is left as written. A tag set by hand is never restyled, and no
featured artist moves where you set the title or the artist; `muman
status` names `style` among what changed a value.

## Overrides by torrent or channel

One rip tags every file the same wrong way, and one channel titles
every upload one way. A `[[tags.from]]` in the song list says what the
sources of one torrent, by its info hash of 40 or 64 hex digits, or of
one channel, by the name its uploads credit where they name no artist
of their own, case aside in any script, are not believed for, before any rule cleans them. Each
table names one torrent or one channel, and muman refuses one naming
both or a hash that is none:

```toml
[[tags.from]]
torrent = "0123456789abcdef0123456789abcdef01234567"
drop = ["work", "label"]
credits = false

[[tags.from]]
channel = "Orchard Archive"
title = "%artist% - %title% (%dummy%)"
```

`drop` leaves out the fields named; `credits = false` leaves out the
artist, artists and album artist; then `title` reads each title, as it
came, by a pattern of `%title%`, `%artist%`, `%album%`, `%track%` and
`%dummy%`, a stretch read as nothing, each part taken as a field of the
source's own, so an artist it reads stands though `credits = false`
left out the source's own. A title the pattern does not fit is left as
it is.

## Cleaning

muman cleans every tag a source offers before ranking it, so `Paper
Comets (Album Version)` agrees with `Paper Comets`. Lyrics are cleaned as a
song is written. Tags you set in a song's or an album's `tags.<name>`
are never cleaned.

Lyrics are read alike from a file and a tag, whatever the switches say:
a control character, as DOS's end-of-file mark or a terminal's colour
code, is dropped. A lyrics file that reads as other data gives no
lyrics, and is said when its song is measured.

Each rule is a switch under `[tags.clean.<tier>]`, or
`[lyrics.clean.<tier>]` for lyrics; the song list lists every one.
`muman status` names each rule that changed a tag, as `<tier>.<key>`.

| Tier | Its tag rules | Its lyric rules |
|---|---|---|
| `tidy` | Fix spacing and punctuation, writing quotes and hyphens as [`[tags]`](configuration.md#settings-in-the-song-list) says; never remove a word | Drop markup, `\h`, invisible characters, stray spaces and blank lines; give a look-alike letter, as a Cyrillic `е` in `hеad`, its word's script |
| `packaging` | Remove words naming the release or video: `(Official Video)`, `[Remastered]`, `VEVO` | Drop cues, credits, the title and artist restated around the lyrics, ID tags in untimed lyrics, a tag given twice, and from subtitles the uploader's notes around the lyrics and descriptions of sounds |
| `structure` | Split or swap a whole value by its shape: artists joined by `;`, a disc number closing an album | Put lines in time order; join a subtitle's line wrapped in two; keep once a subtitle written again for each frame |
| `guesswork` | Read a video title by convention, as `Marlo Venn - Paper Comets (feat. Ada Quill)`; can misread | Keep, as one line, a subtitle line typed out over three lines or more |

Flipping a lyric switch writes every song's lyrics again. A new title
or artist writes again the lyrics of a song cleaned by its old one.
Either way only the lyrics change: a file beside a song is written
again, and the song's own file is retagged, never encoded.
