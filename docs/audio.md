# Audio

How a song's audio is written: the codecs it is copied in or encoded
to, and how loudness is measured and levelled. The settings are in the
song list's `[audio]` and `[loudness]` tables, with the rest in
[configuration](configuration.md).

## Codecs

`[audio] codecs` lists the codecs a song's audio is copied in, packet
for packet: `opus`, `vorbis`, `aac`, `mp3`, `flac`, `alac` and
`wavpack`. Audio in any other codec is encoded, lossy audio to `lossy`
and lossless audio to `lossless`, at that codec's bitrate; audio already
in the codec it would be encoded to is copied. TrueHD, MLP, DTS-HD Master
Audio, DSD, TAK and Shorten count as lossless. Each codec has its own
file:

| Codec | File | Tags |
|---|---|---|
| `opus` | `.opus` (Ogg) | Vorbis comments |
| `vorbis` | `.ogg` (Ogg) | Vorbis comments |
| `flac` | `.flac` | Vorbis comments |
| `mp3` | `.mp3` | ID3v2.4 |
| `aac`, `alac` | `.m4a` (MP4) | iTunes atoms |
| `wavpack` | `.wv` | APEv2 |

In MP3 and MP4, a tag those formats name, such as the title, the track
or the lyrics, becomes their own frame or atom; any other is kept under
its name, as a `TXXX` frame or an iTunes freeform atom. A tag of
several values, as two artists, is one MP4 atom holding each, as Picard
writes it.

Each codec holds some audio and not others, and muman never lets the
encoder mix speakers anew to make it fit:

| Codec | Holds |
|---|---|
| `flac` | Up to 8 channels, any speakers; integer samples of up to 24 bits |
| `alac` | As FLAC, in mono, stereo, 3.0, 4.0, 5.0, 5.1, 6.1 and 7.1 (wide) alone |
| `wavpack` | Any channels; integer samples of up to 32 bits, or 32-bit float |
| `opus` | Any channels: Vorbis layouts as they are, side speakers named as back ones, any other with no speakers named |
| `vorbis` | Any channels, speakers named by their count; resampled to 44.1 or 48 kHz from a rate it cannot encode at |
| `aac` | Up to 8 channels; 5.0 and 5.1 with side speakers named as back ones |
| `mp3` | Up to 2 channels |

Lossless audio that `lossless` cannot hold whole is written as FLAC if
FLAC can hold it, and as WavPack otherwise: a 7.1 master under
`lossless = "alac"` becomes FLAC, a 32-bit or float one WavPack, so a
float master louder than full scale keeps its peaks. 64-bit float is
written at 32. Lossy audio with more channels than `lossy` holds is mixed
into the first of `[audio] downmix` it holds, or else stereo, as below.

`[audio] layouts` lists the speaker layouts a song is written in as it
is, by ffmpeg's names; a name with no variant in parentheses takes its
variants too, so `"5.1"` takes `5.1(side)`, and `"any"` takes every
layout. Audio in any other layout is mixed, before it is encoded, into
the first of `[audio] downmix` with no more channels than it has, or else
the one with the fewest: audio is mixed up only when `downmix` holds no
layout as small, as mono into stereo. Each layout `downmix` names must
be one `layouts` takes. A mix is scaled so no sum of channels passes
full scale, so it plays quieter than its source: its loudness is
measured on the mix itself, and a source's own ReplayGain is not
written for it. A mixed song is always encoded, never copied.

```toml
[audio]
# Stereo and 5.1 as they are; 7.1 and 7.1.4 mixed into 5.1, and quad
# or 3.0 into stereo.
layouts = ["mono", "stereo", "5.1"]
downmix = ["5.1", "stereo"]
```

```toml
[audio]
# Keep what YouTube and most stores serve; make the rest AAC and ALAC,
# which Apple devices play.
codecs = ["opus", "aac", "mp3", "alac"]
lossy = "aac"
lossless = "alac"
```

Copying never loses anything, while every lossy encode costs a
generation, so list every codec your players read. Setting `lossless`
to a lossy codec makes a library with no lossless files.

## Loudness

Every song is measured whole, as EBU R128 and ITU-R BS.1770 measure
loudness, and levelled to `[loudness] target`. An album's loudness is
measured over every song of it the list holds together, so the album
keeps its songs' levels against each other; a song is on an album when
its album and album artist are another song's, and a single, named for
its title, is an album of its own. `[loudness] mode` says where the
gain goes:

| Mode | The gain is | Files |
|---|---|---|
| `tags` | In ReplayGain 2.0 tags, track and album gain and peak, which players apply; in Opus, as `R128_TRACK_GAIN` and `R128_ALBUM_GAIN` | Retagged, never encoded again |
| `header` | As `tags`, and in each Opus file's header too, which every Opus decoder applies, so the song plays levelled in any player | Retagged: the header is two bytes of the file, and no audio changes |
| `audio` | In the samples, for players that read no gain at all | Encoded again, in the codec they were copied in |
| `off` | Not measured: a source's own ReplayGain tags are written as they are | As before |

The tags always describe the file as written: where the header or the
samples hold a gain, the tags hold what is left of it, and the peaks as
they now are. `[loudness] scope` picks the gain `header` and `audio`
apply, the album's or the song's own. A gain that raises a song is
lowered so its peak stays under `[loudness] ceiling`, measured as
`[loudness] true_peak` says; a gain that lowers it never is, and nor
are the tags, since players keep the peaks under full scale from the
peak tags themselves. R128 gains in Opus lie 5 dB under ReplayGain's,
against the −23 LUFS of EBU R128, so a target other than −18 LUFS makes
the tags differ from what other taggers write.

A gain you set by hand, as `tags = { replaygain_track_gain = "-3 dB" }`
on a song or its album, is written instead of the measured one, and is
the one `header` and `audio` apply.

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
source loses a generation; for an Opus library, `header` levels as
well without it. Audio with more channels than MP3 or AAC holds is
mixed by muman into the first `[audio] downmix` layout the codec holds,
and measured as mixed. The first sync after loudness is measured decodes every source
once, in full; later syncs measure only new sources.
