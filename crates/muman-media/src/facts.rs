//! Everything a source offers a song, measured once per revision of its
//! files: its audio and how good it is, its pictures, its lyrics, its
//! tags and its fingerprint.
//!
//! A revision is the files' sizes and times, read without opening them.
//! Facts also keep a digest of the bytes they were measured on
//! ([`content_of`]), so files a copy or a touch gave a new revision but
//! that hold the same bytes keep their facts ([`Facts::moved_to`]), once
//! read through instead of measured again. Facts measured before muman
//! took that digest are measured again when their revision changes.
//!
//! The measuring run also lists the audio's packets, whose sizes sum to
//! what a copy takes, for `[library] max_size`; facts made before are
//! given theirs by [`audio_bytes`] when a limit asks.
//!
//! A media file costs at most three runs: ffprobe, one ffmpeg dumping
//! its attachments, and one ffmpeg writing every measured excerpt as a
//! separate output. That run decodes the whole audio once for its
//! fingerprint, and the same decode goes, as WAV on its stdout, to the
//! analyses [`crate::analysis`] runs over the whole of it. An analysis a
//! source's facts lack, or hold by another method, is caught up by one
//! run of its own, [`analyze`], measuring nothing else again; the Opus
//! header's gain, which ffprobe does not show, is read beside it.
//!
//! Each audio stream, picture and lyrics a source offers is also known
//! by a digest of its content, which plans name it by, so a source
//! fetched again or copied without its file times is the same source
//! when it holds the same content. A stream's digest is the SHA-256 of
//! its packets, copied, not decoded: two downloads of one file a minute
//! apart differed in 425 bytes of their container, where yt-dlp embeds
//! the time and its cookies, and in no packet. Packets survive a remux
//! too: Opus, AAC, FLAC and MP3 each hashed the same in their own
//! container and in Matroska, remuxed once or twice, and a FLAC with
//! new tags and a new cover hashed the same, where the decoded samples
//! of the Opus, AAC and MP3 differed by container, decoders being
//! required to agree only within a tolerance. An attachment, a sidecar
//! and a file of its own are hashed whole. The tags a source offers are
//! known by the fields of its info JSON they are read from, as
//! [`info::TAG_FIELDS`] lists them, the rest of it changing on every
//! fetch, or by a file's own tags, before any cleaning. Facts hashed by
//! another [`HASH_METHOD`] are hashed again by [`hash`], which decodes
//! nothing.
//!
//! A media file's own lyrics tag, as `USLT` or `LYRICS`, is lyrics of
//! that source ([`LyricsAt::Tag`]), read by `lofty` with its tags and
//! known by the digest of its text; facts made before muman read it
//! learn it when their tags are read again, by [`retag`].
//!
//! A MusicBrainz record's facts keep the length it says its recording
//! plays beside its tags, which the vetting of a song's sources holds
//! against the audio; records read before muman kept it are read again
//! by [`retag`], as [`tags_method`] names, which opens no media file.
//!
//! A picture of a file's own tags is known by its bytes: ffmpeg takes its
//! codec from the media type it is labelled with, so it shows a JPEG
//! labelled `image/png` as a PNG it cannot decode, and none labelled
//! `image/x-png` or `application/octet-stream`. Where ffprobe's pictures
//! are not the tags' one for one, each in the format its bytes are, they
//! are read as `lofty` reads them ([`picture::tag_pictures`]) and decoded
//! as their bytes say. A picture that does not decode is no cover, and
//! said when its source is measured ([`Unkept::Picture`]).
//!
//! Each picture is also known by how it looks, [`crate::picture`]'s
//! hashes of the gray pixels its quality is measured from, so a picture
//! given to `add` finds the songs whose covers it is. Facts taken before,
//! or by another `picture::METHOD`, have their pictures looked at again
//! by [`look`], which decodes the pictures alone, and only when a picture
//! is added.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::hash::{digest, digest_file};

use crate::analysis::{self, Analysis, Pass};
use crate::chapters::Chapter;
use crate::ffmpeg::{self, Input, Output, Piped};
use crate::fingerprint::{self, Print};
use crate::info::{self, VideoInfo};
use crate::integrity::{self, Damage, Found, Timeline};
use crate::lyrics::{self, Language, Timing};
use crate::picture::{self, Look};
use crate::probe::{self, Probed};
use crate::quality::{self, AudioQuality, ImageQuality};
use crate::runner::Runner;
use crate::store::{Kind, Located};
use crate::tags::{self, Offers};

/// The measures facts are made by; facts made by others are made again.
#[must_use]
pub fn method() -> String {
    measured_by(quality::METHOD)
}

/// [`method`] with the measures of `quality` in place of
/// `quality::METHOD`.
fn measured_by(quality: &str) -> String {
    format!("{quality}+{}+{}", fingerprint::METHOD, probe::METHOD)
}

/// [`method`] before bandwidth was read where the audio does not clip:
/// facts it made hold, unless their audio clips.
fn before_unclipped() -> String {
    measured_by(quality::BEFORE_UNCLIPPED)
}

/// [`method`] before bandwidth was read at the lowest wall: facts it made
/// hold, unless their audio is lossy, or as [`before_unclipped`]'s do not.
fn before_lowest() -> String {
    measured_by(quality::BEFORE_LOWEST)
}

/// [`method`] before bars running into square art stopped at the frame's
/// centred square: facts it made hold, unless a picture's content was
/// trimmed to no square, or as [`before_lowest`]'s do not.
fn before_square() -> String {
    measured_by(quality::BEFORE_SQUARE)
}

/// [`method`] before transparent pictures were laid over mid-gray: facts
/// it made hold, unless a picture had no detail, or as [`before_square`]'s
/// do not.
fn before_alpha() -> String {
    measured_by(quality::BEFORE_ALPHA)
}

/// [`method`] before excerpts that fell silent were taken again: facts it
/// made hold, unless they found no sound, or as [`before_alpha`]'s do not.
fn before_sound() -> String {
    measured_by(quality::BEFORE_SOUND)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    /// The revision of the files they were measured on.
    pub rev: String,
    /// What those files held: a digest of each one's bytes
    /// ([`content_of`]); none for facts measured before muman took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub method: String,
    /// The file's length, every stream's; [`Facts::length`] is its audio's.
    pub duration: Option<f64>,
    pub audio: Option<AudioFacts>,
    pub covers: Vec<CoverFacts>,
    pub lyrics: Vec<LyricsFacts>,
    pub tags: Offers,
    /// What a MusicBrainz record offers on each other edition of its
    /// album, in place of its release's fields ([`tags::editions`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub editions: Vec<Offers>,
    /// The [`tags_method`] the tags were read by.
    #[serde(default)]
    pub tags_method: String,
    /// The length a MusicBrainz record says its recording plays, in ms;
    /// none for any other source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_ms: Option<u64>,
    /// The artists a MusicBrainz record credits, by their own names and
    /// aliases; none for any other source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artists: Vec<crate::record::CreditedArtist>,
    /// A YouTube Music release, by its info: a track name, or a " - Topic"
    /// channel; read with the tags.
    #[serde(default)]
    pub release: bool,
    /// Its chapters, read with the tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<Chapter>,
    #[serde(default, deserialize_with = "fingerprint::read_stored")]
    pub print: Option<Print>,
    /// What the source holds that a library file cannot keep.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unkept: Vec<Unkept>,
    /// What a site served, for what yt-dlp fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub served: Option<Served>,
    /// The digest of the tags it offers as they are written in it, before
    /// muman reads them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags_digest: Option<String>,
    /// The [`HASH_METHOD`] each part was hashed by, or tried; empty for
    /// facts made before muman hashed sources.
    #[serde(default)]
    pub hash_method: String,
    /// The `picture::METHOD` its pictures' looks were taken by, or tried;
    /// empty for facts made before muman took them.
    #[serde(default)]
    pub picture_method: String,
}

/// What a source's digests are of; facts hashed otherwise are hashed
/// again, decoding nothing, as [`hash`] does.
pub const HASH_METHOD: &str = "hash/2";

