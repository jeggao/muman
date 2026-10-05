//! What a song is made of: for each aspect, the source whose measures
//! say it is best, compared lexicographically over bucketed scores so
//! noise never decides between near-equals. No rule names a kind of
//! source; a pin in the song list is the only override, and the order
//! of `sources` breaks the last tie.
//!
//! - Audio: least that is not the song (a video's intro or skit), then
//!   the widest bandwidth, real stereo, least clipping.
//! - Cover: square content, then effective resolution, then fewest
//!   block artifacts; borders a video frame adds are cropped off.
//! - Lyrics: timed, in a preferred language, from a source whose audio
//!   is the same recording as the chosen audio, covering most of it.
//! - Tags: each offer cleaned first; a structured field over one read
//!   off a title, plain over decorated, agreed on over alone; the
//!   release fields come together from one source so an album never
//!   splits.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::clean::{self, Albums, Settings};
use crate::facts::{CoverAt, Facts, LyricsAt};
use crate::lyrics;
use crate::manifest::{Album, LyricsPin, Song};
use crate::naming::{self, Naming};
use crate::quality::{ImageQuality, Rect};
use crate::settings::{Audio, LyricsPlacement};
use crate::source::SourceKey;
use crate::state::Aligned;
use crate::tags::{self, Field, Offer};

/// Bumped whenever the bytes a plan renders to change, so every song is
/// rendered again.
pub const RENDER_VERSION: u32 = 1;

/// Unmatched sound is judged in steps this long: a fade differs by less,
/// an intro or a skit by more.
const PURITY_STEP_MS: i64 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    /// Opus packets copied into Ogg.
    OpusCopy,
    /// Any other lossy codec, encoded to Opus.
    OpusEncode {
        channels: u32,
        /// The bitrate encoded at, in kbit/s; a new `[audio]` setting
        /// changes the plan, so the song is encoded again.
        kbps: u32,
    },
    FlacCopy,
    /// Any other lossless codec, encoded to FLAC.
    FlacEncode,
}

