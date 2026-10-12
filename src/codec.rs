//! The codecs a library file is written in: what each is called in
//! `songs.toml` and by ffprobe, the container it goes into, and how
//! ffmpeg encodes it.
//!
//! Each codec has one container, so a file's extension says its codec's
//! family: Opus and Vorbis in Ogg (`.opus`, `.ogg`), FLAC, MP3 and WavPack
//! in their own (`.flac`, `.mp3`, `.wv`), AAC and ALAC in MP4 (`.m4a`).
//! Copying needs no more than that container to hold the source's packets
//! as they are.
//!
//! Encoders are ffmpeg's best widely built ones: libopus, libvorbis and
//! libmp3lame, and ffmpeg's own AAC, ALAC, FLAC and WavPack. Lossy codecs
//! encode at a set bitrate, constant for MP3 so its size is known before
//! it is written.
//!
//! # What each codec holds
//!
//! Before any codec, audio in a [`Layout`] `[audio] layouts` takes not
//! is mixed into one `[audio] downmix` names; the codec then sees the
//! mix. ffmpeg scales a mix written as integers so it cannot clip and
//! one written as float not, so every mix is asked to be scaled alike.
//!
//! Each codec holds some shapes of audio and not others, and ffmpeg left
//! to itself would make a source fit by mixing its speakers anew or
//! failing. So [`Codec::adapt`] says, for a source's [`Shape`], how a codec
//! writes it, or that it cannot:
//!
//! - **FLAC** holds up to eight channels of integer samples of up to 24
//!   bits, in any layout. **ALAC** holds the same samples in its own
//!   layouts alone. A lossless codec that cannot hold a source exactly is
//!   not used for it: FLAC is, if it can, and WavPack otherwise.
//! - **WavPack** holds any number of channels, of integer samples of up to
//!   32 bits or 32-bit float, so float above full scale is kept. 64-bit
//!   float is written at 32.
//! - **Opus** holds the Vorbis layouts of up to eight channels. A layout
//!   with side speakers where those have back ones, or with none named, is
//!   relabelled to it, its channels in order; any other is written with
//!   no speakers named, each channel kept.
//! - **Vorbis** names speakers by their count alone, and is resampled to
//!   44.1 or 48 kHz from a rate its bitrate modes do not take.
//! - **AAC** holds eight channels and **MP3** two; ffmpeg folds more into
//!   them.
//!
//! Every container is written bit-exact: ffmpeg otherwise gives each Ogg
//! stream a random serial number, so one plan rendered twice would differ
//! byte for byte, and a library would depend on when its songs were
//! written.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Opus,
    Vorbis,
    Aac,
    Mp3,
    Flac,
    Alac,
    #[serde(rename = "wavpack")]
    WavPack,
}

/// The file a codec is written into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// Ogg, tagged with Vorbis comments.
    Ogg,
    /// Native FLAC, tagged with Vorbis comments.
    Flac,
    /// MPEG audio, tagged with `ID3v2`.
    Mp3,
    /// MP4, tagged with iTunes-style atoms.
    Mp4,
    /// WavPack, tagged with APEv2.
    WavPack,
}

/// What a stream holds that decides which codecs can keep it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape<'a> {
    pub channels: u32,
    /// The speakers in ffmpeg's name, as `5.1(side)`; `None` when the file
    /// names none.
    pub layout: Option<&'a str>,
    pub sample_rate: u32,
    /// Bits a sample keeps; 0 when unknown.
    pub bits: u32,
    pub float: bool,
}

/// ffmpeg's standard speaker layouts and their channel counts.
const LAYOUTS: [(&str, u32); 35] = [
    ("mono", 1),
    ("stereo", 2),
    ("2.1", 3),
    ("3.0", 3),
    ("3.0(back)", 3),
    ("4.0", 4),
    ("quad", 4),
    ("quad(side)", 4),
    ("3.1", 4),
    ("5.0", 5),
    ("5.0(side)", 5),
    ("4.1", 5),
    ("5.1", 6),
    ("5.1(side)", 6),
    ("6.0", 6),
    ("6.0(front)", 6),
    ("3.1.2", 6),
    ("hexagonal", 6),
    ("6.1", 7),
    ("6.1(back)", 7),
    ("6.1(front)", 7),
    ("7.0", 7),
    ("7.0(front)", 7),
    ("7.1", 8),
    ("7.1(wide)", 8),
    ("7.1(wide-side)", 8),
    ("5.1.2", 8),
    ("octagonal", 8),
    ("cube", 8),
    ("5.1.4", 10),
    ("7.1.2", 10),
    ("7.1.4", 12),
    ("hexadecagonal", 16),
    ("22.2", 24),
    ("downmix", 2),
];

