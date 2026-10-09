//! One song written to the library from its plan: the audio copied or
//! encoded, tagged and covered, and its lyrics as a `.lrc` beside it.
//!
//! Every file is written as `<name>.part` and renamed into place, the
//! audio last, so a player scanning the library never sees half a track.
//! A cover or lyrics that fail are reported and the song is written
//! without them. A song that fails leaves the folders made for it, since
//! another song rendered at the same time may be writing into one; the
//! sync clears the folders failures leave once every song is written.
//!
//! Each codec goes into its own container, as [`crate::codec`] says. Tags
//! come from the plan alone: the container's, yt-dlp's description and URL
//! among them, are dropped. They are Vorbis comments in Ogg and FLAC; in
//! MP3 and MP4 each comment `lofty` knows becomes that format's own frame
//! or atom, and any other a `TXXX` frame or an iTunes freeform atom of its
//! name; in WavPack each is an APEv2 item, by the name `lofty` gives it or
//! its own. Opus holds ReplayGain's gains as R128's and no peaks, as
//! RFC 7845 tells its players to read them. The cover is a front-cover
//! picture written through `lofty`: a
//! JPEG or PNG without borders as its own bytes, any other converted to
//! PNG and cropped to the content inside a video frame's bars. A FLAC
//! metadata block holds at most 16 MiB, so a cover past it is made a JPEG
//! at most 3000 pixels wide, and lyrics past it are not embedded. Lyrics are
//! cleaned of cues, symbols and credits and moved by the plan's offset;
//! lyrics from a `.lrc` file need no ffmpeg at all.
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
use lofty::id3::v2::{ExtendedTextFrame, Frame, Id3v2Tag};
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst};
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OggPictureStorage, OpusFile, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType};

use crate::codec::{Codec, Container};
use crate::facts::{CoverAt, LRC_ARGS, LyricsAt};
use crate::ffmpeg::{self, Output};
use crate::lyrics;
use crate::resolve::{CoverRef, Format, LyricsRef, Plan};
use crate::runner::Runner;
use crate::settings::LyricsPlacement;
use crate::source::SourceKey;
use crate::store::Located;
use crate::tags::Field;

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
    /// What the audio and lyrics files take, in bytes, as written.
    pub audio_bytes: u64,
    pub lyrics_bytes: u64,
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

