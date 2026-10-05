//! One song written to the library from its plan: the audio copied or
//! encoded, tagged and covered, and its lyrics as a `.lrc` beside it.
//!
//! Every file is written as `<name>.part` and renamed into place, the
//! audio last, so a player scanning the library never sees half a track.
//! A cover or lyrics that fail are reported and the song is written
//! without them.
//!
//! Opus is written into Ogg. Tags come from the plan alone: the
//! container's, yt-dlp's description and URL among them, are dropped. The
//! cover is a front-cover picture written through `lofty`: a JPEG or PNG
//! without borders as its own bytes, any other converted to PNG and
//! cropped to the content inside a video frame's bars. Lyrics are cleaned
//! of cues, symbols and credits and moved by the plan's offset; lyrics
//! from a `.lrc` file need no ffmpeg at all.
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
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OggPictureStorage, OpusFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::TagExt;

use crate::facts::{CoverAt, LRC_ARGS, LyricsAt};
use crate::ffmpeg::{self, Output};
use crate::lyrics;
use crate::resolve::{CoverRef, Format, LyricsRef, Plan};
use crate::runner::Runner;
use crate::settings::LyricsPlacement;
use crate::source::SourceKey;
use crate::store::Located;

#[derive(Debug)]
pub struct Job<'a> {
    pub plan: &'a Plan,
    /// The song's path in the library, without its extension.
    pub stem: &'a Path,
    pub library: &'a Path,
    pub sources: &'a BTreeMap<SourceKey, Located>,
    /// An empty folder of this song's own for intermediates.
    pub scratch: &'a Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// Paths in the library.
    pub audio: PathBuf,
    pub lyrics: Option<PathBuf>,
    /// What could not be added, each said once.
    pub problems: Vec<String>,
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

fn audio_output(format: Format, input: usize, index: u32, path: &Path) -> Output {
    let mut args: Vec<String> = vec!["-map".into(), format!("{input}:{index}")];
    let codec: Vec<String> = match format {
        Format::OpusCopy | Format::FlacCopy => vec!["-c:a".into(), "copy".into()],
        Format::FlacEncode => vec!["-c:a".into(), "flac".into()],
        Format::OpusEncode { kbps, .. } => vec![
            "-c:a".into(),
            "libopus".into(),
            "-b:a".into(),
            format!("{kbps}k"),
            "-vbr".into(),
            "on".into(),
        ],
    };
    args.extend(codec);
    // Tags are written from the plan alone; ffmpeg would carry the
    // container's, chapters as comments among them.
    args.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(String::from));
    if format.extension() == "opus" {
        // libopusfile refuses a stream that starts before zero.
        args.extend(["-avoid_negative_ts", "make_non_negative", "-f", "opus"].map(String::from));
    } else {
        args.extend(["-f", "flac"].map(String::from));
    }
    Output::new(args.into_iter().map(OsString::from).collect(), path)
}

/// Where the cover's bytes are read from once the run is over, and
/// whether one of its outputs writes them there.
struct CoverPlan {
    path: PathBuf,
    mime: MimeType,
    by_run: bool,
}

fn is_kept_as_is(mimetype: &str) -> Option<MimeType> {
    match mimetype {
        "image/jpeg" => Some(MimeType::Jpeg),
        "image/png" => Some(MimeType::Png),
        _ => None,
    }
}

/// The ffmpeg output options that write one picture as PNG, cropped to
/// its content when it has borders.
fn png_output(input: usize, stream: &str, cover: &CoverRef, path: &Path) -> Output {
    let mut args: Vec<String> = vec![
        "-map".into(),
        format!("{input}:{stream}"),
        "-frames:v".into(),
        "1".into(),
    ];
    if let Some(r) = cover.crop {
        args.push("-vf".into());
        args.push(format!("crop={}:{}:{}:{}", r.width, r.height, r.x, r.y));
    }
    args.extend(["-c:v", "png", "-f", "image2"].map(String::from));
    Output::new(args.into_iter().map(OsString::from).collect(), path)
}

