//! What `songs.toml` sets beyond its songs: how the library is laid out
//! and named, which lyrics are taken, which codecs are kept and what the
//! rest is encoded to, how loud songs are made, how sources are ranked,
//! how yt-dlp and aria2c fetch, which programs muman runs, and how much
//! history `undo` keeps.
//!
//! These live in the song list rather than in a per-user file so that
//! one list renders one library, the same on every machine. The tables
//! are `[library]`, `[lyrics]`, `[audio]`, `[loudness]`
//! ([`crate::loudness`]), `[cover]`, `[ytdlp]`, `[torrent]`
//! ([`crate::aria2`]), `[tools]` ([`crate::tools`]), `[history]` and
//! `[sites]` ([`crate::sites`]). A
//! key these tables do not know is an error naming it, so a misspelled
//! setting is never silently ignored.
//!
//! Each aspect a song is made of keeps what decides it in its own table:
//! `[audio.pick]` and `[cover.pick]` hold the measures their sources are
//! ranked by, and `[lyrics.clean]`, as `[tags.clean]`, the switches of the
//! rules that clean them, which [`crate::manifest`] reads beside the
//! song list's other tables of its own.
//! A flag or a `MUMAN_*` variable says what is where on one machine,
//! and overrides the folder or the program the song list names.
//!
//! Every setting is written out, so the file shows what each is set to.
//! `new.toml`, the file a new home starts from, holds each at its
//! default with the comments explaining it, and every write adds a key
//! the file lacks from there ([`fill`]). A key read as missing takes the
//! default all the same. A setting with no default, such as the library
//! folder, stays a comment: TOML has no null to write it as.
//!
//! A default written out looks like a value chosen, and would stay put
//! when a later muman changes the default. So the top-level `edition`
//! names the defaults the file's settings were written from, as Cargo's
//! `edition` does; a file without one was written from the first. When
//! a default changes, [`EDITION`] rises and [`crate::migrate::HISTORY`]
//! records the key and its old default. A setting still at its edition's default, which
//! a later edition changed, is stale ([`stale`]): `sync`, `status` and
//! `check` say so, and `sync --update-defaults` moves each to the new
//! default ([`update`]). A setting at any other value is the user's and
//! stays. The edition rises on any write that finds nothing stale, so a
//! value set later to an old default is not taken for one left behind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::codec::{Codec, Layout, Shape, Speakers};
use crate::lang::{Lang, Role};
use crate::migrate::FIRST;
pub use crate::migrate::{
    EDITION, EDITIONS, Editions, Stale, edition_of, stale, stale_warnings, update,
};
use crate::quality;
pub use crate::units::Size;
use crate::units::serde_as;

/// The tables read here; every other top-level key belongs to the song
/// list proper.
pub const TABLES: [&str; 10] = [
    "library", "lyrics", "audio", "loudness", "cover", "ytdlp", "torrent", "tools", "history",
    "sites",
];

/// The tables `new.toml` writes that [`fill`] keeps written out in every
/// song list: the settings' and the providers' (`crate::provider`).
pub const WRITTEN: [&str; 11] = [
    "library",
    "lyrics",
    "audio",
    "loudness",
    "cover",
    "ytdlp",
    "torrent",
    "tools",
    "history",
    "sites",
    "providers",
];

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub library: Library,
    pub lyrics: Lyrics,
    pub audio: Audio,
    pub loudness: Loudness,
    pub cover: Cover,
    pub ytdlp: Ytdlp,
    pub torrent: Torrent,
    pub tools: Tools,
    pub history: History,
    pub sites: crate::sites::Sites,
    /// The cleaning rules switched over the song list's for an export
    /// alone ([`crate::export`]); none, the song list's own.
    #[serde(skip)]
    pub clean: Option<crate::clean::Settings>,
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
    /// The longest whole path, library folder included, in characters as
    /// Windows counts them, UTF-16 units, an emoji two; the title is cut
    /// to fit. Windows programs and players without long-path support
    /// stop at 260.
    pub max_path: Option<usize>,
    pub unknown_artist: String,
    pub unknown_album: String,
    pub untitled: String,
    /// Write the comment a source's own tags carry; a comment set by hand
    /// is written either way.
    pub source_comments: bool,
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
            source_comments: false,
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
        whole(self.path.as_deref()?.trim(), home)
    }
}

/// The path `path` names: `~/…` in the user's home, else relative to
/// `home` when relative.
fn whole(path: &str, home: &Path) -> Option<PathBuf> {
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

/// Where a plan's lyrics are written, as `[library] lyrics` said before
/// `[[lyrics.out]]` did, and as a plan of its outputs still says it
/// ([`crate::resolve::LyricsRef`]).
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

/// One place a song's lyrics are written, a `[[lyrics.out]]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LyricsOut {
    pub to: To,
    /// What follows the song's own name in a file's, before its
    /// extension, as `.en`; a tag has none.
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub format: LyricsFormat,
    #[serde(default)]
    pub when: When,
    /// Keep the time of each word, where the lyrics carry them and the
    /// format holds them.
    #[serde(default = "yes")]
    pub words: bool,
}

fn yes() -> bool {
    true
}

/// Where an output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum To {
    /// A file beside the song.
    File,
    /// A tag inside the song's file.
    Tag,
}

/// How lyrics are written ([`crate::cues`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsFormat {
    /// LRC, each line's time before it, as the lyrics are kept.
    #[default]
    Lrc,
    /// WebVTT cues.
    Vtt,
    /// SubRip cues.
    Srt,
    /// Plain text, no times.
    Txt,
}

impl LyricsFormat {
    /// The extension of a file of it.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Lrc => "lrc",
            Self::Vtt => "vtt",
            Self::Srt => "srt",
            Self::Txt => "txt",
        }
    }

    /// Whether it holds only lyrics whose lines carry times, as cues.
    #[must_use]
    pub fn cues(self) -> bool {
        matches!(self, Self::Vtt | Self::Srt)
    }
}

/// Which lyrics an output takes, by whether their lines carry times.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum When {
    #[default]
    Any,
    Timed,
    Untimed,
}

impl When {
    /// Whether it takes lyrics `timed` or not.
    #[must_use]
    pub fn takes(self, timed: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Timed => timed,
            Self::Untimed => !timed,
        }
    }

    /// Whether some lyrics are taken by both.
    #[must_use]
    pub fn meets(self, other: Self) -> bool {
        self.takes(true) && other.takes(true) || self.takes(false) && other.takes(false)
    }
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
    #[serde(rename = "opus_bitrate", deserialize_with = "serde_as::kbps")]
    pub opus_kbps: u32,
    /// Opus bitrate for more than two channels.
    #[serde(rename = "opus_surround_bitrate", deserialize_with = "serde_as::kbps")]
    pub opus_surround_kbps: u32,
    /// Vorbis and AAC bitrates for two channels, raised in proportion for
    /// more.
    #[serde(rename = "vorbis_bitrate", deserialize_with = "serde_as::kbps")]
    pub vorbis_kbps: u32,
    #[serde(rename = "aac_bitrate", deserialize_with = "serde_as::kbps")]
    pub aac_kbps: u32,
    /// MP3 bitrate, constant; MP3 holds two channels at most.
    #[serde(rename = "mp3_bitrate", deserialize_with = "serde_as::kbps")]
    pub mp3_kbps: u32,
    /// The lowest bitrate a song is lowered to so the library fits
    /// `[library] max_size`, for two channels; none, the encoder's lowest.
    #[serde(rename = "min_bitrate", deserialize_with = "serde_as::kbps_opt")]
    pub min_kbps: Option<u32>,
    /// Speaker layouts written as they are; audio in any other is mixed
    /// into one of `downmix`.
    pub layouts: Vec<Speakers>,
    /// What audio in a layout `layouts` takes not is mixed into: the
    /// first with no more channels than it has, or else the fewest.
    pub downmix: Vec<Layout>,
    pub pick: AudioPick,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            codecs: vec![
                Codec::Opus,
                Codec::Vorbis,
                Codec::Aac,
                Codec::Mp3,
                Codec::Flac,
            ],
            lossy: Codec::Opus,
            lossless: Codec::Flac,
            opus_kbps: 160,
            opus_surround_kbps: 256,
            vorbis_kbps: 192,
            aac_kbps: 256,
            mp3_kbps: 320,
            min_kbps: None,
            layouts: vec![Speakers::Any],
            downmix: vec![Layout::STEREO],
            pick: AudioPick::default(),
        }
    }
}

