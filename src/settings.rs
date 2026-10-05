//! What `songs.toml` sets beyond its songs: how the library is laid out
//! and named, which codecs are kept and what the rest is encoded to, how
//! sources are ranked, how yt-dlp fetches, and how much history `undo`
//! keeps.
//!
//! These live in the song list rather than in a per-user file so that
//! one list renders one library, the same on every machine. The tables
//! are `[library]`, `[audio]`, `[quality]`, `[ytdlp]` and `[history]`;
//! every key is
//! optional, and `new.toml`, the file a new home starts from, lists each
//! with its default. A key these tables do not know is an error naming
//! it, so a misspelled setting is never silently ignored. Which machine
//! runs what is no setting: tools and folders come from flags and
//! `MUMAN_*` variables.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use toml_edit::DocumentMut;

use crate::codec::Codec;
use crate::quality;

/// The tables read here; every other top-level key belongs to the song
/// list proper.
pub const TABLES: [&str; 5] = ["library", "audio", "quality", "ytdlp", "history"];

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub library: Library,
    pub audio: Audio,
    pub quality: Quality,
    pub ytdlp: Ytdlp,
    pub history: History,
}

/// The library's place and each song's path in it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Library {
    /// The library folder: absolute, `~/…`, or relative to the home.
    /// `--library` and `MUMAN_LIBRARY` override it on one machine.
    pub path: Option<String>,
    /// Each song's path without its extension, as a MiniJinja template;
    /// `/` separates folders. See [`crate::template`].
    pub template: String,
    /// Characters replaced in every tag value before it is put in a
    /// path, merged over the defaults.
    pub replace: BTreeMap<String, String>,
    pub restrict: Restrict,
    /// The longest file name kept, in UTF-8 bytes, which bounds it on
    /// every filesystem: no character takes more UTF-16 units than bytes.
    pub max_name_bytes: usize,
    pub max_folder_bytes: usize,
    /// The longest whole path, library folder included, in characters;
    /// the title is cut to fit. Windows programs and players without
    /// long-path support stop at 260.
    pub max_path: Option<usize>,
    pub unknown_artist: String,
    pub unknown_album: String,
    pub untitled: String,
    pub lyrics: LyricsPlacement,
    /// Set the library folder's time to now after a run that changed it,
    /// for players that rescan only when it is newer than their last scan.
    pub touch_root: bool,
    /// The most the files muman writes may take; songs are encoded at
    /// lower bitrates to fit. See [`crate::limit`].
    pub max_size: Option<Size>,
    /// Each file counts as taking a whole number of these, the
    /// filesystem's allocation unit.
    pub block_size: Size,
}

/// `<album artist>/<album>/<NN title>`, `<D-NN title>` from a second
/// disc on: a single on no album is named for its title.
pub const DEFAULT_TEMPLATE: &str = "{{ album_artist }}/{{ album }}/{{ disc_track }}{{ title }}";

impl Default for Library {
    fn default() -> Self {
        Self {
            path: None,
            template: DEFAULT_TEMPLATE.to_string(),
            replace: BTreeMap::new(),
            restrict: Restrict::default(),
            max_name_bytes: 200,
            max_folder_bytes: 120,
            max_path: None,
            unknown_artist: "Unknown Artist".into(),
            unknown_album: "Unknown Album".into(),
            untitled: "Untitled".into(),
            lyrics: LyricsPlacement::default(),
            touch_root: false,
            max_size: None,
            block_size: Size(4096),
        }
    }
}

impl Library {
    /// The folder `path` names, relative to `home` when relative.
    #[must_use]
    pub fn folder(&self, home: &Path) -> Option<PathBuf> {
        let path = self.path.as_deref()?.trim();
        let expanded = match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                directories::UserDirs::new()?
                    .home_dir()
                    .join(rest.trim_start_matches(['/', '\\']))
            }
            _ => PathBuf::from(path),
        };
        Some(if expanded.is_absolute() {
            expanded
        } else {
            home.join(expanded)
        })
    }
}

/// A number of bytes, written in `songs.toml` as an integer or as text
/// with a unit: `"32 GiB"`, `"700MB"`; see [`crate::fit::parse_size`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "SizeText")]
pub struct Size(pub u64);

#[derive(Deserialize)]
#[serde(untagged)]
enum SizeText {
    Bytes(u64),
    Text(String),
}

impl TryFrom<SizeText> for Size {
    type Error = String;

    fn try_from(text: SizeText) -> Result<Self, String> {
        match text {
            SizeText::Bytes(n) => Ok(Self(n)),
            SizeText::Text(t) => crate::fit::parse_size(&t).map(Self),
        }
    }
}

