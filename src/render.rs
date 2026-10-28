//! One song written to the library from its plan: the audio copied or
//! encoded and tagged, and its cover and lyrics inside it and in the
//! files beside it its plan names.
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
//! all the same. Ogg, MP3, MP4 and WavPack are `lofty`'s to write.
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
//! each file of it beside the song, as its plan's places say: kept, a
//! JPEG or PNG without borders as its own bytes, a file's only where they
//! are its format; any other made by ffmpeg in the place's format, cropped
//! to the content inside a video frame's bars and scaled down to its
//! longest side, places alike sharing one picture. A picture ffmpeg
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
//! no lyrics past it. Lyrics are cleaned as the plan's lyric switches say
//! ([`crate::clean::lyrics`]), knowing the song's title and artists, and
//! moved by its offset; lyrics from a `.lrc` file need no ffmpeg at all.
//!
//! The audio keeps its source's chapters, as [`crate::chapters`] writes
//! them in each container: ffmpeg writes an MP4's from an `FFMETADATA`
//! file, the tag writers every other's beside the tags.
//!
//! A song whose plan changed in its tags alone is tagged again in place
//! ([`retag`]), and the files beside it written again where they changed
//! ([`beside`]), as a build writes them, with the cover and the lyrics its
//! file holds, read as each container
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

use anyhow::{Context, Result, anyhow, bail};
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

use crate::chapters::Chapter;
use crate::clean;
use crate::codec::{Codec, Container};
use crate::facts::{CoverAt, LRC_ARGS, LyricsAt};
use crate::ffmpeg::{self, Output};
use crate::library::Owned;
use crate::lyrics;
use crate::quality::Rect;
use crate::resolve::{CoverPlace, CoverRef, Format, ImageFormat, LyricsRef, Plan};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::store::Located;
use crate::tags::{Field, Id3, Mp4};

#[derive(Debug)]
pub struct Job<'a> {
    pub plan: &'a Plan,
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
    plan: &Plan,
    input: usize,
    chapters: Option<usize>,
    reserve: Option<u64>,
    path: &Path,
) -> Output {
    let format = plan.format;
    let mut args: Vec<String> = vec!["-map".into(), format!("{input}:{}", plan.audio.index)];
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
        } => codec.encoder_args(kbps, adapt, mix, plan.chain()),
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

/// Where the picture one place of the cover takes is read from once the
/// run is over, what it is, and whether one of the run's outputs writes
/// it there.
struct CoverPlan {
    path: PathBuf,
    format: ImageFormat,
    by_run: bool,
}

/// The format of a picture of `mimetype` that may be kept as its own
/// bytes: JPEG or PNG.
fn kept(mimetype: &str) -> Option<ImageFormat> {
    match mimetype {
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/png" => Some(ImageFormat::Png),
        _ => None,
    }
}

/// How a cover ffmpeg converts is made, raised whenever the bytes it
/// converts to change: [`CoverRef::made`].
pub const COVER_MAKING: u32 = 1;

/// The options every picture ffmpeg encodes is written with: swscale and
/// the JPEG encoder on their exact paths, and no comment naming the
/// encoder's version.
pub const PICTURE_ARGS: [&str; 6] = [
    "-sws_flags",
    "accurate_rnd+bitexact",
    "-flags:v",
    "+bitexact",
    "-dct",
    "int",
];

/// Whether a cover of `mimetype`, cropped or not, is converted rather
/// than written as its own bytes where it is kept as it is.
pub(crate) fn converts(mimetype: &str, cropped: bool) -> bool {
    cropped || kept(mimetype).is_none()
}