/// The filter that mixes audio into `layout`, scaled so no sum of
/// channels passes full scale.
#[must_use]
pub fn mix_filter(layout: Layout) -> String {
    format!("aresample=ochl={layout}:rematrix_maxval=1")
}

/// What is done to the samples on their way to the encoder, beyond what
/// fits them to the codec: kept in whole numbers, so a plan compares
/// exactly, and rendered here alone, in one order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Chain {
    /// A gain, in hundredths of a dB.
    pub gain: i32,
    /// The bits an integer source's samples hold; 0 for float or unknown.
    pub bits: u32,
}

impl Chain {
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.gain == 0
    }
}

/// One of ffmpeg's standard speaker layouts, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Layout(&'static str);

impl Layout {
    pub const STEREO: Self = Self("stereo");

    /// The layout ffmpeg names `name`, case ignored.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        LAYOUTS
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
            .map(|(n, _)| Self(n))
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        self.0
    }

    #[must_use]
    pub fn channels(self) -> u32 {
        LAYOUTS
            .iter()
            .find(|(n, _)| *n == self.0)
            .map_or(0, |(_, c)| *c)
    }

    /// Whether audio of `shape` is in this layout, or, when the layout's
    /// name has no variant in parentheses, in any variant of it: `5.1`
    /// takes `5.1(side)`. Audio whose file names no layout is mono or
    /// stereo by its count, and otherwise in none.
    #[must_use]
    pub fn takes(self, shape: &Shape<'_>) -> bool {
        let layout = shape.layout.or(match shape.channels {
            1 => Some("mono"),
            2 => Some("stereo"),
            _ => None,
        });
        layout.is_some_and(|l| {
            l.eq_ignore_ascii_case(self.0)
                || (!self.0.contains('(')
                    && l.split('(')
                        .next()
                        .is_some_and(|f| f.eq_ignore_ascii_case(self.0)))
        })
    }

    fn known() -> String {
        LAYOUTS.map(|(n, _)| n).join(", ")
    }
}

impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for Layout {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Layout {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Self::named(&name).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "no speaker layout is named \"{name}\"; ffmpeg names {}",
                Self::known()
            ))
        })
    }
}

/// Speaker layouts a song is written in as it is: one, or `any`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speakers {
    Any,
    Of(Layout),
}

impl Speakers {
    #[must_use]
    pub fn takes(self, shape: &Shape<'_>) -> bool {
        match self {
            Self::Any => true,
            Self::Of(layout) => layout.takes(shape),
        }
    }
}

impl<'de> Deserialize<'de> for Speakers {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        if name.trim().eq_ignore_ascii_case("any") {
            return Ok(Self::Any);
        }
        Layout::named(&name).map(Self::Of).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "no speaker layout is named \"{name}\"; name \"any\" or one of {}",
                Layout::known()
            ))
        })
    }
}

/// What a codec does to a stream so it can hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Adapt {
    /// This many channels kept in order, their speakers named as the
    /// Vorbis layout of their count.
    Relabel(u32),
    /// The channels kept in order with no speakers named: Opus's mapping
    /// family 255.
    Discrete,
    /// Resampled to this rate.
    Resample(u32),
}

/// The Vorbis layouts by channel count, the ones Opus holds.
const VORBIS_LAYOUTS: [&str; 8] = ["mono", "stereo", "3.0", "quad", "5.0", "5.1", "6.1", "7.1"];

/// Layouts that are a Vorbis one with side speakers for back ones.
const SIDE_LAYOUTS: [&str; 3] = ["quad(side)", "5.0(side)", "5.1(side)"];

/// The layouts ffmpeg's ALAC encoder holds.
const ALAC_LAYOUTS: [&str; 8] = [
    "mono",
    "stereo",
    "3.0",
    "4.0",
    "5.0",
    "5.1",
    "6.1(back)",
    "7.1(wide)",
];