fn audio_output(plan: &Plan, input: usize, path: &Path) -> Output {
    let format = plan.format;
    let mut args: Vec<String> = vec!["-map".into(), format!("{input}:{}", plan.audio.index)];
    args.extend(match format {
        Format::Copy { .. } => vec!["-c:a".into(), "copy".into()],
        Format::Encode {
            codec,
            kbps,
            adapt,
            mix,
        } => codec.encoder_args(kbps, adapt, mix, plan.chain()),
    });
    // Tags are written from the plan alone; ffmpeg would carry the
    // container's, chapters as comments among them.
    args.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(String::from));
    args.extend(format.codec().muxer_args());
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
    let mut outputs = vec![audio_output(plan, n, &audio_part)];
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
        let flac = plan.format.codec().container() == Container::Flac;
        let picture = cover.and_then(|c| {
            let read = match failed(&c.path).filter(|_| c.by_run) {
                Some(e) => Err(anyhow!("{e}")),
                None => read_picture(&c.path, c.mime),
            };
            read.and_then(|p| {
                if flac {
                    fit_flac(runner, p, &c.path, job.scratch)
                } else {
                    Ok(p)
                }
            })
            .map_err(|e| problems.push(format!("no cover: {e:#}")))
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
                    .map(|t| lyrics::shift_lrc(&lyrics::clean_lrc(&t), l.shift_ms, l.stretch_ppm)),
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
            if flac && text.len() > FLAC_BLOCK {
                problems.push(format!(
                    "lyrics not embedded: {} is more than a FLAC file's tags hold",
                    crate::units::Size(text.len() as u64)
                ));
            } else {
                tags.push(("LYRICS".to_string(), vec![text.clone()]));
            }
        }
        // A cover the tags cannot hold costs the song only its cover.
        if let Err(e) = write_tags(&audio_part, plan.format, &tags, picture.clone()) {
            if picture.is_none() {
                return Err(e);
            }
            problems.push(format!("no cover: {e:#}"));
            write_tags(&audio_part, plan.format, &tags, None)?;
        }
        set_opus_header(&audio_part, plan)?;
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
                audio_bytes: size_of(&audio_path),
                lyrics_bytes: lyrics.as_ref().map_or(0, |_| size_of(&lyrics_path)),
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

/// The most a FLAC metadata block holds, as its 24-bit length allows,
/// less room for a picture block's own fields.
const FLAC_BLOCK: usize = (1 << 24) - 1 - 1024;

/// The widest a cover too large for FLAC is made.
const FLAC_COVER_WIDTH: u32 = 3000;

/// `picture`, read from `path`, as a FLAC file can hold it: as it is when
/// it fits, else made a JPEG at most [`FLAC_COVER_WIDTH`] pixels wide.
fn fit_flac<R: Runner>(
    runner: &R,
    picture: Picture,
    path: &Path,
    scratch: &Path,
) -> Result<Picture> {
    if picture.data().len() <= FLAC_BLOCK {
        return Ok(picture);
    }
    let smaller = scratch.join("cover-fit.jpg");
    let scale = format!("scale='min({FLAC_COVER_WIDTH},iw)':-2");
    let output = Output::of(
        &[
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-vf",
            &scale,
            "-c:v",
            "mjpeg",
            "-q:v",
            "2",
            "-f",
            "image2",
        ],
        &smaller,
    );
    runner.run(&ffmpeg::outputs_command(&[path], &[output]))?;
    let fitted = read_picture(&smaller, MimeType::Jpeg)?;
    if fitted.data().len() > FLAC_BLOCK {
        return Err(anyhow!(
            "{} even as a JPEG, more than a FLAC file holds",
            crate::units::Size(fitted.data().len() as u64)
        ));
    }
    Ok(fitted)
}

/// Write a song whose plan changed only in its tags by writing them into
/// the file it has at `audio`, with the cover and the embedded lyrics
/// that file holds: what [`render`] would write, without decoding or
/// encoding anything. Through `.part`, as [`render`] writes.
pub fn retag(library: &Path, audio: &Path, lyrics: Option<&Path>, plan: &Plan) -> Result<Rendered> {
    use lofty::file::TaggedFileExt;

    let path = library.join(audio);
    let staged = part(&path);
    // Into a new file, as a song written anew is: a copy would carry over
    // a read-only mode and refuse the tags.
    (|| -> std::io::Result<u64> {
        std::io::copy(&mut fs::File::open(&path)?, &mut fs::File::create(&staged)?)
    })()
    .with_context(|| format!("copying {}", path.display()))?;
    let result = (|| -> Result<()> {
        // The copy is the file byte for byte, and a `.part` names no format.
        let held = lofty::read_from_path(&path)
            .with_context(|| format!("reading the tags of {}", path.display()))?;
        let tag = held.primary_tag();
        let picture = tag.and_then(|t| {
            t.pictures()
                .iter()
                .find(|p| p.pic_type() == PictureType::CoverFront)
                .or_else(|| t.pictures().first())
                .cloned()
        });
        let mut tags = plan.tags.clone();
        let embedded = plan.lyrics.as_ref().is_some_and(|l| l.placement.embedded());
        if let Some(text) = tag
            .and_then(|t| {
                t.get_string(ItemKey::Lyrics)
                    .or_else(|| t.get_string(ItemKey::UnsyncLyrics))
            })
            .filter(|_| embedded)
        {
            tags.push(("LYRICS".to_string(), vec![text.to_string()]));
        }
        write_tags(&staged, plan.format, &tags, picture)?;
        set_opus_header(&staged, plan)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&staged);
        return Err(e);
    }
    rename(&staged, &path)?;
    Ok(Rendered {
        audio_bytes: size_of(&path),
        lyrics_bytes: lyrics.map_or(0, |l| size_of(&library.join(l))),
        audio: audio.to_path_buf(),
        lyrics: lyrics.map(Path::to_path_buf),
        problems: Vec::new(),
    })
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
/// place in `library`, as rendering it there would have: lyrics first,
/// audio last, each through `.part`, and the last build's lyrics gone
/// when it has none now.
pub fn place(from: &Path, rendered: &Rendered, library: &Path) -> Result<Rendered> {
    let copy_in = |rel: &Path| -> Result<()> {
        let to = library.join(rel);
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let staged = part(&to);
        fs::copy(from.join(rel), &staged)
            .with_context(|| format!("copying {} into the library", rel.display()))?;
        rename(&staged, &to)
    };
    match &rendered.lyrics {
        Some(lyrics) => copy_in(lyrics)?,
        None => remove_if_present(&library.join(rendered.audio.with_extension("lrc")))?,
    }
    copy_in(&rendered.audio)?;
    Ok(rendered.clone())
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
        lyrics.stretch_ppm,
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
/// cover, keeping the encoder's vendor string and a FLAC's
/// [`CHANNEL_MASK`].
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
    let codec = format.codec();
    let written = match codec.container() {
        Container::Ogg | Container::Flac => {
            let comments = vorbis_comments(&mut file, path, codec, tags, picture)?;
            file.seek(SeekFrom::Start(0))?;
            comments.save_to(&mut file, WriteOptions::default())
        }
        Container::Mp3 => {
            let (generic, own) = generic_tag(TagType::Id3v2, tags, picture);
            let mut id3 = Id3v2Tag::from(generic);
            for (key, values) in own {
                id3.insert(Frame::UserText(ExtendedTextFrame::new(
                    TextEncoding::UTF8,
                    key.clone(),
                    values.join("\0"),
                )));
            }
            canonical_id3(id3).save_to(&mut file, WriteOptions::default())
        }
        Container::Mp4 => {
            let (generic, own) = generic_tag(TagType::Mp4Ilst, tags, picture);
            let mut ilst = Ilst::from(generic);
            for (key, values) in own {
                let ident = AtomIdent::Freeform {
                    mean: "com.apple.iTunes".into(),
                    name: key.clone().into(),
                };
                let data = values.iter().cloned().map(AtomData::UTF8).collect();
                if let Some(atom) = Atom::from_collection(ident, data) {
                    ilst.insert(atom);
                }
            }
            ilst.save_to(&mut file, WriteOptions::default())
        }
        Container::WavPack => {
            let (generic, own) = generic_tag(TagType::Ape, tags, picture);
            let mut ape = ApeTag::from(generic);
            for (key, values) in own {
                if let Ok(item) = ApeItem::new(key.clone(), ItemValue::Text(values.join("\0"))) {
                    ape.insert(item);
                }
            }
            ape.save_to(&mut file, WriteOptions::default())
        }
    };
    written.with_context(|| format!("writing tags to {}", path.display()))
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
) -> Result<VorbisComments> {
    let options = ParseOptions::new();
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
                own.map(|c| c.vendor().to_string()).unwrap_or_default(),
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
        for value in values {
            comments.push(key.clone(), value.clone());
        }
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
        #[allow(clippy::cast_possible_truncation)]
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

/// The tags `lofty` can map into `kind`, as a generic tag holding the
/// cover too, and the rest, which the format keeps under their own names.
fn generic_tag(
    kind: TagType,
    tags: &[(String, Vec<String>)],
    picture: Option<Picture>,
) -> (Tag, Vec<(&String, &Vec<String>)>) {
    // The format writes these as frames of their own kind, or folds them
    // into another, so they map to no key of it.
    let special = [
        ItemKey::TrackNumber,
        ItemKey::TrackTotal,
        ItemKey::DiscNumber,
        ItemKey::DiscTotal,
        ItemKey::UnsyncLyrics,
    ];
    let mut generic = Tag::new(kind);
    let mut own = Vec::new();
    for (key, values) in tags {
        let item = ItemKey::from_key(TagType::VorbisComments, key).map(|item| match item {
            // ID3v2 keeps lyrics in an unsynchronised lyrics frame.
            ItemKey::Lyrics if item.map_key(kind).is_none() => ItemKey::UnsyncLyrics,
            item => item,
        });
        match item.filter(|item| item.map_key(kind).is_some() || special.contains(item)) {
            Some(item) => {
                for value in values {
                    generic.push(TagItem::new(item, ItemValue::Text(value.clone())));
                }
            }
            None => own.push((key, values)),
        }
    }
    if let Some(picture) = picture {
        generic.push_picture(picture);
    }
    (generic, own)
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
