//! One song written to the library from its plan: the audio copied or
//! encoded and tagged, and its cover and lyrics inside it and in the
//! files beside it its plan names. Each file is made from its parts of
//! the plan alone ([`Job`] carries them): the audio from the audio part,
//! the tags from the tag part, so no change a writer reads leaves what it
//! wrote as it was.
//!
//! Every file is written as `<name>.part` and renamed into place, the
//! audio last, so a player scanning the library never sees half a track.
//! A `.part` is made anew, whatever was at its name removed first and the
//! file created only if nothing took its place: a link left there by
//! someone who may write the library folder would send the write
//! elsewhere. ffmpeg, which opens its output itself, writes into a `.part`
//! made so just before it runs. A song's files go into place through
//! [`crate::library::put`] and [`crate::library::clear`], over or in
//! place of only the files the sync claimed as muman's or took over.
//! A cover or lyrics that fail are reported and the song is written
//! without them. A song that fails leaves the folders made for it, since
//! another song rendered at the same time may be writing into one; the
//! sync clears the folders failures leave once every song is written.
//!
//! A FLAC's tags and cover are written into padding ffmpeg reserves for
//! them (`write_flac_blocks`), so the file is written once: `lofty`
//! writes a tag of another length by moving every byte of audio after it,
//! a second copy of the song. A cover past the room goes through `lofty`
//! all the same. Ogg, MP3, MP4 and WavPack are `lofty`'s to write. In an
//! MP4 it keeps the room a shrunken tag leaves as a `free` atom, and fills
//! it on the next write, so a retag would write other bytes than a build;
//! that room is removed (`drop_mp4_padding`).
//!
//! Each codec goes into its own container, as [`crate::codec`] says. Tags
//! come from the plan alone: the container's, yt-dlp's description and URL
//! among them, are dropped. They are Vorbis comments in Ogg and FLAC. In
//! MP3, MP4 and WavPack each field muman knows is written into the frame,
//! atom or item [`crate::tags::TABLE`] names, as Picard writes it: never
//! through `lofty`'s generic tag, which keeps one value of a frame type in
//! `ID3v2`, drops a track that is no bare number, and writes some names
//! under frames that hold no text, which fails the whole write. A track or
//! disc and its total are one `3/12`, a date `ID3v2`'s timestamp frame.
//! A comment of no field takes the text frame or atom `lofty` names for
//! it, if it names one that holds plain text, else a `TXXX` frame, an
//! iTunes freeform atom or an APEv2 item of its name; a producer,
//! arranger, engineer or mixer is a role in `TIPL`. Several values are
//! null-separated in `ID3v2.4` and APEv2, and in MP4 one atom holds a
//! `data` atom for each, as iTunes, Picard and mutagen write one: a reader
//! that takes one atom a name, as ffmpeg's does, would see only the last
//! of several atoms. `ID3v2` marks lyrics with the language their source
//! names them in, `XXX` where it names none, and a comment with English,
//! as Picard does. Opus holds ReplayGain's gains as R128's and no peaks,
//! as RFC 7845 tells its players to read them.
//!
//! The cover is a front-cover picture inside the song and a picture in
//! each file of it beside the song, as its parts' places say, made in
//! `render/cover.rs` from their part alone: kept, a JPEG or PNG without
//! borders as its own bytes, a file's only where they are its format; any
//! other made by ffmpeg in the place's format, cropped to the content
//! inside a video frame's bars and scaled down to its longest side,
//! places alike sharing one picture. A picture ffmpeg
//! encodes takes [`PICTURE_ARGS`], so a JPEG or PNG is the same bytes on
//! every processor: measured on ffmpeg 6.1 against `-cpuflags 0`,
//! swscale's conversion and the JPEG encoder's DCT round otherwise on a
//! processor with other vector instructions, while the number of threads
//! changed nothing. A WebP is libwebp's, whose vector
//! instructions `-cpuflags` does not reach. A FLAC
//! metadata block holds at most 16 MiB, as does the same block in Ogg,
//! where it is base64 and so a third larger, and ffmpeg reads no APEv2
//! tag past that, its other items counted, so in every container a cover
//! past what is left is made a JPEG at most 3000 pixels wide; FLAC embeds
//! no lyrics past it. Lyrics are made in `render/lyrics.rs` from their
//! part alone: the cleaning knows the song's title and artists only
//! where the part holds them, so no change it reads leaves the lyrics as
//! they were. Lyrics from a `.lrc` file need no ffmpeg.
//!
//! The audio keeps its source's chapters, as [`crate::chapters`] writes
//! them in each container: ffmpeg writes an MP4's from an `FFMETADATA`
//! file, the tag writers every other's beside the tags.
//!
//! A song whose plan changed in its tags or the lyrics inside it alone is
//! tagged again in place (`retag`), and the files beside it written again
//! where they changed ([`beside`]), as a build writes them, with the cover
//! its file holds, and its lyrics made anew from their source where their
//! part changed, else as the file holds them, each read as each container
//! holds them: `lofty`'s generic tag reads no WavPack cover, so APEv2's
//! `Cover Art (Front)` is read itself. Every file written is read back as
//! a source's tags are, and a field or a cover it does not hold as
//! written is told as a problem of the song, never lost unseen.
//!
//! Starting ffmpeg can cost 100 ms or more against a few milliseconds of
//! work, so a song is one ffmpeg run writing every output; only when that
//! run fails is each output run alone, so a bad cover or lyrics stream
//! costs only itself.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use lofty::TextEncoding;
use lofty::ape::{ApeItem, ApeTag};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::id3::v2::{
    AttachedPictureFrame, CommentFrame, ExtendedTextFrame, Frame, FrameId, Id3v2Tag, KeyValueFrame,
    TextInformationFrame, TimestampFrame, UniqueFileIdentifierFrame, UnsynchronizedTextFrame,
};
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst};
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OggPictureStorage, OpusFile, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::items::{Lang, Timestamp, UNKNOWN_LANGUAGE};
use lofty::tag::{Accessor, ItemKey, ItemValue, TagExt, TagType};

use crate::artifact::{AudioPart, Parts};
use crate::chapters::Chapter;
use crate::codec::{Codec, Container};
use crate::ffmpeg::{self, Output};
use crate::library::Owned;
use crate::resolve::Format;
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::store::Located;
use crate::tags::{Field, Id3, Mp4};

mod cover;
mod lyrics;

pub(crate) use cover::converts;
pub use cover::{COVER_MAKING, PICTURE_ARGS};
use cover::{CoverPlan, cover_bytes, cover_files, cover_plans, cover_rels, fit_cover};
use cover::{folder_plans, folder_rels, pictures, read_picture};
pub use lyrics::lyrics_file;

#[derive(Debug)]
pub struct Job<'a> {
    /// The parts of the song's plan, which alone its files are made from.
    pub parts: &'a Parts<'a>,
    /// The song's path in the library, without its extension.
    pub stem: &'a Path,
    pub library: &'a Path,
    pub sources: &'a BTreeMap<SourceKey, Located>,
    /// An empty folder of this song's own for intermediates.
    pub scratch: &'a Path,
    /// The files at the song's paths it may write over or remove.
    pub claim: &'a [Owned],
    /// The files the song's last build left beside it, which go unless
    /// written again.
    pub stale: &'a [PathBuf],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The audio's path in the library.
    pub audio: PathBuf,
    /// Each file written beside it, by its path in the library, with what
    /// it takes in bytes.
    pub beside: Vec<(PathBuf, u64)>,
    /// What could not be added, each said once.
    pub problems: Vec<String>,
    /// What the audio file takes, in bytes, as written.
    pub audio_bytes: u64,
}

