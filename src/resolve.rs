//! What a song is made of: for each aspect, the source whose measures
//! say it is best. Audio and covers are scored by the song list's
//! `[quality]`: each measure in whole steps, so noise never decides
//! between near-equals, times its weight, summed; the lowest score wins.
//! No rule names a kind of source; a pin in the song list is the only
//! override, and the order of `sources` breaks the last tie. A source
//! with a measure that could not be taken ranks after every source with
//! fewer such.
//!
//! - Audio: least that is not the song (a video's intro or skit), then
//!   the widest bandwidth, real stereo, least clipping; scored alike,
//!   lossless, then the most bits used, then the lowest rate.
//! - Cover: square content, then effective resolution, then fewest
//!   block artifacts; borders a video frame adds are cropped off.
//! - Lyrics: timed, in a preferred language, from a source whose audio
//!   is the same recording as the chosen audio, covering most of it.
//! - Tags: each offer cleaned first; a structured field over one read
//!   off a title, plain over decorated, agreed on over alone; the
//!   release fields come together from one source so an album never
//!   splits. Each field's [`tags::Scope`] says which it is. A
//!   recording's ISRC and MusicBrainz IDs come only from a source that
//!   titles it as the song is titled, so a record of another recording,
//!   its names outvoted, lends the song none.
//!
//! The default weights rank as the order above does, each measure first
//! by a margin wider than everything after it can make up: a step of
//! purity outweighs any bandwidth up to 500 kHz, a step of bandwidth all
//! of stereo and clipping. Lowering a weight lets the measures after it
//! trade against it.
//!
//! The output format follows the winning audio's codec, not its measures:
//! a codec `[audio] codecs` lists is copied, and so is one already in the
//! codec it would be encoded to; any other lossless codec is encoded to
//! `[audio] lossless` and any other lossy one to `[audio] lossy`, at that
//! codec's bitrate. Lossless audio `[audio] lossless` cannot keep whole
//! is encoded to FLAC if FLAC can, and to WavPack otherwise; a codec that
//! holds it only adapted, as Opus relabelling side speakers, carries the
//! [`Adapt`] in its format. A file cut off is encoded whatever its codec, so the
//! song ends cleanly where its source breaks off. See [`crate::codec`].
//! This is the song at its best;
//! under `[library] max_size`, [`crate::limit`] may lower it once every
//! song is resolved. Plans stored before other codecs than Opus and FLAC
//! were written read as the [`Format`] they mean today.
//!
//! Lyrics from a source other than the chosen audio need that source's
//! audio to be the same recording, and are moved by the offset measured.
//! Lyrics with no audio of their own that state the length they are
//! timed to, as LRCLIB's do, are as sure as that length agrees with the
//! chosen audio's, but never as sure as a measured comparison, so a
//! person's subtitles of the same recording keep winning while LRCLIB's
//! timed lines win over untimed or missing ones. A `.lrc` that states no
//! length is taken as timed for the song.
//!
//! The release fields, album, album artist, track, disc, date, totals,
//! country and release IDs, come from the one source whose album ranks
//! best, so an album's tracks never split across folders and a release's
//! MusicBrainz IDs never mix with another's; with none, the song is a
//! single named for its final title, without release fields but a date.
//! The album artist is one a source names, else the first credited
//! artist as finally set, a hand-set one included, never the joined
//! `ARTIST`.
//!
//! A plan is stored whole and compared whole, which is how a song renders
//! again exactly when its sources' revisions, the picks, the tags or the
//! renderer change; [`render_version`] rises for a codec when the bytes a
//! plan in it renders to change, so only songs in that codec are written
//! again.

use crate::units;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::clean::{self, Albums, Settings};
use crate::codec::{Adapt, Codec, Layout};
use crate::facts::{AudioFacts, CoverAt, Facts, LyricsAt};
use crate::lyrics;
use crate::manifest::{Album, LyricsPin, Song};
use crate::naming::{self, Naming};
use crate::quality::{ImageQuality, Rect};
use crate::settings::{Audio, LyricsPlacement, Quality};
use crate::source::SourceKey;
use crate::state::Aligned;
use crate::tags::{self, Field, Offer, Scope};