/// What a MusicBrainz record's facts keep beside its tags, its length and
/// the artists it credits, and what its tags hold: since record/3, its
/// genres ([`crate::genres`]).
pub const RECORD_METHOD: &str = "record/3";

/// What a lyrics file's facts keep beside its lines: whether the letters
/// it sings are Latin alone.
pub const LYRICS_METHOD: &str = "lyrics/1";

/// How the tags of a source of `kind` are read: `tags::METHOD`, and for
/// a record [`RECORD_METHOD`] too, or for a lyrics file [`LYRICS_METHOD`],
/// so such a file alone is read again when only what its facts keep of it
/// changes, a few kilobytes of text and no audio.
#[must_use]
pub fn tags_method(kind: Kind) -> String {
    match kind {
        Kind::Tags => format!("{}+{RECORD_METHOD}", tags::METHOD),
        Kind::Lyrics => format!("{}+{LYRICS_METHOD}", tags::METHOD),
        Kind::Media | Kind::Image => tags::METHOD.to_string(),
    }
}

/// The format a site served a source in, by yt-dlp's info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Served {
    /// yt-dlp's format ID, as `251` or `399+251`.
    pub format: String,
    /// The size the site gave its audio format, where it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The page it was fetched from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Milliseconds the site said it plays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl Served {
    fn of(info: &VideoInfo) -> Option<Self> {
        Some(Self {
            format: info.format_id.clone()?,
            size: info.audio_format().and_then(|f| f.filesize),
            url: info.page().map(str::to_string),
            duration_ms: info
                .duration
                .filter(|s| s.is_finite() && *s > 0.0)
                .map(|s| (s * 1000.0).round() as u64),
        })
    }
}

impl Facts {
    /// Whether these still describe a source at revision `rev`.
    #[must_use]
    pub fn holds_for(&self, rev: &str) -> bool {
        self.rev_holds(rev) && self.method_holds()
    }

    /// Whether they were measured at revision `rev`. Facts recorded before
    /// revisions held change times hold while the sizes and modification
    /// times do, as they did then.
    fn rev_holds(&self, rev: &str) -> bool {
        self.rev == rev || self.rev == crate::store::without_change_times(rev)
    }

    /// Whether they were measured as muman measures now, or by an earlier
    /// method whose results stand for these.
    fn method_holds(&self) -> bool {
        let lossy = self.audio.as_ref().is_some_and(|a| !a.is_lossless());
        let clips = self.clips();
        self.method == method()
            || self.method == before_unclipped() && !clips
            || self.method == before_lowest() && !lossy && !clips
            || self.method == before_square() && !self.unsquare() && !lossy && !clips
            || self.method == before_alpha()
                && !self.detailless()
                && !self.oversized()
                && !self.unsquare()
                && !lossy
                && !clips
            || self.method == before_sound()
                && !self.soundless()
                && !self.detailless()
                && !self.oversized()
                && !self.unsquare()
                && !lossy
                && !clips
    }

    /// These facts at revision `rev`, for files whose revision changed
    /// but that hold `content`, the bytes they were measured on, as a
    /// copy or a touch leaves them: none unless they were measured as
    /// muman measures now and name every part by its digest, so no plan
    /// names one otherwise.
    #[must_use]
    pub fn moved_to(&self, rev: &str, content: Option<&str>) -> Option<Self> {
        if !self.method_holds() || self.content.is_none() || self.content.as_deref() != content {
            return None;
        }
        let moved = Self {
            rev: rev.to_string(),
            ..self.clone()
        };
        let named = moved.audio_rev() == self.audio_rev()
            && self
                .covers
                .iter()
                .all(|c| moved.cover_rev(c) == self.cover_rev(c))
            && self
                .lyrics
                .iter()
                .all(|l| moved.lyrics_rev(l) == self.lyrics_rev(l));
        named.then_some(moved)
    }

    /// Whether a picture measured is larger than [`quality::MOST_PIXELS`],
    /// which methods before [`quality::METHOD`] measured and it does not.
    fn oversized(&self) -> bool {
        self.covers.iter().any(|c| {
            c.quality
                .is_some_and(|q| u64::from(q.width) * u64::from(q.height) > quality::MOST_PIXELS)
        })
    }

    /// Whether a picture measured no detail, as one drawn in its alpha did
    /// before [`quality::flatten`].
    fn detailless(&self) -> bool {
        self.covers
            .iter()
            .any(|c| c.quality.is_some_and(|q| q.effective == 0))
    }

    /// Whether a picture's content was trimmed to no square, as bars that
    /// ran into its art's edge left it before [`quality::METHOD`].
    fn unsquare(&self) -> bool {
        self.covers.iter().any(|c| {
            c.quality
                .is_some_and(|q| q.is_cropped() && !q.is_square(quality::SQUARE_TOLERANCE))
        })
    }

    /// Whether its audio measured any clipping.
    fn clips(&self) -> bool {
        self.audio
            .as_ref()
            .and_then(|a| a.quality)
            .is_some_and(|q| q.clipping > 0.0)
    }