/// The most libopus encodes a channel at, in kbit/s: it refuses more.
const OPUS_KBPS_A_CHANNEL: u32 = 256;

impl Audio {
    /// The bitrate `codec` encodes `channels` at, in kbit/s; none for a
    /// lossless codec. Opus is held to what libopus takes for the channels.
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
            Codec::Opus => {
                let kbps = if channels > 2 {
                    self.opus_surround_kbps
                } else {
                    self.opus_kbps
                };
                Some(kbps.min(OPUS_KBPS_A_CHANNEL * channels.max(1)))
            }
            Codec::Vorbis => Some(scaled(self.vorbis_kbps)),
            Codec::Aac => Some(scaled(self.aac_kbps)),
            Codec::Mp3 => Some(self.mp3_kbps),
            Codec::Flac | Codec::Alac | Codec::WavPack => None,
        }
    }

    /// The layout audio of `shape` is mixed into, if `layouts` takes not
    /// its own, and the shape it is written in.
    #[must_use]
    pub fn written<'a>(&self, shape: &Shape<'a>) -> (Option<Layout>, Shape<'a>) {
        if self.layouts.iter().any(|l| l.takes(shape)) {
            return (None, shape.clone());
        }
        let fits = self.downmix.iter().find(|l| l.channels() <= shape.channels);
        let Some(mix) = fits.or_else(|| self.downmix.iter().min_by_key(|l| l.channels())) else {
            return (None, shape.clone());
        };
        let mixed = Shape {
            channels: mix.channels(),
            layout: Some(mix.name()),
            ..shape.clone()
        };
        (Some(*mix), mixed)
    }

    /// [`Self::written`] for audio written in `codec`: audio with more
    /// channels than a lossy codec holds is mixed into the first of
    /// `downmix` it holds, else stereo, so the codec never folds them in.
    #[must_use]
    pub fn written_in<'a>(&self, codec: Codec, shape: &Shape<'a>) -> (Option<Layout>, Shape<'a>) {
        let (mix, written) = self.written(shape);
        match codec.max_channels() {
            Some(most) if written.channels > most => {
                let layout = self
                    .downmix
                    .iter()
                    .copied()
                    .find(|l| l.channels() <= most)
                    .unwrap_or(Layout::STEREO);
                let mixed = Shape {
                    channels: layout.channels(),
                    layout: Some(layout.name()),
                    ..shape.clone()
                };
                (Some(layout), mixed)
            }
            _ => (mix, written),
        }
    }

    fn check(&self) -> Result<()> {
        if self.downmix.is_empty() {
            bail!("[audio] downmix names no layout to mix audio into");
        }
        for mix in &self.downmix {
            let shape = Shape {
                channels: mix.channels(),
                layout: Some(mix.name()),
                sample_rate: 0,
                bits: 0,
                float: false,
            };
            if !self.layouts.iter().any(|l| l.takes(&shape)) {
                bail!("[audio] downmix names {mix}, which [audio] layouts does not take");
            }
        }
        if self.lossy.is_lossless() {
            bail!(
                "[audio] lossy = \"{}\" is lossless; name a lossy codec: opus, vorbis, aac or mp3",
                self.lossy
            );
        }
        let rates = [
            ("opus_bitrate", Some(self.opus_kbps)),
            ("opus_surround_bitrate", Some(self.opus_surround_kbps)),
            ("vorbis_bitrate", Some(self.vorbis_kbps)),
            ("aac_bitrate", Some(self.aac_kbps)),
            ("mp3_bitrate", Some(self.mp3_kbps)),
            ("min_bitrate", self.min_kbps),
        ];
        if let Some((name, _)) = rates.iter().find(|(_, kbps)| *kbps == Some(0)) {
            bail!("[audio] {name} must be above 0");
        }
        if self.mp3_kbps > 320 {
            bail!("[audio] mp3_bitrate is over 320 kb/s, the most MP3 holds");
        }
        self.pick.check()
    }
}

/// How loud each song is made, from its loudness measured as EBU R128
/// measures it; [`crate::loudness`] holds the design.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Loudness {
    pub mode: LoudnessMode,
    /// The loudness every song is levelled to, in hundredths of a LUFS.
    #[serde(deserialize_with = "serde_as::lufs")]
    pub target: i32,
    /// Peaks between samples, as a DAC rebuilds them, rather than the
    /// samples' own.
    pub true_peak: bool,
    /// The gain `header` and `audio` apply.
    pub scope: GainScope,
    /// The highest peak a raising gain applied may lift a song to, in
    /// hundredths of a dB against full scale.
    #[serde(deserialize_with = "serde_as::db")]
    pub ceiling: i32,
}

impl Default for Loudness {
    fn default() -> Self {
        Self {
            mode: LoudnessMode::Tags,
            target: -1800,
            true_peak: true,
            scope: GainScope::Album,
            ceiling: -100,
        }
    }
}

impl Loudness {
    /// Whether songs are measured at all.
    #[must_use]
    pub fn measured(&self) -> bool {
        self.mode != LoudnessMode::Off
    }

    fn check(&self) -> Result<()> {
        if !(-7000..=0).contains(&self.target) {
            bail!("[loudness] target must lie between -70 LUFS and 0 LUFS");
        }
        if self.ceiling > 0 {
            bail!("[loudness] ceiling must be 0 dB or less, at most full scale");
        }
        Ok(())
    }
}

/// Where a song's gain is put.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoudnessMode {
    /// Nothing measured: the gains a source's own tags carry are kept.
    Off,
    /// ReplayGain tags players apply, R128 gains in Opus.
    #[default]
    Tags,
    /// Tags, and in Opus the gain in the header every decoder applies.
    Header,
    /// The samples scaled, encoding audio that would be copied.
    Audio,
}

/// Which of a song's gains is applied to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GainScope {
    /// Its album's, so the album keeps its songs' levels; a song on no
    /// album takes its own.
    #[default]
    Album,
    /// Its own.
    Track,
}

/// How the best audio of a song's sources is picked. Each measure scores
/// a source in whole steps; a source's score is each measure's steps
/// times its weight, summed, and the lowest wins. A measure switched off,
/// or weighing 0, counts for nothing; one that could not be taken ranks a
/// source after every one it was taken on. Audio scored alike is told
/// apart as [`crate::resolve`] says; the last ties go to the source
/// listed first.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioPick {
    pub purity: Purity,
    pub bandwidth: Bandwidth,
    pub stereo: Stereo,
    pub clipping: Clipping,
}

impl AudioPick {
    fn check(&self) -> Result<()> {
        let steps = [
            ("purity.step", f64::from(self.purity.step_ms)),
            ("bandwidth.step", f64::from(self.bandwidth.step_hz)),
        ];
        let cutoffs = self
            .clipping
            .cutoffs
            .iter()
            .map(|c| ("clipping.cutoffs", *c));
        let bounds = std::iter::once(("stereo.incoherence", self.stereo.incoherence));
        check_measures("audio.pick", &steps, bounds.chain(cutoffs))
    }
}

/// The song's cover.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cover {
    pub pick: CoverPick,
    /// Where the cover a song takes is written; none, nowhere.
    pub out: Vec<CoverOut>,
}

impl Default for Cover {
    fn default() -> Self {
        Self {
            pick: CoverPick::default(),
            out: vec![CoverOut {
                to: To::Tag,
                scope: Scope::Song,
                suffix: String::new(),
                name: String::new(),
                format: CoverFormat::Keep,
                max_side: None,
                quality: DEFAULT_QUALITY,
                mixed: Mixed::None,
            }],
        }
    }
}

