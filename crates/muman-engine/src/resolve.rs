//! What a song is made of: for each aspect, the source whose measures
//! say it is best. Audio and covers are scored by the song list's
//! `[audio.pick]` and `[cover.pick]`: each measure in whole steps, so noise never decides
//! between near-equals, times its weight, summed; the lowest score wins.
//! No rule names a kind of source; a pin in the song list is the only
//! override, and the order of `sources` breaks the last tie. A source
//! with a measure that could not be taken ranks after every source with
//! fewer such.
//!
//! - Audio: the song's recording before another ([`vet::recordings`]),
//!   then least that is not the song (a video's intro or skit) or
//!   is missing from it (past an excerpt's cut), then
//!   real stereo, the widest bandwidth, least clipping; scored alike,
//!   from the folder most of its album's audio comes from, then
//!   lossless, then the most bits used, then the lowest rate.
//!   Lossless audio cut off as lossy audio is counts as lossy, padded
//!   audio by the bits it uses, and upsampled audio by the rate it
//!   declares, which its bytes take: ranked by the rate its sound needs,
//!   a 96 kHz file made from a CD would beat a 48 kHz copy scored alike,
//!   at twice its size ([`crate::quality::Flags`]).
//! - Cover: the design its album's songs show ([`crate::album`]), then
//!   square content, then effective resolution, then fewest block
//!   artifacts; borders a video frame adds are cropped off, and a banner,
//!   more than twice as wide as tall or as tall as wide, is no cover.
//! - Lyrics: by `[lyrics] prefer`, their role and whether timed, then
//!   in the language read first, originals in none after those in one,
//!   from a source whose audio is the same recording as the chosen
//!   audio, covering most of it.
//! - Tags: each offer cleaned first; a structured field over one read
//!   off a title, plain over decorated, agreed on over alone, offers that
//!   mean one value agreeing as one ([`groups`]: alike in their letters,
//!   or names [`similar::agree`] reads alike that name the same
//!   versions); for credits, the kind of source `[tags] believe` puts
//!   first before decorations count; the value spelled as the source
//!   believed first among those that agree spells it, so a file's typo
//!   gives way to MusicBrainz's spelling while a file's `Song (Live)`
//!   stays apart from MusicBrainz's `Song`, and of two such the one
//!   naming the version wins whatever their order. A field won by the
//!   order of `sources` alone over another value is contested, which
//!   `status` and `check` name. The
//!   release fields come together from one source so a song's release
//!   never mixes with another's. Each field's [`tags::Scope`] says which
//!   it is. A source [`crate::vet`] finds names another recording gives
//!   nothing, no cover nor lyrics either, and one it is unsure of gives
//!   no release fields and no recording IDs, so a record of another song
//!   lends the song neither its album nor its IDs; where the release's
//!   source names its recording, its recording IDs are the song's,
//!   so a release's IDs and its recording's never contradict. With
//!   vetting off, a recording's ISRC and MusicBrainz IDs come only from a
//!   source that titles it as the song is titled, as before it. A
//!   source's comment is taken only with `[library] source_comments`:
//!   most are a store's or an uploader's words, not the song's. A value
//!   that says nothing of the song, a placeholder or a watermark, is
//!   dropped before any offer is ranked ([`crate::detect`]), watermarks
//!   found across the sources the song list names ([`Known`]). An
//!   upload's date, the day it went up, dates the song only where the
//!   upload's channel is the song's artist, set by hand or offered.
//!
//! The default weights rank as the order above does, each measure first
//! by a margin wider than everything after it can make up: a step of
//! purity outweighs mono and any bandwidth up to 240 kHz together, mono
//! any such bandwidth, and a step of bandwidth all of clipping. Mono
//! comes before bandwidth because it is the larger loss to a listener:
//! MP3 at 128 kbit/s measured 16 to 16.5 kHz and Opus at 128 kbit/s 20 to
//! 20.6 kHz ([`crate::quality`]), at least 7 steps apart, so ranked by
//! bandwidth first a mono copy in Opus would beat the stereo MP3. Lowering
//! a weight lets the measures after it trade against it.
//!
//! Purity is a source's sound beyond the song plus the song it lacks, as
//! [`crate::align`] measures them against each other source of the same
//! recording. Sound beyond is the least any other leaves unshared, since
//! another source holding it makes it the song; a source cut off where this
//! one plays on vouches for nothing, so it counts only when none other is
//! whole there. The song lacked is the most any other plays on past a cut.
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
//! The language a song is sung in is its `LANGUAGE` tag, resolved as any
//! tag from what its sources offer: a work's lyrics languages on
//! MusicBrainz, a file's own tag, the language YouTube says a video's
//! audio is in; else the language the audio file's own lyrics tag
//! names. A language read off an upload, as YouTube's, is the default
//! its uploader set, often the channel's own language rather than the
//! song's: measured on a library of about two thousand songs, five of the
//! ninety languages YouTube gave were Korean, Chinese or Thai for a song
//! titled in kana or released in Japanese script. Such a language is
//! taken only where the song's text could be in it, its title's letters
//! but a featured artist's and the script its release names
//! ([`crate::lang::Writing`]); a structured one is a record of the song
//! and is taken as it is.
//!
//! Lyrics in the language sung are the song's own
//! ([`crate::lang::Role`]); in its Latin letters, where it is written in
//! others, a romanization; in
//! another language, a translation, taken only into a language
//! `[lyrics] read` names. `[lyrics] prefer` lists the roles and timing
//! in the order a listener ranks them, each deciding only between lyrics
//! the ones before it rank alike, so one listener may put a translation
//! first and another timing; a role it leaves out is never taken. A
//! translation is preferred so the listener can read the lyrics, so the
//! song's own in a language read count as one too: ranked only by role,
//! a song sung in English would take its Chinese translation from a
//! listener who reads both. Lyrics
//! that say nothing of their language, as LRCLIB's and a `.lrc` file's,
//! are taken as the song's own. Where the language sung is not known, no
//! lyrics' role is, so the roles rank them alike, and only lyrics in a
//! language `read` names are taken, those that say nothing as its first.
//!
//! A media file's own lyrics tag is lyrics of that source, timed when
//! its text is LRC. One in a language no preference names, as a `USLT`
//! frame marked `jpn` where only `en` is read and the song's language is
//! not known, ranks after every language read rather than not at all.
//!
//! The release fields, album, album artist, track, disc, date, totals,
//! country and release IDs, come from the one source whose album ranks
//! best, so a release's MusicBrainz IDs never mix with another's; the
//! songs of one album are then made to agree, on one edition, one
//! spelling and one release's fields ([`crate::album`]), each resolved
//! again with what it takes from them ([`Input::pick`]). Of albums that rank alike,
//! the source holding the most release fields wins before the order of
//! `sources`, as a release's own record over an upload that names its
//! album alone. With no album, the song is a single named for its final
//! title, without release fields but a date.
//! The album artist is one a source names, else the first credited
//! artist as finally set, a hand-set one included, never the joined
//! `ARTIST`.
//!
//! A plan is stored whole and compared whole, which is how a song renders
//! again exactly when its sources' revisions, the picks, the tags or the
//! renderer change; [`render_version`] rises for a codec when the bytes a
//! plan in it renders to change, so only songs in that codec are written
//! again. Its gains are kept as `[loudness] mode` asks, so a plan that
//! [`crate::limit`] encodes into Opus holds them in its header, but are
//! compared as its format holds them: a header gain in FLAC is as tags
//! alone, so a mode that changes only Opus files writes only those again.

use crate::units;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::clean::{self, Albums, Settings};
use crate::codec::{Adapt, Chain, Codec, Container, Layout};
use crate::detect::{self, Watermarks};
use crate::facts::{AudioFacts, CoverAt, Facts, LyricsAt};
use crate::integrity::Damage;
use crate::lang::{Fit, Lang, Role, Writing};
use crate::loudness::Gains;
use crate::lyrics::{self, Language};
use crate::manifest::{Album, LyricsPin, Manifest, Song};
use crate::naming::{self, Naming};
use crate::picture::{self, Alike};
use crate::quality::{ImageQuality, Rect};
use crate::settings::{Audio, LyricsPlacement, Prefer, To};
use crate::similar;
use crate::source::SourceKey;
use crate::state::Aligned;
use crate::tags::{self, Field, Offer, Scope};
use crate::vet::recordings::{self, Recordings};
use crate::vet::{self, Verdict, Vets};

