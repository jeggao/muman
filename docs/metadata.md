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
  source offers it best. The ISRC and the IDs come only from a source
  that titles the recording as the song is titled.
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