impl Cover {
    /// Each `[[cover.out]]` as it may be: one tag at most, holding a
    /// picture players read inside a song, each file's suffix a plain one,
    /// a folder's file a plain name, no name twice, a side and a quality
    /// within bounds.
    fn check(&self) -> Result<()> {
        let tags = self.out.iter().filter(|o| o.to == To::Tag).count();
        if tags > 1 {
            bail!("[[cover.out]] names to = \"tag\" {tags} times; a song holds one front cover");
        }
        for (n, out) in self.out.iter().enumerate() {
            if out.max_side == Some(0) {
                bail!("[[cover.out]] max_side must be above 0");
            }
            if !(1..=100).contains(&out.quality) {
                bail!("[[cover.out]] quality is from 1 to 100");
            }
            if out.scope == Scope::Folder {
                Self::check_folder(&self.out[..n], out)?;
                continue;
            }
            if !out.name.is_empty() {
                bail!(
                    "[[cover.out]] name is a folder's file's, scope = \"folder\"; a song's file \
                     takes a suffix"
                );
            }
            if out.mixed != Mixed::None {
                bail!("[[cover.out]] mixed is for scope = \"folder\"");
            }
            match out.to {
                To::Tag if !out.suffix.is_empty() => {
                    bail!("[[cover.out]] to = \"tag\" takes no suffix; a tag has no name")
                }
                To::Tag if out.format == CoverFormat::Webp => bail!(
                    "[[cover.out]] to = \"tag\" holds keep, jpeg or png; few players read WebP \
                     inside a song"
                ),
                To::Tag => {}
                To::File => {
                    crate::outputs::check_suffix(&out.suffix)
                        .map_err(|e| anyhow::anyhow!("[[cover.out]] suffix: {e}"))?;
                    let names = |o: &CoverOut| -> Vec<String> {
                        o.format
                            .extensions()
                            .iter()
                            .map(|e| format!("{}.{e}", o.suffix).to_uppercase())
                            .collect()
                    };
                    let twice = self.out[..n].iter().any(|o| {
                        o.to == To::File
                            && o.scope == Scope::Song
                            && names(o).iter().any(|a| names(out).contains(a))
                    });
                    if twice {
                        bail!(
                            "[[cover.out]] names the file `<song>{}` twice",
                            names(out).join("` or `<song>")
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// A folder's `out` as it may be, after the outputs `before` it: a
    /// file, named by a plain name rather than a suffix, no name twice.
    fn check_folder(before: &[CoverOut], out: &CoverOut) -> Result<()> {
        if out.to != To::File {
            bail!("[[cover.out]] scope = \"folder\" is a file, to = \"file\"");
        }
        if !out.suffix.is_empty() {
            bail!("[[cover.out]] scope = \"folder\" takes a name, not a suffix");
        }
        crate::outputs::check_name(&out.name)
            .map_err(|e| anyhow::anyhow!("[[cover.out]] name: {e}"))?;
        let names = |o: &CoverOut| -> Vec<String> {
            o.format
                .extensions()
                .iter()
                .map(|e| format!("{}.{e}", o.name).to_uppercase())
                .collect()
        };
        let twice = before
            .iter()
            .any(|o| o.scope == Scope::Folder && names(o).iter().any(|a| names(out).contains(a)));
        if twice {
            bail!(
                "[[cover.out]] names the folder's file `{}` twice",
                names(out).join("` or `")
            );
        }
        Ok(())
    }

    /// The outputs of `scope`.
    pub fn of(&self, scope: Scope) -> impl Iterator<Item = &CoverOut> {
        self.out.iter().filter(move |o| o.scope == scope)
    }
}

/// The quality a cover is encoded at, where `[[cover.out]]` sets none.
pub const DEFAULT_QUALITY: u8 = 90;

/// One place a song's cover is written, a `[[cover.out]]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverOut {
    pub to: To,
    /// A song's own, or a file its folder holds once.
    #[serde(default)]
    pub scope: Scope,
    /// What follows the song's own name in a file's, before its
    /// extension; a tag has none.
    #[serde(default)]
    pub suffix: String,
    /// A folder's file's name, before its extension.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub format: CoverFormat,
    /// The longest side it is scaled down to, in pixels; none keeps it.
    #[serde(default)]
    pub max_side: Option<u32>,
    /// How well a JPEG or WebP keeps the picture, from 1 to 100.
    #[serde(default = "default_quality")]
    pub quality: u8,
    /// What a folder holding songs of several albums takes.
    #[serde(default)]
    pub mixed: Mixed,
}

/// Whose a cover file is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// One song's, named by it.
    #[default]
    Song,
    /// Its folder's, one for every song there.
    Folder,
}

/// The cover a folder holding songs of several albums takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mixed {
    /// None.
    #[default]
    None,
    /// Its album's whose songs are most of the folder's, where one's are.
    Majority,
}

fn default_quality() -> u8 {
    DEFAULT_QUALITY
}

/// How a cover is written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverFormat {
    /// Its own bytes where they are JPEG or PNG, else PNG.
    #[default]
    Keep,
    Jpeg,
    Png,
    Webp,
}

impl CoverFormat {
    /// The extensions a file of it may take.
    #[must_use]
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Keep => &["jpg", "png"],
            Self::Jpeg => &["jpg"],
            Self::Png => &["png"],
            Self::Webp => &["webp"],
        }
    }
}

/// How the best cover of a song's sources is picked, scored as
/// [`AudioPick`] scores audio.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoverPick {
    pub square: Square,
    pub resolution: Resolution,
    pub blockiness: Blockiness,
}

impl CoverPick {
    fn check(&self) -> Result<()> {
        let steps = [
            ("resolution.step", self.resolution.step),
            ("blockiness.step", self.blockiness.step),
        ];
        let bounds = std::iter::once(("square.tolerance", self.square.tolerance));
        check_measures("cover.pick", &steps, bounds)
    }
}

/// Each of `steps` above 0, and each of `bounds` a number of 0 or more,
/// else an error naming it under `table`.
fn check_measures<'a>(
    table: &str,
    steps: &[(&str, f64)],
    bounds: impl Iterator<Item = (&'a str, f64)>,
) -> Result<()> {
    for (key, step) in steps {
        if step.is_nan() || *step <= 0.0 {
            bail!("[{table}.{key}] must be above 0");
        }
    }
    for (key, value) in bounds {
        if !value.is_finite() || value < 0.0 {
            bail!("[{table}.{key}] must be a number of 0 or more");
        }
    }
    Ok(())
}

/// Sound in a source beyond the song, a video's intro, outro or skit, and
/// the song a source cut off lacks.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Purity {
    pub enabled: bool,
    pub weight: u32,
    /// Sound beyond the song is judged in steps this long: a fade differs
    /// by less, an intro or a skit by more.
    #[serde(rename = "step", deserialize_with = "serde_as::ms")]
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
    #[serde(rename = "step", deserialize_with = "serde_as::hz")]
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
    #[serde(deserialize_with = "serde_as::share")]
    pub incoherence: f64,
}

impl Default for Stereo {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 5000,
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
    #[serde(deserialize_with = "serde_as::shares")]
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
    #[serde(deserialize_with = "serde_as::share")]
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
    /// yt-dlp's `--format`: what is kept of a video. muman reads a
    /// source's audio, subtitles and thumbnail, never the video itself, so
    /// a format with video in it only takes room in the store.
    pub format: String,
    /// yt-dlp's `--sub-langs`: the subtitles kept for lyrics. YouTube's
    /// live chat, Niconico's comments and Bilibili's danmaku are listed
    /// as subtitles too, though they are viewers' comments, and none goes
    /// into the Matroska file a source is kept in.
    pub sub_langs: String,
    pub concurrent_fragments: u32,
    /// Load muman's yt-dlp plugins ([`crate::plugins`]), which keep
    /// generated captions out, take a YouTube Music track's square album
    /// art and give each file of an archive.org item its item's details.
    pub plugins: bool,
    /// More arguments for every yt-dlp run: listing, searching, the
    /// audio a comparison downloads, and fetching. They reach yt-dlp in a
    /// config file of the run's own, not on its command line
    /// ([`crate::download`]).
    pub args: Vec<String>,
    /// yt-dlp's `--extractor-args`, by extractor, for a fetch and the
    /// check of what a source offers upstream.
    pub extractor_args: BTreeMap<String, String>,
    /// How long an unfinished download is kept to resume.
    #[serde(deserialize_with = "serde_as::duration")]
    pub keep_partial: std::time::Duration,
}

impl Default for Ytdlp {
    fn default() -> Self {
        Self {
            format: "ba/b".into(),
            sub_langs: "all,-live_chat,-comments,-danmaku".into(),
            concurrent_fragments: 4,
            plugins: true,
            args: Vec::new(),
            extractor_args: BTreeMap::from([(
                "youtube".into(),
                "skip=translated_subs;player_client=default,web_music".into(),
            )]),
            keep_partial: crate::units::Time::days(14).duration(),
        }
    }
}