/// Whether audio in the codec ffprobe names `name`, of `profile`, keeps
/// every sample as recorded: DTS does in its lossless profiles alone. DSD
/// is counted lossless too: it is decoded to float, which
/// [`Codec::WavPack`] keeps whole.
#[must_use]
pub fn is_lossless_name(name: &str, profile: Option<&str>) -> bool {
    if name == "dts" {
        return profile.is_some_and(|p| p.starts_with("DTS-HD MA"));
    }
    matches!(
        name,
        "flac"
            | "alac"
            | "wavpack"
            | "ape"
            | "tta"
            | "truehd"
            | "mlp"
            | "tak"
            | "shorten"
            | "wmalossless"
            | "mp4als"
            | "ralf"
            | "dst"
    ) || name.starts_with("pcm_")
        || name.starts_with("dsd_")
}

impl Codec {
    pub const ALL: [Self; 7] = [
        Self::Opus,
        Self::Vorbis,
        Self::Aac,
        Self::Mp3,
        Self::Flac,
        Self::Alac,
        Self::WavPack,
    ];

    /// The codec ffprobe names `name`, if a library file can hold it.
    #[must_use]
    pub fn probed(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The name `songs.toml` and ffprobe both use.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Vorbis => "vorbis",
            Self::Aac => "aac",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Alac => "alac",
            Self::WavPack => "wavpack",
        }
    }

    /// How the codec is named to a person.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Opus => "Opus",
            Self::Vorbis => "Vorbis",
            Self::Aac => "AAC",
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::Alac => "ALAC",
            Self::WavPack => "WavPack",
        }
    }

    #[must_use]
    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Alac | Self::WavPack)
    }

    /// How this codec writes audio of `shape`: as it is (`Some(None)`),
    /// adapted, or `None` when it cannot keep every channel and sample.
    #[must_use]
    pub fn adapt(self, shape: &Shape<'_>) -> Option<Option<Adapt>> {
        let integer = !shape.float && shape.bits <= 24;
        match self {
            Self::Flac => (integer && shape.channels <= 8).then_some(None),
            Self::Alac => {
                let held = match shape.layout {
                    Some(layout) => ALAC_LAYOUTS.contains(&layout),
                    None => shape.channels <= 2,
                };
                (integer && held).then_some(None)
            }
            Self::Opus => {
                let vorbis = VORBIS_LAYOUTS.get(shape.channels as usize - 1).copied();
                Some(match shape.layout {
                    Some(layout) if Some(layout) == vorbis => None,
                    Some(layout) if vorbis.is_some() && SIDE_LAYOUTS.contains(&layout) => {
                        Some(Adapt::Relabel(shape.channels))
                    }
                    None if vorbis.is_some() => Some(Adapt::Relabel(shape.channels)),
                    _ => Some(Adapt::Discrete),
                })
            }
            Self::Vorbis => Some(match shape.sample_rate {
                16_000 | 32_000 | 44_100 | 48_000 => None,
                r if r % 11_025 == 0 => Some(Adapt::Resample(44_100)),
                _ => Some(Adapt::Resample(48_000)),
            }),
            Self::WavPack | Self::Aac | Self::Mp3 => Some(None),
        }
    }

    #[must_use]
    pub fn container(self) -> Container {
        match self {
            Self::Opus | Self::Vorbis => Container::Ogg,
            Self::Flac => Container::Flac,
            Self::Mp3 => Container::Mp3,
            Self::Aac | Self::Alac => Container::Mp4,
            Self::WavPack => Container::WavPack,
        }
    }

    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Vorbis => "ogg",
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
            Self::Aac | Self::Alac => "m4a",
            Self::WavPack => "wv",
        }
    }

    /// ffmpeg's options that encode to this codec, at `kbps` when lossy,
    /// mixed into `mix`, processed by `chain`, and adapted as `adapt`
    /// says. A mix is scaled so no sum of channels passes full scale:
    /// ffmpeg scales one written as integers so and not one written as
    /// float, which would clip.
    ///
    /// A gain turns integer samples to float, which a lossless encoder
    /// would keep at 24 bits or as float, so a lossless codec has them
    /// back at the source's bits, dithered when that is 16: ffmpeg's
    /// triangular dither is seeded alike on every run, so a song encodes
    /// to the same bytes each time.
    #[must_use]
    pub fn encoder_args(
        self,
        kbps: Option<u32>,
        adapt: Option<Adapt>,
        mix: Option<Layout>,
        chain: Chain,
    ) -> Vec<String> {
        let encoder = match self {
            Self::Opus => "libopus",
            Self::Vorbis => "libvorbis",
            Self::Aac => "aac",
            Self::Mp3 => "libmp3lame",
            Self::Flac => "flac",
            Self::Alac => "alac",
            Self::WavPack => "wavpack",
        };
        let mut args = vec!["-c:a".to_string(), encoder.to_string()];
        let mut filters = Vec::new();
        if let Some(mix) = mix {
            filters.push(mix_filter(mix));
        }
        if chain.gain != 0 {
            filters.push(format!(
                "volume={}dB",
                crate::units::Level(chain.gain).decimal()
            ));
            if self.is_lossless() {
                match chain.bits {
                    1..=16 => filters.push("aresample=osf=s16:dither_method=triangular".into()),
                    17..=32 => filters.push("aresample=osf=s32".into()),
                    _ => {}
                }
            }
        }
        match adapt {
            Some(Adapt::Relabel(channels)) => {
                let layout = VORBIS_LAYOUTS[(channels.clamp(1, 8) - 1) as usize];
                filters.push(format!("channelmap=channel_layout={layout}"));
            }
            Some(Adapt::Discrete) => args.extend(["-mapping_family", "255"].map(String::from)),
            Some(Adapt::Resample(rate)) => args.extend(["-ar".to_string(), rate.to_string()]),
            None => {}
        }
        if !filters.is_empty() {
            args.extend(["-af".to_string(), filters.join(",")]);
        }
        if let Some(kbps) = kbps.filter(|_| !self.is_lossless()) {
            args.extend(["-b:a".to_string(), format!("{kbps}k")]);
        }
        if self == Self::Opus {
            args.extend(["-vbr", "on"].map(String::from));
        }
        args
    }

    /// ffmpeg's options that write this codec's container, the same bytes
    /// every time.
    #[must_use]
    pub fn muxer_args(self) -> Vec<String> {
        let mut args = vec!["-fflags".to_string(), "+bitexact".to_string()];
        let container: &[&str] = match self {
            Self::Opus => &["-f", "opus"],
            Self::Vorbis => &["-f", "ogg"],
            Self::Flac => &["-f", "flac"],
            Self::Mp3 => &["-f", "mp3"],
            Self::Aac | Self::Alac => &["-f", "ipod"],
            Self::WavPack => &["-f", "wv"],
        };
        args.extend(container.iter().map(ToString::to_string));
        args
    }
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_codec_is_found_by_the_name_ffprobe_gives_it() {
        for codec in Codec::ALL {
            assert_eq!(Codec::probed(codec.name()), Some(codec));
        }
        assert_eq!(Codec::probed("pcm_s16le"), None);
    }

    #[test]
    fn a_lossless_codec_takes_no_bitrate() {
        assert_eq!(
            Codec::Flac.encoder_args(Some(160), None, None, Chain::default()),
            ["-c:a", "flac"]
        );
        assert_eq!(
            Codec::Mp3.encoder_args(Some(320), None, None, Chain::default()),
            ["-c:a", "libmp3lame", "-b:a", "320k"]
        );
    }

    fn shape(channels: u32, layout: Option<&str>, bits: u32, float: bool) -> Shape<'_> {
        Shape {
            channels,
            layout,
            sample_rate: 48_000,
            bits,
            float,
        }
    }

    #[test]
    fn a_lossless_codec_takes_only_what_it_keeps_whole() {
        let stereo = shape(2, Some("stereo"), 24, false);
        for codec in [Codec::Flac, Codec::Alac, Codec::WavPack] {
            assert_eq!(codec.adapt(&stereo), Some(None), "{codec}");
        }
        let wide = shape(12, Some("7.1.4"), 24, false);
        let deep = shape(2, Some("stereo"), 32, false);
        let float = shape(2, Some("stereo"), 32, true);
        let side = shape(6, Some("5.1(side)"), 16, false);
        for codec in [Codec::Flac, Codec::Alac] {
            for held in [&wide, &deep, &float] {
                assert_eq!(codec.adapt(held), None, "{codec} {held:?}");
            }
        }
        assert_eq!(Codec::Flac.adapt(&side), Some(None));
        assert_eq!(Codec::Alac.adapt(&side), None);
        for held in [&wide, &deep, &float, &side] {
            assert_eq!(Codec::WavPack.adapt(held), Some(None));
        }
    }

    #[test]
    fn opus_relabels_side_speakers_and_names_none_past_its_layouts() {
        let opus = |channels, layout| Codec::Opus.adapt(&shape(channels, layout, 16, false));
        assert_eq!(opus(6, Some("5.1")), Some(None));
        assert_eq!(opus(6, Some("5.1(side)")), Some(Some(Adapt::Relabel(6))));
        assert_eq!(opus(6, None), Some(Some(Adapt::Relabel(6))));
        assert_eq!(opus(4, Some("4.0")), Some(Some(Adapt::Discrete)));
        assert_eq!(opus(12, Some("7.1.4")), Some(Some(Adapt::Discrete)));
        assert_eq!(
            Codec::Opus.encoder_args(Some(256), Some(Adapt::Relabel(6)), None, Chain::default())
                [2..4],
            ["-af", "channelmap=channel_layout=5.1"]
        );
    }

    #[test]
    fn vorbis_is_resampled_from_a_rate_it_cannot_encode_at() {
        let vorbis = |sample_rate| {
            Codec::Vorbis.adapt(&Shape {
                sample_rate,
                ..shape(2, Some("stereo"), 16, false)
            })
        };
        assert_eq!(vorbis(44_100), Some(None));
        assert_eq!(vorbis(22_050), Some(Some(Adapt::Resample(44_100))));
        assert_eq!(vorbis(176_400), Some(Some(Adapt::Resample(44_100))));
        assert_eq!(vorbis(96_000), Some(Some(Adapt::Resample(48_000))));
    }

    #[test]
    fn lossless_codecs_are_known_by_their_probed_names() {
        for name in [
            "flac",
            "truehd",
            "mlp",
            "dsd_lsbf_planar",
            "pcm_f32le",
            "tak",
        ] {
            assert!(is_lossless_name(name, None), "{name}");
        }
        for name in ["opus", "aac", "ac3", "dts", "mp3"] {
            assert!(!is_lossless_name(name, None), "{name}");
        }
        assert!(is_lossless_name("dts", Some("DTS-HD MA")));
        assert!(!is_lossless_name("dts", Some("DTS-HD HRA")));
    }

    #[test]
    fn a_layout_takes_its_variants_unless_one_is_named() {
        let five = Layout::named("5.1").unwrap();
        let side = Layout::named("5.1(SIDE)").unwrap();
        let of = |layout, channels| shape(channels, layout, 16, false);
        assert!(five.takes(&of(Some("5.1(side)"), 6)));
        assert!(five.takes(&of(Some("5.1"), 6)));
        assert!(!side.takes(&of(Some("5.1"), 6)));
        assert!(!five.takes(&of(Some("5.1.4"), 10)));
        assert!(Layout::STEREO.takes(&of(None, 2)), "two unnamed channels");
        assert!(!five.takes(&of(None, 6)));
        assert!(Speakers::Any.takes(&of(None, 6)));
        assert_eq!(Layout::named("7.1.4").map(Layout::channels), Some(12));
        assert_eq!(Layout::named("5.1 side"), None);
    }

    #[test]
    fn a_mix_is_scaled_before_the_codec_s_own_filter() {
        let args = Codec::Opus.encoder_args(
            Some(256),
            Some(Adapt::Relabel(6)),
            Layout::named("5.1(side)"),
            Chain::default(),
        );
        assert_eq!(
            args[2..4],
            [
                "-af",
                "aresample=ochl=5.1(side):rematrix_maxval=1,channelmap=channel_layout=5.1"
            ]
        );
    }

    #[test]
    fn a_gain_follows_the_mix_and_keeps_a_lossless_codec_s_bits() {
        let chain = Chain {
            gain: -612,
            bits: 16,
        };
        let args = Codec::Opus.encoder_args(
            Some(256),
            Some(Adapt::Relabel(6)),
            Layout::named("5.1(side)"),
            chain,
        );
        assert_eq!(
            args[2..4],
            [
                "-af",
                "aresample=ochl=5.1(side):rematrix_maxval=1,volume=-6.12dB,\
                 channelmap=channel_layout=5.1"
            ]
        );
        let flac = Codec::Flac.encoder_args(None, None, None, chain);
        assert!(
            flac.iter()
                .any(|a| a == "volume=-6.12dB,aresample=osf=s16:dither_method=triangular")
        );
        let deep = Codec::Flac.encoder_args(None, None, None, Chain { gain: 50, bits: 24 });
        assert!(deep.iter().any(|a| a == "volume=0.5dB,aresample=osf=s32"));
        assert!(
            !Codec::Flac
                .encoder_args(None, None, None, Chain::default())
                .contains(&"-af".into())
        );
    }
}