/// The renderer's version for plans in `codec`, raised whenever the bytes
/// such a plan renders to change, so the songs in it are rendered again.
#[must_use]
pub fn render_version(codec: Codec) -> u32 {
    match codec {
        // Opus ended 312 samples late, its timestamps shifted by its delay.
        Codec::Opus => 4,
        _ => 3,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredFormat")]
pub enum Format {
    /// The source's packets copied into the codec's container.
    Copy { codec: Codec },
    /// The source decoded and encoded to `codec`.
    Encode {
        codec: Codec,
        /// The bitrate encoded at, in kbit/s, for a lossy codec; a new
        /// `[audio]` setting changes the plan, so the song is encoded again.
        kbps: Option<u32>,
        /// What is done to the audio so the codec holds it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        adapt: Option<Adapt>,
        /// The layout its speakers are mixed into, where `[audio] layouts`
        /// takes not theirs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mix: Option<Layout>,
    },
}

impl Format {
    #[must_use]
    pub fn codec(self) -> Codec {
        match self {
            Self::Copy { codec } | Self::Encode { codec, .. } => codec,
        }
    }

    #[must_use]
    pub fn extension(self) -> &'static str {
        self.codec().extension()
    }

    #[must_use]
    pub fn is_encoded(self) -> bool {
        matches!(self, Self::Encode { .. })
    }

    /// The format in words: `Opus 96 kbit/s`, `FLAC, copied`,
    /// `FLAC, mixed into 5.1`.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::Copy { codec } => format!("{}, copied", codec.label()),
            Self::Encode {
                codec, kbps, mix, ..
            } => {
                let rate = kbps.map(|k| format!(" {k} kbit/s")).unwrap_or_default();
                let mixed = mix.map(|m| format!(", mixed into {m}")).unwrap_or_default();
                format!("{}{rate}{mixed}", codec.label())
            }
        }
    }

    /// The layout its speakers are mixed into, if they are.
    #[must_use]
    pub fn mix(self) -> Option<Layout> {
        match self {
            Self::Encode { mix, .. } => mix,
            Self::Copy { .. } => None,
        }
    }

    /// The format `audio` is written in under `settings`: copied where its
    /// codec may be and its layout is taken as it is.
    #[must_use]
    pub fn of(audio: &AudioFacts, settings: &Audio) -> Self {
        let encoded = Self::encoded(audio, settings);
        match Codec::probed(&audio.codec) {
            Some(codec)
                if encoded.mix().is_none()
                    && (codec == encoded.codec() || settings.codecs.contains(&codec)) =>
            {
                Self::Copy { codec }
            }
            _ => encoded,
        }
    }

    /// `audio` encoded under `settings`, whatever its codec: mixed into
    /// another layout where `[audio] layouts` takes not its own, then to
    /// `[audio] lossy` or `lossless`, or where a lossless codec there
    /// cannot keep every channel and sample, to FLAC or else WavPack.
    #[must_use]
    pub fn encoded(audio: &AudioFacts, settings: &Audio) -> Self {
        let (mix, shape) = settings.written(&audio.shape());
        let wanted = if audio.is_lossless() {
            settings.lossless
        } else {
            settings.lossy
        };
        let fallbacks: &[Codec] = if wanted.is_lossless() {
            &[Codec::Flac, Codec::WavPack]
        } else {
            &[]
        };
        let (codec, adapt) = std::iter::once(wanted)
            .chain(fallbacks.iter().copied())
            .find_map(|codec| codec.adapt(&shape).map(|adapt| (codec, adapt)))
            .unwrap_or((Codec::WavPack, None));
        Self::Encode {
            codec,
            kbps: settings.kbps(codec, shape.channels),
            adapt,
            mix,
        }
    }
}

/// A [`Format`] as `state.json` holds it, in today's form or as plans
/// stored before other codecs than Opus and FLAC were written.
#[derive(Deserialize)]
enum StoredFormat {
    Copy {
        codec: Codec,
    },
    Encode {
        codec: Codec,
        kbps: Option<u32>,
        #[serde(default)]
        adapt: Option<Adapt>,
        #[serde(default)]
        mix: Option<Layout>,
    },
    OpusCopy,
    OpusEncode {
        kbps: u32,
    },
    FlacCopy,
    FlacEncode,
}

