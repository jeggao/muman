//! The codecs a library file is written in: what each is called in
//! `songs.toml` and by ffprobe, the container it goes into, and how
//! ffmpeg encodes it.
//!
//! Each codec has one container, so a file's extension says its codec's
//! family: Opus and Vorbis in Ogg (`.opus`, `.ogg`), FLAC and MP3 in their
//! own (`.flac`, `.mp3`), AAC and ALAC in MP4 (`.m4a`). Copying needs no
//! more than that container to hold the source's packets as they are.
//!
//! Encoders are ffmpeg's best widely built ones: libopus, libvorbis and
//! libmp3lame, and ffmpeg's own AAC, ALAC and FLAC. Lossy codecs encode
//! at a set bitrate, constant for MP3 so its size is known before it is
//! written. MP3 holds at most two channels; ffmpeg folds more into stereo.
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
}

impl Codec {
    pub const ALL: [Self; 6] = [
        Self::Opus,
        Self::Vorbis,
        Self::Aac,
        Self::Mp3,
        Self::Flac,
        Self::Alac,
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
        }
    }

    #[must_use]
    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Alac)
    }

    #[must_use]
    pub fn container(self) -> Container {
        match self {
            Self::Opus | Self::Vorbis => Container::Ogg,
            Self::Flac => Container::Flac,
            Self::Mp3 => Container::Mp3,
            Self::Aac | Self::Alac => Container::Mp4,
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
        }
    }

    /// ffmpeg's options that encode to this codec, at `kbps` when lossy.
    #[must_use]
    pub fn encoder_args(self, kbps: Option<u32>) -> Vec<String> {
        let encoder = match self {
            Self::Opus => "libopus",
            Self::Vorbis => "libvorbis",
            Self::Aac => "aac",
            Self::Mp3 => "libmp3lame",
            Self::Flac => "flac",
            Self::Alac => "alac",
        };
        let mut args = vec!["-c:a".to_string(), encoder.to_string()];
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
            // libopusfile refuses a stream that starts before zero.
            Self::Opus => &["-avoid_negative_ts", "make_non_negative", "-f", "opus"],
            Self::Vorbis => &["-f", "ogg"],
            Self::Flac => &["-f", "flac"],
            Self::Mp3 => &["-f", "mp3"],
            Self::Aac | Self::Alac => &["-f", "ipod"],
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
        assert_eq!(Codec::Flac.encoder_args(Some(160)), ["-c:a", "flac"]);
        assert_eq!(
            Codec::Mp3.encoder_args(Some(320)),
            ["-c:a", "libmp3lame", "-b:a", "320k"]
        );
    }
}