    /// Whether its audio measured no sound to judge.
    fn soundless(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.quality.is_none())
    }

    /// What a plan names its audio by: its digest, or, where it has
    /// none, the revision of its files.
    #[must_use]
    pub fn audio_rev(&self) -> String {
        self.audio
            .as_ref()
            .and_then(|a| a.digest.clone())
            .unwrap_or_else(|| self.rev.clone())
    }

    /// What a plan names `cover` by, as [`Facts::audio_rev`] its audio.
    #[must_use]
    pub fn cover_rev(&self, cover: &CoverFacts) -> String {
        cover.digest.clone().unwrap_or_else(|| self.rev.clone())
    }

    /// What a plan names `lyrics` by, as [`Facts::audio_rev`] its audio.
    #[must_use]
    pub fn lyrics_rev(&self, lyrics: &LyricsFacts) -> String {
        lyrics.digest.clone().unwrap_or_else(|| self.rev.clone())
    }

    /// Take `digests` and `served`, as [`hash`] read them.
    pub fn take(&mut self, digests: &Digests, served: Option<Served>) {
        if let Some(a) = &mut self.audio {
            a.digest.clone_from(&digests.audio);
        }
        for c in &mut self.covers {
            c.digest = digests.cover(&c.at);
        }
        for l in &mut self.lyrics {
            l.digest = digests.lyrics(&l.at);
        }
        self.tags_digest.clone_from(&digests.tags);
        self.served = served;
        self.hash_method = HASH_METHOD.to_string();
    }

    /// Whether each part was hashed as muman hashes them now.
    #[must_use]
    pub fn hashed(&self) -> bool {
        self.hash_method == HASH_METHOD
    }

    /// What the integrity measure found of its audio, by today's method.
    #[must_use]
    pub fn integrity(&self) -> Option<&integrity::Measure> {
        self.audio.as_ref()?.analysis.integrity()
    }

    /// What its audio lacks, as [`crate::integrity`] judges it: the one place a
    /// source is judged cut off or missing audio.
    #[must_use]
    pub fn damage(&self) -> Option<Damage> {
        let stated = self
            .served
            .as_ref()
            .and_then(|s| s.duration_ms)
            .map(|ms| ms as f64 / 1000.0);
        self.integrity()?.damage_against(stated)
    }

    /// Seconds its audio plays: to where a cut-off file's packets end,
    /// else its audio stream's own length, else the file's, which counts
    /// its other streams too, else its packets' span. Where the file
    /// states no length ([`integrity::Measure::guessed`]), ffprobe's are
    /// guesses, and its packets' span comes before the file's.
    #[must_use]
    pub fn length(&self) -> Option<f64> {
        let measure = self.integrity();
        if let Some(Damage::Cut { ends, .. }) = self.damage() {
            return Some(ends);
        }
        let span = measure.and_then(|m| m.span).map(|(_, end)| end);
        let file = self
            .duration
            .filter(|_| !measure.is_some_and(|m| m.guessed));
        measure.and_then(|m| m.said).or(file).or(span)
    }

    /// The analyses of `wanted` the audio lacks; none for no audio.
    #[must_use]
    pub fn unanalyzed(&self, wanted: &[analysis::Kind]) -> Vec<analysis::Kind> {
        self.audio
            .as_ref()
            .map(|a| a.analysis.stale(wanted))
            .unwrap_or_default()
    }

    /// Whether its pictures' looks were taken the way they are now.
    #[must_use]
    pub fn pictured(&self) -> bool {
        self.covers.is_empty() || self.picture_method == picture::METHOD
    }

    /// Take the looks [`look`] found, by where each picture sits.
    pub fn take_looks(&mut self, looks: &[(CoverAt, Option<Look>)]) {
        for c in &mut self.covers {
            if let Some((_, look)) = looks.iter().find(|(at, _)| *at == c.at) {
                c.look = *look;
            }
        }
        self.picture_method = picture::METHOD.to_string();
    }

    /// Whether these hold no word of the lyrics beside `located`, read or
    /// not: a media file measured while lyrics with a control character
    /// in them read as no text ([`crate::lyrics::is_text`]), which is
    /// measured again.
    #[must_use]
    pub fn missed_sidecar(&self, located: &Located) -> bool {
        located.kind == Kind::Media
            && located.lyrics.is_some()
            && self.audio.is_some()
            && !self.lyrics.iter().any(|l| l.at == LyricsAt::Sidecar)
            && !self.unkept.contains(&Unkept::Lyrics)
    }

    /// Whether the tags of a source of `kind` were read the way they are
    /// now.
    #[must_use]
    pub fn tags_hold(&self, kind: Kind) -> bool {
        self.tags_method == tags_method(kind)
    }

    /// Facts of a file that could not be read at all.
    #[must_use]
    pub fn unreadable(rev: String) -> Self {
        Self {
            rev,
            content: None,
            method: method(),
            duration: None,
            audio: None,
            covers: Vec::new(),
            lyrics: Vec::new(),
            tags: Offers::new(),
            editions: Vec::new(),
            tags_method: tags::METHOD.to_string(),
            length_ms: None,
            artists: Vec::new(),
            release: false,
            chapters: Vec::new(),
            print: None,
            unkept: Vec::new(),
            served: None,
            tags_digest: None,
            hash_method: HASH_METHOD.to_string(),
            picture_method: picture::METHOD.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioFacts {
    /// Absolute stream index.
    pub index: u32,
    pub codec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub channels: u32,
    /// The speakers in ffmpeg's name; `None` when the file names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default)]
    pub sample_rate: u32,
    /// Bits a decoded sample keeps; 0 when unknown.
    #[serde(default)]
    pub bits: u32,
    #[serde(default)]
    pub float: bool,
    pub quality: Option<AudioQuality>,
    /// The bytes of the audio stream's packets, which a copy carries.
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// What the whole of the audio holds.
    #[serde(default, skip_serializing_if = "is_unanalyzed")]
    pub analysis: Analysis,
}

fn is_unanalyzed(a: &Analysis) -> bool {
    *a == Analysis::default()
}

impl AudioFacts {
    /// Whether the codec keeps every sample as recorded.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        crate::codec::is_lossless_name(&self.codec, self.profile.as_deref())
    }

    /// What its measures show it holds less of than it declares, where it
    /// is lossless ([`crate::quality::Flags`]).
    #[must_use]
    pub fn flags(&self) -> crate::quality::Flags {
        match self.quality {
            Some(q) if self.is_lossless() => q.flags(self.sample_rate, self.bits),
            _ => crate::quality::Flags::default(),
        }
    }

    /// What decides which codecs can hold this audio.
    #[must_use]
    pub fn shape(&self) -> crate::codec::Shape<'_> {
        crate::codec::Shape {
            channels: self.channels,
            layout: self.layout.as_deref(),
            sample_rate: self.sample_rate,
            bits: self.bits,
            float: self.float,
        }
    }
}

/// What a source holds that its library file does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unkept {
    /// Its cue sheet marks its audio pre-emphasized, as some early CDs
    /// were mastered: a player de-emphasizes it only by that flag, which
    /// is lost with the cue sheet, so it plays bright.
    PreEmphasis,
    /// A cue sheet of its name beside it, which splits an album image
    /// into its tracks; the image is one song.
    CueSheet,
    /// A picture in it that does not decode, which is no cover.
    Picture,
    /// Lyrics beside it that read as no text, which give it none.
    Lyrics,
}

/// Where a picture sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverAt {
    /// A stream ffmpeg reads the picture from.
    Picture { index: u32 },
    /// An attachment, which must be dumped first.
    Attachment { ordinal: usize },
    /// The `n`th picture that came with a manual file.
    Sidecar(usize),
    /// The source is the picture.
    File,
    /// The `index`th picture of the file's own tags, as `lofty` reads
    /// them by their bytes ([`picture::tag_pictures`]), where ffprobe
    /// shows them otherwise: an MP4's of implicit type, or one labelled
    /// with another media type than its bytes are.
    Atom { index: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverFacts {
    pub at: CoverAt,
    pub mimetype: String,
    pub quality: Option<ImageQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// How it looks, to tell it from other pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
    /// Whether the file holding it, by its picture type or its name, says
    /// it shows no front cover: a back, a disc or a booklet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub other: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LyricsAt {
    /// A subtitle stream.
    Stream { index: u32 },
    /// The `.lrc` that came with a manual file.
    Sidecar,
    /// The source is the `.lrc`.
    File,
    /// The lyrics tag a media file holds, as `USLT` or `LYRICS`
    /// ([`tags::lyrics_in`]).
    Tag,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricsFacts {
    pub at: LyricsAt,
    pub language: Language,
    /// `None` for lyrics with no timed line.
    pub timing: Option<Timing>,
    /// The length of the recording an `.lrc` says it is timed to.
    #[serde(default)]
    pub stated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Whether they sing any word, rather than only `♪` or a credit;
    /// `None` where measured before muman asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sings: Option<bool>,
    /// Whether the letters they sing are Latin alone, which for a song
    /// sung in another script makes them its romanization; `None` where
    /// measured before muman asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latin: Option<bool>,
}

impl LyricsFacts {
    /// Whether they may be a song's lyrics: any that sing a word, and
    /// any not read for it yet.
    #[must_use]
    pub fn sings_any(&self) -> bool {
        self.sings != Some(false)
    }
}

/// Whether lyrics or a subtitle sing any word.
fn sings(text: &str) -> bool {
    !lyrics::sung(text).trim().is_empty()
}

/// Whether the letters lyrics or a subtitle sing are Latin alone.
fn latin(text: &str) -> bool {
    crate::clean::latin_only(&lyrics::sung(text))
}

/// The digests of what one source offers, by where each part sits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Digests {
    pub audio: Option<String>,
    pub covers: Vec<(CoverAt, String)>,
    pub lyrics: Vec<(LyricsAt, String)>,
    pub tags: Option<String>,
}

impl Digests {
    fn cover(&self, at: &CoverAt) -> Option<String> {
        self.covers
            .iter()
            .find(|(a, _)| a == at)
            .map(|(_, d)| d.clone())
    }

    fn lyrics(&self, at: &LyricsAt) -> Option<String> {
        self.lyrics
            .iter()
            .find(|(a, _)| a == at)
            .map(|(_, d)| d.clone())
    }
}

/// The digest an ffmpeg `hash` output wrote, as `SHA256=<hex>`, cut as
/// [`digest`] cuts its own.
fn read_hash(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let hex = text.trim().strip_prefix("SHA256=")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex[..32].to_ascii_lowercase())
}

/// What `located`'s files hold: the digest of each one's bytes, in the
/// order its revision names them; none where one cannot be read. Every
/// byte is read, so it is taken where a revision changed or a source is
/// measured, never to check one that did not.
#[must_use]
pub fn content_of(located: &Located) -> Option<String> {
    let parts: Option<Vec<String>> = std::iter::once(&located.path)
        .chain(&located.lyrics)
        .chain(&located.covers)
        .map(|p| digest_file(p))
        .collect();
    parts.map(|p| p.join(";"))
}

/// The ffmpeg output options that hash the packets of stream
/// `input:index`, copied.
fn hash_output(input: usize, index: u32) -> Vec<std::ffi::OsString> {
    let map = format!("{input}:{index}");
    ["-map", &map, "-c", "copy", "-hash", "sha256", "-f", "hash"]
        .into_iter()
        .map(Into::into)
        .collect()
}