/// The renderer's version for plans in `codec`, raised whenever the bytes
/// such a plan renders to change, so the songs in it are rendered again.
/// A test pins each codec's to a digest of the arguments its audio is
/// written with.
#[must_use]
pub fn render_version(codec: Codec) -> u32 {
    match codec {
        // Opus ended 312 samples late, its timestamps shifted by its delay;
        // MP4 wrote a tag of several values as an atom each.
        Codec::Opus | Codec::Aac | Codec::Alac => 4,
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
    /// codec may be and its layout is taken as it is. A mix only the codec
    /// it would be encoded to asks, as MP3's two channels, keeps no audio
    /// from being copied.
    #[must_use]
    pub fn of(audio: &AudioFacts, settings: &Audio) -> Self {
        let encoded = Self::encoded(audio, settings);
        let (mix, _) = settings.written(&audio.shape());
        match Codec::probed(&audio.codec) {
            Some(codec)
                if mix.is_none()
                    && (codec == encoded.codec() || settings.codecs.contains(&codec)) =>
            {
                Self::Copy { codec }
            }
            _ => encoded,
        }
    }

    /// `audio` encoded to `codec` itself, where it holds the audio as it
    /// is, as a gain applied to audio otherwise copied asks; else as
    /// [`Self::encoded`] would.
    #[must_use]
    pub fn encoded_as(codec: Codec, audio: &AudioFacts, settings: &Audio) -> Self {
        let shape = audio.shape();
        match codec.adapt(&shape) {
            Some(adapt) => Self::Encode {
                codec,
                kbps: settings
                    .kbps(codec, shape.channels)
                    .map(|k| codec.kbps_at(k, shape.sample_rate)),
                adapt,
                mix: None,
            },
            None => Self::encoded(audio, settings),
        }
    }

    /// `audio` encoded under `settings`, whatever its codec: mixed into
    /// another layout where `[audio] layouts` takes not its own or the
    /// codec holds fewer channels ([`Audio::written_in`]), then to
    /// `[audio] lossy` or `lossless`, or where a lossless codec there
    /// cannot keep every channel and sample, to FLAC or else WavPack.
    #[must_use]
    pub fn encoded(audio: &AudioFacts, settings: &Audio) -> Self {
        let wanted = if audio.is_lossless() {
            settings.lossless
        } else {
            settings.lossy
        };
        let (mix, shape) = settings.written_in(wanted, &audio.shape());
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
            kbps: settings
                .kbps(codec, shape.channels)
                .map(|k| codec.kbps_at(k, shape.sample_rate)),
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
    /// [`crate::render::COVER_MAKING`] for a cover ffmpeg makes for any
    /// place, none for one written as its own bytes everywhere: a new way
    /// of making one writes again only the songs whose cover it makes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<u32>,
    /// Where it goes, where not inside the song alone, as its own bytes
    /// or PNG, as it went before `[[cover.out]]`: a plan of that reads as
    /// it did. Kept to the size it is measured at: a place's `max_side` is
    /// none where the picture is no longer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<Vec<CoverPlace>>,
}

impl CoverRef {
    /// Each place it goes.
    #[must_use]
    pub fn places(&self) -> Vec<CoverPlace> {
        self.out
            .clone()
            .unwrap_or_else(|| vec![CoverPlace::default()])
    }

    /// Where it goes inside the song, if it does.
    #[must_use]
    pub fn inside(&self) -> Option<CoverPlace> {
        self.places().into_iter().find(|p| p.file.is_none())
    }

    /// The files beside the song it goes to.
    #[must_use]
    pub fn files(&self) -> Vec<CoverPlace> {
        self.places()
            .into_iter()
            .filter(|p| p.file.is_some())
            .collect()
    }

    /// Whether ffmpeg makes the picture `place` takes, rather than its
    /// own bytes going there.
    #[must_use]
    pub fn encodes(&self, place: &CoverPlace) -> bool {
        !place.keep || crate::render::converts(&self.mimetype, self.crop.is_some())
    }

    /// It as the places `which` takes write it, alone; none where it
    /// takes none. Two plans whose covers go alike to those places write
    /// them alike.
    #[must_use]
    pub fn only(&self, which: impl Fn(&CoverPlace) -> bool) -> Option<Self> {
        let places: Vec<CoverPlace> = self.places().into_iter().filter(which).collect();
        if places.is_empty() {
            return None;
        }
        Some(Self {
            made: self.made.filter(|_| places.iter().any(|p| self.encodes(p))),
            out: Some(places),
            ..self.clone()
        })
    }
}

/// One place a cover goes, as a `[[cover.out]]` names it: the tag inside
/// the song, or a file beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverPlace {
    /// The suffix of the file beside the song; none for the tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub format: ImageFormat,
    /// Its own bytes where they are already `format`, or JPEG or PNG
    /// inside the song, uncropped and unscaled, rather than encoded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
    /// The longest side it is scaled down to, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_side: Option<u32>,
    /// How well a JPEG or WebP keeps it, from 1 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
}

impl Default for CoverPlace {
    /// Inside the song as its own bytes or PNG, as before `[[cover.out]]`.
    fn default() -> Self {
        Self {
            file: None,
            format: ImageFormat::Png,
            keep: true,
            max_side: None,
            quality: None,
        }
    }
}

pub use crate::picture::ImageFormat;

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
    /// Whether they go inside the song, and, unless [`Self::files`] names
    /// the files, whether in a `.lrc` beside it, as `[library] lyrics` said
    /// before `[[lyrics.out]]` did: a plan of those outputs reads as it did.
    pub placement: LyricsPlacement,
    /// The files beside the song they go to, where `placement` cannot say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<LyricsFile>>,
    /// How they are written inside the song, where not as LRC with its
    /// word times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside: Option<LyricsFile>,
    /// Where the song ends, in ms, which ends the last cue of a file of
    /// cues; none where no file holds cues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
    /// The ISO 639-2 code of the language their source names them in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The lyric cleaning rules switched off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clean_off: Vec<String>,
    /// The [`clean::lyrics::EDITION`] they are cleaned by; 0 before muman
    /// named one, so lyrics written then are cleaned again.
    #[serde(default)]
    pub cleaning: u32,
    /// The [`lyrics::WRITING`] they are read, moved and written by. The
    /// first is left out of a stored plan, so a plan stored before muman
    /// named one reads as it, and its plan key is the one it had.
    #[serde(default = "first_writing", skip_serializing_if = "is_first_writing")]
    pub writing: u32,
}

impl LyricsRef {
    /// These lyrics with no language: what their lines are, whatever
    /// their tags mark them as.
    #[must_use]
    pub fn lines(&self) -> Self {
        Self {
            language: None,
            ..self.clone()
        }
    }

    /// Whether they go inside the song.
    #[must_use]
    pub fn tag(&self) -> bool {
        self.placement.embedded()
    }

    /// The files beside the song they go to.
    #[must_use]
    pub fn files(&self) -> Vec<LyricsFile> {
        match &self.files {
            Some(files) => files.clone(),
            None if self.placement.sidecar() => vec![LyricsFile::default()],
            None => Vec::new(),
        }
    }

    /// Where lyrics going inside the song, or not, and to `files` are
    /// written, as `placement` and `files` say it: `files` only where
    /// `placement` cannot.
    #[must_use]
    pub fn placed(tag: bool, files: Vec<LyricsFile>) -> (LyricsPlacement, Option<Vec<LyricsFile>>) {
        let placement = match (tag, files.is_empty()) {
            (true, true) => LyricsPlacement::Embedded,
            (true, false) => LyricsPlacement::Both,
            (false, _) => LyricsPlacement::Sidecar,
        };
        let plain = files.is_empty() || files == [LyricsFile::default()];
        (
            placement,
            (!plain || !tag && files.is_empty()).then_some(files),
        )
    }
}

/// A file of lyrics beside a song, as a `[[lyrics.out]]` names it, or
/// the lyrics tag, which has no suffix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsFile {
    /// What follows the song's own name, before the extension.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suffix: String,
    #[serde(default)]
    pub format: crate::settings::LyricsFormat,
    /// Whether word times are kept.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub words: bool,
}

impl Default for LyricsFile {
    fn default() -> Self {
        Self {
            suffix: String::new(),
            format: crate::settings::LyricsFormat::Lrc,
            words: true,
        }
    }
}

fn yes() -> bool {
    true
}

fn first_writing() -> u32 {
    1
}

// serde hands `skip_serializing_if` a reference.
#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_first_writing(writing: &u32) -> bool {
    *writing == first_writing()
}

// serde hands `skip_serializing_if` a reference.
#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_yes(b: &bool) -> bool {
    *b
}

/// Everything a library file is made from. Stored beside each output and
/// compared whole, so a song is rendered again exactly when this changes;
/// its gains are compared as its format holds them ([`Gains::as_held`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub format: Format,
    pub audio: AudioRef,
    pub cover: Option<CoverRef>,
    pub lyrics: Option<LyricsRef>,
    /// Vorbis comments, in the order written.
    pub tags: Vec<(String, Vec<String>)>,
    /// The gains that level the song, before any is applied; the
    /// ReplayGain tags are derived from them and the format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<Gains>,
    /// How its tags are written into the file, [`TAGGING`] when it was.
    #[serde(default)]
    pub tagging: u32,
    /// Its audio's chapters, where its container holds them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<crate::chapters::Chapter>,
    /// The cover its folder holds once, for a song that holds it for the
    /// folder's other songs ([`crate::folder`]): each place a file named
    /// in the folder rather than after the song.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<CoverRef>,
}

/// How tags are written into a file, raised whenever the frames, atoms or
/// items one plan's tags are written as change, so every song is tagged
/// again, its audio left as it is. A test pins it to a digest of what a
/// build writes in each codec.
pub const TAGGING: u32 = 2;

impl PartialEq for Plan {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            version,
            format,
            audio,
            cover,
            lyrics,
            tags,
            loudness: _,
            tagging,
            chapters,
            folder,
        } = self;
        *version == other.version
            && *format == other.format
            && *audio == other.audio
            && *cover == other.cover
            && *lyrics == other.lyrics
            && *tags == other.tags
            && *tagging == other.tagging
            && *chapters == other.chapters
            && *folder == other.folder
            && self.held_loudness() == other.held_loudness()
    }
}

impl Eq for Plan {}

impl Plan {
    /// Its gains as its file holds them.
    #[must_use]
    pub fn held_loudness(&self) -> Option<Gains> {
        self.loudness
            .map(|g| g.as_held(self.format.codec() == Codec::Opus, self.format.is_encoded()))
    }

    /// This plan written in `format`, by the renderer's version for it,
    /// its loudness tags as that format holds its gains.
    #[must_use]
    pub fn with_format(&self, format: Format) -> Self {
        let plan = Self {
            version: render_version(format.codec()),
            format,
            ..self.clone()
        };
        match plan.loudness {
            Some(gains) => plan.with_loudness(gains),
            None => plan,
        }
    }