/// The ffmpeg output options that make the picture in `stream` of input
/// `input` for `place`: cropped to `crop`, the content inside a frame's
/// bars, scaled down to fit its `max_side`, in its format at its
/// quality. ffmpeg's WebP encoder takes an even width and height, and
/// drops a last odd column or row.
fn picture_output(
    input: usize,
    stream: &str,
    crop: Option<Rect>,
    place: &CoverPlace,
    path: &Path,
) -> Output {
    let mut args: Vec<String> = vec![
        "-map".into(),
        format!("{input}:{stream}"),
        "-frames:v".into(),
        "1".into(),
    ];
    let mut filters = Vec::new();
    if let Some(r) = crop {
        filters.push(format!("crop={}:{}:{}:{}", r.width, r.height, r.x, r.y));
    }
    if let Some(side) = place.max_side {
        filters.push(format!(
            "scale=w='min({side},iw)':h='min({side},ih)':force_original_aspect_ratio=decrease"
        ));
    }
    if !filters.is_empty() {
        args.push("-vf".into());
        args.push(filters.join(","));
    }
    args.extend(PICTURE_ARGS.map(String::from));
    let quality = place.quality.unwrap_or(crate::settings::DEFAULT_QUALITY);
    match place.format {
        ImageFormat::Png => args.extend(["-c:v", "png"].map(String::from)),
        ImageFormat::Jpeg => args.extend([
            "-c:v".into(),
            "mjpeg".into(),
            "-q:v".into(),
            jpeg_scale(quality).to_string(),
        ]),
        ImageFormat::Webp => args.extend([
            "-c:v".into(),
            "libwebp".into(),
            "-quality".into(),
            quality.to_string(),
        ]),
    }
    args.extend(["-f", "image2"].map(String::from));
    Output::new(args.into_iter().map(OsString::from).collect(), path)
}

/// The JPEG encoder's scale for `quality` from 1 to 100: its finest step,
/// 2, at 100, its coarsest, 31, at 1, and evenly between.
fn jpeg_scale(quality: u8) -> u32 {
    2 + (u32::from(100 - quality.clamp(1, 100)) * 29 + 49) / 99
}