/// How strictly names are made safe, beyond replacing `/` and control
/// characters, which every filesystem refuses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Restrict {
    /// Only what this filesystem refuses.
    None,
    /// What Windows, FAT and most phones refuse too, on every system, so
    /// a library copies anywhere.
    #[default]
    Windows,
    /// As `windows`, and everything transliterated to ASCII.
    Ascii,
}

/// Where lyrics are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsPlacement {
    /// A `.lrc` file beside the song, which most players read.
    #[default]
    Sidecar,
    /// The `LYRICS` tag inside the song.
    Embedded,
    Both,
}

impl LyricsPlacement {
    #[must_use]
    pub fn sidecar(self) -> bool {
        matches!(self, Self::Sidecar | Self::Both)
    }

    #[must_use]
    pub fn embedded(self) -> bool {
        matches!(self, Self::Embedded | Self::Both)
    }
}

/// How audio that is not copied is encoded.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Audio {
    /// Codecs copied as they are; audio in any other is encoded. Audio
    /// already in the codec it would be encoded to is copied too.
    pub codecs: Vec<Codec>,
    /// What lossy audio in a codec not copied is encoded to.
    pub lossy: Codec,
    /// What lossless audio in a codec not copied is encoded to; a lossy
    /// codec here encodes at its bitrate below.
    pub lossless: Codec,
    /// Opus bitrate for mono and stereo, in kbit/s; at or above what
    /// YouTube serves, so encoding loses nothing audible.
    pub opus_kbps: u32,
    /// Opus bitrate for more than two channels.
    pub opus_surround_kbps: u32,
    /// Vorbis and AAC bitrates for two channels, raised in proportion for
    /// more.
    pub vorbis_kbps: u32,
    pub aac_kbps: u32,
    /// MP3 bitrate, constant; MP3 holds two channels at most.
    pub mp3_kbps: u32,
    /// The lowest bitrate a song is lowered to so the library fits
    /// `[library] max_size`, for two channels; none, the encoder's lowest.
    pub min_kbps: Option<u32>,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            codecs: vec![Codec::Opus, Codec::Flac],
            lossy: Codec::Opus,
            lossless: Codec::Flac,
            opus_kbps: 160,
            opus_surround_kbps: 256,
            vorbis_kbps: 192,
            aac_kbps: 256,
            mp3_kbps: 320,
            min_kbps: None,
        }
    }
}

impl Audio {
    /// The bitrate `codec` encodes `channels` at, in kbit/s; none for a
    /// lossless codec.
    #[must_use]
    pub fn kbps(&self, codec: Codec, channels: u32) -> Option<u32> {
        let scaled = |kbps: u32| {
            if channels > 2 {
                kbps * channels / 2
            } else {
                kbps
            }
        };
        match codec {
            Codec::Opus if channels > 2 => Some(self.opus_surround_kbps),
            Codec::Opus => Some(self.opus_kbps),
            Codec::Vorbis => Some(scaled(self.vorbis_kbps)),
            Codec::Aac => Some(scaled(self.aac_kbps)),
            Codec::Mp3 => Some(self.mp3_kbps),
            Codec::Flac | Codec::Alac => None,
        }
    }

    fn check(&self) -> Result<()> {
        if self.lossy.is_lossless() {
            bail!(
                "[audio] lossy = \"{}\" is lossless; name a lossy codec: opus, vorbis, aac or mp3",
                self.lossy
            );
        }
        if self.min_kbps == Some(0) {
            bail!("[audio] min_kbps must be above 0");
        }
        if self.mp3_kbps > 320 {
            bail!("[audio] mp3_kbps is over 320, the most MP3 holds");
        }
        Ok(())
    }
}

/// How the best of a song's sources is picked. Each measure scores a
/// source in whole steps; a source's score is each measure's steps times
/// its weight, summed, and the lowest wins. A measure switched off, or
/// weighing 0, counts for nothing; one that could not be taken ranks a
/// source after every one it was taken on. Ties go to the source listed
/// first. See [`crate::resolve`].
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Quality {
    pub purity: Purity,
    pub bandwidth: Bandwidth,
    pub stereo: Stereo,
    pub clipping: Clipping,
    pub square: Square,
    pub resolution: Resolution,
    pub blockiness: Blockiness,
}

