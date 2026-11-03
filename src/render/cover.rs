//! A song's cover and its folder's as the library holds them, inside the
//! song and in the files beside it, made from their parts of the plan
//! alone (`artifact::CoverPart`): the picture its own bytes where a place
//! keeps them, or made by ffmpeg with [`PICTURE_ARGS`], and a cover too
//! large for the tags made a JPEG that fits.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use lofty::picture::{MimeType, Picture, PictureType};

use super::{Inputs, Job, failure};
use crate::artifact::{CoverPart, CoverSource, Parts};
use crate::facts::CoverAt;
use crate::ffmpeg::{self, Output};
use crate::quality::Rect;
use crate::resolve::{CoverPlace, ImageFormat};
use crate::runner::Runner;
use crate::store::Located;

/// Where the picture one place of the cover takes is read from once the
/// run is over, what it is, and whether one of the run's outputs writes
/// it there.
pub(super) struct CoverPlan {
    pub path: PathBuf,
    pub format: ImageFormat,
    pub by_run: bool,
}

/// The format of a picture of `mimetype` that may be kept as its own
/// bytes: JPEG or PNG.
pub(super) fn kept(mimetype: &str) -> Option<ImageFormat> {
    match mimetype {
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/png" => Some(ImageFormat::Png),
        _ => None,
    }
}

/// How a cover ffmpeg converts is made, raised whenever the bytes it
/// converts to change: [`crate::resolve::CoverRef::made`]. A test pins it
/// to a digest of the arguments a picture is converted with.
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
pub(super) fn picture_output(
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
pub(super) fn jpeg_scale(quality: u8) -> u32 {
    2 + (u32::from(100 - quality.clamp(1, 100)) * 29 + 49) / 99
}

/// The files of the cover `made` has beside the song at `stem`.
pub(super) fn cover_rels(stem: &Path, made: &Parts<'_>) -> Vec<PathBuf> {
    made.cover_files()
        .filter_map(|c| crate::outputs::cover_file(stem, &c.place))
        .collect()
}

/// The files of its folder's cover `made` has the song at `stem` hold.
pub(super) fn folder_rels(stem: &Path, made: &Parts<'_>) -> Vec<PathBuf> {
    made.folder_files()
        .filter_map(|c| crate::outputs::folder_file(stem, &c.place))
        .collect()
}

/// The picture `parts` are made from, and each place it goes; none
/// where it goes nowhere. The parts of one cover share its source.
pub(super) fn pictures<'p, 'a: 'p>(
    parts: impl Iterator<Item = &'p CoverPart<'a>>,
) -> Option<(&'p CoverSource<'a>, Vec<CoverPlace>)> {
    let mut source = None;
    let places = parts
        .map(|c| {
            source.get_or_insert(&c.source);
            c.place.clone()
        })
        .collect();
    source.map(|s| (s, places))
}

/// Plan the pictures of `folder`, the cover the song's folder holds, at
/// `places`, in a scratch folder of their own, as [`cover_plans`] plans
/// a song's.
pub(super) fn folder_plans<R: Runner>(
    runner: &R,
    job: &Job<'_>,
    (folder, places): (&CoverSource<'_>, Vec<CoverPlace>),
    inputs: &mut Inputs,
    outputs: &mut Vec<Output>,
) -> Result<Vec<(CoverPlace, CoverPlan)>> {
    let source = job
        .sources
        .get(folder.key)
        .ok_or_else(|| anyhow!("{} is not on disk", folder.key))?;
    let scratch = job.scratch.join("folder");
    fs::create_dir_all(&scratch).with_context(|| format!("creating {}", scratch.display()))?;
    cover_plans(runner, folder, places, source, &scratch, inputs, outputs)
}

/// The widest a cover too large to hold is made.
const COVER_WIDTH: u32 = 3000;

/// `picture`, read from `path`, in `room` bytes ([`super::cover_room`]): as it
/// is when it fits, else made a JPEG at most [`COVER_WIDTH`] pixels wide.
pub(super) fn fit_cover<R: Runner>(
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

/// Plan the picture each of `places` takes: its own bytes where the
/// place keeps a JPEG or PNG without borders as it is, in a file only
/// where they are the file's format; any other made by the run, places
/// alike sharing one.
pub(super) fn cover_plans<R: Runner>(
    runner: &R,
    cover: &CoverSource<'_>,
    places: Vec<CoverPlace>,
    source: &Located,
    scratch: &Path,
    inputs: &mut Inputs,
    outputs: &mut Vec<Output>,
) -> Result<Vec<(CoverPlace, CoverPlan)>> {
    let file = picture_file(runner, cover.at, source, scratch)?;
    // A file's bytes say what it is, whatever its name or its container
    // says: a PNG named `.jpg` is kept as PNG, and a WebP converted.
    let own = match &file {
        Some(f) => crate::picture::sniff_file(f).map_or_else(|| kept(cover.mimetype), kept),
        None => kept(cover.mimetype),
    }
    .filter(|_| cover.crop.is_none());
    let from = file.clone().unwrap_or_else(|| source.path.clone());
    let stream = match cover.at {
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
pub(super) fn picture_file<R: Runner>(
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
pub(super) fn cover_bytes(
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

/// The bytes of `cover`'s files `places`, `rels` in the library, made
/// from its source in a run of their own in `scratch`, as [`super::render`]
/// makes them.
pub(super) fn cover_files<R: Runner>(
    runner: &R,
    job: &Job<'_>,
    cover: &CoverSource<'_>,
    (places, scratch): (Vec<CoverPlace>, &Path),
    rels: &[PathBuf],
    problems: &mut Vec<String>,
) -> Result<Vec<Option<Vec<u8>>>> {
    let source = job
        .sources
        .get(cover.key)
        .ok_or_else(|| anyhow!("{} is not on disk", cover.key))?;
    fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let (mut inputs, mut outputs) = (Inputs::default(), Vec::new());
    let files = cover_plans(
        runner,
        cover,
        places,
        source,
        scratch,
        &mut inputs,
        &mut outputs,
    )?;
    let paths: Vec<&Path> = inputs.0.iter().map(PathBuf::as_path).collect();
    let results = ffmpeg::run_outputs(runner, &paths, &outputs);
    let failed = |path: &Path| failure(&outputs, &results, path);
    Ok(cover_bytes(&files, rels, failed, problems))
}

/// The picture at `path`, of `format` unless its bytes say what it is:
/// JPEG or PNG, which every container holds.
pub(super) fn read_picture(path: &Path, format: ImageFormat) -> Result<Picture> {
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