    /// This plan levelled by `gains`: its ReplayGain tags what is left of
    /// each gain once its format holds what it applies.
    #[must_use]
    pub fn with_loudness(&self, gains: Gains) -> Self {
        let mut plan = self.clone();
        plan.loudness = Some(gains);
        plan.tags
            .retain(|(k, _)| !Field::named(k).is_some_and(Field::is_loudness));
        for (field, value) in gains.tags(plan.applied()) {
            insert_comment(&mut plan.tags, field, vec![value]);
        }
        plan
    }

    /// This plan with no gains and no loudness tags: all it holds but how
    /// it is levelled.
    #[must_use]
    pub fn unlevelled(&self) -> Self {
        let mut plan = self.clone();
        plan.loudness = None;
        plan.tags
            .retain(|(k, _)| !Field::named(k).is_some_and(Field::is_loudness));
        plan
    }

    /// The ISO 639-2 code of the language its lyrics are in, if known.
    #[must_use]
    pub fn lyrics_language(&self) -> Option<&str> {
        self.lyrics.as_ref()?.language.as_deref()
    }

    /// The hundredths of a dB of its gain the file holds, by its header or
    /// in its samples.
    #[must_use]
    pub fn applied(&self) -> i32 {
        self.loudness.map_or(0, |g| {
            g.applied(self.format.codec() == Codec::Opus, self.format.is_encoded())
        })
    }

    /// What is done to the samples on their way to the encoder.
    #[must_use]
    pub fn chain(&self) -> Chain {
        match (self.format, self.loudness) {
            (Format::Encode { .. }, Some(g)) => Chain {
                gain: g.applied(false, true),
                bits: g.bits,
            },
            _ => Chain::default(),
        }
    }

    /// The gain the Opus header is set to once muxed, where it is.
    #[must_use]
    pub fn opus_header(&self) -> Option<i16> {
        self.loudness.and_then(|g| {
            g.header(
                self.format.codec() == Codec::Opus,
                !self.format.is_encoded(),
            )
        })
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
        for c in [plan.cover.as_mut(), plan.folder.as_mut()]
            .into_iter()
            .flatten()
            .filter(|c| c.key == *key)
        {
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
        for c in [plan.cover.as_mut(), plan.folder.as_mut()]
            .into_iter()
            .flatten()
        {
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
    /// Each source set aside ([`crate::vet`]), as `<key>: <why>`.
    pub set_aside: Vec<String>,
    /// Each value dropped for saying nothing of the song
    /// ([`crate::detect`]), as `<FIELD> “<value>” from <key>: <why>`.
    pub dropped: Vec<String>,
    /// Each field whose value won by the order of `sources` alone over
    /// another its sources offer.
    pub contested: Vec<String>,
    /// Each other audio source of another recording than the chosen
    /// audio's, or at another speed than the rest of its own
    /// ([`vet::recordings`]), as `<key>: <which>`.
    pub recordings: Vec<String>,
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

/// What resolving a song reads of the library's other sources: every
/// album they hold, for cleaning, and the watermarks of each source
/// group among the sources the song list names ([`crate::detect`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Known {
    pub albums: Albums,
    pub watermarks: Watermarks,
}

impl Known {
    /// The library `manifest` lists, of `facts`: albums from every source
    /// measured, as cleaning has always read them, and watermarks from the
    /// sources a song lists alone, so a source no song lists any more
    /// changes no pick.
    #[must_use]
    pub fn of(manifest: &Manifest, facts: &BTreeMap<SourceKey, Facts>) -> Self {
        let listed = manifest.keys();
        let groups: Vec<(String, Option<&str>, &tags::Offers)> = listed
            .iter()
            .filter_map(|key| {
                let name = match key {
                    SourceKey::Torrent(t) => manifest
                        .torrents
                        .get(t.hash())
                        .and_then(|t| t.name.as_deref()),
                    _ => None,
                };
                Some((group_of(key), name, &facts.get(key)?.tags))
            })
            .collect();
        Self {
            albums: Albums::of(facts.values().map(|f| &f.tags)),
            watermarks: Watermarks::of(groups.iter().map(|(group, name, offers)| detect::Source {
                group,
                name: *name,
                offers,
            })),
        }
    }
}

/// The group a source's files were shared in, which watermarks are told
/// within: its torrent, its folder under the manual folder, or its site.
fn group_of(key: &SourceKey) -> String {
    match key {
        SourceKey::Torrent(t) => format!("torrent:{}", t.hash()),
        SourceKey::Manual(m) => {
            let first = m.path().components().next();
            format!(
                "manual:{}",
                first.map_or_else(String::new, |c| {
                    c.as_os_str().to_string_lossy().into_owned()
                })
            )
        }
        SourceKey::Remote { site, .. } => site.clone(),
    }
}

/// Why a value `key` offers for `field` says nothing of the song, as a
/// placeholder or a watermark of its group, where the switch for it is on.
fn says_nothing(input: &Input<'_>, key: &SourceKey, field: Field, value: &str) -> Option<String> {
    if input.clean.is_on(clean::PLACEHOLDER) && detect::placeholder(field, value) {
        return Some("a placeholder".into());
    }
    if input.clean.is_on(clean::WATERMARK) && detect::PER_WORK.contains(&field) {
        let group = group_of(key);
        if input.known.watermarks.holds(&group, value) {
            return Some(format!("a watermark of {group}"));
        }
    }
    None
}

/// Each value of the song's sources that says nothing of it, as
/// `<FIELD> “<value>” from <key>: <why>`.
fn dropped(input: &Input<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for (_, key, facts) in input.present() {
        for (field, offer) in &facts.tags {
            if *field == Field::Comment && !input.source_comments {
                continue;
            }
            for value in &offer.values {
                if let Some(why) = says_nothing(input, key, *field, value) {
                    out.push(format!("{} “{value}” from {key}: {why}", field.vorbis()));
                }
            }
        }
    }
    out
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
    /// Which lyrics are taken, and in what order.
    pub lyrics: &'a crate::settings::Lyrics,
    pub clean: &'a Settings,
    pub typography: clean::Typography,
    /// Which kind of source is believed first.
    pub believe: clean::Believe,
    /// What the other songs' sources show: their albums and watermarks.
    pub known: &'a Known,
    pub naming: &'a Naming,
    pub audio: &'a Audio,
    pub cover: &'a crate::settings::Cover,
    /// Whether a comment its sources' tags carry is written.
    pub source_comments: bool,
    /// What the song takes from the other songs of its album
    /// ([`crate::album`]); none when resolved alone.
    pub pick: Option<&'a crate::album::Pick>,
}

impl Input<'_> {
    fn aligned(&self, a: &SourceKey, b: &SourceKey) -> Option<&Aligned> {
        let revs = (
            self.facts.get(a)?.audio_rev(),
            self.facts.get(b)?.audio_rev(),
        );
        self.alignments
            .iter()
            .find(|x| x.revs == revs && x.method == crate::align::METHOD)
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
    let mut candidates = candidates(input);
    let (audio_key, audio_why, recordings) = pick_audio(input, &candidates)?;
    let facts = &input.facts[&audio_key];
    let audio = facts.audio.as_ref().map_or(0, |a| a.index);
    let format = match facts.audio.as_ref() {
        // A file cut off breaks off mid-frame; copied, so would the song.
        Some(a) if matches!(facts.damage(), Some(Damage::Cut { .. })) => {
            Format::encoded(a, input.audio)
        }
        Some(a) => Format::of(a, input.audio),
        None => bail!("no source of this song has audio"),
    };
    let vets = vetted(input, &candidates, ms_of(facts));
    believe_vetted(&mut candidates, &vets);
    let cover = pick_cover(input, &vets);
    let (mut tags, mut tag_why, artists) =
        resolve_tags(input, &candidates, &audio_key, format.mix().is_some());
    let mut sung: Vec<Lang> = tags
        .iter()
        .find(|(k, _)| k == Field::Language.vorbis())
        .into_iter()
        .flat_map(|(_, v)| v)
        .filter_map(|code| Lang::parse(code))
        .collect();
    if sung.is_empty()
        && !tags.iter().any(|(k, _)| k == Field::Language.vorbis())
        && let Some(own) = own_lyrics_language(facts)
    {
        insert_comment(&mut tags, Field::Language, vec![own.iso639_2()]);
        tag_why.push(TagWhy {
            key: Field::Language.vorbis().to_string(),
            from: "the language of the audio file's own lyrics".to_string(),
            cleaned: Vec::new(),
        });
        sung.push(own);
    }
    let lyrics = pick_lyrics(input, &audio_key, &sung, &vets);
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
        original_date: get(Field::OriginalDate),
        track: get(Field::Track),
        disc: get(Field::Disc),
        composer: get(Field::Composer),
        bpm: get(Field::Bpm),
        key: get(Field::InitialKey),
        id: &id,
    })?;
    Ok(Resolved {
        plan: Plan {
            folder: None,
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
            loudness: None,
            tagging: TAGGING,
            chapters: if format.codec().container() == Container::WavPack {
                Vec::new()
            } else {
                let length_ms = facts.length().map(|s| (s * 1000.0).round() as u64);
                crate::chapters::within(&facts.chapters, length_ms)
            },
        },
        stem,
        why: Why {
            audio: audio_why,
            cover: cover.map(|(_, w)| w),
            lyrics: lyrics.map(|(_, w)| w),
            tags: tag_why,
            fit: None,
            set_aside: vets.set_aside(),
            recordings,
            dropped: dropped(input),
            contested: [Field::Title, Field::Artist]
                .iter()
                .filter_map(|f| contested(&candidates, *f))
                .collect(),
        },
    })
}

/// `candidates` as `vets` leaves them: what each source gives, a source
/// vetting is unsure of ranked after every kind.
fn believe_vetted(candidates: &mut BTreeMap<Field, Vec<Candidate<'_>>>, vets: &Vets) {
    for (field, all) in candidates {
        all.retain(|c| vets.gives(c.key, *field));
        for c in all
            .iter_mut()
            .filter(|c| vets.verdict(c.key) == Verdict::Unsure)
        {
            c.rank = UNSURE_RANK;
        }
    }
}

/// The vetting of the song's sources ([`crate::vet`]) by their cleaned
/// structured offers, the tags set by hand and `audio_ms`, the chosen
/// audio's length; none while `[tags.clean.structure] vetting` is off.
fn vetted(
    input: &Input<'_>,
    candidates: &BTreeMap<Field, Vec<Candidate<'_>>>,
    audio_ms: Option<i64>,
) -> Vets {
    if !input.clean.is_on(clean::VETTING) {
        return Vets::default();
    }
    let pinned = |key: &SourceKey| {
        input.song.cover.as_ref() == Some(key)
            || matches!(&input.song.lyrics, Some(LyricsPin::From(k)) if k == key)
    };
    let claims: Vec<vet::Claims<'_>> = input
        .present()
        .map(|(_, key, facts)| vet::Claims {
            key,
            offers: candidates
                .iter()
                .filter_map(|(field, all)| {
                    let c = all.iter().find(|c| c.key == key && c.offer.structured)?;
                    Some((*field, c.offer.values.as_slice()))
                })
                .collect(),
            vetted: facts.audio.is_none() && !pinned(key),
            length_ms: facts.length_ms,
        })
        .collect();
    let hand = input
        .album
        .iter()
        .flat_map(|a| &a.tags)
        .chain(&input.song.tags)
        .filter_map(|(name, values)| Some((Field::named(name)?, values.clone())))
        .collect();
    Vets::of(&claims, &hand, audio_ms)
}

/// How long a source's audio plays, in whole ms.
fn ms_of(facts: &Facts) -> Option<i64> {
    facts.length().map(|s| (s * 1000.0).round() as i64)
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

/// One of the song's audio sources as its measures rank it.
struct Scored<'a> {
    n: usize,
    key: &'a SourceKey,
    facts: &'a Facts,
    /// Its rank by its measures, before the order of `sources`.
    rank: Rank,
    /// The least sound beyond the song it plays against another source of
    /// its recording, where it aligns with one.
    beyond: Option<i64>,
    /// The most of the song another source plays on past its cut.
    lacks: i64,
}

/// The audio the song takes, why, and each other source the recording it
/// holds names ([`vet::recordings`]).
fn pick_audio(
    input: &Input<'_>,
    candidates: &BTreeMap<Field, Vec<Candidate<'_>>>,
) -> Result<(SourceKey, String, Vec<String>)> {
    let audible: Vec<(usize, &SourceKey, &Facts)> = input
        .present()
        .filter(|(_, _, f)| f.audio.is_some())
        .collect();
    if let Some(pin) = &input.song.audio
        && !audible.iter().any(|(_, k, _)| *k == pin)
    {
        bail!("the pinned audio, {pin}, has no audio on disk");
    }
    // Audio a source lacks counts against it as audio another source shows
    // it lacks does; where one of the song's sources lacks some, each one
    // compared with none is scored by its own, so an intact copy wins.
    let any_missing = audible
        .iter()
        .any(|(_, _, f)| matches!(f.damage(), Some(Damage::Missing { .. })));
    let scored: Vec<Scored<'_>> = audible
        .iter()
        .map(|&(n, key, facts)| {
            let compared: Vec<&Aligned> = audible
                .iter()
                .filter(|(_, other, _)| *other != key)
                .filter_map(|(_, other, _)| input.aligned(key, other).filter(|a| a.fits()))
                .collect();
            let beyond = compared
                .iter()
                .filter(|a| a.b_lacks_ms == 0)
                .map(|a| a.unmatched_ms())
                .min()
                .or_else(|| compared.iter().map(|a| a.unmatched_ms()).min());
            let lacks = compared.iter().map(|a| a.a_lacks_ms).max().unwrap_or(0);
            let missing = facts.damage().map_or(0, |d| d.missing_ms());
            let unmatched = beyond
                .map(|ms| ms + lacks.max(missing))
                .or_else(|| any_missing.then_some(missing));
            let q = facts.audio.as_ref().and_then(|a| a.quality);
            let w = &input.audio.pick;
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
                i64::from(!from_album(input, key)),
                -tie_break(facts.audio.as_ref()),
                facts.audio.as_ref().map_or(0, |a| i64::from(a.sample_rate)),
            ];
            Scored {
                n,
                key,
                facts,
                rank,
                beyond,
                lacks,
            }
        })
        .collect();
    let recordings = recordings_of(input, candidates, &scored);
    if let Some(pin) = &input.song.audio {
        let why = if recordings.several() {
            format!("pinned, {}", recordings.describe(pin))
        } else {
            "pinned".to_string()
        };
        return Ok((pin.clone(), why, recordings.others(pin)));
    }
    let apart = |s: &Scored<'_>| {
        if input.audio.pick.recording.enabled {
            recordings.rank(s.key)
        } else {
            0
        }
    };
    let Some(chosen) = scored
        .iter()
        .min_by(|a, b| (apart(a), &a.rank, a.n).cmp(&(apart(b), &b.rank, b.n)))
    else {
        bail!(
            "no source of this song has audio that reads: {}",
            unheard(input).join("; ")
        );
    };
    let why = audio_why(input, chosen, &recordings);
    Ok((chosen.key.clone(), why, recordings.others(chosen.key)))
}