#[allow(clippy::too_many_lines)]
pub fn render<R: Runner>(runner: &R, job: &Job<'_>) -> Result<Rendered> {
    let plan = job.plan;
    let locate = |key: &SourceKey| {
        job.sources
            .get(key)
            .ok_or_else(|| anyhow!("{key} is not on disk"))
    };
    let audio_rel = with_extension(job.stem, plan.format.extension());
    let lyrics_rel = with_extension(job.stem, "lrc");
    let (audio_path, lyrics_path) = (job.library.join(&audio_rel), job.library.join(&lyrics_rel));
    let dir = audio_path.parent().unwrap_or(job.library);
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let (audio_part, lyrics_part) = (part(&audio_path), part(&lyrics_path));
    fs::create_dir_all(job.scratch)?;

    let mut inputs = Inputs::default();
    let source = locate(&plan.audio.key)?;
    let n = inputs.index(&source.path);
    let mut outputs = vec![audio_output(plan.format, n, plan.audio.index, &audio_part)];
    let mut problems = Vec::new();

    let cover = match &plan.cover {
        Some(c) => match cover_plan(
            runner,
            c,
            locate(&c.key)?,
            job.scratch,
            &mut inputs,
            &mut outputs,
        ) {
            Ok(p) => Some(p),
            Err(e) => {
                problems.push(format!("no cover: {e:#}"));
                None
            }
        },
        None => None,
    };
    let lyrics_raw = job.scratch.join("lyrics.lrc");
    let lyrics_text = match &plan.lyrics {
        Some(l) => match lyrics_plan(l, locate(&l.key)?, &lyrics_raw, &mut inputs, &mut outputs) {
            Ok(text) => Some(text),
            Err(e) => {
                problems.push(format!("no lyrics: {e:#}"));
                None
            }
        },
        None => None,
    };

    let input_paths: Vec<&Path> = inputs.0.iter().map(PathBuf::as_path).collect();
    let results = ffmpeg::run_outputs(runner, &input_paths, &outputs);
    let failed = |path: &Path| {
        outputs
            .iter()
            .zip(&results)
            .find(|(o, _)| o.path == path)
            .and_then(|(_, r)| r.as_ref().err().map(|e| format!("{e:#}")))
    };
    let staged = (|| -> Result<Option<PathBuf>> {
        if let Some(e) = failed(&audio_part) {
            return Err(anyhow!("{e}"));
        }
        let picture = cover.and_then(|c| {
            let read = match failed(&c.path).filter(|_| c.by_run) {
                Some(e) => Err(anyhow!("{e}")),
                None => read_picture(&c.path, c.mime),
            };
            read.map_err(|e| problems.push(format!("no cover: {e:#}")))
                .ok()
        });
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
                    .map(|t| lyrics::shift_lrc(&lyrics::clean_lrc(&t), l.shift_ms)),
            },
            _ => None,
        }
        .filter(|text| text.lines().any(|l| !l.trim().is_empty()));
        let placement = plan.lyrics.as_ref().map(|l| l.placement);
        let mut tags = plan.tags.clone();
        if let Some(text) = text
            .as_ref()
            .filter(|_| placement.is_some_and(LyricsPlacement::embedded))
        {
            tags.push(("LYRICS".to_string(), vec![text.clone()]));
        }
        write_tags(&audio_part, plan.format, &tags, picture)?;
        match text.filter(|_| placement.is_some_and(LyricsPlacement::sidecar)) {
            Some(text) => {
                fs::write(&lyrics_part, text)
                    .with_context(|| format!("writing {}", lyrics_part.display()))?;
                Ok(Some(lyrics_rel.clone()))
            }
            None => Ok(None),
        }
    })();
    match staged {
        Ok(lyrics) => {
            if lyrics.is_some() {
                rename(&lyrics_part, &lyrics_path)?;
            } else {
                // A song that has no lyrics now must not keep the last build's.
                remove_if_present(&lyrics_path)?;
            }
            rename(&audio_part, &audio_path)?;
            Ok(Rendered {
                audio: audio_rel,
                lyrics,
                problems,
            })
        }
        Err(e) => {
            let _ = fs::remove_file(&audio_part);
            let _ = fs::remove_file(&lyrics_part);
            Err(e)
        }
    }
}