impl From<StoredFormat> for Format {
    fn from(stored: StoredFormat) -> Self {
        match stored {
            StoredFormat::Copy { codec } => Self::Copy { codec },
            StoredFormat::Encode {
                codec,
                kbps,
                adapt,
                mix,
            } => Self::Encode {
                codec,
                kbps,
                adapt,
                mix,
            },
            StoredFormat::OpusCopy => Self::Copy { codec: Codec::Opus },
            StoredFormat::OpusEncode { kbps } => Self::Encode {
                codec: Codec::Opus,
                kbps: Some(kbps),
                adapt: None,
                mix: None,
            },
            StoredFormat::FlacCopy => Self::Copy { codec: Codec::Flac },
            StoredFormat::FlacEncode => Self::Encode {
                codec: Codec::Flac,
                kbps: None,
                adapt: None,
                mix: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AudioRef {
    pub key: SourceKey,
    pub rev: String,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverRef {
    pub key: SourceKey,
    pub rev: String,
    pub at: CoverAt,
    pub mimetype: String,
    /// The content inside the borders, when there are any.
    pub crop: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsRef {
    pub key: SourceKey,
    pub rev: String,
    pub at: LyricsAt,
    /// How much earlier the lines are moved.
    pub shift_ms: i64,
    /// How much longer the lyrics' source plays the recording, in parts per
    /// million; the lines' times, once moved, are shortened by it. The hand
    /// correction in the shift is shortened too, by a millisecond or two.
    #[serde(default)]
    pub stretch_ppm: i64,
    pub placement: LyricsPlacement,
}

/// Everything a library file is made from. Stored beside each output and
/// compared whole, so a song is rendered again exactly when this changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub format: Format,
    pub audio: AudioRef,
    pub cover: Option<CoverRef>,
    pub lyrics: Option<LyricsRef>,
    /// Vorbis comments, in the order written.
    pub tags: Vec<(String, Vec<String>)>,
}

impl Plan {
    /// This plan written in `format`, by the renderer's version for it.
    #[must_use]
    pub fn with_format(&self, format: Format) -> Self {
        Self {
            version: render_version(format.codec()),
            format,
            ..self.clone()
        }
    }

    /// This plan with each part of `key` it takes named as `to` names it,
    /// where it is named as `from` does: facts of one source's same files,
    /// made at two times.
    #[must_use]
    pub fn renamed(&self, key: &SourceKey, from: &Facts, to: &Facts) -> Self {
        let mut plan = self.clone();
        if plan.audio.key == *key && plan.audio.rev == from.audio_rev() {
            plan.audio.rev = to.audio_rev();
        }
        if let Some(c) = plan.cover.as_mut().filter(|c| c.key == *key) {
            let was = from.covers.iter().find(|f| f.at == c.at);
            let now = to.covers.iter().find(|f| f.at == c.at);
            if let (Some(was), Some(now)) = (was, now)
                && c.rev == from.cover_rev(was)
            {
                c.rev = to.cover_rev(now);
            }
        }
        if let Some(l) = plan.lyrics.as_mut().filter(|l| l.key == *key) {
            let was = from.lyrics.iter().find(|f| f.at == l.at);
            let now = to.lyrics.iter().find(|f| f.at == l.at);
            if let (Some(was), Some(now)) = (was, now)
                && l.rev == from.lyrics_rev(was)
            {
                l.rev = to.lyrics_rev(now);
            }
        }
        plan
    }

    /// This plan with each source key `before` names another way named
    /// so.
    #[must_use]
    pub fn respelled(&self, before: &dyn Fn(&SourceKey) -> Option<SourceKey>) -> Self {
        let mut plan = self.clone();
        let name = |key: &mut SourceKey| {
            if let Some(old) = before(key) {
                *key = old;
            }
        };
        name(&mut plan.audio.key);
        if let Some(c) = plan.cover.as_mut() {
            name(&mut c.key);
        }
        if let Some(l) = plan.lyrics.as_mut() {
            name(&mut l.key);
        }
        plan
    }
}

/// Why each aspect came from where it did, for `status`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Why {
    pub audio: String,
    pub cover: Option<String>,
    pub lyrics: Option<String>,
    /// Each written tag, in the order written.
    pub tags: Vec<TagWhy>,
    /// Why the format is lower than the best, to fit `[library] max_size`.
    pub fit: Option<String>,
}

/// Where a written tag's value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagWhy {
    pub key: String,
    pub from: String,
    /// The cleaning rules that changed it.
    pub cleaned: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub plan: Plan,
    /// The library path without its extension.
    pub stem: PathBuf,
    pub why: Why,
}

/// What resolving a song reads.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub song: &'a Song,
    pub album: Option<&'a Album>,
    /// Facts of each source measured.
    pub facts: &'a BTreeMap<SourceKey, Facts>,
    /// The sources whose files are on disk, the only ones a plan may
    /// take from; unset, every source measured is taken to be.
    pub on_disk: Option<&'a BTreeSet<SourceKey>>,
    pub alignments: &'a [Aligned],
    pub lyrics: &'a [String],
    pub clean: &'a Settings,
    /// Every album the library holds, for cleaning.
    pub albums: &'a Albums,
    pub naming: &'a Naming,
    pub audio: &'a Audio,
    pub quality: &'a Quality,
    pub placement: LyricsPlacement,
}

impl Input<'_> {
    fn aligned(&self, a: &SourceKey, b: &SourceKey) -> Option<&Aligned> {
        let revs = (
            self.facts.get(a)?.audio_rev(),
            self.facts.get(b)?.audio_rev(),
        );
        self.alignments
            .iter()
            .find(|x| &x.a == a && &x.b == b && x.revs == revs && x.method == crate::align::METHOD)
    }

    /// The song's sources on disk, in the song list's order.
    fn present(&self) -> impl Iterator<Item = (usize, &SourceKey, &Facts)> {
        self.song
            .sources
            .iter()
            .enumerate()
            .filter(|(_, k)| self.on_disk.is_none_or(|d| d.contains(*k)))
            .filter_map(|(n, k)| Some((n, k, self.facts.get(k)?)))
    }
}

/// The pairs of sources whose comparison resolving may ask for: every
/// ordered pair of the song's sources with audio.
#[must_use]
pub fn wanted_alignments(
    song: &Song,
    facts: &BTreeMap<SourceKey, Facts>,
) -> Vec<(SourceKey, SourceKey)> {
    let audible: Vec<&SourceKey> = song
        .sources
        .iter()
        .filter(|k| facts.get(*k).is_some_and(|f| f.audio.is_some()))
        .collect();
    let mut pairs = Vec::new();
    for a in &audible {
        for b in &audible {
            if a != b {
                pairs.push(((*a).clone(), (*b).clone()));
            }
        }
    }
    pairs
}

pub fn resolve(input: &Input<'_>) -> Result<Resolved> {
    let (audio_key, audio_why) = pick_audio(input)?;
    let facts = &input.facts[&audio_key];
    let audio = facts.audio.as_ref().map_or(0, |a| a.index);
    let format = match facts.audio.as_ref() {
        // A file cut off breaks off mid-frame; copied, so would the song.
        Some(a) if facts.cut_from.is_some() => Format::encoded(a, input.audio),
        Some(a) => Format::of(a, input.audio),
        None => bail!("no source of this song has audio"),
    };
    let cover = pick_cover(input);
    let lyrics = pick_lyrics(input, &audio_key);
    let (tags, tag_why, artists) = resolve_tags(input, &audio_key, format.mix().is_some());
    let get = |field: Field| {
        tags.iter()
            .find(|(k, _)| k == field.vorbis())
            .and_then(|(_, v)| v.first())
            .map(String::as_str)
    };
    let id = audio_key.short();
    let stem = input.naming.stem(&naming::Tags {
        title: get(Field::Title),
        artists: artists.iter().map(String::as_str).collect(),
        album: get(Field::Album),
        album_artist: get(Field::AlbumArtist),
        genre: get(Field::Genre),
        date: get(Field::Date),
        track: get(Field::Track),
        disc: get(Field::Disc),
        id: &id,
    })?;
    Ok(Resolved {
        plan: Plan {
            version: render_version(format.codec()),
            format,
            audio: AudioRef {
                key: audio_key,
                rev: facts.audio_rev(),
                index: audio,
            },
            cover: cover.as_ref().map(|(c, _)| c.clone()),
            lyrics: lyrics.as_ref().map(|(l, _)| l.clone()),
            tags,
        },
        stem,
        why: Why {
            audio: audio_why,
            cover: cover.map(|(_, w)| w),
            lyrics: lyrics.map(|(_, w)| w),
            tags: tag_why,
            fit: None,
        },
    })
}

/// A ranking: smaller is better, compared in order.
type Rank = Vec<i64>;

/// An unmeasured score ranks after every measured one.
const UNKNOWN: i64 = i64::MAX / 2;

/// A source's score over measures, each its weight and its steps, `None`
/// when it could not be taken: how many of the counted ones could not be,
/// then the weighted sum of the rest.
fn score(measures: &[(i64, Option<i64>)]) -> [i64; 2] {
    let counted = measures.iter().filter(|(weight, _)| *weight != 0);
    let unknown = counted.clone().filter(|(_, steps)| steps.is_none()).count();
    let sum = counted
        .filter_map(|(weight, steps)| Some(weight.saturating_mul((*steps)?)))
        .fold(0_i64, i64::saturating_add);
    [i64::try_from(unknown).unwrap_or(UNKNOWN), sum]
}

fn pick_audio(input: &Input<'_>) -> Result<(SourceKey, String)> {
    let audible: Vec<(usize, &SourceKey, &Facts)> = input
        .present()
        .filter(|(_, _, f)| f.audio.is_some())
        .collect();
    if let Some(pin) = &input.song.audio {
        if audible.iter().any(|(_, k, _)| *k == pin) {
            return Ok((pin.clone(), "pinned".to_string()));
        }
        bail!("the pinned audio, {pin}, has no audio on disk");
    }
    let ranked = audible.iter().map(|(n, key, facts)| {
        let unmatched = audible
            .iter()
            .filter(|(_, other, _)| other != key)
            .filter_map(|(_, other, _)| input.aligned(key, other).filter(|a| a.fits()))
            .map(Aligned::unmatched_ms)
            .min();
        let q = facts.audio.as_ref().and_then(|a| a.quality);
        let w = input.quality;
        let [unknown, sum] = score(&[
            (
                w.purity.weight(),
                unmatched.map(|ms| ms / i64::from(w.purity.step_ms)),
            ),
            (
                w.bandwidth.weight(),
                q.map(|q| -q.bandwidth_bucket(f64::from(w.bandwidth.step_hz))),
            ),
            (
                w.stereo.weight(),
                q.map(|q| i64::from(!q.is_stereo(w.stereo.incoherence))),
            ),
            (
                w.clipping.weight(),
                q.map(|q| q.clipping_bucket(&w.clipping.cutoffs)),
            ),
        ]);
        let rank: Rank = vec![
            unknown,
            sum,
            -tie_break(facts.audio.as_ref()),
            facts.audio.as_ref().map_or(0, |a| i64::from(a.sample_rate)),
            i64::try_from(*n).unwrap_or(UNKNOWN),
        ];
        let why = match (unmatched, q) {
            (u, Some(q)) => format!(
                "{}, {:.1} kHz, {}, {:.2}% clipped",
                u.map_or_else(
                    || "no other recording to compare".to_string(),
                    |ms| {
                        // A song's milliseconds are far below 2^52, held exactly.
                        #[allow(clippy::cast_precision_loss)]
                        let seconds = units::seconds_of_ms(ms as f64);
                        format!(
                            "{:.1} s of sound beyond the song",
                            (seconds * 10.0).floor() / 10.0
                        )
                    }
                ),
                units::khz_of_hz(q.bandwidth_hz),
                speakers(
                    facts.audio.as_ref(),
                    q.is_stereo(input.quality.stereo.incoherence)
                ),
                q.clipping * 100.0
            ),
            (_, None) => "not measured".to_string(),
        };
        (rank, (*key).clone(), why)
    });
    ranked
        .min_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, key, why)| (key, why))
        .ok_or_else(|| anyhow::anyhow!("no source of this song has audio on disk"))
}