/// Measure one located source, writing intermediates to `scratch`, and
/// analyze its audio for each of `wanted`.
pub fn gather<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
    wanted: &[analysis::Kind],
) -> Result<Facts> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let mut facts = Facts::unreadable(located.rev());
    facts.content = content_of(located);
    match located.kind {
        Kind::Lyrics => {
            let text = read_text(&located.path)?;
            facts.lyrics.push(LyricsFacts {
                digest: digest_file(&located.path),
                ..lyrics_of(LyricsAt::File, &text)
            });
            facts.tags_method = tags_method(Kind::Lyrics);
        }
        Kind::Image => {
            let (quality, look) = image_file(runner, located, scratch);
            facts.covers.push(CoverFacts {
                at: CoverAt::File,
                mimetype: mimetype_of(&located.path),
                quality,
                digest: digest_file(&located.path),
                look,
                other: named_other(&located.path),
            });
        }
        Kind::Tags => {
            let record = crate::record::read(&located.path)?;
            facts.tags = tags::from_record(&record);
            facts.editions = tags::editions(&record);
            facts.tags_digest = digest_file(&located.path);
            facts.tags_method = tags_method(Kind::Tags);
            facts.length_ms = record.length_ms;
            facts.artists = record.credited_artists;
        }
        Kind::Media => media(runner, located, scratch, wanted, &mut facts)?,
    }
    Ok(facts)
}

/// What reading a source's tags again finds.
#[derive(Debug, Default)]
pub struct Reread {
    pub tags: Offers,
    pub editions: Vec<Offers>,
    pub release: bool,
    pub chapters: Vec<Chapter>,
    /// The length a record says its recording plays, in ms.
    pub length_ms: Option<u64>,
    /// The artists a record credits.
    pub artists: Vec<crate::record::CreditedArtist>,
    /// The lyrics its tags hold, or a lyrics file's own.
    pub lyrics: Option<LyricsFacts>,
}

/// The tags of one located source, read again with its chapters and the
/// lyrics its tags hold: a media file's are probed, its pictures not
/// dumped nor anything measured.
pub fn retag<R: Runner>(runner: &R, located: &Located, scratch: &Path) -> Result<Reread> {
    match located.kind {
        Kind::Media => {}
        Kind::Tags => {
            let record = crate::record::read(&located.path)?;
            return Ok(Reread {
                tags: tags::from_record(&record),
                editions: tags::editions(&record),
                length_ms: record.length_ms,
                artists: record.credited_artists,
                ..Reread::default()
            });
        }
        Kind::Lyrics => {
            let text = read_text(&located.path)?;
            return Ok(Reread {
                lyrics: Some(LyricsFacts {
                    digest: digest_file(&located.path),
                    ..lyrics_of(LyricsAt::File, &text)
                }),
                ..Reread::default()
            });
        }
        Kind::Image => return Ok(Reread::default()),
    }
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
    let info = dump_attachments(runner, located, &probed, scratch, false);
    Ok(Reread {
        tags: tags_of(info.as_ref(), &probed, &located.path),
        release: info.as_ref().is_some_and(VideoInfo::is_release),
        chapters: probed.chapters,
        lyrics: tag_lyrics(located),
        ..Reread::default()
    })
}

/// Whether each of `at`, lyrics of `located` measured before muman asked,
/// sings any word: subtitles extracted again by one ffmpeg run, files
/// read. What does not read again is left out.
pub fn relyric<R: Runner>(
    runner: &R,
    located: &Located,
    at: &[LyricsAt],
    scratch: &Path,
) -> Result<Vec<(LyricsAt, bool)>> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let mut found = Vec::new();
    let mut streams = Vec::new();
    for place in at {
        let file = match place {
            LyricsAt::Stream { index } => {
                streams.push(*index);
                continue;
            }
            LyricsAt::Sidecar => located.lyrics.as_deref(),
            LyricsAt::File => Some(located.path.as_path()),
            LyricsAt::Tag => {
                if let Some(l) = tag_lyrics(located) {
                    found.push((LyricsAt::Tag, l.sings_any()));
                }
                continue;
            }
        };
        if let Some(text) = file.and_then(|f| read_text(f).ok()) {
            found.push((place.clone(), sings(&text)));
        }
    }
    let outputs: Vec<Output> = streams
        .iter()
        .map(|index| {
            let mut args: Vec<std::ffi::OsString> = ["-map".to_string(), format!("0:{index}")]
                .into_iter()
                .map(Into::into)
                .collect();
            args.extend(LRC_ARGS.iter().map(Into::into));
            Output::new(args, &scratch.join(format!("sub{index}.lrc")))
        })
        .collect();
    let inputs = [Input::from(located.path.as_path())];
    for ((index, output), result) in streams
        .iter()
        .zip(&outputs)
        .zip(ffmpeg::run(runner, &inputs, &outputs))
    {
        if let Some(text) = result.ok().and_then(|()| read_text(&output.path).ok()) {
            found.push((LyricsAt::Stream { index: *index }, sings(&text)));
        }
    }
    Ok(found)
}

/// The digest of the tags a media file offers as written: of the fields
/// of its info JSON, dumped into `scratch`, that tags are read from, for
/// what yt-dlp fetched; else of its own tags.
fn tags_digest(fetched: bool, probed: &Probed, path: &Path, scratch: &Path) -> Option<String> {
    if fetched {
        return info::tags_digest(&std::fs::read(scratch.join("info.json")).ok()?);
    }
    let own = tags::read_file(path).unwrap_or_else(|| {
        probed
            .tags
            .iter()
            .map(|(k, v)| (k.clone(), vec![v.clone()]))
            .collect()
    });
    Some(digest(&serde_json::to_vec(&own).ok()?))
}

/// What a media file's tags offer: its info JSON's, for what yt-dlp
/// fetched; else its own, as `lofty` reads them, or as ffprobe does for a
/// format `lofty` does not read.
fn tags_of(info: Option<&VideoInfo>, probed: &Probed, path: &Path) -> Offers {
    match info {
        Some(info) => tags::from_info(info),
        None => tags::from_container(&tags::read_file(path).unwrap_or_else(|| {
            probed
                .tags
                .iter()
                .map(|(k, v)| (k.clone(), vec![v.clone()]))
                .collect()
        })),
    }
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    crate::lyrics::read(&bytes).with_context(|| format!("{} holds no text", path.display()))
}

/// The lyrics a media file's own tags hold, measured as any lyrics are,
/// in the language a `USLT` frame names, and known by their text's
/// digest.
#[must_use]
pub fn tag_lyrics(located: &Located) -> Option<LyricsFacts> {
    let (text, language) = tags::lyrics_in(&located.path)?;
    Some(LyricsFacts {
        language: language.map_or(Language::Unstated, Language::Tag),
        digest: Some(digest(text.as_bytes())),
        ..lyrics_of(LyricsAt::Tag, &text)
    })
}

fn lyrics_of(at: LyricsAt, text: &str) -> LyricsFacts {
    LyricsFacts {
        at,
        language: Language::Unstated,
        timing: lyrics::timing(&lyrics::clean_lrc(text)),
        stated_ms: lyrics::stated_length(text),
        digest: None,
        sings: Some(sings(text)),
        latin: Some(latin(text)),
    }
}

/// Whether `a` and `b` are named alike but for their extensions, in any
/// case, as a sidecar is matched to its song.
fn named_as(a: &Path, b: &Path) -> bool {
    let stem = |p: &Path| crate::relpath::folded(Path::new(p.file_stem().unwrap_or_default()));
    stem(a) == stem(b)
}

/// Whether a picture file's name says it shows no front cover.
fn named_other(path: &Path) -> bool {
    path.file_stem()
        .is_some_and(|stem| crate::picture::named_other(&stem.to_string_lossy()))
}

/// A picture file's media type by its bytes, else by its name's
/// extension.
fn mimetype_of(path: &Path) -> String {
    if let Some(sniffed) = crate::picture::sniff_file(path) {
        return sniffed.to_string();
    }
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(picture::ImageFormat::of_extension)
        .map_or("image/unknown", picture::ImageFormat::mimetype)
        .to_string()
}