/// Plan the cover: a JPEG or PNG without borders is used as it is; any
/// other is converted, an attachment dumped out of its file first.
fn cover_plan<R: Runner>(
    runner: &R,
    cover: &CoverRef,
    source: &Located,
    scratch: &Path,
    inputs: &mut Inputs,
    outputs: &mut Vec<Output>,
) -> Result<CoverPlan> {
    let made = scratch.join("cover.png");
    let as_is = is_kept_as_is(&cover.mimetype).filter(|_| cover.crop.is_none());
    let file = match &cover.at {
        CoverAt::Picture { index } => {
            let n = inputs.index(&source.path);
            // A picture stream's one packet is the attached file's bytes.
            if let Some(mime) = as_is {
                let copied = scratch.join("cover");
                outputs.push(Output::of(
                    &[
                        "-map",
                        &format!("{n}:{index}"),
                        "-c",
                        "copy",
                        "-frames:v",
                        "1",
                        "-f",
                        "image2",
                    ],
                    &copied,
                ));
                return Ok(CoverPlan {
                    path: copied,
                    mime,
                    by_run: true,
                });
            }
            outputs.push(png_output(n, &index.to_string(), cover, &made));
            return Ok(CoverPlan {
                path: made,
                mime: MimeType::Png,
                by_run: true,
            });
        }
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
    };
    if let Some(mime) = as_is {
        return Ok(CoverPlan {
            path: file,
            mime,
            by_run: false,
        });
    }
    let n = inputs.index(&file);
    outputs.push(png_output(n, "v:0", cover, &made));
    Ok(CoverPlan {
        path: made,
        mime: MimeType::Png,
        by_run: true,
    })
}

/// Plan the lyrics: a file's text now, or a subtitle stream the run
/// converts to `raw`, read once it ran (`None`).
fn lyrics_plan(
    lyrics: &LyricsRef,
    source: &Located,
    raw: &Path,
    inputs: &mut Inputs,
    outputs: &mut Vec<Output>,
) -> Result<Option<String>> {
    let file = match &lyrics.at {
        LyricsAt::Stream { index } => {
            let n = inputs.index(&source.path);
            let mut args: Vec<OsString> = vec!["-map".into(), format!("{n}:{index}").into()];
            args.extend(LRC_ARGS.iter().map(OsString::from));
            outputs.push(Output::new(args, raw));
            return Ok(None);
        }
        LyricsAt::Sidecar => source
            .lyrics
            .clone()
            .ok_or_else(|| anyhow!("the lyrics beside {} are gone", source.path.display()))?,
        LyricsAt::File => source.path.clone(),
    };
    let text =
        lyrics::decode(&fs::read(&file).with_context(|| format!("reading {}", file.display()))?);
    Ok(Some(lyrics::shift_lrc(
        &lyrics::clean_lrc(&text),
        lyrics.shift_ms,
    )))
}

fn read_picture(path: &Path, mime: MimeType) -> Result<Picture> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Picture::unchecked(bytes)
        .pic_type(PictureType::CoverFront)
        .mime_type(mime)
        .build())
}

/// Replace every comment the file carries with the plan's tags and the
/// cover, keeping the encoder's vendor string.
fn write_tags(
    path: &Path,
    format: Format,
    tags: &[(String, Vec<String>)],
    picture: Option<Picture>,
) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let vendor = if format.extension() == "opus" {
        OpusFile::read_from(&mut file, ParseOptions::new())
            .with_context(|| format!("reading {} as Ogg Opus", path.display()))?
            .vorbis_comments()
            .vendor()
            .to_string()
    } else {
        FlacFile::read_from(&mut file, ParseOptions::new())
            .with_context(|| format!("reading {} as FLAC", path.display()))?
            .vorbis_comments()
            .map(|c| c.vendor().to_string())
            .unwrap_or_default()
    };
    let mut comments = VorbisComments::default();
    comments.set_vendor(vendor);
    for (key, values) in tags {
        for value in values {
            comments.push(key.clone(), value.clone());
        }
    }
    if let Some(picture) = picture {
        comments
            .insert_picture(picture, None)
            .context("reading the cover's dimensions")?;
    }
    // lofty identifies the file it writes to from the current position,
    // which reading left at the end.
    file.seek(SeekFrom::Start(0))?;
    comments
        .save_to(&mut file, WriteOptions::default())
        .with_context(|| format!("writing tags to {}", path.display()))
}

fn remove_if_present(path: &Path) -> Result<()> {
    match crate::atomic::remove(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r.with_context(|| format!("removing {}", path.display())),
    }
}

fn rename(from: &Path, to: &Path) -> Result<()> {
    crate::atomic::rename(from, to)
        .with_context(|| format!("renaming {} → {}", from.display(), to.display()))
}

#[cfg(test)]
mod tests;