impl Rendered {
    /// What the files beside the audio take, in whole blocks of `block`
    /// bytes each, as a filesystem spends them.
    #[must_use]
    pub fn beside_blocks(&self, block: u64) -> u64 {
        self.beside
            .iter()
            .map(|(_, bytes)| crate::limit::blocks(*bytes, block))
            .sum()
    }
}

fn with_extension(stem: &Path, ext: &str) -> PathBuf {
    let mut name = stem.as_os_str().to_os_string();
    name.push(".");
    name.push(ext);
    PathBuf::from(name)
}

fn part(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

/// A new, empty file at `part`, whatever was there removed first.
// Makes a song's staged file anew.
#[expect(clippy::disallowed_methods)]
fn stage(part: &Path) -> Result<fs::File> {
    match fs::remove_file(part) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(e).with_context(|| format!("removing {}", part.display()));
        }
        _ => {}
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(part)
        .with_context(|| format!("creating {}", part.display()))
}

/// The inputs of one ffmpeg run, each file once.
#[derive(Default)]
struct Inputs(Vec<PathBuf>);

impl Inputs {
    fn index(&mut self, path: &Path) -> usize {
        if let Some(n) = self.0.iter().position(|p| p == path) {
            return n;
        }
        self.0.push(path.to_path_buf());
        self.0.len() - 1
    }
}

/// The audio's output; `chapters` the input an MP4's chapters are read
/// from, an `FFMETADATA` file. Tags are written from the plan alone, so
/// ffmpeg carries none of the container's, nor its chapters, which it
/// would write as comments; an MP4's chapters are its to write, and it
/// keeps their titles only where the global and stream metadata alone
/// are dropped.
fn audio_output(
    audio: &AudioPart<'_>,
    input: usize,
    chapters: Option<usize>,
    reserve: Option<u64>,
    path: &Path,
) -> Output {
    let format = audio.format;
    let mut args: Vec<String> = vec!["-map".into(), format!("{input}:{}", audio.source.index)];
    if let Some(reserve) = reserve {
        args.extend(["-metadata_header_padding".into(), reserve.to_string()]);
    }
    args.extend(match format {
        Format::Copy { .. } => vec!["-c:a".into(), "copy".into()],
        Format::Encode {
            codec,
            kbps,
            adapt,
            mix,
        } => codec.encoder_args(kbps, adapt, mix, audio.chain),
    });
    match chapters {
        Some(n) => args.extend([
            "-map_metadata:g".into(),
            "-1".into(),
            "-map_metadata:s".into(),
            "-1".into(),
            "-map_chapters".into(),
            n.to_string(),
        ]),
        None => args.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(String::from)),
    }
    args.extend(format.codec().muxer_args());
    Output::new(args.into_iter().map(OsString::from).collect(), path)
}