fn measure_image(pam: &Path) -> (Option<ImageQuality>, Option<Look>) {
    const HEADER: u64 = 128;
    let small = std::fs::metadata(pam).is_ok_and(|m| m.len() <= 2 * quality::MOST_PIXELS + HEADER);
    let Some(bytes) = small.then(|| std::fs::read(pam).ok()).flatten() else {
        return (None, None);
    };
    let Some((w, h, pairs)) = ffmpeg::read_pam(&bytes)
        .filter(|(w, h, _)| u64::from(*w) * u64::from(*h) <= quality::MOST_PIXELS)
    else {
        return (None, None);
    };
    let pixels = quality::flatten(pairs);
    let quality = quality::image(&pixels, w, h);
    let look = quality.and_then(|q| picture::look(&pixels, w, h, q.content));
    (quality, look)
}

/// The measures of the picture `located` is.
fn image_file<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
) -> (Option<ImageQuality>, Option<Look>) {
    let out = scratch.join("file.pam");
    let outputs = [Output::new(quality::gray_output(0, "v:0"), &out)];
    let input = Input::new(quality::whole_pictures(), &located.path);
    let ok = ffmpeg::run(runner, &[input], &outputs);
    match ok.into_iter().next() {
        Some(Ok(())) => measure_image(&out),
        _ => (None, None),
    }
}

/// The outputs writing each picture `probed` holds, and those dumped
/// from its attachments into `scratch`, in gray, each input it
/// needs added to `inputs`; where ffprobe shows the pictures of `path`'s
/// own tags otherwise than their bytes are ([`streams_agree`]), those
/// pictures as `lofty` reads them, each decoded as its bytes say.
fn cover_outputs(
    probed: &Probed,
    path: &Path,
    scratch: &Path,
    inputs: &mut Vec<Input>,
) -> Vec<(Output, CoverAt, String, bool)> {
    let mut outputs = Vec::new();
    let tagged = picture::tag_pictures(path);
    let streams = tagged
        .as_deref()
        .is_none_or(|t| streams_agree(&probed.pictures, t));
    for p in probed.pictures.iter().filter(|_| streams) {
        outputs.push((
            Output::new(
                quality::gray_output(0, &p.index.to_string()),
                &scratch.join(format!("pic{}.pam", p.index)),
            ),
            CoverAt::Picture { index: p.index },
            p.mimetype.clone(),
            p.other,
        ));
    }
    for a in probed.image_attachments() {
        let dumped = scratch.join(format!("att{}", a.ordinal));
        if !dumped.exists() {
            continue;
        }
        inputs.push(Input::new(quality::whole_pictures(), &dumped));
        outputs.push((
            Output::new(
                quality::gray_output(inputs.len() - 1, "v:0"),
                &scratch.join(format!("att{}.pam", a.ordinal)),
            ),
            CoverAt::Attachment { ordinal: a.ordinal },
            crate::picture::sniff_file(&dumped).map_or_else(|| a.mimetype.clone(), str::to_string),
            crate::picture::named_other(
                Path::new(&a.filename)
                    .file_stem()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_default()
                    .as_ref(),
            ),
        ));
    }
    let atoms = if streams {
        Vec::new()
    } else {
        tagged.unwrap_or_default()
    };
    for (index, (bytes, other)) in atoms.into_iter().enumerate() {
        let Ok(name) = crate::atomic::Name::new(format!("atom{index}")) else {
            continue;
        };
        if crate::atomic::write(scratch, &name, &bytes).is_err() {
            continue;
        }
        let file = scratch.join(name.as_str());
        inputs.push(Input::new(quality::whole_pictures(), &file));
        outputs.push((
            Output::new(
                quality::gray_output(inputs.len() - 1, "v:0"),
                &scratch.join(format!("atom{index}.pam")),
            ),
            CoverAt::Atom { index },
            picture::sniff(&bytes)
                .unwrap_or("image/unknown")
                .to_string(),
            other,
        ));
    }
    outputs
}

/// Whether ffprobe's picture `streams` are the pictures a file's tags
/// hold, `tagged`, one for one, each of the format its bytes are.
fn streams_agree(streams: &[probe::Picture], tagged: &[(Vec<u8>, bool)]) -> bool {
    let format = picture::ImageFormat::of_mimetype;
    streams.len() == tagged.len()
        && streams.iter().zip(tagged).all(|(stream, (bytes, _))| {
            picture::sniff(bytes).is_none_or(|is| format(is) == format(&stream.mimetype))
        })
}

/// The run writing the `n`th picture beside a manual file in gray;
/// each has one of its own, since an ffmpeg that cannot open one input
/// fails every output.
fn sidecar_cover<R: Runner>(
    runner: &R,
    cover: &Path,
    n: usize,
    scratch: &Path,
) -> (Output, Result<()>) {
    let output = Output::new(
        quality::gray_output(0, "v:0"),
        &scratch.join(format!("side{n}.pam")),
    );
    let input = Input::new(quality::whole_pictures(), cover);
    let result = ffmpeg::run(runner, &[input], std::slice::from_ref(&output))
        .into_iter()
        .next()
        .unwrap_or_else(|| Err(anyhow::anyhow!("no run")));
    (output, result)
}

/// How each picture of `located` looks, measuring nothing else: one
/// ffmpeg run for a picture, and for a media file its probe, a dump of
/// its attachments and one run for its pictures, besides one for each
/// beside it.
pub fn look<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
) -> Result<Vec<(CoverAt, Option<Look>)>> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    match located.kind {
        Kind::Image => Ok(vec![(
            CoverAt::File,
            image_file(runner, located, scratch).1,
        )]),
        Kind::Media => {
            let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
            dump_attachments(runner, located, &probed, scratch, true);
            let mut inputs = vec![Input::from(located.path.as_path())];
            let covers = cover_outputs(&probed, &located.path, scratch, &mut inputs);
            let mut looks = Vec::new();
            if !covers.is_empty() {
                let plain: Vec<Output> = covers.iter().map(|(o, ..)| o.clone()).collect();
                let results = ffmpeg::run(runner, &inputs, &plain);
                for ((output, at, ..), result) in covers.into_iter().zip(results) {
                    looks.push((at, result.ok().and_then(|()| measure_image(&output.path).1)));
                }
            }
            for (n, cover) in located.covers.iter().enumerate() {
                let (output, result) = sidecar_cover(runner, cover, n, scratch);
                looks.push((
                    CoverAt::Sidecar(n),
                    result.ok().and_then(|()| measure_image(&output.path).1),
                ));
            }
            Ok(looks)
        }
        Kind::Lyrics | Kind::Tags => Ok(Vec::new()),
    }
}

/// Where each measured segment of a recording `duration` long starts.
fn segment_starts(duration: Option<f64>) -> Vec<f64> {
    let Some(d) = duration.filter(|d| *d > 0.0) else {
        return vec![0.0];
    };
    if d <= quality::SEGMENT_SECONDS * 2.0 {
        return vec![0.0];
    }
    quality::SEGMENTS
        .iter()
        .map(|share| (d * share).min(d - quality::SEGMENT_SECONDS).max(0.0))
        .collect()
}

/// Where segments start that sound, by the seconds of a whole decode that
/// do: centred on the seconds at [`quality::SEGMENTS`]' shares of them,
/// within the audio. `None` where no second sounds.
fn sounding_starts(sounding: &[bool]) -> Option<Vec<f64>> {
    let sound: Vec<usize> = (0..sounding.len()).filter(|&s| sounding[s]).collect();
    let last = sound.len().checked_sub(1)?;
    let latest = (sounding.len() as f64 - quality::SEGMENT_SECONDS).max(0.0);
    let mut starts: Vec<f64> = quality::SEGMENTS
        .iter()
        .map(|share| {
            let second = sound[(last as f64 * share).round() as usize] as f64;
            (second + 0.5 - quality::SEGMENT_SECONDS / 2.0).clamp(0.0, latest)
        })
        .collect();
    starts.dedup();
    Some(starts)
}

