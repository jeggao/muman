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
  in part, gives no ISRC or ID.
- **The release's fields**, the album, album artist, track, disc, date,
  totals, country and release IDs, all come from the one source whose
  album ranks best. So one release's IDs never mix with another's. Of
  albums that rank alike, the source with more of the release's fields
  wins.

Then the songs of one album agree: a MusicBrainz release group, else
an album artist and album alike but for case and punctuation, in one
year. Songs on several releases of it take the one most can be on, as
[releases rank](library.md#musicbrainz-and-acoustid); a mono mix on a mono edition keeps its own. An
album spelled two ways, as `On The Border` and `On the Border`, is
spelled as most of its songs spell it, and songs on one release share
its date, label and IDs. A tag set by hand never changes nor counts.
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
  way to MusicBrainz's spelling;
- which artists a song credits when its sources credit others, so a
  concert's record crediting each singer wins over a file crediting its
  star; a record vetting is unsure of never does.

Other fields rank as before: a field kept for it over one read off a
video's title, plain over decorated, agreed on by more sources; and of
two titles that differ only by a version one names, as `(Live)`, the one
naming it, whatever their order. A field still won by the order of
`sources` alone is contested: `muman status` and `muman check` name it,
so you can set it or reorder `sources`.

## Sources set aside

A MusicBrainz record comes to a song by a lookup, and a fingerprint
submitted with another song names that song. So a record's tags count
only once it agrees with what the song's other sources say: their own
tags, never a title read off a video, and the tags you set by hand,
never what the record itself gave the song. muman compares:

- its title, in any two scripts a rule bridges, so a title in kanji
  against one in Latin letters says nothing either way, read as files
  write theirs, as `Kiri Hoshino / Night Ferry`; a take or a date in
  brackets tells nothing apart, and a recording the record calls live is
  not the files' studio one;
- its artists, by one whole name in common or a MusicBrainz artist ID:
  `Marlo Venn Tribute Band` holds `Marlo Venn` but is not that artist;
- an ISRC or MusicBrainz recording ID another source holds too, which
  outweighs every name;
- the length it states, against the audio's;
- its album and album artist, against those the song's files name.

A record whose title and artists both disagree names another song: it
gives the song nothing, and neither does the Cover Art Archive's cover
of its album. One that may name another, as a cover act's record whose
title agrees but whose artist and album do not, gives no release field
and no ID. One whose artists share no name with the song's gives no ID,
though it may give its album, as a concert's record crediting each
singer. A source with audio of its own, a pinned source and the tags
you set by hand are never set aside. An AcoustID lookup refuses such a
recording from the start: one whose title and first artist both
disagree with the song's is no match.

`muman status` names each source set aside and why, as `set aside
musicbrainz:…: titled "Night Ferry" by Kiri Hoshino: no title or credit
in common`. `[tags.clean.structure] vetting = false` turns this off.

## Values that say nothing

Some values a source's tags carry name nothing about the song, and
muman drops them before ranking, so they neither win a field nor count
against another source:

- **Placeholders**, which stand for no value: MusicBrainz's `[unknown]`,
  `[no label]` and `[non-album tracks]`, a tagger's `Unknown Artist`, a
  video site's category given as a genre, and an artist that is a
  soundtrack's short name, as `Night Ferry OST 2`.
- **Watermarks**, the name of whoever shared the files, written into
  the work, label, copyright, comment or grouping: a value that stands as
  a tag in its torrent's name, as `GRP` in `Marlo Venn - Discography
  [GRP]`; one a file writes in two of those fields; or a work shared by
  ten titles or more across three albums or more of one torrent, which no
  piece of music is. A label a discography's albums share stays.

`muman status` names each value dropped, as `dropped  WORK “GRP” from
torrent:…: a watermark of torrent:…`. `[tags.clean.packaging]
placeholder` and `watermark` turn each off.

A video's upload date is when it went up, so it dates a song only when
the channel that uploaded it is the song's artist, as one releasing its
own music; a date from anyone else's upload is left out.

## Cleaning

muman cleans every tag a source offers before ranking it, so `Paper
Comets (Album Version)` agrees with `Paper Comets`. Lyrics are cleaned as a
song is written. Tags you set in a song's or an album's `tags.<name>`
are never cleaned.

Each rule is a switch under `[tags.clean.<tier>]`, or
`[lyrics.clean.<tier>]` for lyrics; the song list lists every one.
`muman status` names each rule that changed a tag, as `<tier>.<key>`.

| Tier | Its tag rules | Its lyric rules |
|---|---|---|
| `tidy` | Fix spacing and punctuation, writing quotes and hyphens as [`[tags]`](configuration.md#settings-in-the-song-list) says; never remove a word | Drop markup, `\h`, invisible characters, stray spaces and blank lines; give a look-alike letter, as a Cyrillic `е` in `hеad`, its word's script |
| `packaging` | Remove words naming the release or video: `(Official Video)`, `[Remastered]`, `VEVO` | Drop cues, credits, the title and artist restated around the lyrics, ID tags in untimed lyrics, and a tag given twice |
| `structure` | Split or swap a whole value by its shape: artists joined by `;`, a disc number closing an album | Put lines in time order; keep once a subtitle written again for each frame |
| `guesswork` | Read a video title by convention, as `Marlo Venn - Paper Comets (feat. Ada Quill)`; can misread | Keep, as one line, a subtitle line typed out over three lines or more |

Flipping a lyric switch writes every song's lyrics again. A new title
or artist writes again the lyrics of a song cleaned by its old one.
Either way only the lyrics change: a file beside a song is written
again, and the song's own file is retagged, never encoded.