/// What breaks a tie between sources the measures score alike, most
/// first: lossless audio, then the bits it uses. A rate past what its
/// bandwidth needs holds nothing more, so of two the lower wins next.
fn tie_break(audio: Option<&AudioFacts>) -> i64 {
    audio
        .filter(|a| a.is_lossless())
        .map_or(0, |a| 1 + i64::from(a.quality.map_or(0, |q| q.bits)))
}

/// The speakers audio plays on in words: its layout past two channels,
/// as `5.1(side)`, and otherwise whether its two are really stereo.
fn speakers(audio: Option<&AudioFacts>, stereo: bool) -> String {
    match audio.filter(|a| a.channels > 2) {
        Some(a) => a
            .layout
            .clone()
            .unwrap_or_else(|| format!("{} channels", a.channels)),
        None if stereo => "stereo".to_string(),
        None => "mono".to_string(),
    }
}

fn pick_cover(input: &Input<'_>) -> Option<(CoverRef, String)> {
    let mut order = 0_i64;
    let mut best: Option<(Rank, CoverRef, String)> = None;
    for (_, key, facts) in input.present() {
        if input.song.cover.as_ref().is_some_and(|pin| pin != key) {
            continue;
        }
        for cover in &facts.covers {
            order += 1;
            let q = cover.quality;
            let w = input.quality;
            let [unknown, sum] = score(&[
                (
                    w.square.weight(),
                    q.map(|q| i64::from(!q.is_square(w.square.tolerance))),
                ),
                (
                    w.resolution.weight(),
                    q.map(|q| -q.resolution_bucket(w.resolution.step)),
                ),
                (
                    w.blockiness.weight(),
                    q.map(|q| q.blockiness_bucket(w.blockiness.step)),
                ),
            ]);
            let rank: Rank = vec![unknown, sum, order];
            if best.as_ref().is_some_and(|(r, _, _)| *r <= rank) {
                continue;
            }
            let why = q.map_or_else(
                || "not measured".to_string(),
                |q| {
                    format!(
                        "{}×{}{}, detail of {} px, blockiness {:.2}",
                        q.content.width,
                        q.content.height,
                        if q.is_cropped() {
                            " inside borders"
                        } else {
                            ""
                        },
                        q.effective,
                        q.blockiness
                    )
                },
            );
            best = Some((
                rank,
                CoverRef {
                    key: key.clone(),
                    rev: facts.cover_rev(cover),
                    at: cover.at.clone(),
                    mimetype: cover.mimetype.clone(),
                    crop: q.filter(ImageQuality::is_cropped).map(|q| q.content),
                },
                why,
            ));
        }
    }
    best.map(|(_, c, w)| (c, w))
}