/// How aria2c fetches a torrent's files.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Torrent {
    /// More arguments for every aria2c run, as a port range or a speed
    /// limit.
    pub args: Vec<String>,
    /// How long a fetch waits with no byte arriving before it fails, to
    /// be tried again by a later sync.
    #[serde(deserialize_with = "serde_as::duration")]
    pub stall: std::time::Duration,
    /// How long a fetched torrent is seeded before the run goes on.
    #[serde(deserialize_with = "serde_as::duration")]
    pub seed_time: std::time::Duration,
    /// How long an unfinished fetch is kept to resume.
    #[serde(deserialize_with = "serde_as::duration")]
    pub keep_partial: std::time::Duration,
}

impl Default for Torrent {
    fn default() -> Self {
        Self {
            args: Vec::new(),
            stall: std::time::Duration::from_secs(10 * 60),
            seed_time: std::time::Duration::ZERO,
            keep_partial: crate::units::Time::days(30).duration(),
        }
    }
}

/// The programs muman runs: `[tools.ffmpeg]`, `[tools.ffprobe]`,
/// `[tools."yt-dlp"]` and `[tools.aria2c]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "WrittenTools")]
pub struct Tools {
    pub ffmpeg: Program,
    pub ffprobe: Program,
    pub yt_dlp: Program,
    pub aria2c: Program,
}

/// One program muman runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// A name, looked for beside muman and then on `PATH`, or a path:
    /// absolute, `~/…` or relative to the home. Then its arguments.
    pub command: Vec<String>,
    /// Refuse a release older than the oldest muman runs.
    pub version_check: bool,
}

impl Program {
    fn called(name: &str) -> Self {
        Self {
            command: vec![name.into()],
            version_check: true,
        }
    }

    /// The program as [`crate::tools::find`] looks for it, a path in it
    /// made whole against `home`.
    #[must_use]
    pub fn named(&self, home: &Path) -> crate::tools::Named {
        let mut command: Vec<std::ffi::OsString> =
            self.command.iter().map(std::ffi::OsString::from).collect();
        if let Some(first) = self.command.first()
            && !crate::tools::is_name(first.as_ref())
            && let Some(path) = whole(first, home)
        {
            command[0] = path.into_os_string();
        }
        crate::tools::Named {
            command,
            version_check: self.version_check,
        }
    }
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            ffmpeg: Program::called("ffmpeg"),
            ffprobe: Program::called("ffprobe"),
            yt_dlp: Program::called("yt-dlp"),
            aria2c: Program::called("aria2c"),
        }
    }
}

impl Tools {
    /// Each of [`crate::tools::TOOLS`] as named here, in that order.
    #[must_use]
    pub fn named(&self, home: &Path) -> [crate::tools::Named; crate::tools::TOOLS.len()] {
        crate::tools::TOOLS.each_ref().map(|tool| {
            match tool.name {
                "ffmpeg" => &self.ffmpeg,
                "ffprobe" => &self.ffprobe,
                "aria2c" => &self.aria2c,
                _ => &self.yt_dlp,
            }
            .named(home)
        })
    }
}

/// `[tools]` as written, where a program or a key left out takes its
/// default.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WrittenTools {
    ffmpeg: WrittenProgram,
    ffprobe: WrittenProgram,
    #[serde(rename = "yt-dlp")]
    yt_dlp: WrittenProgram,
    aria2c: WrittenProgram,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WrittenProgram {
    command: Option<Vec<String>>,
    version_check: Option<bool>,
}

impl WrittenProgram {
    fn or(self, default: Program) -> Program {
        Program {
            command: self.command.unwrap_or(default.command),
            version_check: self.version_check.unwrap_or(default.version_check),
        }
    }
}

impl From<WrittenTools> for Tools {
    fn from(w: WrittenTools) -> Self {
        let d = Self::default();
        Self {
            ffmpeg: w.ffmpeg.or(d.ffmpeg),
            ffprobe: w.ffprobe.or(d.ffprobe),
            yt_dlp: w.yt_dlp.or(d.yt_dlp),
            aria2c: w.aria2c.or(d.aria2c),
        }
    }
}

/// Which lyrics a song takes, by their language: see [`crate::resolve`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lyrics {
    /// The languages the listener reads, best first: a translation is
    /// taken only into one of them, and lyrics rank by it among those
    /// [`Lyrics::prefer`] ranks alike.
    pub read: Vec<Lang>,
    /// What lyrics a song takes, best first: lyrics rank by the first of
    /// these that tells them apart. A role left out is never taken; with
    /// neither `timed` nor `untimed`, timing decides nothing.
    pub prefer: Vec<Prefer>,
    /// Where the lyrics a song takes are written; none, nowhere.
    pub out: Vec<LyricsOut>,
}

impl Default for Lyrics {
    fn default() -> Self {
        Self {
            read: Lang::parse("en").into_iter().collect(),
            prefer: vec![
                Prefer::Translation,
                Prefer::Original,
                Prefer::Romanization,
                Prefer::Timed,
            ],
            out: vec![LyricsOut {
                to: To::File,
                suffix: String::new(),
                format: LyricsFormat::Lrc,
                when: When::Any,
                words: true,
            }],
        }
    }
}