impl Quality {
    fn check(&self) -> Result<()> {
        let steps = [
            ("purity.step_ms", f64::from(self.purity.step_ms)),
            ("bandwidth.step_hz", f64::from(self.bandwidth.step_hz)),
            ("resolution.step", self.resolution.step),
            ("blockiness.step", self.blockiness.step),
        ];
        for (key, step) in steps {
            if step.is_nan() || step <= 0.0 {
                bail!("[quality.{key}] must be above 0");
            }
        }
        let cutoffs = self
            .clipping
            .cutoffs
            .iter()
            .map(|c| ("clipping.cutoffs", *c));
        let bounds = [
            ("stereo.incoherence", self.stereo.incoherence),
            ("square.tolerance", self.square.tolerance),
        ];
        for (key, value) in bounds.into_iter().chain(cutoffs) {
            if !value.is_finite() || value < 0.0 {
                bail!("[quality.{key}] must be a number of 0 or more");
            }
        }
        Ok(())
    }
}

/// Sound in a source beyond the song: a video's intro, outro or skit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Purity {
    pub enabled: bool,
    pub weight: u32,
    /// Sound beyond the song is judged in steps this long: a fade differs
    /// by less, an intro or a skit by more.
    pub step_ms: u32,
}

impl Default for Purity {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 10_000,
            step_ms: 2000,
        }
    }
}

/// Where a lowpass cuts the audio off; wider is better.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Bandwidth {
    pub enabled: bool,
    pub weight: u32,
    /// Wide enough that noise never decides between near-equals.
    pub step_hz: u32,
}

impl Default for Bandwidth {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 10,
            step_hz: 500,
        }
    }
}

/// Real stereo over mono copied into two channels.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stereo {
    pub enabled: bool,
    pub weight: u32,
    /// The share of the channels' energy no gain or lag between them
    /// explains, at or below which audio counts as mono; see
    /// [`quality::STEREO_INCOHERENCE`].
    pub incoherence: f64,
}

impl Default for Stereo {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 5,
            incoherence: quality::STEREO_INCOHERENCE,
        }
    }
}

/// Samples clipped at full scale.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Clipping {
    pub enabled: bool,
    pub weight: u32,
    /// Shares of clipped samples, each one reached a step: by order of
    /// magnitude from the inaudible.
    pub cutoffs: Vec<f64>,
}

impl Default for Clipping {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 1,
            cutoffs: vec![1e-3, 1e-2],
        }
    }
}

/// A square cover over one of another shape.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Square {
    pub enabled: bool,
    pub weight: u32,
    /// How far from square, as the log of the sides' ratio, still counts.
    pub tolerance: f64,
}

impl Default for Square {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 10_000,
            tolerance: quality::SQUARE_TOLERANCE,
        }
    }
}

/// A cover's effective resolution; higher is better.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Resolution {
    pub enabled: bool,
    pub weight: u32,
    /// Each step is this share more resolution: 0.1 is 10%.
    pub step: f64,
}

impl Default for Resolution {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 20,
            step: 0.1,
        }
    }
}

/// A cover's JPEG block artifacts; fewer is better.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Blockiness {
    pub enabled: bool,
    pub weight: u32,
    /// Steps past the tenth count no more, so artifacts never outweigh a
    /// step of resolution at the default weights.
    pub step: f64,
}

impl Default for Blockiness {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 1,
            step: 0.1,
        }
    }
}

macro_rules! weighed {
    ($($measure:ty),*) => {$(
        impl $measure {
            /// What one step weighs: nothing when the measure is off.
            #[must_use]
            pub fn weight(&self) -> i64 {
                if self.enabled { i64::from(self.weight) } else { 0 }
            }
        }
    )*};
}

weighed!(
    Purity, Bandwidth, Stereo, Clipping, Square, Resolution, Blockiness
);

/// How yt-dlp fetches a source.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ytdlp {
    /// yt-dlp's `--format`: what is kept of a video.
    pub format: String,
    /// yt-dlp's `--sub-langs`: the subtitles kept for lyrics.
    pub sub_langs: String,
    pub concurrent_fragments: u32,
    /// Load muman's two postprocessors, which keep generated captions
    /// out and take a YouTube Music track's square album art.
    pub plugins: bool,
    /// More arguments, such as `["--cookies-from-browser", "firefox"]`.
    pub args: Vec<String>,
    /// Days an unfinished download is kept to resume.
    pub partial_days: u64,
}

impl Default for Ytdlp {
    fn default() -> Self {
        Self {
            format: "bv*+ba/b".into(),
            sub_langs: "all,-live_chat".into(),
            concurrent_fragments: 4,
            plugins: true,
            args: Vec::new(),
            partial_days: 14,
        }
    }
}

/// What `undo` can put back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct History {
    /// The changing runs kept.
    pub runs: usize,
    /// The most the kept runs' replaced files may take, in MiB.
    pub max_mib: u64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            runs: 3,
            max_mib: 2048,
        }
    }
}