// The steps read in the order they run; split up, the order would be in the call sites.
#[expect(clippy::too_many_lines)]
// Removes a song's staged files when it fails.
#[expect(clippy::disallowed_methods)]
pub fn render<R: Runner>(runner: &R, job: &Job<'_>) -> Result<Rendered> {
    let made = job.parts;
    let locate = |key: &SourceKey| {
        job.sources
            .get(key)
            .ok_or_else(|| anyhow!("{key} is not on disk"))
    };
    let audio_rel = with_extension(job.stem, made.audio.format.extension());
    let (lyrics_rels, cover_rels) = (lyrics::rels(job.stem, made), cover_rels(job.stem, made));
    let folder_rels = folder_rels(job.stem, made);
    let rels: Vec<PathBuf> = lyrics_rels
        .iter()
        .chain(&cover_rels)
        .chain(&folder_rels)
        .cloned()
        .collect();
    let audio_path = job.library.join(&audio_rel);
    let dir = audio_path.parent().unwrap_or(job.library);
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let audio_part = part(&audio_path);
    let parts: Vec<PathBuf> = rels.iter().map(|r| part(&job.library.join(r))).collect();
    fs::create_dir_all(job.scratch)?;

    let mut inputs = Inputs::default();
    let source = locate(&made.audio.source.key)?;
    let n = inputs.index(&source.path);
    let chapters = if made.audio.mp4_chapters.is_empty() {
        None
    } else {
        let name = crate::atomic::Name::new("chapters.ffmeta".to_string())?;
        crate::atomic::write(
            job.scratch,
            &name,
            crate::chapters::ffmetadata(made.audio.mp4_chapters).as_bytes(),
        )?;
        Some(inputs.index(&job.scratch.join(name.as_str())))
    };
    let mut problems = Vec::new();
    let mut cover_outputs = Vec::new();
    let covers = match pictures(made.covers()) {
        Some((c, places)) => match cover_plans(
            runner,
            c,
            places,
            locate(c.key)?,
            job.scratch,
            &mut inputs,
            &mut cover_outputs,
        ) {
            Ok(p) => p,
            Err(e) => {
                problems.push(format!("no cover: {e:#}"));
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let folder_files = match pictures(made.folder_covers()) {
        Some((f, places)) => {
            folder_plans(runner, job, (f, places), &mut inputs, &mut cover_outputs).unwrap_or_else(
                |e| {
                    problems.push(format!("no folder cover: {e:#}"));
                    Vec::new()
                },
            )
        }
        None => Vec::new(),
    };
    let (inside, cover_files): (Vec<_>, Vec<_>) =
        covers.into_iter().partition(|(p, _)| p.file.is_none());
    let cover = inside.into_iter().next().map(|(_, c)| c);
    let reserve = flac_reserve(made.audio.format, cover.as_ref());
    let mut outputs = vec![audio_output(&made.audio, n, chapters, reserve, &audio_part)];
    outputs.append(&mut cover_outputs);
    let lyrics_raw = job.scratch.join("lyrics.lrc");
    let lyrics_text = match made.lyric_text() {
        Some(l) => match lyrics::planned(l, locate(l.key)?, &lyrics_raw, &mut inputs, &mut outputs)
        {
            Ok(text) => Some(text),
            Err(e) => {
                problems.push(format!("no lyrics: {e:#}"));
                None
            }
        },
        None => None,
    };

    let input_paths: Vec<&Path> = inputs.0.iter().map(PathBuf::as_path).collect();
    stage(&audio_part)?;
    let results = ffmpeg::run_outputs(runner, &input_paths, &outputs);
    let failed = |path: &Path| failure(&outputs, &results, path);
    let staged = (|| -> Result<Vec<PathBuf>> {
        if let Some(e) = failed(&audio_part) {
            return Err(anyhow!("{e}"));
        }
        let text = match (made.lyric_text(), lyrics_text) {
            (Some(_), Some(Some(text))) => Some(text),
            (Some(l), Some(None)) => match failed(&lyrics_raw) {
                Some(e) => {
                    problems.push(format!("no lyrics: {e}"));
                    None
                }
                None => fs::read(&lyrics_raw)
                    .map_err(|e| problems.push(format!("no lyrics: {e}")))
                    .ok()
                    .map(|b| lyrics::cleaned(&crate::lyrics::decode(&b), l)),
            },
            _ => None,
        }
        .filter(|text| text.lines().any(|l| !l.trim().is_empty()));
        let mut tags = made.tags.tags.to_vec();
        if let Some(text) = text
            .as_deref()
            .and_then(|t| embeddable(made, t, &mut problems))
        {
            tags.push(("LYRICS".to_string(), vec![text]));
        }
        let room = cover_room(made.audio.format.codec().container(), &tags);
        let picture = cover.and_then(|c| {
            let read = match failed(&c.path).filter(|_| c.by_run) {
                Some(e) => Err(anyhow!("{e}")),
                None => read_picture(&c.path, c.format),
            };
            read.and_then(|p| fit_cover(runner, p, room, &c.path, job.scratch))
                .map_err(|e| problems.push(format!("no cover: {e:#}")))
                .ok()
        });
        // A cover the tags cannot hold costs the song only its cover.
        let write = |picture| {
            write_tags(
                &audio_part,
                made.audio.format,
                &tags,
                made.tags.language,
                picture,
                made.tags.chapters,
            )
        };
        if let Err(e) = write(picture.clone()) {
            if picture.is_none() {
                return Err(e);
            }
            problems.push(format!("no cover: {e:#}"));
            write(None)?;
        }
        set_opus_header(&audio_part, made.tags.opus_header)?;
        problems.extend(verify(&audio_part, &tags, picture.is_some()));
        let mut held = match text {
            Some(text) => lyrics::contents(made, &text),
            None => vec![None; lyrics_rels.len()],
        };
        held.extend(cover_bytes(
            &cover_files,
            &cover_rels,
            failed,
            &mut problems,
        ));
        held.resize(lyrics_rels.len() + cover_rels.len(), None);
        held.extend(cover_bytes(
            &folder_files,
            &folder_rels,
            failed,
            &mut problems,
        ));
        stage_beside(job, (&rels, &parts), &held, &mut problems)
    })();
    let placed = staged.and_then(|written| {
        let beside = put_beside(job, &parts, &rels, &written)?;
        crate::library::put(job.library, &audio_part, &audio_rel, job.claim)?;
        Ok(beside)
    });
    match placed {
        Ok(beside) => Ok(Rendered {
            audio_bytes: size_of(&audio_path),
            audio: audio_rel,
            beside,
            problems,
        }),
        Err(e) => {
            for staged in std::iter::once(&audio_part).chain(&parts) {
                let _ = fs::remove_file(staged);
            }
            Err(e)
        }
    }
}

/// Why the output of `outputs` writing `path` failed, by `results`.
fn failure(outputs: &[Output], results: &[Result<()>], path: &Path) -> Option<String> {
    outputs
        .iter()
        .zip(results)
        .find(|(o, _)| o.path == path)
        .and_then(|(_, r)| r.as_ref().err().map(|e| format!("{e:#}")))
}

/// Each of `contents` staged at its file's `.part`, made anew: the files
/// of `rels` staged. A folder's file at a name a file not muman's holds
/// is left alone, said in `problems`.
fn stage_beside(
    job: &Job<'_>,
    (rels, parts): (&[PathBuf], &[PathBuf]),
    contents: &[Option<Vec<u8>>],
    problems: &mut Vec<String>,
) -> Result<Vec<PathBuf>> {
    let mut staged = Vec::new();
    for ((rel, part), content) in rels.iter().zip(parts).zip(contents) {
        let Some(content) = content else {
            continue;
        };
        if left_alone(job, rel) {
            problems.push(format!(
                "{} left alone: not muman's",
                crate::relpath::show(rel)
            ));
            continue;
        }
        std::io::Write::write_all(&mut stage(part)?, content)
            .with_context(|| format!("writing {}", part.display()))?;
        staged.push(rel.clone());
    }
    Ok(staged)
}

/// Put each of `rels` staged at `parts` that `written` names in its
/// place, and clear the files the song's last build left beside it that
/// are not written again, its lyrics that are none now among them; each
/// put with what it takes.
fn put_beside(
    job: &Job<'_>,
    parts: &[PathBuf],
    rels: &[PathBuf],
    written: &[PathBuf],
) -> Result<Vec<(PathBuf, u64)>> {
    let mut beside = Vec::new();
    for (staged, rel) in parts.iter().zip(rels) {
        if written.contains(rel) {
            crate::library::put(job.library, staged, rel, job.claim)?;
            beside.push((rel.clone(), size_of(&job.library.join(rel))));
        }
    }
    for gone in job.stale.iter().chain(rels) {
        if !written.contains(gone) && !left_alone(job, gone) {
            crate::library::clear(job.library, gone, job.claim)?;
        }
    }
    Ok(beside)
}

/// Whether `rel` is a file of the song's folder, of `job`'s plan, that a
/// file not muman's holds: the user's own cover, which the song leaves
/// alone and is written beside. Any other such file fails the song.
fn left_alone(job: &Job<'_>, rel: &Path) -> bool {
    folder_rels(job.stem, job.parts).iter().any(|f| f == rel)
        && !crate::library::may_change(job.library, rel, job.claim)
}

/// The most a FLAC metadata block holds, as its 24-bit length allows,
/// less room for a picture block's own fields. Ogg holds a cover as such
/// a block too, and ffmpeg reads no APEv2 tag past 16 MiB, so a cover is
/// held to it in every container.
const FLAC_BLOCK: usize = (1 << 24) - 1 - 1024;

/// The most bytes of picture `container` holds beside `tags`: a FLAC
/// block's; in Ogg, three quarters of it, as the block is a comment in
/// base64, four bytes for every three, and `lofty` reads no comment past
/// 16 MiB; in WavPack, what the APEv2 tag's 16 MiB leaves beside the
/// other items, embedded lyrics among them.
fn cover_room(container: Container, tags: &[(String, Vec<String>)]) -> usize {
    match container {
        Container::Ogg => FLAC_BLOCK / 4 * 3 - 1024,
        Container::WavPack => {
            let items: usize = tags
                .iter()
                .flat_map(|(name, values)| values.iter().map(move |v| name.len() + v.len() + 16))
                .sum();
            FLAC_BLOCK.saturating_sub(items)
        }
        Container::Flac | Container::Mp3 | Container::Mp4 => FLAC_BLOCK,
    }
}

/// The lyrics `made` holds inside the song, from their `text`, as the
/// tags take them; none, said in `problems`, for lyrics past what a FLAC
/// file's tags hold.
fn embeddable(made: &Parts<'_>, text: &str, problems: &mut Vec<String>) -> Option<String> {
    let text = lyrics::inside(made, text)?;
    let flac = made.audio.format.codec().container() == Container::Flac;
    if flac && text.len() > FLAC_BLOCK {
        problems.push(format!(
            "lyrics not embedded: {} is more than a FLAC file's tags hold",
            crate::units::Size(text.len() as u64)
        ));
        return None;
    }
    Some(text)
}

/// Write a song whose plan changed only in its tags, and in the lyrics
/// inside it, by writing them into the file it has at `audio`, with the
/// cover that file holds: what [`render`] would write, without decoding
/// or encoding anything. The lyrics inside are made anew from their
/// source where `lyrics_anew`, as [`render`] makes them, else kept as the
/// file holds them. Through `.part`, as [`render`] writes, put over the
/// file `job`'s claim holds.
// Removes a song's staged file when its tags fail.
#[expect(clippy::disallowed_methods)]
pub(crate) fn retag<R: Runner>(
    runner: &R,
    job: &Job<'_>,
    (audio, beside): (&Path, &[PathBuf]),
    lyrics_anew: bool,
) -> Result<Rendered> {
    use lofty::file::TaggedFileExt;

    let (made, library) = (job.parts, job.library);
    let mut problems = Vec::new();
    let anew = made
        .lyric_text()
        .filter(|_| lyrics_anew && made.lyrics.is_some())
        .map(|l| {
            lyrics::text(runner, job, l)
                .map_err(|e| problems.push(format!("no lyrics: {e:#}")))
                .ok()
                .flatten()
                .and_then(|t| embeddable(made, &t, &mut problems))
        });
    let path = library.join(audio);
    let staged = part(&path);
    // Into a new file, as a song written anew is: a copy would carry over
    // a read-only mode and refuse the tags.
    let mut into = stage(&staged)?;
    std::io::copy(
        &mut fs::File::open(&path).with_context(|| format!("opening {}", path.display()))?,
        &mut into,
    )
    .with_context(|| format!("copying {}", path.display()))?;
    drop(into);
    let result = (|| -> Result<Vec<String>> {
        // The copy is the file byte for byte, and a `.part` names no format.
        let held = crate::tags::read_tagged(&path)
            .with_context(|| format!("reading the tags of {}", path.display()))?;
        let tag = held.primary_tag();
        let picture = tag
            .and_then(|t| {
                t.pictures()
                    .iter()
                    .find(|p| p.pic_type() == PictureType::CoverFront)
                    .or_else(|| t.pictures().first())
                    .cloned()
            })
            .or_else(|| ape_cover(&path));
        let mut tags = made.tags.tags.to_vec();
        let kept = || {
            tag.and_then(|t| {
                t.get_string(ItemKey::Lyrics)
                    .or_else(|| t.get_string(ItemKey::UnsyncLyrics))
            })
            .filter(|_| made.lyrics.is_some())
            .map(str::to_string)
        };
        if let Some(text) = anew.clone().unwrap_or_else(kept) {
            tags.push(("LYRICS".to_string(), vec![text]));
        }
        let covered = picture.is_some();
        write_tags(
            &staged,
            made.audio.format,
            &tags,
            made.tags.language,
            picture,
            made.tags.chapters,
        )?;
        set_opus_header(&staged, made.tags.opus_header)?;
        Ok(verify(&staged, &tags, covered))
    })();
    let problems = match result {
        Ok(verified) => problems.into_iter().chain(verified).collect(),
        Err(e) => {
            let _ = fs::remove_file(&staged);
            return Err(e);
        }
    };
    if let Err(e) = crate::library::put(library, &staged, audio, job.claim) {
        let _ = fs::remove_file(&staged);
        return Err(e);
    }
    Ok(Rendered {
        audio_bytes: size_of(&path),
        audio: audio.to_path_buf(),
        beside: beside
            .iter()
            .map(|b| (b.clone(), size_of(&library.join(b))))
            .collect(),
        problems,
    })
}

/// The front cover of the WavPack file at `path`, which `lofty`'s
/// generic tag reads not from APEv2's `Cover Art (Front)`.
fn ape_cover(path: &Path) -> Option<Picture> {
    let key = PictureType::CoverFront.as_ape_key()?;
    let mut file = fs::File::open(path).ok()?;
    let options = ParseOptions::new().read_properties(false);
    let wv = lofty::wavpack::WavPackFile::read_from(&mut file, options).ok()?;
    match wv.ape()?.get(key)?.value() {
        ItemValue::Binary(bytes) => Picture::from_ape_bytes(key, bytes).ok(),
        _ => None,
    }
}

/// What of `tags` and the cover the file at `path` does not hold as they
/// were written, read back as a source's tags are read: each field but
/// the gains, which Opus holds rounded to its own steps.
fn verify(path: &Path, tags: &[(String, Vec<String>)], covered: bool) -> Vec<String> {
    use lofty::file::TaggedFileExt;

    let as_read = |t: BTreeMap<String, Vec<String>>| crate::tags::from_container(&t);
    let wanted = as_read(
        tags.iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
            .collect(),
    );
    let held = crate::tags::read_file(path)
        .map(as_read)
        .unwrap_or_default();
    // A container holding whole numbers holds a tempo rounded.
    let alike = |f: &Field, a: &[String], b: &[String]| {
        a == b || (f.spec().kind == crate::tags::Kind::Decimal && wholes(a) == wholes(b))
    };
    let differ: Vec<&str> = wanted
        .iter()
        .filter(|(f, offer)| {
            !f.is_loudness()
                && !held
                    .get(f)
                    .is_some_and(|o| alike(f, &o.values, &offer.values))
        })
        .map(|(f, _)| f.vorbis())
        .collect();
    let mut problems = Vec::new();
    if !differ.is_empty() {
        problems.push(format!("tags not held as written: {}", differ.join(", ")));
    }
    let pictured = || {
        crate::tags::read_tagged(path)
            .ok()
            .and_then(|f| f.primary_tag().map(|t| !t.pictures().is_empty()))
            .unwrap_or(false)
            || ape_cover(path).is_some()
    };
    if covered && !pictured() {
        problems.push("cover not held as written".to_string());
    }
    problems
}

/// The `gain` an Opus header holds, set in `path` once its tags are
/// written, as [`crate::ogg`] sets it.
// Sets the gain of a staged file's Opus header in place.
#[expect(clippy::disallowed_methods)]
fn set_opus_header(path: &Path, gain: Option<i16>) -> Result<()> {
    let Some(gain) = gain else {
        return Ok(());
    };
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    crate::ogg::set_output_gain(&mut file, gain)
        .with_context(|| format!("setting the gain of {}", path.display()))
}

fn size_of(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}

/// Put a song [`render`] wrote into the library folder `from` at its
/// place in `library`, as rendering it there would have: the files beside
/// it first, audio last, each through `.part`, and those of `stale` the
/// last build left that it has none of now gone: over or in place of only
/// the files `claim` holds, a file of its `folder` at a name a file not
/// muman's holds left alone, said.
// Removes a song's staged files when they cannot be put in place.
#[expect(clippy::disallowed_methods)]
pub fn place(
    from: &Path,
    rendered: &Rendered,
    library: &Path,
    (claim, folder): (&[Owned], &[PathBuf]),
    stale: &[PathBuf],
) -> Result<Rendered> {
    let left_alone =
        |rel: &PathBuf| folder.contains(rel) && !crate::library::may_change(library, rel, claim);
    let copy_in = |rel: &Path| -> Result<()> {
        let to = library.join(rel);
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let staged = part(&to);
        let copied = stage(&staged).and_then(|mut into| {
            let mut made = fs::File::open(from.join(rel))?;
            std::io::copy(&mut made, &mut into)?;
            Ok(())
        });
        copied
            .and_then(|()| crate::library::put(library, &staged, rel, claim))
            .inspect_err(|_| {
                let _ = fs::remove_file(&staged);
            })
            .with_context(|| format!("copying {} into the library", rel.display()))
    };
    let (left, beside): (Vec<_>, Vec<_>) = rendered
        .beside
        .iter()
        .cloned()
        .partition(|(file, _)| left_alone(file));
    for (file, _) in &beside {
        copy_in(file)?;
    }
    for gone in stale {
        if !left_alone(gone) && !beside.iter().any(|(b, _)| b == gone) {
            crate::library::clear(library, gone, claim)?;
        }
    }
    copy_in(&rendered.audio)?;
    let mut problems = rendered.problems.clone();
    problems.extend(
        left.iter()
            .map(|(file, _)| format!("{} left alone: not muman's", crate::relpath::show(file))),
    );
    Ok(Rendered {
        beside,
        problems,
        ..rendered.clone()
    })
}

/// What [`beside`] wrote: each file beside the song, with what it takes
/// in bytes, and what could not be written, each said once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beside {
    pub files: Vec<(PathBuf, u64)>,
    pub problems: Vec<String>,
}

/// Write the files `job`'s plan has beside the song, its lyrics and its
/// cover, from their sources, as [`render`] writes them, leaving the
/// song's audio file as it is: for a song whose file holds what it would
/// be written with, its tags aside, as `retag` writes them. Those of
/// `keep`, as they would be written already, are left as they are, and
/// nothing is made for them. A file the last build left beside it that
/// the plan has none of goes, as [`render`] clears it.
// Removes the staged files beside a song when they cannot be put in place.
#[expect(clippy::disallowed_methods)]
pub fn beside<R: Runner>(runner: &R, job: &Job<'_>, keep: &[PathBuf]) -> Result<Beside> {
    let made = job.parts;
    let mut problems = Vec::new();
    let (lyrics_rels, cover_rels) = (lyrics::rels(job.stem, made), cover_rels(job.stem, made));
    let folder_rels = folder_rels(job.stem, made);
    let due = |rels: &[PathBuf]| rels.iter().any(|r| !keep.contains(r));
    let mut held = match made.lyric_text() {
        Some(l) if due(&lyrics_rels) => {
            let text = lyrics::text(runner, job, l)
                .map_err(|e| problems.push(format!("no lyrics: {e:#}")))
                .ok()
                .flatten();
            text.map_or_else(
                || vec![None; lyrics_rels.len()],
                |t| lyrics::contents(made, &t),
            )
        }
        _ => Vec::new(),
    };
    held.resize(lyrics_rels.len(), None);
    if let Some((c, places)) = pictures(made.covers_beside()).filter(|_| due(&cover_rels)) {
        match cover_files(
            runner,
            job,
            c,
            (places, job.scratch),
            &cover_rels,
            &mut problems,
        ) {
            Ok(bytes) => held.extend(bytes),
            Err(e) => problems.push(format!("no cover: {e:#}")),
        }
    }
    held.resize(lyrics_rels.len() + cover_rels.len(), None);
    if let Some((f, places)) = pictures(made.folder_covers()).filter(|_| due(&folder_rels)) {
        let scratch = job.scratch.join("folder");
        match cover_files(
            runner,
            job,
            f,
            (places, &scratch),
            &folder_rels,
            &mut problems,
        ) {
            Ok(bytes) => held.extend(bytes),
            Err(e) => problems.push(format!("no folder cover: {e:#}")),
        }
    }
    held.resize(
        lyrics_rels.len() + cover_rels.len() + folder_rels.len(),
        None,
    );
    let (rels, held): (Vec<PathBuf>, Vec<Option<Vec<u8>>>) = lyrics_rels
        .into_iter()
        .chain(cover_rels)
        .chain(folder_rels)
        .zip(held)
        .filter(|(rel, _)| !keep.contains(rel))
        .unzip();
    let parts: Vec<PathBuf> = rels.iter().map(|r| part(&job.library.join(r))).collect();
    let placed = stage_beside(job, (&rels, &parts), &held, &mut problems)
        .and_then(|written| put_beside(job, &parts, &rels, &written));
    match placed {
        Ok(files) => Ok(Beside { files, problems }),
        Err(e) => {
            for staged in &parts {
                let _ = fs::remove_file(staged);
            }
            Err(e)
        }
    }
}

/// The vendor string ffmpeg names its Vorbis comments with, bit-exact.
const FFMPEG_VENDOR: &str = "ffmpeg";

/// Replace every comment the file carries with the plan's tags, the
/// cover and the chapters, keeping the encoder's vendor string and a
/// FLAC's [`CHANNEL_MASK`]. An MP4's chapters are ffmpeg's, and kept.
/// `lyrics` is the ISO 639-2 code of the language the `LYRICS` tag is in.
// Writes a staged file's tags in place.
#[expect(clippy::disallowed_methods)]
fn write_tags(
    path: &Path,
    format: Format,
    tags: &[(String, Vec<String>)],
    lyrics: Option<&str>,
    picture: Option<Picture>,
    chapters: &[Chapter],
) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let codec = format.codec();
    let written = match codec.container() {
        Container::Flac => {
            let comments = vorbis_comments(&mut file, path, codec, tags, picture, chapters)?;
            if write_flac_blocks(&mut file, &comments)
                .with_context(|| format!("writing tags to {}", path.display()))?
            {
                return Ok(());
            }
            file.seek(SeekFrom::Start(0))?;
            comments.save_to(&mut file, WriteOptions::default())
        }
        Container::Ogg => {
            let comments = vorbis_comments(&mut file, path, codec, tags, picture, chapters)?;
            file.seek(SeekFrom::Start(0))?;
            comments.save_to(&mut file, WriteOptions::default())
        }
        Container::Mp3 => {
            id3_tag(tags, lyrics, picture, chapters).save_to(&mut file, WriteOptions::default())
        }
        Container::Mp4 => ilst_of(tags, picture).save_to(&mut file, WriteOptions::default()),
        Container::WavPack => ape_tag(tags, picture).save_to(&mut file, WriteOptions::default()),
    };
    written.with_context(|| format!("writing tags to {}", path.display()))?;
    if codec.container() == Container::Mp4 {
        drop_mp4_padding(&mut file)
            .with_context(|| format!("writing tags to {}", path.display()))?;
    }
    Ok(())
}

/// Remove the `free` atoms in an MP4's `moov.udta.meta`, the room
/// `lofty` leaves where a tag shrank and fills on the next write, so the
/// file's bytes do not follow its history. Only where `moov` ends the
/// file, as ffmpeg's `ipod` muxer writes it: no chunk offset then points
/// past what moves.
fn drop_mp4_padding(file: &mut fs::File) -> std::io::Result<()> {
    use std::io::{Read as _, Write as _};

    let len = file.metadata()?.len();
    let mut at = 0;
    let mut header = [0_u8; 8];
    let moov_size = loop {
        if at + 8 > len {
            return Ok(());
        }
        file.seek(SeekFrom::Start(at))?;
        file.read_exact(&mut header)?;
        let size = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        if size < 8 {
            return Ok(());
        }
        if &header[4..] == b"moov" {
            break size;
        }
        at += size;
    };
    if at + moov_size != len {
        return Ok(());
    }
    let mut moov = vec![0; usize::try_from(moov_size).map_err(std::io::Error::other)?];
    file.seek(SeekFrom::Start(at))?;
    file.read_exact(&mut moov)?;
    let Some(udta) = child_atom(&moov, 0, 8, *b"udta") else {
        return Ok(());
    };
    let Some(meta) = child_atom(&moov, udta, 8, *b"meta") else {
        return Ok(());
    };
    let frees: Vec<(usize, usize)> = atoms(&moov, meta, 12)
        .filter(|&(.., kind)| kind == *b"free")
        .map(|(start, size, _)| (start, size))
        .collect();
    if frees.is_empty() {
        return Ok(());
    }
    let removed: usize = frees.iter().map(|(_, size)| size).sum();
    for &(start, size) in frees.iter().rev() {
        moov.drain(start..start + size);
    }
    for parent in [0, udta, meta] {
        let size = atom_size(&moov, parent) - removed;
        let size = u32::try_from(size).map_err(std::io::Error::other)?;
        moov[parent..parent + 4].copy_from_slice(&size.to_be_bytes());
    }
    file.seek(SeekFrom::Start(at))?;
    file.write_all(&moov)?;
    file.set_len(at + moov.len() as u64)
}

/// The 32-bit size of the atom starting at `start` in `bytes`.
fn atom_size(bytes: &[u8], start: usize) -> usize {
    let size: [u8; 4] = bytes[start..start + 4].try_into().unwrap_or_default();
    u32::from_be_bytes(size) as usize
}

/// The atoms inside the one at `parent` in `bytes`, its children starting
/// `skip` bytes in, each as its start, size and type; they stop at one
/// whose size does not fit inside it.
fn atoms(
    bytes: &[u8],
    parent: usize,
    skip: usize,
) -> impl Iterator<Item = (usize, usize, [u8; 4])> {
    let end = (parent + atom_size(bytes, parent)).min(bytes.len());
    let mut at = parent + skip;
    std::iter::from_fn(move || {
        let kind: [u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let size = atom_size(bytes, at);
        if size < 8 || at + size > end {
            return None;
        }
        let atom = (at, size, kind);
        at += size;
        Some(atom)
    })
}

/// Where the first atom of type `kind` inside the one at `parent` starts.
fn child_atom(bytes: &[u8], parent: usize, skip: usize, kind: [u8; 4]) -> Option<usize> {
    atoms(bytes, parent, skip)
        .find(|(_, _, k)| *k == kind)
        .map(|(start, ..)| start)
}

/// Bytes ffmpeg leaves as padding in a FLAC's metadata beyond its cover,
/// for the comments [`write_flac_blocks`] writes in their place.
const FLAC_RESERVE: u64 = 16 * 1024;

/// Padding reserved for a cover the run makes, whose size is not known
/// before it: most covers, as JPEGs, are under it.
const FLAC_COVER_RESERVE: u64 = 256 * 1024;

/// FLAC's block types.
const FLAC_PADDING: u8 = 1;
const FLAC_COMMENT: u8 = 4;
const FLAC_PICTURE: u8 = 6;

/// The padding a FLAC output is written with, so its tags and cover are
/// written into it in place; none for another container.
fn flac_reserve(format: Format, cover: Option<&CoverPlan>) -> Option<u64> {
    (format.codec().container() == Container::Flac).then(|| {
        let cover = cover.map_or(0, |c| {
            if c.by_run {
                FLAC_COVER_RESERVE
            } else {
                fs::metadata(&c.path).map_or(FLAC_COVER_RESERVE, |m| m.len())
            }
        });
        FLAC_RESERVE + cover
    })
}

/// Write `comments` into the FLAC `file`'s metadata where it stands: its
/// other blocks stay, the comments and pictures follow them as `lofty`
/// orders them, and what is left of the room is padding. `Ok(false)`,
/// the file untouched, when they do not fit, and `lofty` rewrites the
/// whole file: it moves every byte of audio for a tag of another length,
/// which costs a copy of the file for each song written.
fn write_flac_blocks(file: &mut fs::File, comments: &VorbisComments) -> std::io::Result<bool> {
    use std::io::{Read, Write};
    const MAX_BLOCK: usize = 0xFF_FFFF;
    file.seek(SeekFrom::Start(0))?;
    let mut marker = [0; 4];
    file.read_exact(&mut marker)?;
    if &marker != b"fLaC" {
        return Ok(false);
    }
    let mut blocks = Vec::new();
    let end = loop {
        let mut header = [0; 4];
        file.read_exact(&mut header)?;
        let (ty, len) = (
            header[0] & 0x7f,
            u32::from_be_bytes([0, header[1], header[2], header[3]]),
        );
        if matches!(ty, FLAC_PADDING | FLAC_COMMENT | FLAC_PICTURE) {
            file.seek(SeekFrom::Current(i64::from(len)))?;
        } else {
            let mut content = vec![0; len as usize];
            file.read_exact(&mut content)?;
            blocks.push((ty, content));
        }
        if header[0] & 0x80 != 0 {
            break file.stream_position()?;
        }
    };
    // As lofty: a file of no comments gets no block, and its vendor goes.
    if comments.items().len() > 0 {
        blocks.push((FLAC_COMMENT, comment_block(comments)));
    }
    for (picture, info) in comments.pictures() {
        blocks.push((FLAC_PICTURE, picture.as_flac_bytes(*info, false)));
    }
    let room = usize::try_from(end - u64::from(marker.len() as u32)).unwrap_or(usize::MAX);
    let used: usize = blocks.iter().map(|(_, c)| 4 + c.len()).sum();
    let Some(padding) = room.checked_sub(used + 4) else {
        return Ok(false);
    };
    if padding > MAX_BLOCK || blocks.iter().any(|(_, c)| c.len() > MAX_BLOCK) {
        return Ok(false);
    }
    let mut bytes = Vec::with_capacity(room);
    for (ty, content) in &blocks {
        bytes.push(*ty);
        bytes.extend_from_slice(&(content.len() as u32).to_be_bytes()[1..]);
        bytes.extend_from_slice(content);
    }
    bytes.push(FLAC_PADDING | 0x80);
    bytes.extend_from_slice(&(padding as u32).to_be_bytes()[1..]);
    bytes.resize(room, 0);
    file.seek(SeekFrom::Start(marker.len() as u64))?;
    file.write_all(&bytes)?;
    Ok(true)
}

/// A `VORBIS_COMMENT` block's content, as `lofty` encodes one: the
/// vendor, then each comment with a value as `NAME=value`, each behind
/// its length.
fn comment_block(comments: &VorbisComments) -> Vec<u8> {
    let mut bytes = Vec::new();
    let vendor = comments.vendor().as_bytes();
    bytes.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    bytes.extend_from_slice(vendor);
    let items: Vec<String> = comments
        .items()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    bytes.extend_from_slice(&(items.len() as u32).to_le_bytes());
    for item in &items {
        bytes.extend_from_slice(&(item.len() as u32).to_le_bytes());
        bytes.extend_from_slice(item.as_bytes());
    }
    bytes
}

/// `tag` with its frames in one order, by ID and a user frame's
/// description: converted from a generic tag, lofty orders them anew on
/// each run, so one song wrote different bytes from one build to the
/// next.
fn canonical_id3(tag: Id3v2Tag) -> Id3v2Tag {
    let order = |f: &Frame<'_>| {
        let detail = match f {
            Frame::UserText(t) => t.description.to_string(),
            _ => String::new(),
        };
        (f.id_str().to_string(), detail)
    };
    let mut frames: Vec<Frame<'static>> = tag.into_iter().collect();
    frames.sort_by_key(order);
    let mut sorted = Id3v2Tag::new();
    for frame in frames {
        sorted.insert(frame);
    }
    sorted
}

/// The plan's tags and the cover as Vorbis comments, keeping the
/// encoder's vendor string, read from `file`.
/// The comment ffmpeg writes a FLAC's speakers in where its channel
/// count alone does not say them, as for 5.1 with back speakers.
const CHANNEL_MASK: &str = "WAVEFORMATEXTENSIBLE_CHANNEL_MASK";

fn vorbis_comments(
    file: &mut fs::File,
    path: &Path,
    codec: Codec,
    tags: &[(String, Vec<String>)],
    picture: Option<Picture>,
    chapters: &[Chapter],
) -> Result<VorbisComments> {
    // The vendor string is all that is read: the audio's properties are
    // not, as lofty refuses an Opus stream of mapping family 255 for them.
    let options = ParseOptions::new().read_properties(false);
    let reading = || format!("reading {} as {codec}", path.display());
    let (vendor, mask) = match codec {
        Codec::Opus => (
            OpusFile::read_from(file, options)
                .with_context(reading)?
                .vorbis_comments()
                .vendor()
                .to_string(),
            None,
        ),
        Codec::Vorbis => (
            VorbisFile::read_from(file, options)
                .with_context(reading)?
                .vorbis_comments()
                .vendor()
                .to_string(),
            None,
        ),
        _ => {
            let flac = FlacFile::read_from(file, options).with_context(reading)?;
            let own = flac.vorbis_comments();
            (
                // ffmpeg writes no comment block for a FLAC of no tags, and
                // the vendor it would have named with it.
                own.map_or_else(|| FFMPEG_VENDOR.to_string(), |c| c.vendor().to_string()),
                own.and_then(|c| c.get(CHANNEL_MASK)).map(str::to_string),
            )
        }
    };
    let mut comments = VorbisComments::default();
    comments.set_vendor(vendor);
    if let Some(mask) = mask {
        comments.push(CHANNEL_MASK.to_string(), mask);
    }
    let opus;
    let tags = if codec == Codec::Opus {
        opus = opus_gains(tags);
        &opus
    } else {
        tags
    };
    for (key, values) in tags {
        let also = Field::named(key).map_or(&[][..], |f| f.spec().also_vorbis);
        for name in std::iter::once(key.as_str()).chain(also.iter().copied()) {
            for value in values {
                comments.push(name.to_string(), value.clone());
            }
        }
    }
    for (name, value) in crate::chapters::vorbis(chapters) {
        comments.push(name, value);
    }
    if let Some(picture) = picture {
        comments
            .insert_picture(picture, None)
            .context("reading the cover's dimensions")?;
    }
    Ok(comments)
}

/// `tags` as Opus holds gains: ReplayGain's gains as R128's, in 1/256 dB
/// against -23 LUFS, and no peaks, which Opus players are told to
/// ignore along with ReplayGain's tags.
fn opus_gains(tags: &[(String, Vec<String>)]) -> Vec<(String, Vec<String>)> {
    let r128 = |value: &String| {
        let db: f64 = value.trim_end_matches(" dB").parse().ok()?;
        let steps = ((db - crate::tags::R128_BELOW_REPLAYGAIN_DB) * 256.0).round();
        let steps = steps.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
        Some(steps.to_string())
    };
    tags.iter()
        .filter_map(|(key, values)| {
            let renamed = match Field::named(key) {
                Some(Field::TrackGain) => "R128_TRACK_GAIN",
                Some(Field::AlbumGain) => "R128_ALBUM_GAIN",
                Some(Field::TrackPeak | Field::AlbumPeak) => return None,
                _ => return Some((key.clone(), values.clone())),
            };
            let values: Vec<String> = values.iter().filter_map(r128).collect();
            (!values.is_empty()).then(|| (renamed.to_string(), values))
        })
        .collect()
}

/// The values `tags` holds under `field`'s name.
fn values_of(tags: &[(String, Vec<String>)], field: Field) -> &[String] {
    tags.iter()
        .find(|(k, _)| k == field.vorbis())
        .map_or(&[], |(_, v)| v.as_slice())
}

/// Numbers as a format holding whole ones takes them, rounded: a tempo of
/// 127.5 is 128 in `TBPM` and `tmpo`.
fn wholes(values: &[String]) -> Vec<i64> {
    values
        .iter()
        .filter_map(|v| v.parse::<f64>().ok())
        .filter(|n| n.is_finite())
        .map(|n| n.round() as i64)
        .collect()
}

/// The total a place's field is counted against, and the place a total
/// counts.
fn paired(field: Field) -> Option<Field> {
    match field {
        Field::Track => Some(Field::TrackTotal),
        Field::Disc => Some(Field::DiscTotal),
        Field::TrackTotal => Some(Field::Track),
        Field::DiscTotal => Some(Field::Disc),
        _ => None,
    }
}

/// A place and the total it is counted against as one text: `3/12`, or
/// `3` with no total.
fn place_text(tags: &[(String, Vec<String>)], field: Field, values: &[String]) -> String {
    let n = values
        .first()
        .map_or("", |v| v.split('/').next().unwrap_or(v));
    match paired(field).and_then(|t| values_of(tags, t).first()) {
        Some(total) => format!("{n}/{total}"),
        None => n.to_string(),
    }
}

/// A total written where its place cannot hold it: no place is set.
fn total_alone(tags: &[(String, Vec<String>)], field: Field) -> bool {
    paired(field).is_some_and(|place| values_of(tags, place).is_empty())
}

/// The language a `COMM` frame is marked with, as Picard marks it: Picard
/// reads a comment in any other as a tag apart, as `comment:jpn`.
const COMMENT_LANGUAGE: Lang = *b"eng";

/// The language `ID3v2` marks lyrics in `code` with: its ISO 639-2 code,
/// else `XXX`, unknown.
fn lyrics_language(code: Option<&str>) -> Lang {
    code.and_then(|code| <Lang>::try_from(code.to_ascii_lowercase().as_bytes()).ok())
        .filter(|code| code.iter().all(u8::is_ascii_lowercase))
        .unwrap_or(UNKNOWN_LANGUAGE)
}

/// The role a tag names in `ID3v2`'s involved people list, `TIPL`.
fn involved(key: &str) -> Option<&'static str> {
    match ItemKey::from_key(TagType::VorbisComments, key)? {
        ItemKey::Producer => Some("producer"),
        ItemKey::Arranger => Some("arranger"),
        ItemKey::Engineer => Some("engineer"),
        ItemKey::MixDj => Some("DJ-mix"),
        ItemKey::MixEngineer => Some("mix"),
        _ => None,
    }
}

