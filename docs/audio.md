# Audio

muman copies a song's audio when you allow its codec, since a copy
loses nothing, and encodes the rest. This page covers which codecs
it copies and which it encodes to, how it levels loudness, and how it
fits a library into a size. The settings live in the song list's
`[audio]` and `[loudness]` tables and in `[library] max_size`; the rest
are in [configuration](configuration.md).

## Codecs

muman copies a song's audio, packet for packet, when its codec is one
`[audio] codecs` lists: `opus`, `vorbis`, `aac`, `mp3`, `flac`, `alac`
or `wavpack`. It encodes any other audio: lossy audio to `[audio]
lossy`, lossless audio to `[audio] lossless`, each at that codec's
bitrate. It copies audio already in the codec it would be encoded to.
TrueHD, MLP, DTS-HD Master Audio, DSD, TAK and
Shorten count as lossless.

Every lossy encode costs a generation, so list every codec your players
read. Setting `lossless` to a lossy codec makes a library with no
lossless files.

```toml
[audio]
# Keep what YouTube and most stores serve; make the rest AAC and ALAC,
# which Apple devices play.
codecs = ["opus", "aac", "mp3", "alac"]
lossy = "aac"
lossless = "alac"
```

Each codec has its own file:

| Codec | File | Tags |
|---|---|---|
| `opus` | `.opus` (Ogg) | Vorbis comments |
| `vorbis` | `.ogg` (Ogg) | Vorbis comments |
| `flac` | `.flac` | Vorbis comments |
| `mp3` | `.mp3` | ID3v2.4 |
| `aac`, `alac` | `.m4a` (MP4) | iTunes atoms |
| `wavpack` | `.wv` | APEv2 |

In MP3 and MP4, a tag the format has a frame or atom for, such as the
title, the track or the lyrics, goes there. Any other keeps its name in
a `TXXX` frame or an iTunes freeform atom. A tag of several values, as
two artists, is one MP4 atom holding each, as Picard writes it.

### What each codec holds

No codec holds every kind of audio, and muman never lets an encoder
remix the speakers to make audio fit:

| Codec | Holds |
|---|---|
| `flac` | Up to 8 channels, in any layout; integer samples of up to 24 bits |
| `alac` | As FLAC, but only in mono, stereo, 3.0, 4.0, 5.0, 5.1, 6.1 and 7.1 (wide) |
| `wavpack` | Any number of channels; integer samples of up to 32 bits, or 32-bit float |
| `opus` | Any number of channels: Vorbis layouts as they are, side speakers named as back ones, any other layout with no speakers named |
| `vorbis` | Any number of channels, its speakers named by their count; resampled to 44.1 or 48 kHz from a rate it cannot encode |
| `aac` | Up to 8 channels; 5.0 and 5.1 with side speakers named as back ones |
| `mp3` | Up to 2 channels |

Lossless audio that `lossless` cannot hold whole is written as FLAC if
FLAC can hold it, else as WavPack. So a 7.1 master under `lossless =
"alac"` becomes FLAC, and a 32-bit or float one WavPack, which keeps a
float master's peaks above full scale. 64-bit float is written at 32.

### Speaker layouts

`[audio] layouts` lists the speaker layouts muman writes as they are,
by ffmpeg's names. A name without a variant in parentheses takes its
variants too, so `"5.1"` takes `5.1(side)`, and `"any"` takes every
layout.

muman mixes audio in any other layout before it encodes it. The mix
goes into the first layout of `[audio] downmix` with no more channels
than the audio has, or, failing that, the one with the fewest. So
audio is mixed up only when `downmix` holds no layout as small, as
mono into stereo. Every layout `downmix` names must be one `layouts`
takes. Lossy audio with more channels than `lossy` holds is mixed into
the first `downmix` layout that codec holds, or into stereo.

muman scales a mix so no sum of channels passes full scale, which makes
it play quieter than its source. It measures loudness on the mix itself
and does not write the source's own ReplayGain for it. A mixed song is
always encoded, never copied.

```toml
[audio]
# Stereo and 5.1 as they are; 7.1 and 7.1.4 mixed into 5.1, and quad
# or 3.0 into stereo.
layouts = ["mono", "stereo", "5.1"]
downmix = ["5.1", "stereo"]
```