// The steps read in the order they run; split up, the order would be in the call sites.
#[expect(clippy::too_many_lines)]
pub fn render<R: Runner>(runner: &R, job: &Job<'_>) -> Result<Rendered> {
    let plan = job.plan;
    let locate = |key: &SourceKey| {
        job.sources
            .get(key)
            .ok_or_else(|| anyhow!("{key} is not on disk"))
    };
    let audio_rel = with_extension(job.stem, plan.format.extension());
    let (lyrics_rels, cover_rels) = (lyrics_rels(job.stem, plan), cover_rels(job.stem, plan));
    let rels: Vec<PathBuf> = lyrics_rels.iter().chain(&cover_rels).cloned().collect();
    let audio_path = job.library.join(&audio_rel);
    let dir = audio_path.parent().unwrap_or(job.library);
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let audio_part = part(&audio_path);
    let parts: Vec<PathBuf> = rels.iter().map(|r| part(&job.library.join(r))).collect();
    fs::create_dir_all(job.scratch)?;

    let mut inputs = Inputs::default();
    let source = locate(&plan.audio.key)?;
    let n = inputs.index(&source.path);
    let chapters = if plan.format.codec().container() == Container::Mp4 && !plan.chapters.is_empty()
    {
        let name = crate::atomic::Name::new("chapters.ffmeta".to_string())?;
        crate::atomic::write(
            job.scratch,
            &name,
            crate::chapters::ffmetadata(&plan.chapters).as_bytes(),
        )?;
        Some(inputs.index(&job.scratch.join(name.as_str())))
    } else {
        None
    };
    let mut problems = Vec::new();
    let mut cover_outputs = Vec::new();
    let covers = match &plan.cover {
        Some(c) => match cover_plans(
            runner,
            c,
            c.places(),
            locate(&c.key)?,
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
    let (inside, cover_files): (Vec<_>, Vec<_>) =
        covers.into_iter().partition(|(p, _)| p.file.is_none());
    let cover = inside.into_iter().next().map(|(_, c)| c);
    let reserve = flac_reserve(plan, cover.as_ref());
    let mut outputs = vec![audio_output(plan, n, chapters, reserve, &audio_part)];
    outputs.append(&mut cover_outputs);
    let lyrics_raw = job.scratch.join("lyrics.lrc");
    let lyrics_text = match &plan.lyrics {
        Some(l) => match lyrics_plan(
            l,
            &lyrics_context(plan, l),
            locate(&l.key)?,
            &lyrics_raw,
            &mut inputs,
            &mut outputs,
        ) {
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
        let flac = plan.format.codec().container() == Container::Flac;
        let text = match (&plan.lyrics, lyrics_text) {
            (Some(_), Some(Some(text))) => Some(text),
            (Some(l), Some(None)) => match failed(&lyrics_raw) {
                Some(e) => {
                    problems.push(format!("no lyrics: {e}"));
                    None
                }
                None => fs::read(&lyrics_raw)
                    .map(|b| lyrics::decode(&b))
                    .map_err(|e| problems.push(format!("no lyrics: {e}")))
                    .ok()
                    .map(|t| cleaned(&t, l, &lyrics_context(plan, l))),
            },
            _ => None,
        }
        .filter(|text| text.lines().any(|l| !l.trim().is_empty()));
        let mut tags = plan.tags.clone();
        let inside = plan.lyrics.as_ref().filter(|l| l.tag()).zip(text.as_ref());
        if let Some(text) = inside.and_then(|(l, text)| {
            let form = l.inside.clone().unwrap_or_default();
            crate::cues::write(text, form.format, form.words, None)
        }) {
            if flac && text.len() > FLAC_BLOCK {
                problems.push(format!(
                    "lyrics not embedded: {} is more than a FLAC file's tags hold",
                    crate::units::Size(text.len() as u64)
                ));
            } else {
                tags.push(("LYRICS".to_string(), vec![text.clone()]));
            }
        }
        let room = cover_room(plan.format.codec().container(), &tags);
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
        if let Err(e) = write_tags(
            &audio_part,
            plan.format,
            &tags,
            plan.lyrics_language(),
            picture.clone(),
            &plan.chapters,
        ) {
            if picture.is_none() {
                return Err(e);
            }
            problems.push(format!("no cover: {e:#}"));
            let language = plan.lyrics_language();
            write_tags(
                &audio_part,
                plan.format,
                &tags,
                language,
                None,
                &plan.chapters,
            )?;
        }
        set_opus_header(&audio_part, plan)?;
        problems.extend(verify(&audio_part, &tags, picture.is_some()));
        let mut held = match plan.lyrics.as_ref().zip(text) {
            Some((l, text)) => contents(l, &text),
            None => vec![None; lyrics_rels.len()],
        };
        held.extend(cover_bytes(
            &cover_files,
            &cover_rels,
            failed,
            &mut problems,
        ));
        stage_beside(&rels, &parts, &held)
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

/// The files of lyrics `plan` has beside the song at `stem`.
fn lyrics_rels(stem: &Path, plan: &Plan) -> Vec<PathBuf> {
    plan.lyrics.as_ref().map_or_else(Vec::new, |l| {
        l.files()
            .iter()
            .map(|f| crate::outputs::lyrics_file(stem, f))
            .collect()
    })
}

/// The files of the cover `plan` has beside the song at `stem`.
fn cover_rels(stem: &Path, plan: &Plan) -> Vec<PathBuf> {
    plan.cover.as_ref().map_or_else(Vec::new, |c| {
        c.files()
            .iter()
            .filter_map(|p| crate::outputs::cover_file(stem, p))
            .collect()
    })
}

/// Why the output of `outputs` writing `path` failed, by `results`.
fn failure(outputs: &[Output], results: &[Result<()>], path: &Path) -> Option<String> {
    outputs
        .iter()
        .zip(results)
        .find(|(o, _)| o.path == path)
        .and_then(|(_, r)| r.as_ref().err().map(|e| format!("{e:#}")))
}

/// What each file of `lyrics` beside the song holds, from their `text`
/// as kept; none for one whose format takes no such lyrics.
fn contents(lyrics: &LyricsRef, text: &str) -> Vec<Option<Vec<u8>>> {
    lyrics
        .files()
        .iter()
        .map(|f| crate::cues::write(text, f.format, f.words, lyrics.end_ms).map(String::into_bytes))
        .collect()
}

/// Each of `contents` staged at its file's `.part`, made anew: the files
/// of `rels` staged.
fn stage_beside(
    rels: &[PathBuf],
    parts: &[PathBuf],
    contents: &[Option<Vec<u8>>],
) -> Result<Vec<PathBuf>> {
    let mut staged = Vec::new();
    for ((rel, part), content) in rels.iter().zip(parts).zip(contents) {
        let Some(content) = content else {
            continue;
        };
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
        if !written.contains(gone) {
            crate::library::clear(job.library, gone, job.claim)?;
        }
    }
    Ok(beside)
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

/// The widest a cover too large to hold is made.
const COVER_WIDTH: u32 = 3000;

/// `picture`, read from `path`, in `room` bytes ([`cover_room`]): as it
/// is when it fits, else made a JPEG at most [`COVER_WIDTH`] pixels wide.
fn fit_cover<R: Runner>(
    runner: &R,
    picture: Picture,
    room: usize,
    path: &Path,
    scratch: &Path,
) -> Result<Picture> {
    if picture.data().len() <= room {
        return Ok(picture);
    }
    let smaller = scratch.join("cover-fit.jpg");
    let scale = format!("scale='min({COVER_WIDTH},iw)':-2");
    let mut args = vec!["-map", "0:v:0", "-frames:v", "1", "-vf", &scale];
    args.extend(PICTURE_ARGS);
    args.extend(["-c:v", "mjpeg", "-q:v", "2", "-f", "image2"]);
    let output = Output::of(&args, &smaller);
    runner.run(&ffmpeg::outputs_command(&[path], &[output]))?;
    let fitted = read_picture(&smaller, ImageFormat::Jpeg)?;
    if fitted.data().len() > room {
        return Err(anyhow!(
            "{} even as a JPEG, more than the file's tags hold",
            crate::units::Size(fitted.data().len() as u64)
        ));
    }
    Ok(fitted)
}

/// Write a song whose plan changed only in its tags by writing them into
/// the file it has at `audio`, with the cover and the embedded lyrics
/// that file holds: what [`render`] would write, without decoding or
/// encoding anything. Through `.part`, as [`render`] writes, put over the
/// file `claim` holds.
pub fn retag(
    library: &Path,
    audio: &Path,
    beside: &[PathBuf],
    plan: &Plan,
    claim: &[Owned],
) -> Result<Rendered> {
    use lofty::file::TaggedFileExt;

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
        let mut tags = plan.tags.clone();
        let embedded = plan.lyrics.as_ref().is_some_and(LyricsRef::tag);
        if let Some(text) = tag
            .and_then(|t| {
                t.get_string(ItemKey::Lyrics)
                    .or_else(|| t.get_string(ItemKey::UnsyncLyrics))
            })
            .filter(|_| embedded)
        {
            tags.push(("LYRICS".to_string(), vec![text.to_string()]));
        }
        let covered = picture.is_some();
        let language = plan.lyrics_language();
        write_tags(
            &staged,
            plan.format,
            &tags,
            language,
            picture,
            &plan.chapters,
        )?;
        set_opus_header(&staged, plan)?;
        Ok(verify(&staged, &tags, covered))
    })();
    let problems = match result {
        Ok(problems) => problems,
        Err(e) => {
            let _ = fs::remove_file(&staged);
            return Err(e);
        }
    };
    if let Err(e) = crate::library::put(library, &staged, audio, claim) {
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

/// The gain `plan` gives its Opus header, set in `path` once its tags
/// are written, as [`crate::ogg`] sets it.
fn set_opus_header(path: &Path, plan: &Plan) -> Result<()> {
    let Some(gain) = plan.opus_header() else {
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
/// the files `claim` holds.
pub fn place(
    from: &Path,
    rendered: &Rendered,
    library: &Path,
    claim: &[Owned],
    stale: &[PathBuf],
) -> Result<Rendered> {
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
    for (file, _) in &rendered.beside {
        copy_in(file)?;
    }
    for gone in stale {
        if !rendered.beside.iter().any(|(b, _)| b == gone) {
            crate::library::clear(library, gone, claim)?;
        }
    }
    copy_in(&rendered.audio)?;
    Ok(rendered.clone())
}

/// Plan the picture each of `places` takes: its own bytes where the
/// place keeps a JPEG or PNG without borders as it is, in a file only
/// where they are the file's format; any other made by the run, places
/// alike sharing one.
fn cover_plans<R: Runner>(
    runner: &R,
    cover: &CoverRef,
    places: Vec<CoverPlace>,
    source: &Located,
    scratch: &Path,
    inputs: &mut Inputs,
    outputs: &mut Vec<Output>,
) -> Result<Vec<(CoverPlace, CoverPlan)>> {
    let file = picture_file(runner, &cover.at, source, scratch)?;
    // A file's bytes say what it is, whatever its name or its container
    // says: a PNG named `.jpg` is kept as PNG, and a WebP converted.
    let own = match &file {
        Some(f) => crate::picture::sniff_file(f).map_or_else(|| kept(&cover.mimetype), kept),
        None => kept(&cover.mimetype),
    }
    .filter(|_| cover.crop.is_none());
    let from = file.clone().unwrap_or_else(|| source.path.clone());
    let stream = match &cover.at {
        CoverAt::Picture { index } => index.to_string(),
        _ => "v:0".to_string(),
    };
    let mut made: Vec<(CoverPlace, PathBuf)> = Vec::new();
    let mut plans = Vec::new();
    for place in places {
        let keeps = own.filter(|f| place.keep && (place.file.is_none() || *f == place.format));
        let plan = match (keeps, &file) {
            (Some(format), Some(file)) => CoverPlan {
                path: file.clone(),
                format,
                by_run: false,
            },
            (Some(format), None) => {
                // A picture stream's one packet is the attached file's bytes.
                let copied = scratch.join("cover");
                if !outputs.iter().any(|o| o.path == copied) {
                    let n = inputs.index(&from);
                    outputs.push(Output::of(
                        &[
                            "-map",
                            &format!("{n}:{stream}"),
                            "-c",
                            "copy",
                            "-frames:v",
                            "1",
                            "-f",
                            "image2",
                        ],
                        &copied,
                    ));
                }
                CoverPlan {
                    path: copied,
                    format,
                    by_run: true,
                }
            }
            (None, _) => {
                let alike = CoverPlace {
                    file: None,
                    keep: false,
                    ..place.clone()
                };
                let path = if let Some((_, path)) = made.iter().find(|(m, _)| *m == alike) {
                    path.clone()
                } else {
                    let path =
                        scratch.join(format!("cover-{}.{}", made.len(), place.format.extension()));
                    let n = inputs.index(&from);
                    outputs.push(picture_output(n, &stream, cover.crop, &place, &path));
                    made.push((alike, path.clone()));
                    path
                };
                CoverPlan {
                    path,
                    format: place.format,
                    by_run: true,
                }
            }
        };
        plans.push((place, plan));
    }
    Ok(plans)
}

/// The file the cover at `at` of `source` is in, as its own bytes; none
/// for a picture stream, which a run reads. An attachment is dumped out of
/// its file into `scratch`, as is an MP4's picture.
fn picture_file<R: Runner>(
    runner: &R,
    at: &CoverAt,
    source: &Located,
    scratch: &Path,
) -> Result<Option<PathBuf>> {
    Ok(Some(match at {
        CoverAt::Picture { .. } => return Ok(None),
        CoverAt::Attachment { ordinal } => {
            let dumped = scratch.join("attachment");
            runner.run(&ffmpeg::dump_command(&[(
                source.path.as_path(),
                vec![(*ordinal, dumped.clone())],
            )]))?;
            dumped
        }
        CoverAt::Sidecar(n) => source
            .covers
            .get(*n)
            .cloned()
            .ok_or_else(|| anyhow!("the picture beside {} is gone", source.path.display()))?,
        CoverAt::File => source.path.clone(),
        CoverAt::Atom { index } => {
            let bytes = crate::picture::mp4_covers(&source.path)
                .into_iter()
                .nth(*index)
                .ok_or_else(|| anyhow!("{} holds no picture {index}", source.path.display()))?;
            let name = crate::atomic::Name::new("atom".to_string())?;
            crate::atomic::write(scratch, &name, &bytes)?;
            scratch.join(name.as_str())
        }
    }))
}

/// The bytes of each of `files`, the cover's places beside the song
/// named `rels`, as the run made them, each checked to be its format;
/// none, said in `problems`, for one it could not make.
fn cover_bytes(
    files: &[(CoverPlace, CoverPlan)],
    rels: &[PathBuf],
    failed: impl Fn(&Path) -> Option<String>,
    problems: &mut Vec<String>,
) -> Vec<Option<Vec<u8>>> {
    files
        .iter()
        .zip(rels)
        .map(|((place, c), rel)| {
            let read = match failed(&c.path).filter(|_| c.by_run) {
                Some(e) => Err(anyhow!("{e}")),
                None => fs::read(&c.path).with_context(|| format!("reading {}", c.path.display())),
            };
            let wanted = place.format.mimetype();
            read.and_then(|bytes| match crate::picture::sniff(&bytes) {
                Some(is) if is == wanted => Ok(bytes),
                is => Err(anyhow!(
                    "the picture is {}, not {wanted}",
                    is.unwrap_or("of no format known")
                )),
            })
            .map_err(|e| {
                let name = rel.file_name().unwrap_or_default().to_string_lossy();
                problems.push(format!("no cover file {name}: {e:#}"));
            })
            .ok()
        })
        .collect()
}

/// The song's title and artists its lyrics are cleaned by, where a rule
/// switched on reads them.
pub(crate) fn lyrics_names(plan: &Plan) -> Option<(Option<&str>, Vec<&str>)> {
    let lyrics = plan.lyrics.as_ref()?;
    let reads = clean::lyrics::NAMED
        .iter()
        .any(|rule| !lyrics.clean_off.iter().any(|off| off == rule));
    reads.then(|| {
        let ctx = lyrics_context(plan, lyrics);
        (ctx.title, ctx.artists)
    })
}

/// What lyric cleaning knows of the song a plan writes.
fn lyrics_context<'p>(plan: &'p Plan, lyrics: &LyricsRef) -> clean::lyrics::Context<'p> {
    let values = |name: &'static str| {
        plan.tags
            .iter()
            .filter(move |(k, _)| k == name)
            .flat_map(|(_, v)| v.iter().map(String::as_str))
    };
    clean::lyrics::Context {
        title: values("TITLE").next(),
        artists: values("ARTIST").chain(values("ARTISTS")).collect(),
        subtitle: matches!(lyrics.at, LyricsAt::Stream { .. }),
    }
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
/// be written with, its tags aside, as [`retag`] writes them. A file the
/// last build left beside it that the plan has none of goes, as
/// [`render`] clears it.
pub fn beside<R: Runner>(runner: &R, job: &Job<'_>) -> Result<Beside> {
    let plan = job.plan;
    let mut problems = Vec::new();
    let (lyrics_rels, cover_rels) = (lyrics_rels(job.stem, plan), cover_rels(job.stem, plan));
    let mut held = match &plan.lyrics {
        Some(l) if !lyrics_rels.is_empty() => {
            let text = lyrics_text(runner, job, l)
                .map_err(|e| problems.push(format!("no lyrics: {e:#}")))
                .ok()
                .flatten();
            text.map_or_else(|| vec![None; lyrics_rels.len()], |t| contents(l, &t))
        }
        _ => Vec::new(),
    };
    if let Some(c) = plan.cover.as_ref().filter(|_| !cover_rels.is_empty()) {
        match cover_files(runner, job, c, &cover_rels, &mut problems) {
            Ok(bytes) => held.extend(bytes),
            Err(e) => problems.push(format!("no cover: {e:#}")),
        }
    }
    let rels: Vec<PathBuf> = lyrics_rels.into_iter().chain(cover_rels).collect();
    let parts: Vec<PathBuf> = rels.iter().map(|r| part(&job.library.join(r))).collect();
    let placed = stage_beside(&rels, &parts, &held)
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

/// The bytes of `cover`'s files beside the song, `rels`, made from its
/// source in a run of their own, as [`render`] makes them.
fn cover_files<R: Runner>(
    runner: &R,
    job: &Job<'_>,
    cover: &CoverRef,
    rels: &[PathBuf],
    problems: &mut Vec<String>,
) -> Result<Vec<Option<Vec<u8>>>> {
    let source = job
        .sources
        .get(&cover.key)
        .ok_or_else(|| anyhow!("{} is not on disk", cover.key))?;
    fs::create_dir_all(job.scratch)?;
    let (mut inputs, mut outputs) = (Inputs::default(), Vec::new());
    let files = cover_plans(
        runner,
        cover,
        cover.files(),
        source,
        job.scratch,
        &mut inputs,
        &mut outputs,
    )?;
    let paths: Vec<&Path> = inputs.0.iter().map(PathBuf::as_path).collect();
    let results = ffmpeg::run_outputs(runner, &paths, &outputs);
    let failed = |path: &Path| failure(&outputs, &results, path);
    Ok(cover_bytes(&files, rels, failed, problems))
}

/// The text of `lyrics` as `job`'s plan writes them, read from their
/// source, a subtitle stream through ffmpeg alone; none when no line is.
fn lyrics_text<R: Runner>(runner: &R, job: &Job<'_>, lyrics: &LyricsRef) -> Result<Option<String>> {
    let source = job
        .sources
        .get(&lyrics.key)
        .ok_or_else(|| anyhow!("{} is not on disk", lyrics.key))?;
    fs::create_dir_all(job.scratch)?;
    let ctx = lyrics_context(job.plan, lyrics);
    let raw = job.scratch.join("lyrics.lrc");
    let (mut inputs, mut outputs) = (Inputs::default(), Vec::new());
    let mut text = lyrics_plan(lyrics, &ctx, source, &raw, &mut inputs, &mut outputs)?;
    if text.is_none() {
        let paths: Vec<&Path> = inputs.0.iter().map(PathBuf::as_path).collect();
        for result in ffmpeg::run_outputs(runner, &paths, &outputs) {
            result?;
        }
        let read = fs::read(&raw).with_context(|| format!("reading {}", raw.display()))?;
        text = Some(cleaned(&lyrics::decode(&read), lyrics, &ctx));
    }
    Ok(text.filter(|t| t.lines().any(|l| !l.trim().is_empty())))
}

/// `text` cleaned as the plan's lyrics say, then moved to the audio.
fn cleaned(text: &str, lyrics: &LyricsRef, ctx: &clean::lyrics::Context<'_>) -> String {
    let text = clean::lyrics::clean(text, ctx, &lyrics.clean_off).text;
    lyrics::shift_lrc(&text, lyrics.shift_ms, lyrics.stretch_ppm)
}

/// Plan the lyrics: a file's text now, or a subtitle stream the run
/// converts to `raw`, read once it ran (`None`).
fn lyrics_plan(
    lyrics: &LyricsRef,
    ctx: &clean::lyrics::Context<'_>,
    source: &Located,
    raw: &Path,
    inputs: &mut Inputs,
    outputs: &mut Vec<Output>,
) -> Result<Option<String>> {
    if let LyricsAt::Stream { index } = &lyrics.at {
        let n = inputs.index(&source.path);
        let mut args: Vec<OsString> = vec!["-map".into(), format!("{n}:{index}").into()];
        args.extend(LRC_ARGS.iter().map(OsString::from));
        outputs.push(Output::new(args, raw));
        return Ok(None);
    }
    let text = if lyrics.at == LyricsAt::Tag {
        crate::tags::lyrics_in(&source.path)
            .ok_or_else(|| anyhow!("the lyrics in {} are gone", source.path.display()))?
            .0
    } else {
        let file = lyrics_file(&lyrics.at, source)
            .ok_or_else(|| anyhow!("the lyrics beside {} are gone", source.path.display()))?;
        lyrics::decode(&fs::read(&file).with_context(|| format!("reading {}", file.display()))?)
    };
    Ok(Some(cleaned(&text, lyrics, ctx)))
}

/// The file lyrics at `at` of `source` are read from; none for a stream
/// or a tag, or lyrics beside it that are gone.
pub(crate) fn lyrics_file(at: &LyricsAt, source: &Located) -> Option<PathBuf> {
    match at {
        LyricsAt::Stream { .. } | LyricsAt::Tag => None,
        LyricsAt::Sidecar => source.lyrics.clone(),
        LyricsAt::File => Some(source.path.clone()),
    }
}

/// The picture at `path`, of `format` unless its bytes say what it is:
/// JPEG or PNG, which every container holds.
fn read_picture(path: &Path, format: ImageFormat) -> Result<Picture> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let format = match crate::picture::sniff(&bytes) {
        Some(sniffed) => kept(sniffed).ok_or_else(|| {
            anyhow!(
                "the cover {} is {sniffed}, which no file holds",
                path.display()
            )
        })?,
        None => format,
    };
    let mime = match format {
        ImageFormat::Jpeg => MimeType::Jpeg,
        ImageFormat::Png => MimeType::Png,
        ImageFormat::Webp => bail!("the cover {} is WebP, which no file holds", path.display()),
    };
    Ok(Picture::unchecked(bytes)
        .pic_type(PictureType::CoverFront)
        .mime_type(mime)
        .build())
}

/// The vendor string ffmpeg names its Vorbis comments with, bit-exact.
const FFMPEG_VENDOR: &str = "ffmpeg";

/// Replace every comment the file carries with the plan's tags, the
/// cover and the chapters, keeping the encoder's vendor string and a
/// FLAC's [`CHANNEL_MASK`]. An MP4's chapters are ffmpeg's, and kept.
/// `lyrics` is the ISO 639-2 code of the language the `LYRICS` tag is in.
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
    written.with_context(|| format!("writing tags to {}", path.display()))
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
fn flac_reserve(plan: &Plan, cover: Option<&CoverPlan>) -> Option<u64> {
    (plan.format.codec().container() == Container::Flac).then(|| {
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
