//! What `songs.toml` sets beyond its songs: how the library is laid out
//! and named, which codecs are kept and what the rest is encoded to, how
//! loud songs are made, how sources are ranked, how yt-dlp fetches, and how much history `undo`
//! keeps.
//!
//! These live in the song list rather than in a per-user file so that
//! one list renders one library, the same on every machine. The tables
//! are `[library]`, `[audio]`, `[loudness]` ([`crate::loudness`]),
//! `[quality]`, `[ytdlp]`, `[history]` and `[sites]` ([`crate::sites`]).
//! A key these tables do not know is an error naming it, so a misspelled
//! setting is never silently ignored. Which machine runs what is no
//! setting: tools and folders come from flags and `MUMAN_*` variables.
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

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::codec::{Codec, Layout, Shape, Speakers};
use crate::migrate::FIRST;
pub use crate::migrate::{
    EDITION, EDITIONS, Editions, Stale, edition_of, stale, stale_warnings, update,
};
use crate::quality;
pub use crate::units::Size;
use crate::units::serde_as;

/// The tables read here; every other top-level key belongs to the song
/// list proper.
pub const TABLES: [&str; 7] = [
    "library", "audio", "loudness", "quality", "ytdlp", "history", "sites",
];

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub library: Library,
    pub audio: Audio,
    pub loudness: Loudness,
    pub quality: Quality,
    pub ytdlp: Ytdlp,
    pub history: History,
    pub sites: crate::sites::Sites,
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
            layouts: vec![Speakers::Any],
            downmix: vec![Layout::STEREO],
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
        Ok(())
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

/// How the best of a song's sources is picked. Each measure scores a
/// source in whole steps; a source's score is each measure's steps times
/// its weight, summed, and the lowest wins. A measure switched off, or
/// weighing 0, counts for nothing; one that could not be taken ranks a
/// source after every one it was taken on. Audio scored alike is told
/// apart as [`crate::resolve`] says; the last ties go to the source
/// listed first.
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
            ("purity.step", f64::from(self.purity.step_ms)),
            ("bandwidth.step", f64::from(self.bandwidth.step_hz)),
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
    /// How long an unfinished download is kept to resume.
    #[serde(deserialize_with = "serde_as::duration")]
    pub keep_partial: std::time::Duration,
}

impl Default for Ytdlp {
    fn default() -> Self {
        Self {
            format: "bv*+ba/b".into(),
            sub_langs: "all,-live_chat".into(),
            concurrent_fragments: 4,
            plugins: true,
            args: Vec::new(),
            keep_partial: crate::units::Time::days(14).duration(),
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
    for name in TABLES {
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
                    if let Some(key) = default.key(key) {
                        table.insert_formatted(key, want);
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
    crate::migrate::rename_all(&mut only, &EDITIONS)?;
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
            crate::naming::MIN_NAME_BYTES,
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
    settings.quality.check()?;
    Ok(settings)
}

/// `error`, which names a line of `text`, the settings alone, named by
/// its line in `whole`, the song list as written: under the same table
/// header, the first line that reads the same.
fn at_line(error: &toml::de::Error, text: &str, whole: &str) -> anyhow::Error {
    let message = error.message();
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

/// The bytes a file or folder name may take on ext4, NTFS, APFS and
/// FAT alike.
const MOST_NAME_BYTES: usize = 255;

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

    fn doc(text: &str) -> DocumentMut {
        text.parse().unwrap()
    }

    #[test]
    fn every_setting_a_list_lacks_is_added_ahead_of_its_own_tables() {
        let mut d = doc(
            "version = 1\n\n[defaults]\nlyrics = [\"en\"]\n\n[audio]\n# Mine.\nopus_bitrate = \"192 kb/s\"\n\n\
             [quality.stereo]\nweight = 0\n",
        );
        assert!(fill(&mut d, &EDITIONS));
        let text = d.to_string();
        let mut again = doc(&text);
        assert!(!fill(&mut again, &EDITIONS), "{text}");
        assert_eq!(again.to_string(), text);
        assert!(text.starts_with("version = 1\nedition = 2\n"), "{text}");
        assert!(text.find("[library]") < text.find("[defaults]"), "{text}");
        assert!(
            text.contains("# Mine.\nopus_bitrate = \"192 kb/s\"\n"),
            "{text}"
        );
        let s = read(&again).unwrap();
        assert_eq!(s.audio.opus_kbps, 192);
        assert_eq!(s.quality.stereo.weight, 0);
        assert_eq!(
            Settings {
                audio: Audio::default(),
                quality: Quality::default(),
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