/// A text frame of its own `lofty` names for a tag muman knows no field
/// for, as `TCOM` for `COMPOSER`; never a frame another field writes or
/// one holding no plain text.
fn text_frame(key: &str) -> Option<&'static str> {
    let id = ItemKey::from_key(TagType::VorbisComments, key)?.map_key(TagType::Id3v2)?;
    let taken = ["TXXX", "TIPL", "TMCL", "TRCK", "TPOS"];
    (id.len() == 4 && id.starts_with('T') && !id.starts_with("TD") && !taken.contains(&id))
        .then_some(id)
}

/// `ID3v2` frames written as Picard writes them: each field into the
/// frame [`crate::tags::TABLE`] names, several values null-separated as
/// version 2.4 separates them; a tag of no field into a text frame of its
/// own where it has one, else a user text frame of its name.
fn id3_tag(
    tags: &[(String, Vec<String>)],
    lyrics: Option<&str>,
    picture: Option<Picture>,
    chapters: &[Chapter],
) -> Id3v2Tag {
    let text = |id: &'static str, value: String| {
        FrameId::new(id)
            .ok()
            .map(|id| Frame::Text(TextInformationFrame::new(id, TextEncoding::UTF8, value)))
    };
    let user = |key: &str, value: String| {
        Frame::UserText(ExtendedTextFrame::new(
            TextEncoding::UTF8,
            key.to_string(),
            value,
        ))
    };
    let mut id3 = Id3v2Tag::new();
    let mut roles = Vec::new();
    for (key, values) in tags {
        let joined = values.join("\0");
        let frame = match Field::named(key).map(Field::spec) {
            Some(spec) => match spec.id3 {
                Id3::Text(id) => text(id, joined),
                Id3::Time(id) => values
                    .first()
                    .and_then(|v| v.parse::<Timestamp>().ok())
                    .zip(FrameId::new(id).ok())
                    .map(|(at, id)| {
                        Frame::Timestamp(TimestampFrame::new(id, TextEncoding::UTF8, at))
                    }),
                Id3::User(name) => Some(user(name, joined)),
                Id3::Ufid => values.first().map(|id| {
                    Frame::UniqueFileIdentifier(UniqueFileIdentifierFrame::new(
                        crate::tags::UFID_OWNER,
                        id.as_bytes().to_vec(),
                    ))
                }),
                Id3::Place(id) => text(id, place_text(tags, spec.field, values)),
                Id3::Within => total_alone(tags, spec.field).then(|| user(spec.vorbis, joined)),
                Id3::Whole(id) => {
                    let numbers: Vec<String> = wholes(values).iter().map(i64::to_string).collect();
                    text(id, numbers.join("\0"))
                }
                Id3::Key if values.iter().all(|k| crate::tags::id3_key(k)) => text("TKEY", joined),
                Id3::Key => Some(user("INITIALKEY", joined)),
                Id3::Comment => Some(Frame::Comment(CommentFrame::new(
                    TextEncoding::UTF8,
                    COMMENT_LANGUAGE,
                    String::new(),
                    joined,
                ))),
            },
            None => match key.as_str() {
                "LYRICS" => Some(Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(
                    TextEncoding::UTF8,
                    lyrics_language(lyrics),
                    String::new(),
                    joined,
                ))),
                _ => match (involved(key), text_frame(key)) {
                    (Some(role), _) => {
                        roles.extend(values.iter().map(|v| (role.into(), v.clone().into())));
                        None
                    }
                    (None, Some(id)) => text(id, joined),
                    (None, None) => Some(user(key, joined)),
                },
            },
        };
        if let Some(frame) = frame {
            id3.insert(frame);
        }
    }
    if !roles.is_empty()
        && let Ok(id) = FrameId::new("TIPL")
    {
        id3.insert(Frame::KeyValue(KeyValueFrame::new(
            id,
            TextEncoding::UTF8,
            roles,
        )));
    }
    if let Some(picture) = picture {
        id3.insert(Frame::Picture(AttachedPictureFrame::new(
            TextEncoding::UTF8,
            picture,
        )));
    }
    for frame in crate::chapters::id3(chapters) {
        id3.insert(frame);
    }
    canonical_id3(id3)
}