impl Lyrics {
    /// Each `[[lyrics.out]]` as it may be: one tag at most, a tag with no
    /// suffix, each file's suffix a plain one ([`crate::outputs`]), and no
    /// two files of one name for one song's lyrics.
    fn check(&self) -> Result<()> {
        let tags = self.out.iter().filter(|o| o.to == To::Tag).count();
        if tags > 1 {
            bail!("[[lyrics.out]] names to = \"tag\" {tags} times; a song holds one lyrics tag");
        }
        for (n, out) in self.out.iter().enumerate() {
            match out.to {
                To::Tag if !out.suffix.is_empty() => {
                    bail!("[[lyrics.out]] to = \"tag\" takes no suffix; a tag has no name")
                }
                To::Tag if out.format.cues() => bail!(
                    "[[lyrics.out]] to = \"tag\" holds lrc or txt, not {}; a lyrics tag holds \
                     no cues",
                    out.format.extension()
                ),
                To::Tag => {}
                To::File if out.format.cues() && out.when == When::Untimed => bail!(
                    "[[lyrics.out]] format = \"{}\" takes no untimed lyrics; cues need times",
                    out.format.extension()
                ),
                To::File => {
                    crate::outputs::check_suffix(&out.suffix)
                        .map_err(|e| anyhow::anyhow!("[[lyrics.out]] suffix: {e}"))?;
                    let name = |o: &LyricsOut| {
                        format!("{}.{}", o.suffix, o.format.extension()).to_uppercase()
                    };
                    let twice = self.out[..n].iter().any(|o| {
                        o.to == To::File && name(o) == name(out) && o.when.meets(out.when)
                    });
                    if twice {
                        bail!(
                            "[[lyrics.out]] names the file `<song>{}` twice for one song's lyrics",
                            name(out)
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether lyrics of `role` are taken at all.
    #[must_use]
    pub fn takes(&self, role: Role) -> bool {
        self.prefer.iter().any(|p| p.role() == Some(role))
    }
}

/// One trait lyrics are ranked by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Prefer {
    /// In the language the song is sung in.
    Original,
    /// In that language's Latin letters, where it is written in others.
    Romanization,
    /// In a language the listener reads, other than the one sung; the
    /// song's own lyrics in a language read have this trait too.
    Translation,
    /// Their lines carry times.
    Timed,
    /// Their lines carry none.
    Untimed,
}

impl Prefer {
    /// The role it prefers, if it prefers one.
    #[must_use]
    pub fn role(self) -> Option<Role> {
        match self {
            Self::Original => Some(Role::Original),
            Self::Romanization => Some(Role::Romanization),
            Self::Translation => Some(Role::Translation),
            Self::Timed | Self::Untimed => None,
        }
    }
}

/// What `undo` can put back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct History {
    /// The changing runs kept.
    pub runs: usize,
    /// The most the kept runs' replaced files may take.
    pub max_size: Size,
}

impl Default for History {
    fn default() -> Self {
        Self {
            runs: 3,
            max_size: Size(2 << 30),
        }
    }
}

/// The settings tables as `new.toml` writes them: every default, each
/// with its comments.
pub(crate) fn defaults() -> DocumentMut {
    crate::manifest::NEW
        .parse()
        .unwrap_or_else(|_| DocumentMut::new())
}

/// Every setting `doc` lacks added at its current default, with the
/// comments `new.toml` gives it: a missing table whole, ahead of the
/// file's own tables, and a missing key at the end of its table. Then
/// `edition` written, raised to the current when no setting is
/// [`stale`]. Whether anything changed.
pub fn fill(doc: &mut DocumentMut, editions: &Editions) -> bool {
    let defaults = defaults();
    let mut changed = crate::migrate::rename_all(doc, editions).is_ok_and(|r| !r.is_empty());
    for name in WRITTEN {
        let Some(default) = defaults.get(name).and_then(Item::as_table) else {
            continue;
        };
        match doc.get_mut(name) {
            None => {
                let mut table = default.clone();
                ahead(&mut table);
                doc.insert(name, Item::Table(table));
                changed = true;
            }
            Some(item) => changed |= fill_item(item, default),
        }
    }
    let file = edition_of(doc);
    let edition = if stale(doc, editions).is_empty() {
        file.max(editions.current)
    } else {
        file
    };
    if doc.get("edition").and_then(Item::as_integer) != Some(edition) {
        doc["edition"] = value(edition);
        changed = true;
    }
    changed
}

/// The keys of `default` that `item`, a table, lacks, added.
fn fill_item(item: &mut Item, default: &Table) -> bool {
    let mut changed = false;
    if let Some(table) = item.as_table_mut() {
        for (key, want) in default {
            match (table.get_mut(key), want) {
                (Some(have @ Item::Table(_)), Item::Table(want)) => {
                    changed |= fill_item(have, want);
                }
                (Some(_), _) => {}
                (None, want) => {
                    let mut want = want.clone();
                    if let Some(t) = want.as_table_mut() {
                        ahead(t);
                    }
                    if let Some(tables) = want.as_array_of_tables_mut() {
                        tables.iter_mut().for_each(ahead);
                    }
                    if let Some(key) = default.key(key) {
                        let key = without_hints_held(key, table);
                        table.insert_formatted(&key, want);
                    }
                    changed = true;
                }
            }
        }
    } else if let Some(table) = item.as_inline_table_mut() {
        for (key, want) in default {
            if !table.contains_key(key)
                && let Ok(want) = want.clone().into_value()
            {
                table.insert(key, want);
                changed = true;
            }
        }
    }
    changed
}

/// `key` from `new.toml`, with the comments above it, for `table` to
/// take: a commented-out setting ahead of it in `new.toml`, as
/// `# min_bitrate = "64 kb/s"` ahead of `layouts`, comes along in its
/// comments, so each block of lines ending in such a hint is dropped
/// where `table` sets that setting or hints at it already, by its name
/// or an older one.
fn without_hints_held(key: &toml_edit::Key, table: &Table) -> toml_edit::Key {
    let mut key = key.clone();
    let Some(prefix) = key.leaf_decor().prefix().and_then(|p| p.as_str()) else {
        return key;
    };
    let hinted: Vec<String> = table
        .iter()
        .filter_map(|(k, _)| table.key(k)?.leaf_decor().prefix()?.as_str())
        .flat_map(|p| p.lines().filter_map(hint).map(str::to_string))
        .collect();
    let has = |name: &str| table.contains_key(name) || hinted.iter().any(|h| h == name);
    let held = |name: &str| has(name) || crate::migrate::old_names(name).any(has);
    let mut kept = String::new();
    let mut block = String::new();
    for line in prefix.split_inclusive('\n') {
        block.push_str(line);
        if let Some(name) = hint(line) {
            if !held(name) {
                kept.push_str(&block);
            }
            block.clear();
        }
    }
    kept.push_str(&block);
    key.leaf_decor_mut().set_prefix(kept);
    key
}

/// The setting a commented-out line sets, as `max_path` of
/// `# max_path = 250`.
fn hint(line: &str) -> Option<&str> {
    let (name, _) = line.trim().strip_prefix('#')?.split_once('=')?;
    let name = name.trim();
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')).then_some(name)
}

/// `table` and the tables in it placed before every table a file read
/// holds, whose places count up from 0, in the order `new.toml` has them.
fn ahead(table: &mut Table) {
    table.set_position(table.position().map(|at| at - AHEAD));
    for (_, item) in table.iter_mut() {
        if let Some(t) = item.as_table_mut() {
            ahead(t);
        }
    }
}

/// How far before the file's own tables [`ahead`] places the defaults'.
const AHEAD: isize = 1 << 20;

/// The settings of the song list in `home`, if it has one.
pub fn of_home(home: &Path) -> Result<Option<Settings>> {
    let file = home.join(crate::dirs::MANIFEST);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let doc: DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML", file.display()))?;
    read(&doc).map(Some)
}

/// The settings `doc` sets, the defaults where it sets none.
pub fn read(doc: &DocumentMut) -> Result<Settings> {
    read_with(doc, &[])
}

/// One setting given apart from the song list, as `table.key = value`.
#[derive(Debug, Clone)]
pub struct Override {
    pub table: String,
    pub key: String,
    pub value: toml_edit::Value,
}

/// The settings `doc` sets with `overrides` set over them, checked as
/// the song list's own are. They are set by today's names, after the
/// file's older names are renamed, so one never collides with another.
pub fn read_with(doc: &DocumentMut, overrides: &[Override]) -> Result<Settings> {
    let mut only = DocumentMut::new();
    for table in TABLES.into_iter().chain(crate::migrate::moved_from()) {
        if let Some(item) = doc.get(table) {
            only.insert(table, item.clone());
        }
    }
    crate::migrate::rename_all(&mut only, &EDITIONS)?;
    if let Some(left) = crate::migrate::left_behind(&only) {
        bail!("{left}");
    }
    only.retain(|table, _| TABLES.contains(&table));
    // Its cleaning switches are the song list's own, read with `[tags.clean]`.
    if let Some(lyrics) = only.get_mut("lyrics").and_then(Item::as_table_like_mut) {
        lyrics.remove("clean");
    }
    for o in overrides {
        let table = only
            .entry(&o.table)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .with_context(|| format!("`{}` in the song list is no table", o.table))?;
        table.insert(&o.key, value(o.value.clone()));
    }
    let text = only.to_string();
    let settings: Settings = toml::from_str(&text)
        .map_err(|e| at_line(&e, &text, &doc.to_string()))
        .context("reading the settings")
        .map_err(|e| crate::migrate::explain(e, doc))?;
    if doc
        .get("edition")
        .is_some_and(|e| e.as_integer().is_none_or(|e| e < FIRST))
    {
        bail!("`edition` must be a whole number of {FIRST} or more");
    }
    for (name, program) in [
        ("ffmpeg", &settings.tools.ffmpeg),
        ("ffprobe", &settings.tools.ffprobe),
        ("yt-dlp", &settings.tools.yt_dlp),
    ] {
        if program.command.first().is_none_or(|p| p.trim().is_empty()) {
            bail!("[tools.{name}] command names no program");
        }
    }
    let prefer = &settings.lyrics.prefer;
    if let Some((_, p)) = prefer
        .iter()
        .enumerate()
        .find(|(n, p)| prefer[..*n].contains(p))
    {
        bail!(
            "[lyrics] prefer lists `{}` twice",
            format!("{p:?}").to_lowercase()
        );
    }
    if prefer.contains(&Prefer::Timed) && prefer.contains(&Prefer::Untimed) {
        bail!("[lyrics] prefer lists both timed and untimed; keep the one you prefer");
    }
    let library = &settings.library;
    if library.block_size.0 == 0 {
        bail!("[library] block_size must be above 0");
    }
    if library
        .template
        .split('/')
        .all(|part| part.trim().is_empty())
    {
        bail!("[library] template names no path for a song");
    }
    let least = [
        (
            "max_name_bytes",
            library.max_name_bytes,
            crate::naming::min_name_bytes(crate::outputs::tail(&settings)),
        ),
        (
            "max_folder_bytes",
            library.max_folder_bytes,
            crate::naming::MIN_FOLDER_BYTES,
        ),
    ];
    if let Some((key, _, min)) = least.iter().find(|(_, set, min)| set < min) {
        bail!("[library] {key} must be at least {min}, room for a name and what tells it apart");
    }
    if let Some((key, _, _)) = least.iter().find(|(_, set, _)| *set > MOST_NAME_BYTES) {
        bail!(
            "[library] {key} must be at most {MOST_NAME_BYTES}, the most a name may take on \
             any filesystem muman writes to"
        );
    }
    settings.audio.check()?;
    settings.loudness.check()?;
    settings.cover.pick.check()?;
    settings.cover.check()?;
    settings.lyrics.check()?;
    // aria2c counts its stop in whole seconds, and 0 as never stopping.
    if settings.torrent.stall < std::time::Duration::from_secs(1) {
        bail!("[torrent] stall must be at least 1 s; under it, a fetch would wait for ever");
    }
    Ok(settings)
}

/// `error`, which names a line of `text`, the settings alone, named by
/// its line in `whole`, the song list as written: under the same table
/// header, the first line that reads the same.
fn at_line(error: &toml::de::Error, text: &str, whole: &str) -> anyhow::Error {
    let message = plainly(error.message());
    let message = message.as_str();
    let Some(at) = error.span().map(|s| s.start.min(text.len())) else {
        return anyhow::anyhow!("{message}");
    };
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let line = text[start..end].trim();
    let header = text[..start]
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('['));
    let lines: Vec<&str> = whole.lines().collect();
    let from = header
        .and_then(|h| lines.iter().position(|l| l.trim() == h))
        .unwrap_or(0);
    match lines[from..].iter().position(|l| l.trim() == line) {
        Some(n) => anyhow::anyhow!("line {}, `{line}`: {message}", from + n + 1),
        None => anyhow::anyhow!("`{line}`: {message}"),
    }
}

/// A deserializer's `message` in the song list's words: `expected u32`
/// says nothing to whoever edits it, `expected a whole number` does.
fn plainly(message: &str) -> String {
    static RUST: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:u8|u16|u32|u64|usize|i8|i16|i32|i64|isize)\b|\bf(?:32|64)\b")
            .expect("valid")
    });
    let message = message.replace("unknown variant", "unknown value");
    let message = message.replace("invalid type: ", "");
    RUST.replace_all(&message, |c: &regex::Captures| {
        if c[0].starts_with('f') {
            "a number"
        } else {
            "a whole number"
        }
    })
    .into_owned()
}