/// The library folder the song list in `home` names, if it names one.
pub fn library_folder(home: &Path) -> Result<Option<PathBuf>> {
    let file = home.join(crate::dirs::MANIFEST);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let doc: DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML", file.display()))?;
    Ok(read(&doc)?.library.folder(home))
}

/// The settings `doc` sets, the defaults where it sets none.
pub fn read(doc: &DocumentMut) -> Result<Settings> {
    let mut only = DocumentMut::new();
    for table in TABLES {
        if let Some(item) = doc.get(table) {
            only.insert(table, item.clone());
        }
    }
    let settings: Settings = toml::from_str(&only.to_string()).context("reading the settings")?;
    if settings.library.block_size.0 == 0 {
        bail!("[library] block_size must be above 0");
    }
    settings.audio.check()?;
    settings.quality.check()?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(text: &str) -> Result<Settings> {
        read(&text.parse().unwrap())
    }

    #[test]
    fn nothing_set_is_every_default() {
        assert_eq!(settings("version = 1\n").unwrap(), Settings::default());
    }

    #[test]
    fn a_table_sets_only_what_it_names() {
        let s = settings(
            "[library]\ntemplate = \"{{ artist }}/{{ title }}\"\nlyrics = \"both\"\n\
             [audio]\nopus_kbps = 192\n",
        )
        .unwrap();
        assert_eq!(s.library.template, "{{ artist }}/{{ title }}");
        assert_eq!(s.library.lyrics, LyricsPlacement::Both);
        assert_eq!(s.library.untitled, "Untitled");
        assert_eq!(s.audio.opus_kbps, 192);
        assert_eq!(s.audio.opus_surround_kbps, 256);
    }

    #[test]
    fn a_misspelled_key_is_named() {
        let e = settings("[audio]\nopus_kpbs = 192\n").unwrap_err();
        assert!(format!("{e:#}").contains("opus_kpbs"), "{e:#}");
        assert!(settings("[library]\nrestrict = \"dos\"\n").is_err());
    }

    #[test]
    fn codecs_are_named_and_a_lossy_target_must_be_lossy() {
        let s = settings(
            "[audio]\ncodecs = [\"aac\", \"mp3\", \"alac\"]\nlossy = \"mp3\"\nlossless = \"alac\"\n",
        )
        .unwrap();
        assert_eq!(s.audio.codecs, [Codec::Aac, Codec::Mp3, Codec::Alac]);
        assert_eq!(s.audio.kbps(Codec::Mp3, 6), Some(320));
        assert_eq!(s.audio.kbps(Codec::Aac, 6), Some(768));
        assert_eq!(s.audio.kbps(Codec::Alac, 2), None);
        let e = settings("[audio]\nlossy = \"flac\"\n").unwrap_err();
        assert!(format!("{e:#}").contains("lossless"), "{e:#}");
        assert!(settings("[audio]\ncodecs = [\"wma\"]\n").is_err());
    }

    #[test]
    fn a_quality_table_sets_only_what_it_names() {
        let s = settings("[quality.stereo]\nweight = 0\n[quality.clipping]\nenabled = false\n")
            .unwrap();
        assert_eq!(s.quality.stereo.weight(), 0);
        assert_eq!(
            s.quality.stereo,
            Stereo {
                weight: 0,
                ..Stereo::default()
            }
        );
        assert_eq!(s.quality.clipping.weight(), 0);
        assert_eq!(s.quality.bandwidth, Bandwidth::default());
        assert!(settings("[quality.bandwidth]\nstep_hz = 0\n").is_err());
        assert!(settings("[quality.clipping]\ncutoffs = [-1.0]\n").is_err());
        assert!(settings("[quality.loudness]\nweight = 1\n").is_err());
    }

    #[test]
    fn a_size_is_bytes_or_text_with_a_unit() {
        let s = settings("[library]\nmax_size = \"32 GiB\"\nblock_size = 32768\n").unwrap();
        assert_eq!(s.library.max_size, Some(Size(32 << 30)));
        assert_eq!(s.library.block_size, Size(32 << 10));
        let e = settings("[library]\nmax_size = \"lots\"\n").unwrap_err();
        assert!(format!("{e:#}").contains("lots"), "{e:#}");
        assert!(settings("[library]\nblock_size = 0\n").is_err());
    }

    #[test]
    fn a_relative_library_is_under_the_home() {
        let library = Library {
            path: Some("music".into()),
            ..Library::default()
        };
        let home = Path::new("home");
        assert_eq!(library.folder(home), Some(home.join("music")));
        assert_eq!(Library::default().folder(home), None);
    }
}