/// How far a stated length may be from the audio's before lyrics timed
/// to it are no use.
const STATED_SPAN_MS: f64 = 4000.0;
/// A stated length that agrees proves less than an audio comparison, so
/// it never ranks over a subtitle of the same recording.
const STATED_MAX: f64 = 0.9;

/// The confidence lyrics stating a length are the chosen audio's.
fn stated_confidence(stated_ms: i64, audio_ms: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let gap = (stated_ms as f64 - audio_ms).abs();
    STATED_MAX * (1.0 - gap / STATED_SPAN_MS).max(0.0)
}

fn pick_lyrics(input: &Input<'_>, audio: &SourceKey) -> Option<(LyricsRef, String)> {
    let pin = match &input.song.lyrics {
        Some(LyricsPin::None) => return None,
        Some(LyricsPin::From(k)) => Some(k),
        None => None,
    };
    let duration_ms = input
        .facts
        .get(audio)
        .and_then(|f| f.duration)
        .map_or(0.0, units::ms_of_seconds);
    let mut best: Option<(Rank, LyricsRef, String)> = None;
    for (n, key, facts) in input.present() {
        if pin.is_some_and(|p| p != key) {
            continue;
        }
        // The audio's own lyrics, and a file of lyrics with no audio to
        // compare, are taken as timed for it.
        let (confidence, offset, stretch_ppm) = if key == audio || facts.audio.is_none() {
            (1.0, 0, 0)
        } else {
            match input.aligned(key, audio) {
                Some(a) if a.fits() || pin.is_some() => (a.score, a.offset_ms, a.stretch_ppm),
                None if pin.is_some() => (0.0, 0, 0),
                _ => continue,
            }
        };
        for l in &facts.lyrics {
            let Some(preference) = lyrics::preference(input.lyrics, &l.language) else {
                continue;
            };
            let confidence = match l.stated_ms {
                Some(stated) if facts.audio.is_none() => {
                    let sure = stated_confidence(stated, duration_ms);
                    if sure <= 0.0 && pin.is_none() {
                        continue;
                    }
                    sure
                }
                _ => confidence,
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let coverage = l.timing.map_or(0, |t| {
                if duration_ms > 0.0 {
                    ((t.last_ms - t.first_ms) as f64 / duration_ms * 10.0).floor() as i64
                } else {
                    0
                }
            });
            #[allow(clippy::cast_possible_truncation)]
            let rank: Rank = vec![
                i64::from(l.timing.is_none()),
                i64::try_from(preference).unwrap_or(UNKNOWN),
                -((confidence * 20.0).floor() as i64),
                -coverage,
                i64::try_from(n).unwrap_or(UNKNOWN),
            ];
            if best.as_ref().is_some_and(|(r, _, _)| *r <= rank) {
                continue;
            }
            let why = format!(
                "{}, {} line(s), {} the chosen audio",
                if l.timing.is_some() {
                    "timed"
                } else {
                    "untimed"
                },
                l.timing.map_or(0, |t| t.lines),
                if key == audio {
                    "from".to_string()
                } else {
                    format!("{offset} ms from")
                }
            );
            best = Some((
                rank,
                LyricsRef {
                    key: key.clone(),
                    rev: facts.lyrics_rev(l),
                    at: l.at.clone(),
                    shift_ms: offset - input.song.lyrics_offset_ms,
                    stretch_ppm,
                    placement: input.placement,
                },
                why,
            ));
        }
    }
    best.map(|(_, l, w)| (l, w))
}

/// One source's offer for a field, cleaned.
struct Candidate<'a> {
    n: usize,
    key: &'a SourceKey,
    offer: Offer,
    cleaned: Vec<&'static str>,
    /// As the source offered it.
    raw: &'a Offer,
}