/// MP4's iTunes atoms written as Picard writes them: each field into the
/// atom [`crate::tags::TABLE`] names, a value of several one atom of
/// several data; a tag of no field into the text atom `lofty` names for
/// it, else iTunes' freeform atom of its name.
fn ilst_of(tags: &[(String, Vec<String>)], picture: Option<Picture>) -> Ilst {
    let free = |name: &str| AtomIdent::Freeform {
        mean: "com.apple.iTunes".into(),
        name: name.to_string().into(),
    };
    let own = |key: &str| -> AtomIdent<'static> {
        let named = match key {
            "LYRICS" => Some("©lyr"),
            _ => ItemKey::from_key(TagType::VorbisComments, key)
                .and_then(|k| k.map_key(TagType::Mp4Ilst)),
        };
        match named {
            Some(n) if n.starts_with('©') && n.chars().count() == 4 => {
                AtomIdent::Fourcc(crate::tags::fourcc(n))
            }
            Some(n) if ["aART", "soar", "soaa", "soal", "sonm", "soco"].contains(&n) => {
                AtomIdent::Fourcc(crate::tags::fourcc(n))
            }
            Some(n) if n.starts_with("----:com.apple.iTunes:") => {
                free(&n["----:com.apple.iTunes:".len()..])
            }
            _ => free(key),
        }
    };
    let mut ilst = Ilst::new();
    let number = |v: Option<&String>| v.and_then(|v| v.split('/').next()?.parse::<u32>().ok());
    for (key, values) in tags {
        let ident = match Field::named(key).map(Field::spec) {
            Some(spec) => match spec.mp4 {
                Mp4::Text(name) => AtomIdent::Fourcc(crate::tags::fourcc(name)),
                Mp4::Free(name) => free(name),
                // iTunes holds a tempo as a 16-bit integer, and a flag as a byte.
                Mp4::Whole(name) => {
                    if let Some(n) = wholes(values).first().and_then(|n| i16::try_from(*n).ok()) {
                        ilst.insert(Atom::new(
                            AtomIdent::Fourcc(crate::tags::fourcc(name)),
                            AtomData::Unknown {
                                code: lofty::mp4::DataType::BeSignedInteger,
                                data: n.to_be_bytes().to_vec(),
                            },
                        ));
                    }
                    continue;
                }
                Mp4::Flag(name) => {
                    let on = values.first().is_some_and(|v| v == "1");
                    ilst.insert(Atom::new(
                        AtomIdent::Fourcc(crate::tags::fourcc(name)),
                        AtomData::Bool(on),
                    ));
                    continue;
                }
                Mp4::Place(_) | Mp4::Within => {
                    let n = number(values.first());
                    match (spec.field, n) {
                        (Field::Track, Some(n)) => ilst.set_track(n),
                        (Field::Disc, Some(n)) => ilst.set_disk(n),
                        (Field::TrackTotal, Some(n)) => ilst.set_track_total(n),
                        (Field::DiscTotal, Some(n)) => ilst.set_disk_total(n),
                        _ => {}
                    }
                    continue;
                }
            },
            None => own(key),
        };
        let data = values.iter().cloned().map(AtomData::UTF8).collect();
        if let Some(atom) = Atom::from_collection(ident, data) {
            ilst.insert(atom);
        }
    }
    if let Some(picture) = picture {
        ilst.insert_picture(picture);
    }
    ilst
}