/// The bytes a file or folder name may take on ext4, NTFS, APFS and
/// FAT alike.
const MOST_NAME_BYTES: usize = 255;

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys serde takes in the table at `path`, or in an entry of the
    /// list of tables there when `listed`: read from the error a key it
    /// takes nowhere meets, which names every one it does.
    fn taken(path: &[&str], listed: bool) -> Vec<String> {
        let text = match (path.split_last(), listed) {
            (None, _) => "__probe__ = 0\n".to_string(),
            (Some((last, head)), true) => {
                format!("[{}]\n{last} = [{{ __probe__ = 0 }}]\n", head.join("."))
            }
            (Some(_), false) => format!("[{}]\n__probe__ = 0\n", path.join(".")),
        };
        let e = toml::from_str::<Settings>(&text)
            .unwrap_err()
            .message()
            .to_string();
        let (_, expected) = e
            .split_once("expected")
            .unwrap_or_else(|| panic!("{path:?}: {e}"));
        expected
            .split('`')
            .skip(1)
            .step_by(2)
            .map(String::from)
            .collect()
    }

    /// Each key the table at `path` takes that `written`, new.toml's, has
    /// neither set nor hinted at as `# key = …`, as a setting with no
    /// default is; the tables in it walked too.
    fn unwritten(path: &[&str], listed: bool, written: &dyn toml_edit::TableLike) -> Vec<String> {
        let mut missing = Vec::new();
        for key in taken(path, listed) {
            let at: Vec<&str> = path.iter().copied().chain([key.as_str()]).collect();
            match written.get(&key) {
                Some(toml_edit::Item::Table(t)) => missing.extend(unwritten(&at, false, t)),
                Some(toml_edit::Item::ArrayOfTables(list)) => {
                    for t in list {
                        missing.extend(unwritten(&at, true, t));
                    }
                }
                Some(_) => {}
                None if crate::manifest::NEW.contains(&format!("# {key} = ")) => {}
                None => missing.push(at.join(".")),
            }
        }
        missing
    }

    #[test]
    fn every_setting_the_code_takes_is_written_in_new_toml() {
        assert_eq!(
            taken(&[], false),
            TABLES,
            "a settings table the song list's reading leaves out"
        );
        let doc: DocumentMut = crate::manifest::NEW.parse().unwrap();
        // `[sites]` is a map of sites, built from new.toml itself.
        let missing: Vec<String> = TABLES
            .iter()
            .filter(|t| **t != "sites")
            .flat_map(|t| unwritten(&[t], false, doc[t].as_table().unwrap()))
            .collect();
        assert_eq!(missing, Vec::<String>::new(), "set them in new.toml");
    }

    #[test]
    fn a_type_error_names_no_rust_type() {
        assert_eq!(
            plainly("invalid type: string \"two hundred\", expected usize"),
            "string \"two hundred\", expected a whole number"
        );
        assert_eq!(plainly("expected f64"), "expected a number");
        assert_eq!(
            plainly("unknown variant `wma`, expected one of `opus`, `flac`"),
            "unknown value `wma`, expected one of `opus`, `flac`"
        );
    }

    fn settings(text: &str) -> Result<Settings> {
        read(&text.parse().unwrap())
    }

    #[test]
    fn nothing_set_is_every_default() {
        assert_eq!(settings("version = 1\n").unwrap(), Settings::default());
    }

    #[test]
    fn a_setting_given_apart_is_set_over_the_file_s_and_checked_as_it_is() {
        let doc: DocumentMut = "version = 1\n[audio]\nlossy = \"aac\"\n".parse().unwrap();
        let given = |key: &str, value: &str| Override {
            table: "audio".into(),
            key: key.into(),
            value: value.into(),
        };
        let read = read_with(&doc, &[given("lossy", "mp3")]).unwrap();
        assert_eq!(read.audio.lossy, Codec::Mp3);
        let e = read_with(&doc, &[given("lossy", "flac")]).unwrap_err();
        assert!(format!("{e:#}").contains("is lossless"), "{e:#}");
        let e = read_with(&doc, &[given("lossey", "mp3")]).unwrap_err();
        assert!(format!("{e:#}").contains("lossey"), "{e:#}");
    }

    #[test]
    fn a_table_sets_only_what_it_names() {
        let s = settings(
            "[library]\ntemplate = \"{{ artist }}/{{ title }}\"\nlyrics = \"both\"\n\
             [audio]\nopus_kbps = 192\n",
        )
        .unwrap();
        assert_eq!(s.library.template, "{{ artist }}/{{ title }}");
        let to: Vec<To> = s.lyrics.out.iter().map(|o| o.to).collect();
        assert_eq!(to, [To::File, To::Tag], "read as [[lyrics.out]]");
        assert_eq!(s.library.untitled, "Untitled");
        assert_eq!(s.audio.opus_kbps, 192);
        assert_eq!(s.audio.opus_surround_kbps, 256);
    }

    #[test]
    fn each_place_lyrics_go_is_one_of_its_own() {
        let s = settings(
            "[[lyrics.out]]\nto = \"file\"\n[[lyrics.out]]\nto = \"file\"\nsuffix = \".en\"\n\
             [[lyrics.out]]\nto = \"tag\"\nwhen = \"timed\"\n",
        )
        .unwrap();
        assert_eq!(s.lyrics.out.len(), 3);
        assert_eq!(s.lyrics.out[1].suffix, ".en");
        assert_eq!(s.lyrics.out[2].when, When::Timed);
        let none = settings("[lyrics]\nout = []\n").unwrap();
        assert!(none.lyrics.out.is_empty(), "none at all");
        assert_eq!(Settings::default().lyrics.out.len(), 1, "a .lrc unset");
        for (bad, why) in [
            (
                "[[lyrics.out]]\nto = \"tag\"\n[[lyrics.out]]\nto = \"tag\"\n",
                "a song holds one lyrics tag",
            ),
            (
                "[[lyrics.out]]\nto = \"tag\"\nsuffix = \".en\"\n",
                "takes no suffix",
            ),
            (
                "[[lyrics.out]]\nto = \"file\"\nsuffix = \"/en\"\n",
                "must begin with a dot",
            ),
            (
                "[[lyrics.out]]\nto = \"file\"\n[[lyrics.out]]\nto = \"file\"\nwhen = \"timed\"\n",
                "twice",
            ),
            ("[[lyrics.out]]\nto = \"aside\"\n", "aside"),
        ] {
            let e = settings(bad).unwrap_err();
            assert!(format!("{e:#}").contains(why), "{bad}: {e:#}");
        }
        let apart = "[[lyrics.out]]\nto = \"file\"\nwhen = \"timed\"\n\
                     [[lyrics.out]]\nto = \"file\"\nwhen = \"untimed\"\n";
        assert!(settings(apart).is_ok(), "never one song's at once");
        let long = "[library]\nmax_name_bytes = 40\n[[lyrics.out]]\nto = \"file\"\nsuffix = \".zh-Hant\"\n";
        let e = settings(long).unwrap_err();
        assert!(
            format!("{e:#}").contains("max_name_bytes must be at least"),
            "{e:#}"
        );
    }

    #[test]
    fn each_place_a_cover_goes_is_one_of_its_own() {
        let s = settings(
            "[[cover.out]]\nto = \"tag\"\nformat = \"jpeg\"\nmax_side = 1200\n\
             [[cover.out]]\nto = \"file\"\nsuffix = \".small\"\nformat = \"webp\"\nquality = 70\n",
        )
        .unwrap();
        assert_eq!(s.cover.out[0].max_side, Some(1200));
        assert_eq!(s.cover.out[1].format, CoverFormat::Webp);
        assert_eq!(s.cover.out[1].quality, 70);
        let none = settings("[cover]\nout = []\n").unwrap();
        assert!(none.cover.out.is_empty(), "none at all");
        let folder = settings(
            "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n\
             mixed = \"majority\"\n[[cover.out]]\nto = \"file\"\n",
        )
        .unwrap();
        assert_eq!(folder.cover.of(Scope::Folder).count(), 1);
        assert_eq!(folder.cover.out[0].mixed, Mixed::Majority);
        assert_eq!(folder.cover.of(Scope::Song).count(), 1, "names apart");
        for (bad, why) in [
            (
                "[[cover.out]]\nto = \"tag\"\n[[cover.out]]\nto = \"tag\"\n",
                "one front cover",
            ),
            (
                "[[cover.out]]\nto = \"tag\"\nformat = \"webp\"\n",
                "keep, jpeg or png",
            ),
            (
                "[[cover.out]]\nto = \"file\"\n[[cover.out]]\nto = \"file\"\nformat = \"png\"\n",
                "twice",
            ),
            (
                "[[cover.out]]\nto = \"file\"\nquality = 0\n",
                "from 1 to 100",
            ),
            ("[[cover.out]]\nto = \"file\"\nmax_side = 0\n", "above 0"),
            (
                "[[cover.out]]\nto = \"tag\"\nscope = \"folder\"\nname = \"cover\"\n",
                "is a file",
            ),
            (
                "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\n",
                "must name a file",
            ),
            (
                "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"a/cover\"\n",
                "a file name cannot",
            ),
            (
                "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\nsuffix = \".x\"\n",
                "not a suffix",
            ),
            (
                "[[cover.out]]\nto = \"file\"\nname = \"cover\"\n",
                "a folder's file's",
            ),
            (
                "[[cover.out]]\nto = \"file\"\nmixed = \"majority\"\n",
                "mixed is for",
            ),
            (
                "[[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"cover\"\n\
                 [[cover.out]]\nto = \"file\"\nscope = \"folder\"\nname = \"Cover\"\nformat = \"png\"\n",
                "twice",
            ),
        ] {
            let e = settings(bad).unwrap_err();
            assert!(format!("{e:#}").contains(why), "{bad}: {e:#}");
        }
        let long = "[library]\nmax_name_bytes = 40\n\
                    [[cover.out]]\nto = \"file\"\nsuffix = \".thumbnail\"\nformat = \"webp\"\n";
        let e = settings(long).unwrap_err();
        assert!(
            format!("{e:#}").contains("max_name_bytes must be at least"),
            "{e:#}"
        );
    }

    #[test]
    fn loudness_takes_its_units_and_refuses_what_cannot_be() {
        let s = settings("[loudness]\nmode = \"header\"\ntarget = \"-23 LKFS\"\nceiling = \"-2 dBTP\"\nscope = \"track\"\n").unwrap();
        assert_eq!(s.loudness.mode, LoudnessMode::Header);
        assert_eq!(s.loudness.target, -2300);
        assert_eq!(s.loudness.ceiling, -200);
        assert_eq!(s.loudness.scope, GainScope::Track);
        assert!(settings("[loudness]\ntarget = -18\n").is_err());
        assert!(settings("[loudness]\ntarget = \"-80 LUFS\"\n").is_err());
        assert!(settings("[loudness]\nceiling = \"1 dB\"\n").is_err());
        assert!(settings("[loudness]\nmode = \"loud\"\n").is_err());
    }

    #[test]
    fn a_misspelled_key_is_named() {
        let e = settings("[audio]\nopus_kpbs = 192\n").unwrap_err();
        assert!(format!("{e:#}").contains("opus_kpbs"), "{e:#}");
        assert!(settings("[library]\nrestrict = \"dos\"\n").is_err());
    }

    #[test]
    fn a_setting_that_does_not_read_is_named_by_its_line() {
        let text = "# muman's song list\n# more words\n\n[history]\nruns = 3\n\n[library]\nblock_size = \"1e3 KiB\"\n";
        let e = settings(text).unwrap_err();
        assert!(
            format!("{e:#}").contains("line 8, `block_size = \"1e3 KiB\"`"),
            "{e:#}"
        );
    }

    #[test]
    fn a_stall_under_a_second_is_refused() {
        for short in ["0 s", "1 ms"] {
            let e = settings(&format!("[torrent]\nstall = \"{short}\"\n")).unwrap_err();
            assert!(
                format!("{e:#}").contains("stall must be at least 1 s"),
                "{e:#}"
            );
        }
        assert!(settings("[torrent]\nstall = \"1 s\"\n").is_ok());
    }

    #[test]
    fn a_name_too_short_to_tell_apart_or_an_empty_template_is_refused() {
        let e = settings("[library]\nmax_name_bytes = 8\n").unwrap_err();
        assert!(
            format!("{e:#}").contains("max_name_bytes must be at least 40"),
            "{e:#}"
        );
        assert!(settings("[library]\nmax_folder_bytes = 3\n").is_err());
        assert!(settings("[library]\nmax_name_bytes = 40\nmax_folder_bytes = 16\n").is_ok());
        assert!(settings("[library]\ntemplate = \" / / \"\n").is_err());
        let e = settings("[library]\nmax_folder_bytes = 1000\n").unwrap_err();
        assert!(format!("{e:#}").contains("at most 255"), "{e:#}");
        assert!(settings("[library]\nmax_name_bytes = 255\n").is_ok());
    }

    #[test]
    fn a_bitrate_is_above_0_and_opus_within_what_libopus_takes() {
        let e = settings("[audio]\nopus_kbps = 0\n").unwrap_err();
        assert!(
            format!("{e:#}").contains("opus_bitrate must be above 0"),
            "{e:#}"
        );
        assert!(settings("[audio]\nmin_kbps = 0\n").is_err());
        let s = settings("[audio]\nopus_kbps = 400\nopus_surround_kbps = 2000\n").unwrap();
        assert_eq!(s.audio.kbps(Codec::Opus, 1), Some(256));
        assert_eq!(s.audio.kbps(Codec::Opus, 2), Some(400));
        assert_eq!(s.audio.kbps(Codec::Opus, 6), Some(1536));
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
        let e = settings("[audio]\nlayouts = [\"5.1 side\"]\n").unwrap_err();
        assert!(
            format!("{e:#}").contains("5.1(side)"),
            "names the ones known: {e:#}"
        );
        let e = settings("[audio]\nlayouts = [\"stereo\"]\ndownmix = [\"5.1\"]\n").unwrap_err();
        assert!(format!("{e:#}").contains("does not take"), "{e:#}");
        assert!(settings("[audio]\ndownmix = []\n").is_err());
    }

    #[test]
    fn a_pick_table_sets_only_what_it_names() {
        let s = settings(
            "[audio.pick.stereo]\nweight = 0\n[audio.pick.clipping]\nenabled = false\n\
             [cover.pick.square]\nenabled = false\n",
        )
        .unwrap();
        assert_eq!(s.audio.pick.stereo.weight(), 0);
        assert_eq!(
            s.audio.pick.stereo,
            Stereo {
                weight: 0,
                ..Stereo::default()
            }
        );
        assert_eq!(s.audio.pick.clipping.weight(), 0);
        assert_eq!(s.audio.pick.bandwidth, Bandwidth::default());
        assert_eq!(s.cover.pick.square.weight(), 0);
        assert_eq!(s.cover.pick.resolution, Resolution::default());
        let e = settings("[audio.pick.bandwidth]\nstep = 0\n").unwrap_err();
        assert!(
            format!("{e:#}").contains("[audio.pick.bandwidth.step]"),
            "{e:#}"
        );
        let e = settings("[cover.pick.square]\ntolerance = -1.0\n").unwrap_err();
        assert!(
            format!("{e:#}").contains("[cover.pick.square.tolerance]"),
            "{e:#}"
        );
        assert!(settings("[audio.pick.clipping]\ncutoffs = [-1.0]\n").is_err());
        assert!(settings("[audio.pick.loudness]\nweight = 1\n").is_err());
        assert!(settings("[cover.pick.stereo]\nweight = 1\n").is_err());
    }

    #[test]
    fn the_old_quality_tables_read_in_their_new_places() {
        let s = settings("[quality.stereo]\nweight = 0\n[quality.square]\nweight = 3\n").unwrap();
        assert_eq!(s.audio.pick.stereo.weight, 0);
        assert_eq!(s.cover.pick.square.weight, 3);
        let e =
            settings("[quality.stereo]\nweight = 0\n[quality.loudness]\nweight = 1\n").unwrap_err();
        assert!(
            format!("{e:#}").contains(
                "[quality.loudness] is no setting: what [quality] held is under [audio.pick] \
                 and [cover.pick] now"
            ),
            "{e:#}"
        );
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
    fn lyrics_preferences_name_each_trait_once_and_one_timing() {
        let s = settings("[lyrics]\nprefer = [\"untimed\", \"romanization\"]\n").unwrap();
        assert_eq!(s.lyrics.prefer, [Prefer::Untimed, Prefer::Romanization]);
        assert!(s.lyrics.takes(Role::Romanization));
        assert!(!s.lyrics.takes(Role::Original));
        let twice = settings("[lyrics]\nprefer = [\"original\", \"original\"]\n").unwrap_err();
        assert!(
            format!("{twice:#}").contains("`original` twice"),
            "{twice:#}"
        );
        let both = settings("[lyrics]\nprefer = [\"timed\", \"untimed\"]\n").unwrap_err();
        assert!(
            format!("{both:#}").contains("both timed and untimed"),
            "{both:#}"
        );
        assert!(settings("[lyrics]\nprefer = [\"sung\"]\n").is_err());
    }

    fn doc(text: &str) -> DocumentMut {
        text.parse().unwrap()
    }

    #[test]
    fn every_setting_a_list_lacks_is_added_ahead_of_its_own_tables() {
        let mut d = doc(
            "version = 1\n\n[clean.tidy]\nspacing = true\n\n[audio]\n# Mine.\nopus_bitrate = \"192 kb/s\"\n\n\
             [quality.stereo]\nweight = 0\n",
        );
        assert!(fill(&mut d, &EDITIONS));
        let text = d.to_string();
        let mut again = doc(&text);
        assert!(!fill(&mut again, &EDITIONS), "{text}");
        assert_eq!(again.to_string(), text);
        assert!(
            text.starts_with(&format!("version = 1\nedition = {EDITION}\n")),
            "{text}"
        );
        assert!(
            text.find("[library]") < text.find("[tags.clean.tidy]"),
            "{text}"
        );
        assert!(text.contains("[audio.pick.stereo]\nweight = 0\n"), "{text}");
        assert!(!text.contains("[quality"), "{text}");
        assert!(
            text.contains("# Mine.\nopus_bitrate = \"192 kb/s\"\n"),
            "{text}"
        );
        let s = read(&again).unwrap();
        assert_eq!(s.audio.opus_kbps, 192);
        assert_eq!(s.audio.pick.stereo.weight, 0);
        assert_eq!(
            Settings {
                audio: Audio::default(),
                ..s
            },
            Settings::default()
        );
        for table in TABLES {
            assert!(again.contains_key(table), "{table} is not shown");
        }
        let defaults = defaults();
        let mut keys = Vec::new();
        for table in TABLES {
            crate::migrate::leaves(&[table], defaults[table].as_table().unwrap(), &mut keys);
        }
        for (path, _) in keys {
            assert!(
                crate::migrate::value_at(&again, &path).is_some(),
                "{path:?} is not set"
            );
        }
    }

    #[test]
    fn a_setting_added_brings_no_hint_at_one_the_table_has() {
        let filled = |text: &str| {
            let mut d = doc(text);
            fill(&mut d, &EDITIONS);
            d["audio"].as_table().unwrap().to_string()
        };
        let hints = |text: &str| text.matches("# min_bitrate = ").count();
        let set = filled("[audio]\nmin_bitrate = \"96 kb/s\"\n");
        assert_eq!(hints(&set), 0, "{set}");
        assert!(set.contains("layouts = "), "{set}");
        let old = filled("[audio]\n# min_kbps = 64\nlossy = \"opus\"\n");
        assert_eq!(hints(&old), 0, "{old}");
        assert!(old.contains("# Speaker layouts"), "{old}");
        let bare = filled("[audio]\nlossy = \"opus\"\n");
        assert_eq!(hints(&bare), 1, "{bare}");
        let library = |text: &str| {
            let mut d = doc(text);
            fill(&mut d, &EDITIONS);
            d["library"].as_table().unwrap().to_string()
        };
        let max_path = library("[library]\nmax_path = 250\n");
        assert!(!max_path.contains("# max_path = "), "{max_path}");
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

    #[test]
    fn a_tools_table_names_only_what_it_sets() {
        let s = settings(
            "[tools.yt-dlp]\ncommand = [\"python3\", \"-m\", \"yt_dlp\"]\n\
             [tools.ffprobe]\nversion_check = false\n",
        )
        .unwrap();
        assert_eq!(s.tools.yt_dlp.command, ["python3", "-m", "yt_dlp"]);
        assert!(s.tools.yt_dlp.version_check);
        assert_eq!(s.tools.ffprobe.command, ["ffprobe"]);
        assert!(!s.tools.ffprobe.version_check);
        assert_eq!(s.tools.ffmpeg, Tools::default().ffmpeg);
        let e = settings("[tools.ffmpeg]\ncommand = []\n").unwrap_err();
        assert!(format!("{e:#}").contains("[tools.ffmpeg] command"), "{e:#}");
        let e = settings("[tools.youtube-dl]\ncommand = [\"x\"]\n").unwrap_err();
        assert!(format!("{e:#}").contains("youtube-dl"), "{e:#}");
        let e = settings("[tools.ffmpeg]\npath = \"x\"\n").unwrap_err();
        assert!(format!("{e:#}").contains("path"), "{e:#}");
    }

    #[test]
    fn a_program_named_by_a_relative_path_is_under_the_home() {
        let home = Path::new("home");
        let program = |command: &[&str]| Program {
            command: command.iter().map(|w| (*w).to_string()).collect(),
            version_check: true,
        };
        let relative = Path::new("bin").join("yt-dlp");
        let named = program(&[relative.to_str().unwrap(), "-v"]).named(home);
        assert_eq!(
            named.command,
            [home.join(&relative).into_os_string(), "-v".into()]
        );
        let named = program(&["yt-dlp"]).named(home);
        assert_eq!(named.command, ["yt-dlp"]);
        let programs: Vec<_> = Tools::default()
            .named(home)
            .map(|n| n.command[0].clone())
            .into();
        assert_eq!(programs, ["ffmpeg", "ffprobe", "yt-dlp", "aria2c"]);
    }

    #[test]
    fn audio_in_a_layout_not_taken_is_mixed_into_the_first_that_fits() {
        let s = settings(
            "[audio]\nlayouts = [\"mono\", \"stereo\", \"5.1\"]\ndownmix = [\"5.1\", \"stereo\"]\n",
        )
        .unwrap();
        let mixed = |channels, layout| {
            let shape = Shape {
                channels,
                layout,
                sample_rate: 48_000,
                bits: 24,
                float: false,
            };
            s.audio.written(&shape).0.map(Layout::name)
        };
        assert_eq!(mixed(6, Some("5.1(side)")), None, "5.1 takes it");
        assert_eq!(mixed(12, Some("7.1.4")), Some("5.1"));
        assert_eq!(mixed(4, Some("4.0")), Some("stereo"), "never mixed up");
        assert_eq!(mixed(1, None), None);
        let mono = settings("[audio]\nlayouts = [\"stereo\"]\n").unwrap();
        let shape = Shape {
            channels: 1,
            layout: Some("mono"),
            sample_rate: 48_000,
            bits: 16,
            float: false,
        };
        let (mix, written) = mono.audio.written(&shape);
        assert_eq!(
            (mix, written.channels),
            (Some(Layout::STEREO), 2),
            "the fewest"
        );
    }
}
