//! The part of a plan each library file is made from, stated once in
//! types, so what a change to a plan writes again can be read off the
//! parts that differ rather than off projections kept in step by hand.
//!
//! | Part | Written by | A change |
//! |---|---|---|
//! | [`AudioPart`]: renderer version, format, audio source, gain and bits, MP4 chapters, an Opus header | ffmpeg's encode or copy | builds the file again |
//! | the cover inside ([`CoverPart`]) | the build: a FLAC's padding is sized from it, and it from the room the container leaves | builds the file again |
//! | the lyrics inside ([`LyricsInside`]) | the tag writers, made anew from their source | tags the file again |
//! | [`TagsPart`]: tags, tagging version, the lyrics' language, other containers' chapters, gains as the file holds them | the tag writers | tags the file again |
//! | each file beside ([`Beside`]) | the lyric and cover writers | writes that file again |
//!
//! A lyric file's text holds what its cleaning reads of the song's names
//! ([`LyricText::names`]), so a retitled song's lyrics are told apart;
//! [`crate::render`] cleans and writes lyrics from these parts alone. A
//! cover's `made` counts only where a place encodes it, as
//! [`CoverRef::only`] keeps it.
//!
//! Every file a plan writes beside a song is named, guessed at in size and
//! told shared or not here, in one order: lyrics, covers, folder covers
//! ([`Parts::files`]). Its name comes from [`crate::outputs`], and the
//! files a sync may write or clear for a song are these and those its last
//! build left ([`Parts::touched`]).
//!
//! A sync decides by [`redo`] whether a song is built again, tagged
//! again, and has the files beside it written again; whether it is kept
//! is still the whole plan's to say. The parts are checked against the
//! plan they come from, over every part a plan holds.

use std::path::{Path, PathBuf};

use crate::chapters::Chapter;
use crate::codec::{Chain, Container};
use crate::facts::{CoverAt, LyricsAt};
use crate::loudness::Gains;
use crate::quality::Rect;
use crate::resolve::{AudioRef, CoverPlace, CoverRef, Format, LyricsFile, LyricsRef, Plan};
use crate::source::SourceKey;
use crate::state::Written;

/// Every part of `plan`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parts<'a> {
    pub audio: AudioPart<'a>,
    pub tags: TagsPart<'a>,
    pub cover: Option<CoverPart<'a>>,
    pub lyrics: Option<LyricsInside<'a>>,
    pub beside: Vec<Beside<'a>>,
}

/// What the audio of the song's file is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioPart<'a> {
    pub version: u32,
    pub format: Format,
    pub source: &'a AudioRef,
    pub chain: Chain,
    /// Chapters an MP4 holds, which ffmpeg writes.
    pub mp4_chapters: &'a [Chapter],
    /// Whether an Opus header's gain is set, which a retag sets but only a
    /// build puts back once unset.
    pub header: bool,
}

/// What the tags of the song's file are made from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TagsPart<'a> {
    pub tags: &'a [(String, Vec<String>)],
    pub tagging: u32,
    pub language: Option<&'a str>,
    /// Chapters a container other than MP4 holds, which the tag writers
    /// write.
    pub chapters: &'a [Chapter],
    pub gains: Option<Gains>,
    pub opus_header: Option<i16>,
}

/// What a song's lyrics' text is made from, wherever it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricText<'a> {
    pub key: &'a SourceKey,
    pub rev: &'a str,
    pub at: &'a LyricsAt,
    pub shift_ms: i64,
    pub stretch_ppm: i64,
    pub clean_off: &'a [String],
    pub cleaning: u32,
    pub writing: u32,
    /// The song's title and artists, where a cleaning rule reads them.
    pub names: Option<(Option<&'a str>, Vec<&'a str>)>,
}

/// The lyrics inside the song, and how they are written there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsInside<'a> {
    pub text: LyricText<'a>,
    pub form: LyricsFile,
}

/// A file of lyrics beside the song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsFilePart<'a> {
    pub text: LyricText<'a>,
    pub file: LyricsFile,
    /// Where the song ends, which ends a file of cues' last cue.
    pub end_ms: Option<i64>,
}

/// What a cover picture is made from, wherever it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoverSource<'a> {
    pub key: &'a SourceKey,
    pub rev: &'a str,
    pub at: &'a CoverAt,
    pub mimetype: &'a str,
    pub crop: Option<Rect>,
}

/// One place a cover goes, and what it is made from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoverPart<'a> {
    pub source: CoverSource<'a>,
    pub place: CoverPlace,
    /// How it is made, where this place encodes it.
    pub made: Option<u32>,
}