impl Format {
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::OpusCopy | Self::OpusEncode { .. } => "opus",
            Self::FlacCopy | Self::FlacEncode => "flac",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Why each aspect came from where it did, for `status`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Why {
    pub audio: String,
    pub cover: Option<String>,
    pub lyrics: Option<String>,
    /// Each written tag, in the order written.
    pub tags: Vec<TagWhy>,
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
    /// Facts of each source on disk.
    pub facts: &'a BTreeMap<SourceKey, Facts>,
    pub alignments: &'a [Aligned],
    pub lyrics: &'a [String],
    pub clean: &'a Settings,
    /// Every album the library holds, for cleaning.
    pub albums: &'a Albums,
    pub naming: &'a Naming,
    pub audio: &'a Audio,
    pub placement: LyricsPlacement,
}

impl Input<'_> {
    fn aligned(&self, a: &SourceKey, b: &SourceKey) -> Option<&Aligned> {
        let revs = (
            self.facts.get(a)?.rev.clone(),
            self.facts.get(b)?.rev.clone(),
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
        Some(a) if a.codec == "opus" => Format::OpusCopy,
        Some(a) if a.codec == "flac" => Format::FlacCopy,
        Some(a) if a.is_lossless() => Format::FlacEncode,
        Some(a) => Format::OpusEncode {
            channels: a.channels,
            kbps: if a.channels > 2 {
                input.audio.opus_surround_kbps
            } else {
                input.audio.opus_kbps
            },
        },
        None => bail!("no source of this song has audio"),
    };
    let cover = pick_cover(input);
    let lyrics = pick_lyrics(input, &audio_key);
    let (tags, tag_why) = resolve_tags(input);
    let get = |field: Field| {
        tags.iter()
            .find(|(k, _)| k == field.vorbis())
            .and_then(|(_, v)| v.first())
            .map(String::as_str)
    };
    let all = |field: Field| {
        tags.iter()
            .find(|(k, _)| k == field.vorbis())
            .map(|(_, v)| v.iter().map(String::as_str).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let id = audio_key.short();
    let stem = input.naming.stem(&naming::Tags {
        title: get(Field::Title),
        artists: all(Field::Artist),
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
            version: RENDER_VERSION,
            format,
            audio: AudioRef {
                key: audio_key,
                rev: facts.rev.clone(),
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
        },
    })
}

/// A ranking: smaller is better, compared in order.
type Rank = Vec<i64>;

/// An unmeasured score ranks after every measured one.
const UNKNOWN: i64 = i64::MAX / 2;

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
        let rank: Rank = vec![
            unmatched.map_or(UNKNOWN, |ms| ms / PURITY_STEP_MS),
            q.map_or(UNKNOWN, |q| -q.bandwidth_bucket()),
            q.map_or(UNKNOWN, |q| i64::from(!q.is_stereo())),
            q.map_or(UNKNOWN, |q| q.clipping_bucket()),
            i64::try_from(*n).unwrap_or(UNKNOWN),
        ];
        let why = match (unmatched, q) {
            (u, Some(q)) => format!(
                "{}, {:.1} kHz, {}, {:.2}% clipped",
                u.map_or_else(
                    || "no other recording to compare".to_string(),
                    |ms| format!(
                        "{}.{} s of sound beyond the song",
                        ms / 1000,
                        ms % 1000 / 100
                    )
                ),
                q.bandwidth_hz / 1000.0,
                if q.is_stereo() { "stereo" } else { "mono" },
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
            let rank: Rank = vec![
                q.map_or(UNKNOWN, |q| i64::from(!q.is_square())),
                q.map_or(UNKNOWN, |q| -q.resolution_bucket()),
                q.map_or(UNKNOWN, |q| q.blockiness_bucket()),
                order,
            ];
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
                    rev: facts.rev.clone(),
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
        .map_or(0.0, |d| d * 1000.0);
    let mut best: Option<(Rank, LyricsRef, String)> = None;
    for (n, key, facts) in input.present() {
        if pin.is_some_and(|p| p != key) {
            continue;
        }
        // The audio's own lyrics, and a file of lyrics with no audio to
        // compare, are taken as timed for it.
        let (confidence, offset) = if key == audio || facts.audio.is_none() {
            (1.0, 0)
        } else {
            match input.aligned(key, audio) {
                Some(a) if a.fits() || pin.is_some() => (a.score, a.offset_ms),
                None if pin.is_some() => (0.0, 0),
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
                    rev: facts.rev.clone(),
                    at: l.at.clone(),
                    shift_ms: offset - input.song.lyrics_offset_ms,
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

/// The best offer for one field among the song's sources, by their
/// cleaned values.
fn best_offer<'c, 'a>(
    candidates: &'c BTreeMap<Field, Vec<Candidate<'a>>>,
    field: Field,
    only: Option<&SourceKey>,
) -> Option<&'c Candidate<'a>> {
    let offers: Vec<&Candidate<'a>> = candidates
        .get(&field)?
        .iter()
        .filter(|c| only.is_none_or(|o| o == c.key))
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

/// An album artist taken from the artist follows the artist as finally
/// set, a hand-set one too.
fn follow_artist(written: &mut [(String, Slot)]) {
    let artist = written
        .iter()
        .find(|(k, s)| {
            k == Field::Artist.vorbis() && (s.from == "song.tags" || s.from == "album.tags")
        })
        .and_then(|(_, s)| Some((s.values.first()?.clone(), s.from.clone())));
    if let (Some((first, from)), Some((_, slot))) = (
        artist,
        written
            .iter_mut()
            .find(|(k, s)| k == Field::AlbumArtist.vorbis() && s.from.starts_with(FIRST_ARTIST)),
    ) {
        slot.values = vec![first];
        slot.from = format!("{FIRST_ARTIST}{from}");
        slot.cleaned.clear();
    }
}

/// Vorbis comments in the order written, each a name and its values.
type Comments = Vec<(String, Vec<String>)>;

/// The song's tags, and where each came from.
fn resolve_tags(input: &Input<'_>) -> (Comments, Vec<TagWhy>) {
    let candidates = candidates(input);
    let mut fields: BTreeMap<Field, Slot> = BTreeMap::new();
    for field in [Field::Title, Field::Artist, Field::Genre] {
        if let Some(c) = best_offer(&candidates, field, None) {
            fields.insert(field, Slot::of(c));
        }
    }
    // The release fields come whole from the source with the best album.
    let release = best_offer(&candidates, Field::Album, None).filter(|c| c.offer.structured);
    match release {
        Some(album) => {
            for field in Field::RELEASE {
                if let Some(c) = best_offer(&candidates, field, Some(album.key)) {
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
        None => {
            if let Some(title) = fields.get(&Field::Title) {
                let values = title.values.clone();
                fields.insert(
                    Field::Album,
                    Slot {
                        values,
                        from: SINGLE.to_string(),
                        cleaned: Vec::new(),
                    },
                );
            }
        }
    }
    if !fields.contains_key(&Field::Date)
        && let Some(c) = best_offer(&candidates, Field::Date, None)
    {
        fields.insert(Field::Date, Slot::of(c));
    }
    if !fields.contains_key(&Field::AlbumArtist)
        && let Some(artist) = fields.get(&Field::Artist)
    {
        let slot = Slot {
            values: artist.values.iter().take(1).cloned().collect(),
            from: format!("{FIRST_ARTIST}{}", artist.from),
            cleaned: artist.cleaned.clone(),
        };
        fields.insert(Field::AlbumArtist, slot);
    }
    if let Some(artist) = fields.get_mut(&Field::Artist) {
        artist.values = vec![artist.values.join(", ")];
    }

    let mut written: Vec<(String, Slot)> = Field::ALL
        .iter()
        .filter_map(|f| fields.remove(f).map(|s| (f.vorbis().to_string(), s)))
        .collect();
    set_by_hand(input, &mut written);
    follow_artist(&mut written);
    // A single is named for its title as finally set, a hand-set one too.
    let title = written
        .iter()
        .find(|(k, _)| k == Field::Title.vorbis())
        .map(|(_, s)| s.values.clone());
    if let (Some(title), Some((_, album))) = (
        title,
        written
            .iter_mut()
            .find(|(k, s)| k == Field::Album.vorbis() && s.from == SINGLE),
    ) {
        album.values = title;
    }
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