## Loudness

muman measures every song whole, as EBU R128 and ITU-R BS.1770 measure
loudness, and levels it to `[loudness] target`. It measures an album's
loudness over all of its songs in the list together, so the album keeps
its songs' levels against each other. Songs share an album when they
share its name and album artist; a single, named for its title, is an
album of its own.

`[loudness] mode` says where the gain goes:

| Mode | The gain goes | Files |
|---|---|---|
| `tags` | Into ReplayGain 2.0 tags, track and album gain and peak, which players apply; in Opus, into `R128_TRACK_GAIN` and `R128_ALBUM_GAIN` | Retagged, never encoded again |
| `header` | As `tags`, and into each Opus file's header too, which every Opus decoder applies, so the song plays levelled in any player | Retagged; the header gain is two bytes of the file, and no audio changes |
| `audio` | Into the samples, for players that read no gain at all | Encoded again, in the codec they were copied in |
| `off` | Nowhere: nothing is measured, and a source's own ReplayGain tags are written as they are | As before |

The tags always describe the file as written. Where the header or the
samples hold a gain, the tags hold what is left of it, and the peaks as
they now are. `[loudness] scope` picks the gain `header` and `audio`
apply: the album's, or the song's own.

muman lowers a gain that raises a song so its peak stays under
`[loudness] ceiling`, measured as `[loudness] true_peak` says. It never
lowers a gain that lowers a song, nor the tags, since players keep the
peaks under full scale from the peak tags themselves.

Opus holds its gains as R128 gains, which lie 5 dB under ReplayGain's:
they count from EBU R128's −23 LUFS, where ReplayGain 2.0 counts from
−18 LUFS. With a target other than −18 LUFS, the R128 tags muman writes
differ from what other taggers write. You cannot set a gain by hand;
muman measures every gain itself.

```toml
[loudness]
# Every Opus song plays levelled, in any player, without encoding again.
mode = "header"
```

```toml
[loudness]
# For a car stereo that reads no tags: each song as loud as the next.
mode = "audio"
scope = "track"
```

`audio` encodes again every song it would have copied, so a lossy
source loses a generation. For an Opus library, `header` levels as well
without that cost.

The first sync that measures loudness decodes every source once, in
full. Later syncs measure only new sources.

## Library size

`[library] max_size` caps what the files muman writes may take, as
`"32 GiB"` for a card. Each `sync` fits the library under it: a song
that does not fit at its best is written at a lower bitrate of `[audio]
lossy`, never below `[audio] min_bitrate`.

muman lowers first the songs that lose the least audible quality for
each byte saved. Lossless songs go first, since an encoder at a high
bitrate loses nothing audible of them. Lossy songs go last, since
encoding them again costs a generation. Once the library fits, muman
raises each lowered song back to the best format that still fits,
starting with the songs whose last step down lost the most for each
byte saved, so no song stays lower than the room needs.

The fit depends on the song list, the sources, the settings and the
ffmpeg in use, never on what the library held before. A library fitted
song by song ends exactly as one fitted at once, and two homes with the
same song list and sources write the same songs in the same formats.
But one ffmpeg can write other bytes on another processor: its FLAC and
AAC encoders, among others, round differently with other vector
instructions. So two such homes may fit a song differently where it
fits by a few bytes.

That exactness costs time:

- Adding a song can lower others, and removing one or raising the limit
  raises them again.
- muman renders a song to learn exactly what it takes before it counts
  toward the limit. It keeps what it learns in `state.json`, so a sync
  that changes nothing renders nothing, and a song rendered to be
  measured moves into the library rather than being rendered twice.

Each file counts as whole `[library] block_size` blocks. Set
`block_size` to the allocation unit of the filesystem the library is
copied to: `"32 KiB"` on a FAT32 card counts every small `.lrc` as 32
KiB, as the card does.

When even the lowest bitrates cannot fit, `sync` says how much room the
library needs and leaves songs out, the last listed first, until the
rest fit. muman removes their files, and the run exits with code 4.
`status` shows each lowered song's format and why, and what the library
would take; `info` shows what it takes against `max_size`.

A song you changed since muman wrote it counts as it is and is never
encoded again, unless `sync --force` writes it again, which fits it as
any other song. muman does not count files in the library that it did
not write.