/// What one ffmpeg output measures, to read back once it ran.
enum Measured {
    OpusHead,
    Print,
    Packets,
    Segment(PathBuf),
    Subtitle(u32, Language),
    /// A picture, its media type, and whether it shows no front cover.
    Cover(CoverAt, String, bool),
    Hash(Part),
}

/// A part of a source a digest is of.
enum Part {
    Audio,
    Cover(CoverAt),
    Lyrics(LyricsAt),
}

/// The outputs hashing each stream of `probed` a plan may take: its
/// audio, its pictures and its subtitles.
fn hash_outputs(probed: &Probed, scratch: &Path) -> Vec<(Output, Part)> {
    let audio = probed.audio.iter().map(|a| (a.index, Part::Audio));
    let pictures = probed
        .pictures
        .iter()
        .map(|p| (p.index, Part::Cover(CoverAt::Picture { index: p.index })));
    let subtitles = probed
        .subtitles
        .iter()
        .map(|s| (s.index, Part::Lyrics(LyricsAt::Stream { index: s.index })));
    audio
        .chain(pictures)
        .chain(subtitles)
        .map(|(index, part)| {
            let path = scratch.join(format!("hash{index}"));
            (Output::new(hash_output(0, index), &path), part)
        })
        .collect()
}

/// Put the digest an output wrote under its part in `digests`.
fn note_hash(digests: &mut Digests, part: Part, path: &Path) {
    let Some(d) = read_hash(path) else {
        return;
    };
    match part {
        Part::Audio => digests.audio = Some(d),
        Part::Cover(at) => digests.covers.push((at, d)),
        Part::Lyrics(at) => digests.lyrics.push((at, d)),
    }
}

/// The digests of the files a media source brings whole: its image
/// attachments, dumped into `scratch`, and its sidecars.
fn file_digests(located: &Located, probed: &Probed, scratch: &Path, digests: &mut Digests) {
    for a in probed.image_attachments() {
        if let Some(d) = digest_file(&scratch.join(format!("att{}", a.ordinal))) {
            digests
                .covers
                .push((CoverAt::Attachment { ordinal: a.ordinal }, d));
        }
    }
    for (n, cover) in located.covers.iter().enumerate() {
        if let Some(d) = digest_file(cover) {
            digests.covers.push((CoverAt::Sidecar(n), d));
        }
    }
    if let Some(d) = located.lyrics.as_deref().and_then(digest_file) {
        digests.lyrics.push((LyricsAt::Sidecar, d));
    }
}

/// The digests of one located source and what a site served it in, by
/// one ffmpeg run that copies its streams and decodes nothing: for facts
/// measured before muman hashed sources.
pub fn hash<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
) -> Result<(Digests, Option<Served>)> {
    let mut digests = Digests::default();
    match located.kind {
        Kind::Lyrics => {
            if let Some(d) = digest_file(&located.path) {
                digests.lyrics.push((LyricsAt::File, d));
            }
            return Ok((digests, None));
        }
        Kind::Image => {
            if let Some(d) = digest_file(&located.path) {
                digests.covers.push((CoverAt::File, d));
            }
            return Ok((digests, None));
        }
        Kind::Tags => {
            digests.tags = digest_file(&located.path);
            return Ok((digests, None));
        }
        Kind::Media => {}
    }
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
    let info = dump_attachments(runner, located, &probed, scratch, true);
    let outputs = hash_outputs(&probed, scratch);
    let plain: Vec<Output> = outputs.iter().map(|(o, _)| o.clone()).collect();
    let ran = ffmpeg::run_outputs(runner, &[located.path.as_path()], &plain);
    for ((output, part), result) in outputs.into_iter().zip(ran) {
        if result.is_ok() {
            note_hash(&mut digests, part, &output.path);
        }
    }
    file_digests(located, &probed, scratch, &mut digests);
    if let Some(d) = tag_lyrics(located).and_then(|l| l.digest) {
        digests.lyrics.push((LyricsAt::Tag, d));
    }
    digests.tags = tags_digest(info.is_some(), &probed, &located.path, scratch);
    Ok((digests, info.as_ref().and_then(Served::of)))
}

// The steps read in the order they run; split up, the order would be in the call sites.
#[expect(clippy::too_many_lines)]
fn media<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
    wanted: &[analysis::Kind],
    facts: &mut Facts,
) -> Result<()> {
    let probed = probe::read(runner, &located.path)?;
    facts.duration = probed.duration.filter(|_| !probed.guessed);
    let info = dump_attachments(runner, located, &probed, scratch, true);
    facts.tags = tags_of(info.as_ref(), &probed, &located.path);
    facts.release = info.as_ref().is_some_and(VideoInfo::is_release);
    facts.chapters.clone_from(&probed.chapters);
    let served = info.as_ref().and_then(Served::of);
    let tags = tags_digest(info.is_some(), &probed, &located.path, scratch);
    let info = info.unwrap_or_default();

    let mut inputs = vec![Input::new(quality::whole_pictures(), &located.path)];
    let mut outputs: Vec<(Output, Measured)> = hash_outputs(&probed, scratch)
        .into_iter()
        .map(|(o, part)| (o, Measured::Hash(part)))
        .collect();
    if let Some(a) = &probed.audio {
        outputs.push((
            Output::new(fingerprint::output(0, a.index), &scratch.join("print.pcm")),
            Measured::Print,
        ));
        outputs.push((
            Output::new(packets_output(0, a.index), &scratch.join("packets.crc")),
            Measured::Packets,
        ));
        if wanted.contains(&analysis::Kind::Loudness) && a.codec == "opus" {
            outputs.push((
                Output::new(opus_head_output(0, a.index), &scratch.join("head.opus")),
                Measured::OpusHead,
            ));
        }
        for (n, start) in segment_starts(probed.duration).into_iter().enumerate() {
            let path = scratch.join(format!("segment{n}"));
            inputs.push(Input::new(quality::segment_input(start), &located.path));
            outputs.push((
                Output::new(quality::segment_output(inputs.len() - 1, a.index), &path),
                Measured::Segment(path),
            ));
        }
    }
    for sub in &probed.subtitles {
        if let Some(language) = lyrics::language(sub, &info) {
            let args = ["-map".to_string(), format!("0:{}", sub.index)];
            let mut args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
            args.extend(LRC_ARGS.iter().map(Into::into));
            outputs.push((
                Output::new(args, &scratch.join(format!("sub{}.lrc", sub.index))),
                Measured::Subtitle(sub.index, language),
            ));
        }
    }
    for (output, at, mimetype, other) in cover_outputs(&probed, &located.path, scratch, &mut inputs)
    {
        outputs.push((output, Measured::Cover(at, mimetype, other)));
    }

    let mut plain: Vec<Output> = outputs.iter().map(|(o, _)| o.clone()).collect();
    let mut pass = Pass::new(if probed.audio.is_some() {
        wanted.to_vec()
    } else {
        Vec::new()
    });
    let mut results = match probed.audio.as_ref().filter(|_| pass.wants()) {
        Some(a) => {
            let piped = Output::new(analysis_output(0, a.index, None), Path::new(ffmpeg::PIPE));
            ffmpeg::run_piped(runner, &inputs, &plain, &piped, &mut |p| pass.piped(&p)).0
        }
        None => ffmpeg::run(runner, &inputs, &plain),
    };
    for (n, cover) in located.covers.iter().enumerate() {
        let (output, result) = sidecar_cover(runner, cover, n, scratch);
        results.push(result);
        plain.push(output.clone());
        outputs.push((
            output,
            Measured::Cover(
                CoverAt::Sidecar(n),
                mimetype_of(cover),
                // Named as its song, a sidecar says nothing of its side.
                named_other(cover) && !named_as(cover, &located.path),
            ),
        ));
    }
    let channels = probed.audio.as_ref().map_or(1, |a| a.channels as usize);
    let headroom = probed
        .audio
        .as_ref()
        .is_some_and(|a| a.float && a.is_lossless());
    let mut segments = Vec::new();
    let mut bytes = None;
    let mut timeline = None;
    let mut digests = Digests::default();
    let mut opus_gain = 0;
    let mut undecoded = false;
    for ((output, measured), result) in outputs.into_iter().zip(results) {
        if result.is_err() {
            undecoded |= matches!(measured, Measured::Cover(..));
            continue;
        }
        match measured {
            Measured::Print => {
                facts.print = runner.fingerprint(&output.path).ok().and_then(Print::new);
            }
            Measured::Packets => {
                let text = std::fs::read_to_string(&output.path).unwrap_or_default();
                bytes = packet_bytes(&text);
                timeline = Timeline::parse(&text);
            }
            Measured::Segment(path) => segments.push(quality::read_segment(&path, channels)),
            Measured::Subtitle(index, language) => {
                if let Ok(text) = read_text(&output.path) {
                    facts.lyrics.push(LyricsFacts {
                        at: LyricsAt::Stream { index },
                        language,
                        timing: lyrics::timing(&lyrics::clean_lrc(&text)),
                        stated_ms: None,
                        digest: None,
                        sings: Some(sings(&text)),
                        latin: Some(latin(&text)),
                    });
                }
            }
            Measured::Cover(at, mimetype, other) => {
                let (quality, look) = measure_image(&output.path);
                facts.covers.push(CoverFacts {
                    at,
                    mimetype,
                    quality,
                    digest: None,
                    look,
                    other,
                });
            }
            Measured::Hash(part) => note_hash(&mut digests, part, &output.path),
            Measured::OpusHead => opus_gain = read_opus_gain(&output.path),
        }
    }
    let found = Found {
        said: probed.audio.as_ref().and_then(|a| a.duration),
        guessed: probed.guessed,
        timeline,
    };
    let sounding = pass.sounding();
    let analysis = with_opus_gain(pass.finish(&found), opus_gain);
    file_digests(located, &probed, scratch, &mut digests);
    digests.tags = tags;
    // Lyrics beside a song that do not read cost the song only them.
    let sidecar = located.lyrics.as_deref().map(read_text);
    if let Some(Ok(text)) = &sidecar {
        facts.lyrics.push(lyrics_of(LyricsAt::Sidecar, text));
    }
    if let Some(l) = tag_lyrics(located) {
        digests
            .lyrics
            .extend(l.digest.clone().map(|d| (LyricsAt::Tag, d)));
        facts.lyrics.push(l);
    }
    // Segments placed by a length the packets fall short of read nothing;
    // placed by a guessed one, they miss the shares of the audio there is.
    let misplaced = timeline.map(|t| t.end).filter(|ends| {
        probed.guessed
            && probed
                .duration
                .is_some_and(|d| (d - ends).abs() > integrity::SLACK_S)
    });
    if let (Some(Damage::Cut { ends, .. }), Some(a)) = (
        analysis.integrity().and_then(integrity::Measure::damage),
        &probed.audio,
    ) && quality::audio(&segments, a.sample_rate, headroom).is_none()
    {
        segments = measure_segments(runner, located, a, &segment_starts(Some(ends)), scratch);
    } else if let (Some(ends), Some(a)) = (misplaced, &probed.audio) {
        segments = measure_segments(runner, located, a, &segment_starts(Some(ends)), scratch);
    }
    // Excerpts that all fell in silence are taken again where it sounds.
    if let Some(a) = &probed.audio
        && quality::audio(&segments, a.sample_rate, headroom).is_none()
        && let Some(starts) = sounding_starts(&sounding)
    {
        segments = measure_segments(runner, located, a, &starts, scratch);
    }
    facts.unkept = unkept(&located.path);
    if undecoded {
        facts.unkept.push(Unkept::Picture);
    }
    if sidecar.is_some_and(|r| r.is_err()) {
        facts.unkept.push(Unkept::Lyrics);
    }
    facts.audio = probed.audio.map(|a| AudioFacts {
        quality: quality::audio(&segments, a.sample_rate, headroom),
        index: a.index,
        codec: a.codec,
        profile: a.profile,
        channels: a.channels,
        layout: a.layout,
        sample_rate: a.sample_rate,
        bits: a.bits,
        float: a.float,
        bytes,
        digest: None,
        analysis,
    });
    facts.take(&digests, served);
    Ok(())
}