/// APEv2 items written as Picard writes them, several values
/// null-separated; the cover as `Cover Art (Front)`, named by a file name
/// whose extension tells readers its format.
fn ape_tag(tags: &[(String, Vec<String>)], picture: Option<Picture>) -> ApeTag {
    let mut ape = ApeTag::new();
    for (key, values) in tags {
        let (name, value) = if let Some(spec) = Field::named(key).map(Field::spec) {
            match spec.ape {
                Some(name) if matches!(spec.id3, Id3::Place(_)) => {
                    (name.to_string(), place_text(tags, spec.field, values))
                }
                Some(name) => (name.to_string(), values.join("\0")),
                None if total_alone(tags, spec.field) => {
                    (spec.vorbis.to_string(), values.join("\0"))
                }
                None => continue,
            }
        } else {
            let name = match key.as_str() {
                "LYRICS" => "Lyrics".to_string(),
                _ => ItemKey::from_key(TagType::VorbisComments, key)
                    .and_then(|k| k.map_key(TagType::Ape))
                    .map_or_else(|| key.clone(), str::to_string),
            };
            (name, values.join("\0"))
        };
        if let Ok(item) = ApeItem::new(name, ItemValue::Text(value)) {
            ape.insert(item);
        }
    }
    if let Some(mut picture) = picture {
        let extension = match picture.mime_type() {
            Some(MimeType::Png) => "png",
            _ => "jpg",
        };
        picture.set_description(Some(format!("cover.{extension}")));
        let key = PictureType::CoverFront
            .as_ape_key()
            .unwrap_or("Cover Art (Front)");
        if let Ok(item) = ApeItem::new(key.to_string(), ItemValue::Binary(picture.as_ape_bytes())) {
            ape.insert(item);
        }
    }
    ape
}

#[cfg(test)]
mod tests;