/// Whether `key`'s file came from the folder most of the song's album's
/// audio comes from ([`crate::album::consistent`]).
fn from_album(input: &Input<'_>, key: &SourceKey) -> bool {
    input
        .pick
        .and_then(|p| p.audio_from.as_deref())
        .is_some_and(|from| crate::album::origin_of(key).as_deref() == Some(from))
}

/// Whether a picture's content is more than twice as wide as tall, or as
/// tall as wide: a banner or a strip, no album's cover.
fn is_banner(content: Rect) -> bool {
    let (w, h) = (u64::from(content.width), u64::from(content.height));
    w > 2 * h || h > 2 * w
}

/// The song's audio sources in recordings ([`vet::recordings`]), held to
/// the records vetting believes on their names alone, a record's length
/// being what the recordings are held to; none for a song with one.
fn recordings_of(
    input: &Input<'_>,
    candidates: &BTreeMap<Field, Vec<Candidate<'_>>>,
    scored: &[Scored<'_>],
) -> Recordings {
    let best = scored.iter().map(|s| &s.rank).min();
    let audible: Vec<recordings::Audible<'_>> = scored
        .iter()
        .map(|s| recordings::Audible {
            key: s.key,
            ms: ms_of(s.facts),
            digest: s.facts.audio.as_ref().and_then(|a| a.digest.as_deref()),
            first: Some(&s.rank) == best,
        })
        .collect();
    if audible.len() < 2 {
        return Recordings::of(&audible, &[], |_, _| None);
    }
    let ids = |facts: &Facts| -> Vec<String> {
        facts
            .tags
            .get(&Field::MusicBrainzTrackId)
            .into_iter()
            .flat_map(|o| o.values.iter().map(|id| id.trim().to_lowercase()))
            .collect()
    };
    let named: BTreeSet<String> = input
        .present()
        .filter(|(_, key, facts)| key.file().is_some() && facts.audio.is_some())
        .flat_map(|(_, _, facts)| ids(facts))
        .collect();
    let believed = vetted(input, candidates, None);
    let records: Vec<recordings::Record<'_>> = input
        .present()
        .filter(|(_, key, facts)| facts.audio.is_none() && believed.verdict(key) == Verdict::Same)
        .filter_map(|(_, key, facts)| {
            Some(recordings::Record {
                key,
                ms: i64::try_from(facts.length_ms?).ok()?,
                named: ids(facts).iter().any(|id| named.contains(id)),
            })
        })
        .collect();
    Recordings::of(&audible, &records, |a, b| input.aligned(a, b))
}

/// Why `chosen` is the song's audio: what it plays beyond the song or
/// lacks of it, else which recording it holds, then how it measures.
fn audio_why(input: &Input<'_>, chosen: &Scored<'_>, recordings: &Recordings) -> String {
    let (facts, lacks) = (chosen.facts, chosen.lacks);
    // A song's milliseconds are far below 2^52, held exactly.
    let tenths = |ms: i64| (units::seconds_of_ms(ms as f64) * 10.0).floor() / 10.0;
    let heard = match chosen.beyond {
        None => recordings.describe(chosen.key),
        Some(ms) => {
            let beyond = match lacks {
                0 => format!("{:.1} s of sound beyond the song", tenths(ms)),
                _ => format!(
                    "{:.1} s of sound beyond the song, {:.1} s of the song missing",
                    tenths(ms),
                    tenths(lacks)
                ),
            };
            match recordings.aside(chosen.key) {
                Some(aside) => format!("{beyond}, {aside}"),
                None => beyond,
            }
        }
    };
    let why = match facts
        .audio
        .as_ref()
        .zip(facts.audio.as_ref().and_then(|a| a.quality))
    {
        Some((audio, q)) => [format!(
            "{heard}, {:.1} kHz, {}, {:.2}% clipped",
            units::khz_of_hz(q.bandwidth_hz),
            speakers(
                Some(audio),
                q.is_stereo(input.audio.pick.stereo.incoherence)
            ),
            q.clipping * 100.0
        )]
        .into_iter()
        .chain(flagged(audio))
        .collect::<Vec<_>>()
        .join(", "),
        None => "not measured".to_string(),
    };
    let why = if from_album(input, chosen.key) {
        format!("{why}, from its album's folder")
    } else {
        why
    };
    match facts.damage() {
        Some(Damage::Cut { ends, .. }) => format!("{why}, cut off at {ends:.0} s"),
        Some(Damage::Missing { seconds, .. }) => format!("{why}, {seconds:.1} s missing"),
        Some(Damage::Short { ends, .. }) => format!("{why}, cut short at {ends:.0} s"),
        None => why,
    }
}

/// Why each of the song's sources gives it no audio: gone from the
/// sources, not read, or holding none.
fn unheard(input: &Input<'_>) -> Vec<String> {
    input
        .song
        .sources
        .iter()
        .map(|k| {
            if input.on_disk.is_some_and(|d| !d.contains(k)) {
                format!("{k} is not in the sources, or does not read")
            } else {
                match input.facts.get(k) {
                    None => format!("{k} could not be read"),
                    Some(f) if f.audio.is_none() => format!("{k} holds no audio"),
                    Some(_) => format!("{k} reads"),
                }
            }
        })
        .collect()
}

/// What breaks a tie between sources the measures score alike, most
/// first: lossless audio, then the bits it uses, so a padded file counts
/// the bits it uses rather than those it declares; lossless audio cut off
/// as lossy audio is was made from it and counts as lossy
/// ([`crate::quality::Flags`]). A rate past what its bandwidth needs holds
/// nothing more, so of two the lower wins next.
fn tie_break(audio: Option<&AudioFacts>) -> i64 {
    audio
        .filter(|a| a.is_lossless() && !a.flags().band_limited)
        .map_or(0, |a| 1 + i64::from(a.quality.map_or(0, |q| q.bits)))
}

/// What lossless `audio` holds less of than it declares, in words:
/// `96 kHz, upsampled: nothing above 22.1 kHz`, `lossless, cut off at
/// 16.0 kHz as lossy audio is`, `24-bit, 16 used`.
fn flagged(audio: &AudioFacts) -> Vec<String> {
    let (flags, Some(q)) = (audio.flags(), audio.quality) else {
        return Vec::new();
    };
    let khz = units::khz_of_hz(q.bandwidth_hz);
    let rate = units::khz_of_hz(f64::from(audio.sample_rate));
    let rate = if rate.fract() == 0.0 {
        format!("{rate:.0}")
    } else {
        format!("{rate:.1}")
    };
    let upsampled = format!("{rate} kHz, upsampled: nothing above {khz:.1} kHz");
    let band_limited = format!("lossless, cut off at {khz:.1} kHz as lossy audio is");
    let padded = format!("{}-bit, {} used", audio.bits, q.bits);
    [
        (flags.upsampled, upsampled),
        (flags.band_limited, band_limited),
        (flags.padded, padded),
    ]
    .into_iter()
    .filter_map(|(on, said)| on.then_some(said))
    .collect()
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

/// The language the lyrics tag of the audio's own file names, as a
/// `USLT` frame does: a file's own lyrics are the song's as someone kept
/// it, so most likely in the language it is sung in.
fn own_lyrics_language(facts: &Facts) -> Option<Lang> {
    facts
        .lyrics
        .iter()
        .find(|l| l.at == LyricsAt::Tag)?
        .language
        .langs()
        .into_iter()
        .next()
}

/// How lyrics fit a song and its listener.
struct Fitting {
    /// What they are to the song, where the language it is sung in is known.
    role: Option<Role>,
    /// Whether they are the song's own lyrics in a language read, which
    /// a listener reads as well as any translation.
    read_original: bool,
    /// Where they rank by the languages read, least first.
    rank: i64,
}

/// How lyrics in `language` fit a song sung in `sung`, if that is known,
/// and a listener who reads `read`: `None` for lyrics this song does not
/// take. Where the language sung is not known, a file's own lyrics tag in
/// no language read ranks after every language read rather than not at
/// all: a subtitle is one of many a site offers, but a file's own lyrics
/// are what someone chose to keep. A translation is taken only into a
/// language read; the song's own lyrics in no language read rank after
/// those in one.
fn lyrics_fit(sung: &[Lang], read: &[Lang], language: &Language, at: &LyricsAt) -> Option<Fitting> {
    let wide = |n: usize| i64::try_from(n).unwrap_or(UNKNOWN);
    let unread = wide(read.len()) * 2;
    let read_rank = |language: &Language| {
        lyrics::preference(read, language).map(|(n, fit)| wide(n) * 2 + i64::from(fit == Fit::Near))
    };
    if sung.is_empty() {
        let kept = (*at == LyricsAt::Tag && !read.is_empty()).then_some(unread);
        return Some(Fitting {
            role: None,
            read_original: false,
            rank: read_rank(language).or(kept)?,
        });
    }
    // Lyrics that say nothing of their language are taken as the song's.
    let (role, rank) = if *language == Language::Unstated {
        let codes = sung.iter().map(ToString::to_string).collect();
        (Role::Original, read_rank(&Language::Codes(codes)))
    } else {
        let role = language
            .langs()
            .iter()
            .flat_map(|l| sung.iter().map(|s| l.role(s)))
            .min()?;
        (role, read_rank(language))
    };
    match role {
        Role::Original => Some(Fitting {
            role: Some(role),
            read_original: rank.is_some(),
            rank: rank.unwrap_or(unread),
        }),
        Role::Romanization => Some(Fitting {
            role: Some(role),
            read_original: false,
            rank: 0,
        }),
        Role::Translation => Some(Fitting {
            role: Some(role),
            read_original: false,
            rank: rank?,
        }),
    }
}

/// Where lyrics fitting as `fit` and `timed` or not rank by `prefer`, a
/// place for each entry: 0 where they have its trait, else 1. A
/// translation is preferred so the listener can read the lyrics, so the
/// song's own in a language read have its trait too. Lyrics whose role
/// is not known have every role's trait alike.
fn preferred(prefer: &[Prefer], fit: &Fitting, timed: bool) -> Vec<i64> {
    prefer
        .iter()
        .map(|p| {
            let has = match (p, p.role()) {
                (Prefer::Timed, _) => timed,
                (Prefer::Untimed, _) => !timed,
                (Prefer::Translation, _) if fit.read_original => true,
                (_, wanted) => fit.role.is_none() || fit.role == wanted,
            };
            i64::from(!has)
        })
        .collect()
}

fn pick_cover(input: &Input<'_>, vets: &Vets) -> Option<(CoverRef, String)> {
    let mut order = 0_i64;
    let mut best: Option<(Rank, CoverRef, String)> = None;
    for (_, key, facts) in input.present() {
        if input.song.cover.as_ref().is_some_and(|pin| pin != key)
            || vets.verdict(key) == Verdict::Other
        {
            continue;
        }
        for cover in &facts.covers {
            order += 1;
            let q = cover.quality;
            if q.is_some_and(|q| is_banner(q.content)) {
                continue;
            }
            let w = &input.cover.pick;
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
            let album_s = input.pick.and_then(|p| p.cover.as_ref()).map(|design| {
                cover
                    .look
                    .is_some_and(|l| picture::alike(&l, design) == Alike::Same)
            });
            // A back cover, a disc or a booklet is no song's cover while
            // any other picture is.
            let rank: Rank = vec![
                i64::from(cover.other),
                i64::from(album_s == Some(false)),
                unknown,
                sum,
                order,
            ];
            if best.as_ref().is_some_and(|(r, _, _)| *r <= rank) {
                continue;
            }
            let mut why = q.map_or_else(
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
            if album_s == Some(true) {
                why.push_str(", the design its album's songs show");
            }
            best = Some((
                rank,
                {
                    let mut made = CoverRef {
                        key: key.clone(),
                        rev: facts.cover_rev(cover),
                        at: cover.at.clone(),
                        mimetype: cover.mimetype.clone(),
                        crop: q.filter(ImageQuality::is_cropped).map(|q| q.content),
                        made: None,
                        out: song_places(input.cover, &cover.mimetype, q),
                    };
                    made.made = made
                        .places()
                        .iter()
                        .any(|p| made.encodes(p))
                        .then_some(crate::render::COVER_MAKING);
                    made
                },
                why,
            ));
        }
    }
    // A cover no output takes is written nowhere; one a folder's file
    // alone takes is the song's all the same, for its folder to take.
    best.filter(|_| !input.cover.out.is_empty())
        .map(|(_, c, w)| (c, w))
}

/// `cover` as the places `places` write it, made by ffmpeg where any
/// of them is.
#[must_use]
pub(crate) fn placed(cover: &CoverRef, places: Vec<CoverPlace>) -> CoverRef {
    let made = places
        .iter()
        .any(|p| cover.encodes(p))
        .then_some(crate::render::COVER_MAKING);
    CoverRef {
        out: Some(places),
        made,
        ..cover.clone()
    }
}

/// Where a song's cover of `mimetype`, measured as `q` if it was, goes,
/// as the song's `[[cover.out]]` say, in a plan's terms: none where it
/// goes inside the song as it went before them.
fn song_places(
    cover: &crate::settings::Cover,
    mimetype: &str,
    q: Option<ImageQuality>,
) -> Option<Vec<CoverPlace>> {
    let places: Vec<CoverPlace> = cover
        .of(crate::settings::Scope::Song)
        .map(|o| place(o, mimetype, q, o.suffix.clone()))
        .collect();
    (places != [CoverPlace::default()]).then_some(places)
}

/// The place `out` gives a cover of `mimetype`, measured as `q` if it
/// was, a file of it named by `file`. A place keeping it as it is keeps a
/// JPEG or a PNG without borders as its own bytes, and makes any other
/// PNG; a file of it takes the extension of the picture it is, so its
/// name is known before it is written. A picture longer than the place's
/// `max_side` is scaled down there, in its own format where kept; one not
/// measured is scaled as one that may be, never up.
pub(crate) fn place(
    out: &crate::settings::CoverOut,
    mimetype: &str,
    q: Option<ImageQuality>,
    file: String,
) -> CoverPlace {
    use crate::settings::CoverFormat;
    let cropped = q.is_some_and(|q| q.is_cropped());
    let own = if mimetype == "image/jpeg" && !cropped {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Png
    };
    let side = q.map(|q| q.content.width.max(q.content.height));
    let max_side = out.max_side.filter(|&m| side.is_none_or(|s| s > m));
    let (format, keep) = match out.format {
        CoverFormat::Keep if max_side.is_some() => (own, false),
        // Inside the song, what is kept is its own bytes or PNG.
        CoverFormat::Keep if out.to == To::Tag => (ImageFormat::Png, true),
        CoverFormat::Keep => (own, true),
        CoverFormat::Jpeg => (ImageFormat::Jpeg, false),
        CoverFormat::Png => (ImageFormat::Png, false),
        CoverFormat::Webp => (ImageFormat::Webp, false),
    };
    let lossy = matches!(format, ImageFormat::Jpeg | ImageFormat::Webp);
    CoverPlace {
        file: (out.to == To::File).then_some(file),
        format,
        keep,
        max_side,
        quality: (lossy && !keep).then_some(out.quality),
    }
}

/// How far a stated length may be from the audio's before lyrics timed
/// to it are no use.
pub const STATED_SPAN_MS: f64 = 4000.0;
/// A stated length that agrees proves less than an audio comparison, so
/// it never ranks over a subtitle of the same recording.
const STATED_MAX: f64 = 0.9;

/// How many tenths of audio `duration_ms` long lyrics of `timing` span;
/// none for untimed lyrics.
fn coverage(timing: Option<lyrics::Timing>, duration_ms: f64) -> i64 {
    timing.map_or(0, |t| {
        if duration_ms > 0.0 {
            ((t.last_ms - t.first_ms) as f64 / duration_ms * 10.0).floor() as i64
        } else {
            0
        }
    })
}

/// The confidence lyrics stating a length are the chosen audio's.
fn stated_confidence(stated_ms: i64, audio_ms: f64) -> f64 {
    let gap = (stated_ms as f64 - audio_ms).abs();
    STATED_MAX * (1.0 - gap / STATED_SPAN_MS).max(0.0)
}

fn pick_lyrics(
    input: &Input<'_>,
    audio: &SourceKey,
    sung: &[Lang],
    vets: &Vets,
) -> Option<(LyricsRef, String)> {
    let pin = match &input.song.lyrics {
        Some(LyricsPin::None) => return None,
        Some(LyricsPin::From(k)) => Some(k),
        None => None,
    };
    let duration_ms = input
        .facts
        .get(audio)
        .and_then(Facts::length)
        .map_or(0.0, units::ms_of_seconds);
    let mut best: Option<(Rank, LyricsRef, String)> = None;
    for (n, key, facts) in input.present() {
        if pin.is_some_and(|p| p != key) || vets.verdict(key) == Verdict::Other {
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
        // A subtitle of only `♪` writes no lyrics, so is none.
        for l in facts.lyrics.iter().filter(|l| l.sings_any()) {
            let read = &input.lyrics.read;
            let Some(fit) = lyrics_fit(sung, read, &l.language, &l.at) else {
                continue;
            };
            if fit.role.is_some_and(|r| !input.lyrics.takes(r)) {
                continue;
            }
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
            let mut rank: Rank = preferred(&input.lyrics.prefer, &fit, l.timing.is_some());
            rank.extend([
                fit.rank,
                -((confidence * 20.0).floor() as i64),
                -coverage(l.timing, duration_ms),
                i64::try_from(n).unwrap_or(UNKNOWN),
            ]);
            if best.as_ref().is_some_and(|(r, _, _)| *r <= rank) {
                continue;
            }
            let timing = match l.timing {
                None => "untimed".to_string(),
                Some(t) => format!(
                    "timed, {} line(s), {} the chosen audio",
                    t.lines,
                    if key == audio {
                        "from".to_string()
                    } else {
                        format!("{offset} ms from")
                    }
                ),
            };
            let (placement, files, inside, end_ms) =
                lyrics_outputs(input, audio, l.timing.is_some());
            let why = match fit.role {
                Some(Role::Original) | None => timing,
                Some(Role::Romanization) => format!("a romanization, {timing}"),
                Some(Role::Translation) => format!("a translation, {timing}"),
            };
            best = Some((
                rank,
                LyricsRef {
                    key: key.clone(),
                    rev: facts.lyrics_rev(l),
                    at: l.at.clone(),
                    shift_ms: offset - input.song.lyrics_offset_ms,
                    stretch_ppm,
                    placement,
                    files,
                    inside,
                    end_ms,
                    language: l.language.langs().first().map(Lang::iso639_2),
                    clean_off: clean::lyrics::off(input.clean),
                    cleaning: clean::lyrics::EDITION,
                    writing: lyrics::WRITING,
                },
                why,
            ));
        }
    }
    // Lyrics no output takes are written nowhere.
    best.filter(|(_, l, _)| l.tag() || !l.files().is_empty())
        .map(|(_, l, w)| (l, w))
}

/// Where a song's lyrics `timed` or not go, as the `[[lyrics.out]]` taking
/// them say, in a plan's terms: the placement and files of
/// [`LyricsRef::placed`], how they are written inside the song, and
/// where the song of `audio` ends, for a file of cues.
fn lyrics_outputs(
    input: &Input<'_>,
    audio: &SourceKey,
    timed: bool,
) -> (
    LyricsPlacement,
    Option<Vec<LyricsFile>>,
    Option<LyricsFile>,
    Option<i64>,
) {
    let outs = input
        .lyrics
        .out
        .iter()
        .filter(|o| o.when.takes(timed) && (timed || !o.format.cues()));
    let form = |o: &crate::settings::LyricsOut| LyricsFile {
        suffix: o.suffix.clone(),
        format: o.format,
        words: o.words,
    };
    let tag = outs.clone().find(|o| o.to == To::Tag);
    let files: Vec<LyricsFile> = outs.filter(|o| o.to == To::File).map(form).collect();
    let cues = files.iter().any(|f| f.format.cues());
    let end_ms = input
        .facts
        .get(audio)
        .and_then(Facts::length)
        .filter(|_| cues)
        .map(|s| (s * 1000.0).round() as i64);
    let inside = tag.map(form).filter(|f| *f != LyricsFile::default());
    let (placement, files) = LyricsRef::placed(tag.is_some(), files);
    (placement, files, inside, end_ms)
}

/// What one source offers for a field, as cleaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub key: SourceKey,
    pub values: Vec<String>,
    /// The cleaning rules that changed it.
    pub cleaned: Vec<&'static str>,
}

/// What each of the song's sources offers for each field, cleaned as the
/// song's tags are, in the order of its sources.
#[must_use]
pub fn offers(input: &Input<'_>) -> BTreeMap<Field, Vec<Offered>> {
    candidates(input)
        .into_iter()
        .map(|(field, all)| {
            let offered = all
                .into_iter()
                .map(|c| Offered {
                    key: c.key.clone(),
                    values: c.offer.values,
                    cleaned: c.cleaned,
                })
                .collect();
            (field, offered)
        })
        .collect()
}

/// One source's offer for a field, cleaned.
struct Candidate<'a> {
    n: usize,
    key: &'a SourceKey,
    offer: Offer,
    cleaned: Vec<&'static str>,
    /// As the source offered it.
    raw: &'a Offer,
    /// How far `[tags] believe` believes its kind of source, least first.
    rank: u8,
    /// Whether offers that mean one value group, as every `believe` but
    /// `sources` asks.
    grouping: bool,
}

/// The group `i` joins, its members' first.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// A source's rank once vetting is unsure of it: after every kind.
const UNSURE_RANK: u8 = 4;

/// How far `believe` believes `key`'s kind of source, least first: a
/// MusicBrainz record, a file's own tags, a site's release, an upload.
fn believed(believe: clean::Believe, key: &SourceKey, facts: &Facts) -> u8 {
    let kind = match key {
        SourceKey::Remote { site, .. } if site == crate::provider::MUSICBRAINZ => 0,
        SourceKey::Torrent(_) | SourceKey::Manual(_) => 1,
        SourceKey::Remote { .. } if facts.release => 2,
        SourceKey::Remote { .. } => 3,
    };
    match believe {
        clean::Believe::MusicbrainzFirst => kind,
        clean::Believe::FilesFirst => [1, 0, 2, 3][usize::from(kind)],
        clean::Believe::Sources => 0,
    }
}

/// The fields whose offers are names, which group by [`similar::agree`]
/// as well as by their letters.
const NAMED: [Field; 5] = [
    Field::Title,
    Field::Artist,
    Field::Artists,
    Field::Album,
    Field::AlbumArtist,
];

/// Which of `offers` mean one value, as a group number each: offers alike
/// in their letters, and for a name, two [`similar::agree`] reads alike
/// that name the same versions, so `Song (Live)` keeps apart from `Song`.
/// Groups are connected, so the order offers come in changes none.
fn groups(offers: &[&Candidate<'_>], field: Field) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..offers.len()).collect();
    let letters: Vec<String> = offers
        .iter()
        .map(|c| tags::normalized(&c.offer.values))
        .collect();
    let by_name = NAMED.contains(&field) && offers.iter().all(|c| c.grouping);
    let names: Vec<Option<(similar::Name, BTreeSet<&'static str>)>> = offers
        .iter()
        .map(|c| {
            by_name.then(|| {
                let joined = c.offer.values.join("; ");
                (similar::Name::new(&joined), similar::versions_in(&joined))
            })
        })
        .collect();
    for i in 0..offers.len() {
        for j in i + 1..offers.len() {
            let alike = letters[i] == letters[j]
                || matches!((&names[i], &names[j]), (Some((a, va)), Some((b, vb)))
                    if va == vb && similar::agree(a, b) == Some(true));
            if alike {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    (0..offers.len()).map(|i| root(&mut parent, i)).collect()
}

/// Every source's offer for every field, cleaned against the names the
/// song's sources and its hand-set tags credit.
fn candidates<'a>(input: &'a Input<'_>) -> BTreeMap<Field, Vec<Candidate<'a>>> {
    let credited = |name: &str| {
        matches!(
            Field::named(name),
            Some(Field::Artist | Field::Artists | Field::AlbumArtist)
        )
    };
    let hand_set = input
        .song
        .tags
        .iter()
        .chain(input.album.iter().flat_map(|a| &a.tags))
        .filter(|(k, _)| credited(k))
        .flat_map(|(_, v)| v);
    let offered = input
        .present()
        .flat_map(|(_, _, f)| {
            [Field::Artist, Field::Artists, Field::AlbumArtist].map(|field| f.tags.get(&field))
        })
        .flatten()
        .flat_map(|o| &o.values);
    let names = clean::names(offered.chain(hand_set).map(String::as_str));
    let mut all: BTreeMap<Field, Vec<Candidate<'a>>> = BTreeMap::new();
    for (n, key, facts) in input.present() {
        let ctx = clean::Context {
            names: &names,
            albums: &input.known.albums,
            offers: &facts.tags,
            typography: input.typography,
        };
        let edition = input
            .pick
            .and_then(|p| p.edition.as_ref())
            .filter(|(k, _)| k == key)
            .and_then(|(_, at)| facts.editions.get(*at));
        let own = facts
            .tags
            .iter()
            .filter(|(field, _)| edition.is_none() || field.scope() != Scope::Release);
        for (field, raw) in own.chain(edition.into_iter().flatten()) {
            if *field == Field::Comment && !input.source_comments {
                continue;
            }
            let c = clean::clean(*field, raw, &ctx, input.clean);
            let values: Vec<String> = (c.values.into_iter())
                .filter(|v| says_nothing(input, key, *field, v).is_none())
                .collect();
            if values.is_empty() {
                continue;
            }
            all.entry(*field).or_default().push(Candidate {
                n,
                key,
                offer: Offer {
                    values,
                    structured: raw.structured,
                },
                cleaned: c.rules,
                raw,
                rank: believed(input.believe, key, facts),
                grouping: input.believe != clean::Believe::Sources,
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
    best_offer_then(candidates, field, from, |_| ())
}

/// The source of the best album: offers alike in name rank by the release
/// fields their source holds, most first, so a source holding the whole
/// release wins over one naming its album alone.
fn best_album<'c, 'a>(
    candidates: &'c BTreeMap<Field, Vec<Candidate<'a>>>,
) -> Option<&'c Candidate<'a>> {
    let holds = |key: &SourceKey| {
        Field::of(Scope::Release)
            .filter(|f| {
                candidates
                    .get(f)
                    .into_iter()
                    .flatten()
                    .any(|c| c.key == key && !c.offer.values.is_empty())
            })
            .count()
    };
    best_offer_then(candidates, Field::Album, &|_| true, |key| {
        std::cmp::Reverse(holds(key))
    })
    .filter(|c| c.offer.structured)
}

/// The album of the source the release fields come whole from: the one
/// giving the edition its album's songs share, else the best album.
fn release_of<'c, 'a>(
    input: &Input<'_>,
    candidates: &'c BTreeMap<Field, Vec<Candidate<'a>>>,
) -> Option<&'c Candidate<'a>> {
    match input.pick.and_then(|p| p.edition.as_ref()) {
        Some((key, _)) => candidates
            .get(&Field::Album)
            .and_then(|all| all.iter().find(|c| c.key == key)),
        None => best_album(candidates),
    }
}

/// [`best_offer`], with offers that rank alike ranked by `then`, least
/// first, before the order of `sources` breaks the last tie.
fn best_offer_then<'c, 'a, K: Ord>(
    candidates: &'c BTreeMap<Field, Vec<Candidate<'a>>>,
    field: Field,
    from: &dyn Fn(&SourceKey) -> bool,
    then: impl Fn(&SourceKey) -> K,
) -> Option<&'c Candidate<'a>> {
    let offers: Vec<&Candidate<'a>> = candidates
        .get(&field)?
        .iter()
        .filter(|c| from(c.key))
        .collect();
    let group = groups(&offers, field);
    let ranked = ranked(&offers, &group, field, &then);
    let best = *ranked.first()?;
    let spelled = (0..offers.len())
        .filter(|&i| group[i] == group[best])
        .min_by_key(|&i| {
            let c = offers[i];
            (!c.offer.structured, c.rank, decorated(c), then(c.key), c.n)
        })?;
    Some(offers[spelled])
}

/// How much packaging an offer's values carry ([`tags::decorations`]).
fn decorated(c: &Candidate<'_>) -> usize {
    c.offer.values.iter().map(|v| tags::decorations(v)).sum()
}

/// How `offers[i]` ranks, before the order of `sources`: a field kept for
/// it over one read off a title; for credits, the kind of source `[tags]
/// believe` puts first; plain over decorated; agreed on by more offers of
/// its `group`; as `then` ranks its source; a source vetting is unsure of
/// last; and, where offers group, one naming more versions first, so a
/// file's `Song (Live)` keeps its version against MusicBrainz's `Song`.
fn rank_of<K: Ord>(
    offers: &[&Candidate<'_>],
    group: &[usize],
    field: Field,
    then: &impl Fn(&SourceKey) -> K,
    i: usize,
) -> (
    bool,
    u8,
    usize,
    std::cmp::Reverse<usize>,
    K,
    bool,
    std::cmp::Reverse<usize>,
) {
    let c = offers[i];
    let credits = matches!(field, Field::Artist | Field::Artists);
    let agreeing = group.iter().filter(|g| **g == group[i]).count();
    let versions = if c.grouping {
        similar::versions_in(&c.offer.values.join("; ")).len()
    } else {
        0
    };
    (
        !c.offer.structured,
        if credits { c.rank } else { 0 },
        decorated(c),
        std::cmp::Reverse(agreeing),
        then(c.key),
        c.rank == UNSURE_RANK,
        std::cmp::Reverse(versions),
    )
}

/// `offers`' indices, best first: as [`rank_of`] ranks them, then by the
/// order of `sources`.
fn ranked<K: Ord>(
    offers: &[&Candidate<'_>],
    group: &[usize],
    field: Field,
    then: &impl Fn(&SourceKey) -> K,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..offers.len()).collect();
    order.sort_by_cached_key(|&i| (rank_of(offers, group, field, then, i), offers[i].n));
    order
}

/// Why the best offer for `field` won only by the order of `sources`,
/// where another group of offers ranks alike in all else; none where
/// nothing else came so close, or where offers do not group, as `believe
/// = "sources"` asks.
fn contested(candidates: &BTreeMap<Field, Vec<Candidate<'_>>>, field: Field) -> Option<String> {
    let offers: Vec<&Candidate<'_>> = candidates.get(&field)?.iter().collect();
    if !offers.iter().all(|c| c.grouping) {
        return None;
    }
    let group = groups(&offers, field);
    let none = |_: &SourceKey| ();
    let ranked = ranked(&offers, &group, field, &none);
    let (&best, rest) = ranked.split_first()?;
    let alike = |i: usize| rank_of(&offers, &group, field, &none, i);
    let runner = rest
        .iter()
        .copied()
        .find(|&i| group[i] != group[best] && alike(i) == alike(best))?;
    Some(format!(
        "{} “{}” from {} over “{}” from {}, by the order of `sources`",
        field.vorbis(),
        offers[best].offer.values.join("; "),
        offers[best].key,
        offers[runner].offer.values.join("; "),
        offers[runner].key,
    ))
}

/// The best date of a song with no release's: a field kept for it, or an
/// upload's date where the upload's channel is the song's artist, set by
/// hand or `offered`. An upload's date is the day it went up, the
/// release's only where its uploader is the artist releasing it.
fn date_of<'c, 'a>(
    input: &Input<'_>,
    candidates: &'c BTreeMap<Field, Vec<Candidate<'a>>>,
    offered: Option<&Slot>,
) -> Option<&'c Candidate<'a>> {
    let by_hand = (input.song.tags.iter())
        .chain(input.album.iter().flat_map(|a| &a.tags))
        .find(|(name, values)| Field::named(name) == Some(Field::Artist) && !values.is_empty())
        .map(|(_, values)| values.as_slice());
    let artist: Vec<&str> = by_hand
        .or(offered.map(|s| s.values.as_slice()))
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    let dated_so = |key: &SourceKey| {
        let structured = (candidates.get(&Field::Date).into_iter().flatten())
            .any(|c| c.key == key && c.offer.structured);
        let uploaded_by_artist = (candidates.get(&Field::Artist).into_iter().flatten())
            .filter(|c| c.key == key && !c.offer.structured)
            .any(|c| {
                let channel: Vec<&str> = c.offer.values.iter().map(String::as_str).collect();
                similar::credits_agree(&channel, &artist) == Some(true)
            });
        structured || uploaded_by_artist
    };
    best_offer(candidates, Field::Date, &dated_so)
}

/// Where the album of a song on none comes from: its own title.
pub const SINGLE: &str = "a single, named for its title";

/// Where an album artist taken from the artist comes from, before the
/// artist's own source.
const FIRST_ARTIST: &str = "the first artist of ";

/// Whether a tag from `from` is one muman fills in itself, which no
/// source nor the song list gives: an album named for a single's title,
/// or an album artist taken from the artist.
#[must_use]
pub fn derived(from: &str) -> bool {
    from == SINGLE || from.starts_with(FIRST_ARTIST)
}

/// Take the MusicBrainz recording IDs of `release`, the source the
/// release fields came from, where it names another recording than
/// `fields` hold: a release's IDs name one recording on it, so the IDs
/// written never contradict. An ID of the other recording it lacks goes.
fn recording_of_release(
    fields: &mut BTreeMap<Field, Slot>,
    candidates: &BTreeMap<Field, Vec<Candidate<'_>>>,
    release: &SourceKey,
) {
    let of_release = |k: &SourceKey| k == release;
    let Some(track) = best_offer(candidates, Field::MusicBrainzTrackId, &of_release) else {
        return;
    };
    if fields
        .get(&Field::MusicBrainzTrackId)
        .is_none_or(|s| s.values == track.offer.values)
    {
        return;
    }
    for field in [
        Field::MusicBrainzTrackId,
        Field::MusicBrainzArtistId,
        Field::MusicBrainzWorkId,
    ] {
        match best_offer(candidates, field, &of_release) {
            Some(c) => fields.insert(field, Slot::of(c)),
            None => fields.remove(&field),
        };
    }
}

/// What the song's own text is written in: its title's letters, less the
/// featured artists it credits, whose names are theirs and not the
/// song's, and the script its release names.
fn writing(fields: &BTreeMap<Field, Slot>) -> Writing {
    let values = |field: Field| fields.get(&field).into_iter().flat_map(|s| &s.values);
    let title = values(Field::Title).map(|t| Writing::of_text(clean::without_credits(t)));
    let script = values(Field::Script).map(|c| Writing::of_code(c));
    title.chain(script).fold(Writing::default(), Writing::and)
}

/// Whether the language `key` offers may be the one the song is sung in,
/// written as `writing`. A language read off an upload is the default
/// its uploader set, as often a channel's own language as the song's, so
/// it is taken only where the song's text could be in it; a structured
/// one, as a work's lyrics languages, is a record of the song itself.
fn language_fits(
    candidates: &BTreeMap<Field, Vec<Candidate<'_>>>,
    key: &SourceKey,
    writing: &Writing,
) -> bool {
    candidates
        .get(&Field::Language)
        .into_iter()
        .flatten()
        .filter(|c| c.key == key)
        .all(|c| {
            let langs: Vec<Lang> = c
                .offer
                .values
                .iter()
                .filter_map(|v| Lang::parse(v))
                .collect();
            c.offer.structured || langs.is_empty() || writing.fits(&langs)
        })
}

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

    fn agreed(a: &crate::album::Agreed) -> Self {
        Self {
            values: a.values.clone(),
            from: a.from.clone(),
            cleaned: Vec::new(),
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
    // Several artists are written joined, and listed as Picard lists them.
    if get(written, Field::Artists).is_none()
        && artists.len() > 1
        && let Some(artist) = get(written, Field::Artist)
    {
        let slot = Slot {
            values: artists.to_vec(),
            from: artist.from,
            cleaned: artist.cleaned,
        };
        insert_in_order(written, Field::Artists, slot);
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

/// `values` of `field` put in `tags` before the first known field after
/// it.
fn insert_comment(tags: &mut Comments, field: Field, values: Vec<String>) {
    let rank = |f: Field| Field::ALL.iter().position(|g| *g == f);
    let at = tags
        .iter()
        .position(|(k, _)| Field::named(k).and_then(rank) > rank(field))
        .unwrap_or(tags.len());
    tags.insert(at, (field.vorbis().to_string(), values));
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
    candidates: &BTreeMap<Field, Vec<Candidate<'_>>>,
    audio: &SourceKey,
    mixed: bool,
) -> (Comments, Vec<TagWhy>, Vec<String>) {
    let mut fields: BTreeMap<Field, Slot> = BTreeMap::new();
    let any = |_: &SourceKey| true;
    for field in Field::of(Scope::Recording).filter(|f| *f != Field::Language) {
        if let Some(c) = best_offer(candidates, field, &any) {
            fields.insert(field, Slot::of(c));
        }
    }
    let title = fields
        .get(&Field::Title)
        .map(|s| tags::normalized(&s.values));
    let vetting = input.clean.is_on(clean::VETTING);
    let named_so = |key: &SourceKey| {
        vetting
            || candidates
                .get(&Field::Title)
                .into_iter()
                .flatten()
                .filter(|c| c.key == key)
                .all(|c| Some(tags::normalized(&c.offer.values)) == title)
    };
    for field in Field::of(Scope::RecordingId) {
        if let Some(c) = best_offer(candidates, field, &named_so) {
            fields.insert(field, Slot::of(c));
        }
    }
    let release = release_of(input, candidates);
    if let Some(album) = release {
        for field in Field::of(Scope::Release) {
            if let Some(c) = best_offer(candidates, field, &|k| k == album.key) {
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
    if let Some(album) = release.filter(|a| named_so(a.key)) {
        recording_of_release(&mut fields, candidates, album.key);
    }
    let writing = writing(&fields);
    let sung_so = |key: &SourceKey| language_fits(candidates, key, &writing);
    if let Some(c) = best_offer(candidates, Field::Language, &sung_so) {
        fields.insert(Field::Language, Slot::of(c));
    }
    let heard = |k: &SourceKey| !mixed && k == audio;
    let album_heard = |k: &SourceKey| heard(k) && release.is_some_and(|r| r.key == audio);
    for (scope, from) in [
        (Scope::Loudness, &heard as &dyn Fn(&SourceKey) -> bool),
        (Scope::AlbumLoudness, &album_heard),
    ] {
        for field in Field::of(scope) {
            if let Some(c) = best_offer(candidates, field, from) {
                fields.insert(field, Slot::of(c));
            }
        }
    }
    if !fields.contains_key(&Field::Date)
        && let Some(c) = date_of(input, candidates, fields.get(&Field::Artist))
    {
        fields.insert(Field::Date, Slot::of(c));
    }
    let mut artists = joined_artists(&mut fields);
    for agreed in input.pick.iter().flat_map(|p| &p.agreed) {
        fields.insert(agreed.field, Slot::agreed(agreed));
    }

    let mut written: Vec<(String, Slot)> = Field::ALL
        .iter()
        .filter_map(|f| fields.remove(f).map(|s| (f.vorbis().to_string(), s)))
        .collect();
    let offered = artist_of(&written);
    set_by_hand(input, &mut written);
    // Set by hand, the artists are what the song list says, a list or one.
    if let Some(artist) = artist_of(&written)
        && offered.as_ref() != Some(&artist)
    {
        artists = artist;
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

/// The artists one by one, the artist joined as it is written: an
/// artist offered as its credit joins them is listed by its source's
/// `ARTISTS`.
fn joined_artists(fields: &mut BTreeMap<Field, Slot>) -> Vec<String> {
    let mut artists = Vec::new();
    if let Some(artist) = fields.get_mut(&Field::Artist) {
        artists.clone_from(&artist.values);
        artist.values = vec![artist.values.join(", ")];
    }
    if let (Some(artist), Some(each)) = (fields.get(&Field::Artist), fields.get(&Field::Artists))
        && artist.from == each.from
    {
        artists.clone_from(&each.values);
    }
    artists
}

/// The values of `written`'s artist.
fn artist_of(written: &[(String, Slot)]) -> Option<Vec<String>> {
    written
        .iter()
        .find(|(k, _)| k == Field::Artist.vorbis())
        .map(|(_, s)| s.values.clone())
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
    split_places(input, written);
}

/// A track or disc set by hand as `3/12` written as its number, its count
/// written as the total, unless the song list names the total itself.
fn split_places(input: &Input<'_>, written: &mut Vec<(String, Slot)>) {
    let named = |field: Field| {
        let hand = input
            .song
            .tags
            .iter()
            .chain(input.album.iter().flat_map(|a| &a.tags));
        hand.into_iter()
            .any(|(name, _)| Field::named(name) == Some(field))
            || (field == Field::TrackTotal && input.album.is_some_and(|a| a.tracks.is_some()))
    };
    for (place, total) in [
        (Field::Track, Field::TrackTotal),
        (Field::Disc, Field::DiscTotal),
    ] {
        let Some(slot) = written
            .iter_mut()
            .find(|(k, _)| k == place.vorbis())
            .map(|(_, s)| s)
        else {
            continue;
        };
        let Some((n, of)) = slot.values.first().and_then(|v| v.split_once('/')) else {
            continue;
        };
        let (n, of) = (n.to_string(), of.to_string());
        slot.values = vec![n];
        let from = slot.from.clone();
        if named(total) {
            continue;
        }
        let total_slot = Slot {
            values: vec![of],
            from,
            cleaned: Vec::new(),
        };
        match written.iter_mut().find(|(k, _)| k == total.vorbis()) {
            Some((_, s)) => *s = total_slot,
            None => insert_in_order(written, total, total_slot),
        }
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