/// Every source's offer for every field, cleaned against the names the
/// song's sources and its hand-set tags credit.
fn candidates<'a>(input: &'a Input<'_>) -> BTreeMap<Field, Vec<Candidate<'a>>> {
    let credited =
        |name: &str| matches!(Field::named(name), Some(Field::Artist | Field::AlbumArtist));
    let hand_set = input
        .song
        .tags
        .iter()
        .chain(input.album.iter().flat_map(|a| &a.tags))
        .filter(|(k, _)| credited(k))
        .flat_map(|(_, v)| v);
    let offered = input
        .present()
        .flat_map(|(_, _, f)| [f.tags.get(&Field::Artist), f.tags.get(&Field::AlbumArtist)])
        .flatten()
        .flat_map(|o| &o.values);
    let names = clean::names(offered.chain(hand_set).map(String::as_str));
    let mut all: BTreeMap<Field, Vec<Candidate<'a>>> = BTreeMap::new();
    for (n, key, facts) in input.present() {
        let ctx = clean::Context {
            names: &names,
            albums: input.albums,
            offers: &facts.tags,
        };
        for (field, raw) in &facts.tags {
            let c = clean::clean(*field, raw, &ctx, input.clean);
            all.entry(*field).or_default().push(Candidate {
                n,
                key,
                offer: Offer {
                    values: c.values,
                    structured: raw.structured,
                },
                cleaned: c.rules,
                raw,
            });
        }
    }
    all
}