/// What `path` holds, or has beside it, that a library file does not
/// keep.
fn unkept(path: &Path) -> Vec<Unkept> {
    let cue = path.with_extension("cue");
    let sheet = std::fs::read(&cue).ok().map(|b| crate::lyrics::decode(&b));
    let mut unkept = Vec::new();
    let flagged = sheet.as_deref().is_some_and(|text| {
        text.lines().any(|l| {
            let mut words = l.split_whitespace();
            words
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case("FLAGS"))
                && words.any(|w| w.eq_ignore_ascii_case("PRE"))
        })
    });
    if flagged || flac_emphasis(path) {
        unkept.push(Unkept::PreEmphasis);
    }
    if sheet.is_some() {
        unkept.push(Unkept::CueSheet);
    }
    unkept
}

/// Whether a FLAC file's `CUESHEET` block marks any track pre-emphasized.
fn flac_emphasis(path: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    // Metadata blocks come first and are small but for pictures, which
    // are skipped unread.
    let mut file = std::io::BufReader::new(file);
    let mut magic = [0; 4];
    if file.read_exact(&mut magic).is_err() || &magic != b"fLaC" {
        return false;
    }
    loop {
        let mut header = [0; 4];
        if file.read_exact(&mut header).is_err() {
            return false;
        }
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let length = u32::from_be_bytes([0, header[1], header[2], header[3]]);
        if kind == CUESHEET {
            let mut block = vec![0; length as usize];
            return file.read_exact(&mut block).is_ok() && cuesheet_emphasis(&block);
        }
        if last
            || std::io::copy(
                &mut (&mut file).take(u64::from(length)),
                &mut std::io::sink(),
            )
            .is_err()
        {
            return false;
        }
    }
}

const CUESHEET: u8 = 5;

/// Whether a `CUESHEET` block's tracks hold one flagged pre-emphasized.
fn cuesheet_emphasis(block: &[u8]) -> bool {
    // The catalog number, lead-in, flags and reserved bytes, then the
    // track count; each track its offset, number, ISRC, flags, reserved
    // bytes and index count, then its indices.
    const TRACKS_AT: usize = 128 + 8 + 1 + 258;
    let Some(&count) = block.get(TRACKS_AT) else {
        return false;
    };
    let mut at = TRACKS_AT + 1;
    for _ in 0..count {
        let (Some(&flags), Some(&indices)) = (block.get(at + 21), block.get(at + 35)) else {
            return false;
        };
        if flags & 0x40 != 0 {
            return true;
        }
        at += 36 + 12 * usize::from(indices);
    }
    false
}

/// The segments of a source's audio from `starts`, decoded again: where
/// its audio stream said it ran longer than it holds, or where the first
/// segments all fell in silence.
fn measure_segments<R: Runner>(
    runner: &R,
    located: &Located,
    audio: &probe::Audio,
    starts: &[f64],
    scratch: &Path,
) -> Vec<quality::Segment> {
    let inputs: Vec<Input> = starts
        .iter()
        .map(|start| Input::new(quality::segment_input(*start), &located.path))
        .collect();
    let paths: Vec<PathBuf> = (0..starts.len())
        .map(|n| scratch.join(format!("held{n}")))
        .collect();
    let outputs: Vec<Output> = paths
        .iter()
        .enumerate()
        .map(|(n, path)| Output::new(quality::segment_output(n, audio.index), path))
        .collect();
    let channels = audio.channels as usize;
    ffmpeg::run(runner, &inputs, &outputs)
        .into_iter()
        .zip(paths)
        .filter(|(result, _)| result.is_ok())
        .map(|(_, path)| quality::read_segment(&path, channels))
        .collect()
}

impl Pass {
    fn piped(&mut self, piped: &Piped<'_>) {
        match piped {
            Piped::Bytes(b) => self.take(b),
            Piped::Again => self.again(),
        }
    }
}