/// A file written beside the song, in the order they are named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Beside<'a> {
    Lyrics(LyricsFilePart<'a>),
    Cover(CoverPart<'a>),
    Folder(CoverPart<'a>),
}

impl Beside<'_> {
    /// Where the file is, beside the song whose audio file is at `audio`,
    /// as [`crate::outputs`] names it.
    #[must_use]
    pub(crate) fn path(&self, audio: &Path) -> Option<PathBuf> {
        let stem = audio.with_extension("");
        match self {
            Self::Lyrics(l) => Some(crate::outputs::lyrics_file(&stem, &l.file)),
            Self::Cover(c) => crate::outputs::cover_file(&stem, &c.place),
            Self::Folder(c) => crate::outputs::folder_file(&stem, &c.place),
        }
    }

    /// Whether other songs of the folder may hold it too: a folder's cover.
    #[must_use]
    pub(crate) fn shared(&self) -> bool {
        matches!(self, Self::Folder(_))
    }

    /// What it is guessed to take before it is written.
    #[must_use]
    pub(crate) fn guess(&self) -> u64 {
        match self {
            Self::Lyrics(_) => LYRICS_GUESS,
            Self::Cover(_) | Self::Folder(_) => COVER_GUESS,
        }
    }
}

/// Bytes a file holds besides its stream, cover and tags.
const HEADERS: u64 = 4 * 1024;
/// The cover guessed for a song: a 1000 px JPEG.
const COVER_GUESS: u64 = 200 * 1024;
/// Lyrics guessed for a song that has them.
const LYRICS_GUESS: u64 = 3 * 1024;

/// What a plan's parts that differ from an old plan's write again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Redo {
    pub build: bool,
    pub retag: bool,
    pub beside: bool,
}

/// The parts of `plan`.
#[must_use]
pub(crate) fn parts(plan: &Plan) -> Parts<'_> {
    let mp4 = plan.format.codec().container() == Container::Mp4;
    let chapters = plan.chapters.as_slice();
    let text = plan.lyrics.as_ref().map(|l| lyric_text(plan, l));
    let mut beside: Vec<Beside<'_>> = Vec::new();
    if let (Some(l), Some(text)) = (&plan.lyrics, &text) {
        beside.extend(l.files().into_iter().map(|file| {
            Beside::Lyrics(LyricsFilePart {
                text: text.clone(),
                file,
                end_ms: l.end_ms,
            })
        }));
    }
    if let Some(c) = &plan.cover {
        beside.extend(placed(c, true).into_iter().map(Beside::Cover));
    }
    if let Some(f) = &plan.folder {
        beside.extend(placed(f, true).into_iter().map(Beside::Folder));
    }
    Parts {
        audio: AudioPart {
            version: plan.version,
            format: plan.format,
            source: &plan.audio,
            chain: plan.chain(),
            mp4_chapters: if mp4 { chapters } else { &[] },
            header: plan.opus_header().is_some(),
        },
        tags: TagsPart {
            tags: &plan.tags,
            tagging: plan.tagging,
            language: plan.lyrics_language(),
            chapters: if mp4 { &[] } else { chapters },
            gains: plan.held_loudness(),
            opus_header: plan.opus_header(),
        },
        cover: plan
            .cover
            .as_ref()
            .and_then(|c| placed(c, false).into_iter().next()),
        lyrics: plan
            .lyrics
            .as_ref()
            .filter(|l| l.tag())
            .zip(text)
            .map(|(l, text)| LyricsInside {
                text,
                form: l.inside.clone().unwrap_or_default(),
            }),
        beside,
    }
}

fn lyric_text<'a>(plan: &'a Plan, l: &'a LyricsRef) -> LyricText<'a> {
    LyricText {
        key: &l.key,
        rev: &l.rev,
        at: &l.at,
        shift_ms: l.shift_ms,
        stretch_ppm: l.stretch_ppm,
        clean_off: &l.clean_off,
        cleaning: l.cleaning,
        writing: l.writing,
        names: names(plan, l),
    }
}

/// The song's title and artists `l` is cleaned by, where a rule switched
/// on reads them.
fn names<'a>(plan: &'a Plan, l: &LyricsRef) -> Option<(Option<&'a str>, Vec<&'a str>)> {
    let reads = crate::clean::lyrics::NAMED
        .iter()
        .any(|rule| !l.clean_off.iter().any(|off| off == rule));
    let values = |name: &'static str| {
        plan.tags
            .iter()
            .filter(move |(k, _)| k == name)
            .flat_map(|(_, v)| v.iter().map(String::as_str))
    };
    reads.then(|| {
        (
            values("TITLE").next(),
            values("ARTIST").chain(values("ARTISTS")).collect(),
        )
    })
}

/// The places of `c` beside the song, or inside it.
fn placed(c: &CoverRef, file: bool) -> Vec<CoverPart<'_>> {
    c.places()
        .into_iter()
        .filter(|p| p.file.is_some() == file)
        .map(|place| cover_part(c, place))
        .collect()
}

fn cover_part(c: &CoverRef, place: CoverPlace) -> CoverPart<'_> {
    CoverPart {
        source: CoverSource {
            key: &c.key,
            rev: &c.rev,
            at: &c.at,
            mimetype: &c.mimetype,
            crop: c.crop,
        },
        made: c.made.filter(|_| c.encodes(&place)),
        place,
    }
}