/// The best offer for one field among the song's sources `from` takes,
/// by their cleaned values.
fn best_offer<'c, 'a>(
    candidates: &'c BTreeMap<Field, Vec<Candidate<'a>>>,
    field: Field,
    from: &dyn Fn(&SourceKey) -> bool,
) -> Option<&'c Candidate<'a>> {
    let offers: Vec<&Candidate<'a>> = candidates
        .get(&field)?
        .iter()
        .filter(|c| from(c.key))
        .collect();
    let agreeing = |o: &Offer| {
        let mine = tags::normalized(&o.values);
        offers
            .iter()
            .filter(|c| tags::normalized(&c.offer.values) == mine)
            .count()
    };
    offers.iter().copied().min_by_key(|c| {
        (
            !c.offer.structured,
            c.offer
                .values
                .iter()
                .map(|v| tags::decorations(v))
                .sum::<usize>(),
            std::cmp::Reverse(agreeing(&c.offer)),
            c.n,
        )
    })
}

/// Where the album of a song on none comes from: its own title.
pub const SINGLE: &str = "a single, named for its title";

/// Where an album artist taken from the artist comes from, before the
/// artist's own source.
const FIRST_ARTIST: &str = "the first artist of ";

/// A tag as it will be written, and where it came from.
#[derive(Clone)]
struct Slot {
    values: Vec<String>,
    from: String,
    cleaned: Vec<&'static str>,
}

impl Slot {
    fn of(c: &Candidate<'_>) -> Self {
        Self {
            values: c.offer.values.clone(),
            from: c.key.to_string(),
            cleaned: c.cleaned.clone(),
        }
    }
}

/// What the tags of a song, from its sources and its song list merged,
/// lack and say all the same: an album artist, its first artist; and an
/// album, for a song on none, its title, as a single. Derived once, from
/// the tags as finally set, each at its field's place, so a hand-set
/// artist or title names the song as one a source offered would.
fn derive(written: &mut Vec<(String, Slot)>, artists: &[String]) {
    fn get(w: &[(String, Slot)], f: Field) -> Option<Slot> {
        w.iter()
            .find(|(k, _)| k == f.vorbis())
            .map(|(_, s)| s.clone())
    }
    if get(written, Field::AlbumArtist).is_none()
        && let (Some(artist), Some(first)) = (get(written, Field::Artist), artists.first())
    {
        let slot = Slot {
            values: vec![first.clone()],
            from: format!("{FIRST_ARTIST}{}", artist.from),
            cleaned: artist.cleaned,
        };
        insert_in_order(written, Field::AlbumArtist, slot);
    }
    if get(written, Field::Album).is_none()
        && let Some(title) = get(written, Field::Title)
    {
        let slot = Slot {
            values: title.values,
            from: SINGLE.to_string(),
            cleaned: Vec::new(),
        };
        insert_in_order(written, Field::Album, slot);
    }
}

/// `slot` put in `written` before the first known field after `field`.
fn insert_in_order(written: &mut Vec<(String, Slot)>, field: Field, slot: Slot) {
    let rank = |f: Field| Field::ALL.iter().position(|g| *g == f);
    let at = written
        .iter()
        .position(|(k, _)| Field::named(k).and_then(rank) > rank(field))
        .unwrap_or(written.len());
    written.insert(at, (field.vorbis().to_string(), slot));
}

/// Vorbis comments in the order written, each a name and its values.
type Comments = Vec<(String, Vec<String>)>;