/// The ffmpeg output options that decode the audio stream `input:index`
/// whole for [`crate::analysis`]: 32-bit float WAV at its own rate and
/// channels, or mixed through `filter` as a song is mixed.
fn analysis_output(input: usize, index: u32, filter: Option<String>) -> Vec<std::ffi::OsString> {
    let mut args = vec!["-map".to_string(), format!("{input}:{index}")];
    if let Some(f) = filter {
        args.extend(["-af".to_string(), f]);
    }
    args.extend(["-c:a", "pcm_f32le", "-f", "wav"].map(String::from));
    args.into_iter().map(Into::into).collect()
}

/// The ffmpeg output options that copy the first packet of the Opus stream
/// `input:index` into Ogg, for the gain its header holds.
fn opus_head_output(input: usize, index: u32) -> Vec<std::ffi::OsString> {
    let map = format!("{input}:{index}");
    ["-map", &map, "-c:a", "copy", "-frames:a", "1", "-f", "opus"]
        .into_iter()
        .map(Into::into)
        .collect()
}

fn read_opus_gain(path: &Path) -> i16 {
    std::fs::read(path)
        .ok()
        .and_then(|b| crate::ogg::output_gain(&b))
        .unwrap_or(0)
}

fn with_opus_gain(mut analysis: Analysis, gain: i16) -> Analysis {
    if let Some(m) = analysis.loudness.as_mut().and_then(|m| m.value.as_mut()) {
        m.opus_gain = gain;
    }
    analysis
}

/// The analyses of `wanted` of `audio`, a stream of `located`, by one run
/// of their own: for facts measured before they were asked for. The
/// integrity measure reads the stream's own length again and lists its
/// packets in that run, which facts kept from before it do not hold.
pub fn analyze<R: Runner>(
    runner: &R,
    located: &Located,
    audio: &AudioFacts,
    wanted: &[analysis::Kind],
    scratch: &Path,
) -> Result<Analysis> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let integrity = wanted.contains(&analysis::Kind::Integrity);
    let head = scratch.join("head.opus");
    let packets = scratch.join("packets.crc");
    let mut outputs = Vec::new();
    if wanted.contains(&analysis::Kind::Loudness) && audio.codec == "opus" {
        outputs.push(Output::new(opus_head_output(0, audio.index), &head));
    }
    if integrity {
        outputs.push(Output::new(packets_output(0, audio.index), &packets));
    }
    let piped = Output::new(
        analysis_output(0, audio.index, None),
        Path::new(ffmpeg::PIPE),
    );
    let mut pass = Pass::new(wanted.to_vec());
    let inputs = [Input::from(located.path.as_path())];
    let (results, _) =
        ffmpeg::run_piped(runner, &inputs, &outputs, &piped, &mut |p| pass.piped(&p));
    let wrote = |path: &Path| {
        outputs
            .iter()
            .zip(&results)
            .any(|(o, r)| o.path == path && r.is_ok())
    };
    let gain = if wrote(&head) {
        read_opus_gain(&head)
    } else {
        0
    };
    let found = if integrity {
        let probed = probe::read(runner, &located.path).ok();
        let guessed = probed.as_ref().is_some_and(|p| p.guessed);
        let said = probed
            .and_then(|p| p.audio)
            .filter(|a| a.index == audio.index)
            .and_then(|a| a.duration);
        let timeline = wrote(&packets)
            .then(|| std::fs::read_to_string(&packets).ok())
            .flatten()
            .and_then(|text| Timeline::parse(&text));
        Found {
            said,
            guessed,
            timeline,
        }
    } else {
        Found::default()
    };
    Ok(with_opus_gain(pass.finish(&found), gain))
}

/// The integrity of `located`'s audio measured afresh, by one ffprobe and
/// one ffmpeg run, whatever its facts say; `None` for a file of no audio.
pub fn integrity<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
) -> Result<Option<integrity::Measure>> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let probed = probe::read(runner, &located.path)?;
    let Some(audio) = probed.audio else {
        return Ok(None);
    };
    let packets = scratch.join("packets.crc");
    let outputs = [Output::new(packets_output(0, audio.index), &packets)];
    let piped = Output::new(
        analysis_output(0, audio.index, None),
        Path::new(ffmpeg::PIPE),
    );
    let mut pass = Pass::new(vec![analysis::Kind::Integrity]);
    let inputs = [Input::from(located.path.as_path())];
    let (listed, decoded) =
        ffmpeg::run_piped(runner, &inputs, &outputs, &piped, &mut |p| pass.piped(&p));
    decoded?;
    let timeline = listed
        .first()
        .is_some_and(Result::is_ok)
        .then(|| std::fs::read_to_string(&packets).ok())
        .flatten()
        .and_then(|text| Timeline::parse(&text));
    let found = Found {
        said: audio.duration,
        guessed: probed.guessed,
        timeline,
    };
    Ok(pass.finish(&found).integrity().copied())
}

/// The analyses of `wanted` of `audio` mixed through `filter`, as a song
/// mixed into another layout is written.
pub fn analyze_mix<R: Runner>(
    runner: &R,
    located: &Located,
    audio: &AudioFacts,
    filter: String,
    wanted: &[analysis::Kind],
) -> Analysis {
    let piped = Output::new(
        analysis_output(0, audio.index, Some(filter)),
        Path::new(ffmpeg::PIPE),
    );
    let mut pass = Pass::new(wanted.to_vec());
    let inputs = [Input::from(located.path.as_path())];
    let _ = ffmpeg::run_piped(runner, &inputs, &[], &piped, &mut |p| pass.piped(&p));
    pass.finish(&Found::default())
}

/// The ffmpeg output options that list every packet of the audio stream
/// `input:index`, copied, one line each with its size.
fn packets_output(input: usize, index: u32) -> Vec<std::ffi::OsString> {
    let map = format!("{input}:{index}");
    ["-map", &map, "-c:a", "copy", "-f", "framecrc"]
        .into_iter()
        .map(Into::into)
        .collect()
}

/// The sum of the packet sizes a `framecrc` list holds: the fifth field
/// of each line that is no `#` comment.
#[must_use]
pub fn packet_bytes(text: &str) -> Option<u64> {
    let sizes: Option<Vec<u64>> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split(',').nth(4)?.trim().parse().ok())
        .collect();
    sizes.filter(|s| !s.is_empty()).map(|s| s.iter().sum())
}

fn read_packets(path: &Path) -> Option<u64> {
    packet_bytes(&std::fs::read_to_string(path).ok()?)
}

/// The bytes of a located source's audio stream `index`, by one ffmpeg
/// run that copies its packets and decodes nothing.
pub fn audio_bytes<R: Runner>(
    runner: &R,
    located: &Located,
    index: u32,
    scratch: &Path,
) -> Result<u64> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let path = scratch.join("packets.crc");
    let outputs = [Output::new(packets_output(0, index), &path)];
    let ran = ffmpeg::run_outputs(runner, &[located.path.as_path()], &outputs);
    ran.into_iter().next().unwrap_or(Ok(()))?;
    read_packets(&path).with_context(|| format!("listing the packets of {}", located.key))
}

/// The options that write one subtitle stream as LRC. Without
/// `-map_metadata -1` the muxer writes every container tag as an LRC
/// header line, and bitexact drops its own version lines.
pub const LRC_ARGS: [&str; 6] = ["-map_metadata", "-1", "-fflags", "+bitexact", "-f", "lrc"];

/// Dump the info JSON, and the image attachments when `images`, into
/// `scratch`, and read the info JSON. A dump that fails costs those facts only.
fn dump_attachments<R: Runner>(
    runner: &R,
    located: &Located,
    probed: &Probed,
    scratch: &Path,
    images: bool,
) -> Option<VideoInfo> {
    let info_json = scratch.join("info.json");
    let mut dumps: Vec<(usize, PathBuf)> = probed
        .image_attachments()
        .filter(|_| images)
        .map(|a| (a.ordinal, scratch.join(format!("att{}", a.ordinal))))
        .collect();
    if let Some(a) = probed.attachment_named("info.json") {
        dumps.push((a.ordinal, info_json.clone()));
    }
    if dumps.is_empty() || probed.audio.is_none() {
        return None;
    }
    let _ = runner.run(&ffmpeg::dump_command(&[(located.path.as_path(), dumps)]));
    info::parse(&std::fs::read(&info_json).ok()?).ok()
}

#[cfg(test)]
mod tests;