/// What writing `new` over a song built as `old` writes again.
#[must_use]
pub(crate) fn redo(old: &Plan, new: &Plan) -> Redo {
    parts(new).redo_from(&parts(old))
}

impl AudioPart<'_> {
    /// Whether a file built as `old` holds the audio this does: the same
    /// in every way, but that an Opus header `old` set is set here too.
    #[must_use]
    pub(crate) fn keeps(&self, old: &AudioPart<'_>) -> bool {
        let Self { header, .. } = *self;
        AudioPart {
            header,
            ..old.clone()
        } == *self
            && (!old.header || header)
    }
}

impl LyricText<'_> {
    /// What the lyrics' cleaning knows of the song: its names only where
    /// a rule switched on reads them.
    #[must_use]
    pub(crate) fn context(&self) -> crate::clean::lyrics::Context<'_> {
        let (title, artists) = self.names.clone().unwrap_or_default();
        crate::clean::lyrics::Context {
            title,
            artists,
            subtitle: matches!(self.at, LyricsAt::Stream { .. }),
        }
    }
}

impl<'a> Parts<'a> {
    /// The text of the song's lyrics, where some file holds them.
    #[must_use]
    pub(crate) fn lyric_text(&self) -> Option<&LyricText<'a>> {
        self.lyrics
            .as_ref()
            .map(|l| &l.text)
            .or_else(|| self.lyric_files().next().map(|f| &f.text))
    }

    /// The places of the song's cover: inside it, then each file beside.
    pub(crate) fn covers(&self) -> impl Iterator<Item = &CoverPart<'a>> {
        self.cover.iter().chain(self.covers_beside())
    }

    /// The files of the song's cover beside it.
    pub(crate) fn covers_beside(&self) -> impl Iterator<Item = &CoverPart<'a>> {
        self.beside.iter().filter_map(|b| match b {
            Beside::Cover(c) => Some(c),
            Beside::Lyrics(_) | Beside::Folder(_) => None,
        })
    }

    /// The files of its folder's cover the song holds.
    pub(crate) fn folder_covers(&self) -> impl Iterator<Item = &CoverPart<'a>> {
        self.beside.iter().filter_map(|b| match b {
            Beside::Folder(c) => Some(c),
            Beside::Lyrics(_) | Beside::Cover(_) => None,
        })
    }

    /// The files written beside the song whose audio file is at `audio`:
    /// its lyrics', its cover's, then its folder's.
    #[must_use]
    pub(crate) fn files(&self, audio: &Path) -> Vec<PathBuf> {
        self.beside.iter().filter_map(|b| b.path(audio)).collect()
    }

    /// The files of its folder the song whose audio file is at `audio`
    /// holds, which other songs of the folder may hold too.
    #[must_use]
    pub(crate) fn folder_files(&self, audio: &Path) -> Vec<PathBuf> {
        self.beside
            .iter()
            .filter(|b| b.shared())
            .filter_map(|b| b.path(audio))
            .collect()
    }

    /// The files beside the song at `audio` that writing it may write or
    /// clear: those these parts write, and those `recorded` there as its
    /// last build's. A file of the user's own at any other name is not
    /// the song's to touch.
    #[must_use]
    pub(crate) fn touched(&self, audio: &Path, recorded: Option<&Written>) -> Vec<PathBuf> {
        let mut files = self.files(audio);
        for path in recorded.into_iter().flat_map(Written::sidecar_paths) {
            if !files.contains(path) {
                files.push(path.clone());
            }
        }
        files
    }

    /// What the song's file is guessed to hold besides its audio, and
    /// each file beside it to take, before any is written.
    #[must_use]
    pub(crate) fn guess(&self) -> (u64, Vec<u64>) {
        let tags: usize = self
            .tags
            .tags
            .iter()
            .flat_map(|(k, values)| values.iter().map(move |v| k.len() + v.len() + 8))
            .sum();
        let cover = self.cover.as_ref().map_or(0, |_| COVER_GUESS);
        let lyrics = self.lyrics.as_ref().map_or(0, |_| LYRICS_GUESS);
        let beside = self.beside.iter().map(Beside::guess).collect();
        (tags as u64 + cover + lyrics + HEADERS, beside)
    }

    /// The files of lyrics beside the song.
    pub(crate) fn lyric_files(&self) -> impl Iterator<Item = &LyricsFilePart<'a>> {
        self.beside.iter().filter_map(|b| match b {
            Beside::Lyrics(f) => Some(f),
            Beside::Cover(_) | Beside::Folder(_) => None,
        })
    }

    /// What writing these parts over a song built as `old` writes again.
    #[must_use]
    pub(crate) fn redo_from(&self, old: &Parts<'_>) -> Redo {
        Redo {
            build: !self.audio.keeps(&old.audio) || self.cover != old.cover,
            retag: self.tags != old.tags || self.lyrics != old.lyrics,
            beside: self.beside != old.beside,
        }
    }
}

#[cfg(test)]
mod tests;