/// The song's tags, why each is what it is, and its artists one by one,
/// as the path template takes them, though ARTIST is written joined.
/// A mix is as loud as no source measured, so `mixed` takes no loudness.
fn resolve_tags(
    input: &Input<'_>,
    audio: &SourceKey,
    mixed: bool,
) -> (Comments, Vec<TagWhy>, Vec<String>) {
    let candidates = candidates(input);
    let mut fields: BTreeMap<Field, Slot> = BTreeMap::new();
    let any = |_: &SourceKey| true;
    for field in Field::of(Scope::Recording) {
        if let Some(c) = best_offer(&candidates, field, &any) {
            fields.insert(field, Slot::of(c));
        }
    }
    let title = fields
        .get(&Field::Title)
        .map(|s| tags::normalized(&s.values));
    let named_so = |key: &SourceKey| {
        candidates
            .get(&Field::Title)
            .into_iter()
            .flatten()
            .filter(|c| c.key == key)
            .all(|c| Some(tags::normalized(&c.offer.values)) == title)
    };
    for field in Field::of(Scope::RecordingId) {
        if let Some(c) = best_offer(&candidates, field, &named_so) {
            fields.insert(field, Slot::of(c));
        }
    }
    // The release fields come whole from the source with the best album.
    let release = best_offer(&candidates, Field::Album, &any).filter(|c| c.offer.structured);
    if let Some(album) = release {
        for field in Field::of(Scope::Release) {
            if let Some(c) = best_offer(&candidates, field, &|k| k == album.key) {
                fields.insert(field, Slot::of(c));
            }
        }
        if !fields.contains_key(&Field::Disc)
            && album.cleaned.contains(&clean::DISC_IN_ALBUM)
            && let Some((_, disc)) = album.raw.values.first().and_then(|a| clean::disc_in(a))
        {
            fields.insert(
                Field::Disc,
                Slot {
                    values: vec![disc.to_string()],
                    from: album.key.to_string(),
                    cleaned: vec![clean::DISC_IN_ALBUM],
                },
            );
        }
    }
    let heard = |k: &SourceKey| !mixed && k == audio;
    let album_heard = |k: &SourceKey| heard(k) && release.is_some_and(|r| r.key == audio);
    for (scope, from) in [
        (Scope::Loudness, &heard as &dyn Fn(&SourceKey) -> bool),
        (Scope::AlbumLoudness, &album_heard),
    ] {
        for field in Field::of(scope) {
            if let Some(c) = best_offer(&candidates, field, from) {
                fields.insert(field, Slot::of(c));
            }
        }
    }
    if !fields.contains_key(&Field::Date)
        && let Some(c) = best_offer(&candidates, Field::Date, &any)
    {
        fields.insert(Field::Date, Slot::of(c));
    }
    let mut artists = Vec::new();
    if let Some(artist) = fields.get_mut(&Field::Artist) {
        artists.clone_from(&artist.values);
        artist.values = vec![artist.values.join(", ")];
    }

    let mut written: Vec<(String, Slot)> = Field::ALL
        .iter()
        .filter_map(|f| fields.remove(f).map(|s| (f.vorbis().to_string(), s)))
        .collect();
    set_by_hand(input, &mut written);
    // Set by hand, the artists are what the song list says, a list or one.
    if let Some((_, artist)) = written.iter().find(|(k, _)| k == Field::Artist.vorbis())
        && artist.values != [artists.join(", ")]
    {
        artists.clone_from(&artist.values);
    }
    derive(&mut written, &artists);
    let why = written
        .iter()
        .map(|(k, s)| TagWhy {
            key: k.clone(),
            from: s.from.clone(),
            cleaned: s.cleaned.clone(),
        })
        .collect();
    (
        written.into_iter().map(|(k, s)| (k, s.values)).collect(),
        why,
        artists,
    )
}

/// The album's tags and length, the song's place in it and the song's
/// own tags, set over what the sources offer, uncleaned.
fn set_by_hand(input: &Input<'_>, written: &mut Vec<(String, Slot)>) {
    let mut set = |key: String, values: Vec<String>, from: &str| {
        let slot = Slot {
            values,
            from: from.to_string(),
            cleaned: Vec::new(),
        };
        match written.iter_mut().find(|(k, _)| *k == key) {
            Some((_, s)) => *s = slot,
            None => written.push((key, slot)),
        }
    };
    if let Some(album) = input.album {
        for (name, values) in &album.tags {
            set(tags::vorbis_key(name), values.clone(), "album.tags");
        }
        if let Some(n) = album.tracks {
            set(
                "TRACKTOTAL".to_string(),
                vec![n.to_string()],
                "the album's length",
            );
        }
    }
    if let Some(track) = input.song.track {
        set(
            Field::Track.vorbis().to_string(),
            vec![track.to_string()],
            "its place in the album",
        );
    }
    for (name, values) in &input.song.tags {
        set(tags::vorbis_key(name), values.clone(), "song.tags");
    }
}

#[cfg(test)]
mod tests;
