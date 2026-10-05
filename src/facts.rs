//! Everything a source offers a song, measured once per revision of its
//! files: its audio and how good it is, its pictures, its lyrics, its
//! tags and its fingerprint.
//!
//! A media file costs at most three runs: ffprobe, one ffmpeg dumping
//! its attachments, and one ffmpeg writing every measured excerpt as a
//! separate output. That run also lists the audio's packets, whose sizes
//! sum to what a copy of the audio takes, for fitting a library into
//! `[library] max_size`; facts measured before it are given theirs by
//! [`audio_bytes`] when a size limit asks for them.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::ffmpeg::{self, Output};
use crate::fingerprint::{self, Print};
use crate::info::{self, VideoInfo};
use crate::lyrics::{self, Language, Timing};
use crate::probe::{self, Probed};
use crate::quality::{self, AudioQuality, ImageQuality};
use crate::runner::Runner;
use crate::store::{Kind, Located};
use crate::tags::{self, Offers};

/// The measures facts are made by; facts made by others are made again.
#[must_use]
pub fn method() -> String {
    format!("{}+{}", quality::METHOD, fingerprint::METHOD)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    /// The revision of the files they were measured on.
    pub rev: String,
    pub method: String,
    pub duration: Option<f64>,
    pub audio: Option<AudioFacts>,
    pub covers: Vec<CoverFacts>,
    pub lyrics: Vec<LyricsFacts>,
    pub tags: Offers,
    /// The `tags::METHOD` the tags were read by.
    #[serde(default)]
    pub tags_method: String,
    /// A YouTube Music release, by its info: a track name, or a " - Topic"
    /// channel; read with the tags.
    #[serde(default)]
    pub release: bool,
    pub print: Option<Print>,
}

impl Facts {
    /// Whether these still describe a source at revision `rev`.
    #[must_use]
    pub fn holds_for(&self, rev: &str) -> bool {
        self.rev == rev && self.method == method()
    }

    /// Whether the tags were read the way they are now.
    #[must_use]
    pub fn tags_hold(&self) -> bool {
        self.tags_method == tags::METHOD
    }

    /// Facts of a file that could not be read at all.
    #[must_use]
    pub fn unreadable(rev: String) -> Self {
        Self {
            rev,
            method: method(),
            duration: None,
            audio: None,
            covers: Vec::new(),
            lyrics: Vec::new(),
            tags: Offers::new(),
            tags_method: tags::METHOD.to_string(),
            release: false,
            print: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioFacts {
    /// Absolute stream index.
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub quality: Option<AudioQuality>,
    /// The bytes of the audio stream's packets, which a copy carries.
    #[serde(default)]
    pub bytes: Option<u64>,
}

impl AudioFacts {
    /// Whether the codec keeps every sample as recorded.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.codec == "flac"
            || self.codec == "alac"
            || self.codec == "wavpack"
            || self.codec == "ape"
            || self.codec == "tta"
            || self.codec.starts_with("pcm_")
    }
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverFacts {
    pub at: CoverAt,
    pub mimetype: String,
    pub quality: Option<ImageQuality>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LyricsAt {
    /// A subtitle stream.
    Stream { index: u32 },
    /// The `.lrc` that came with a manual file.
    Sidecar,
    /// The source is the `.lrc`.
    File,
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
}

/// Measure one located source, writing intermediates to `scratch`.
pub fn gather<R: Runner>(runner: &R, located: &Located, scratch: &Path) -> Result<Facts> {
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let mut facts = Facts::unreadable(located.rev());
    match located.kind {
        Kind::Lyrics => {
            let text = read_text(&located.path)?;
            facts.lyrics.push(lyrics_of(LyricsAt::File, &text));
        }
        Kind::Image => {
            let out = scratch.join("file.pgm");
            let outputs = [Output::new(quality::gray_output(0, "v:0"), &out)];
            let ok = ffmpeg::run_outputs(runner, &[located.path.as_path()], &outputs);
            facts.covers.push(CoverFacts {
                at: CoverAt::File,
                mimetype: mimetype_of(&located.path),
                quality: ok
                    .into_iter()
                    .next()
                    .and_then(Result::ok)
                    .and_then(|()| measure_image(&out)),
            });
        }
        Kind::Media => media(runner, located, scratch, &mut facts)?,
    }
    Ok(facts)
}

/// The tags of one located source, read again: a media file's are
/// probed, its pictures not dumped nor anything measured.
pub fn retag<R: Runner>(runner: &R, located: &Located, scratch: &Path) -> Result<(Offers, bool)> {
    if located.kind != Kind::Media {
        return Ok((Offers::new(), false));
    }
    std::fs::create_dir_all(scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
    let info = dump_attachments(runner, located, &probed, scratch, false);
    let release = info.as_ref().is_some_and(VideoInfo::is_release);
    Ok((tags_of(info.as_ref(), &probed), release))
}

fn tags_of(info: Option<&VideoInfo>, probed: &Probed) -> Offers {
    match info {
        Some(info) => tags::from_info(info),
        None => tags::from_container(&probed.tags),
    }
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(crate::lyrics::decode(&bytes))
}

fn lyrics_of(at: LyricsAt, text: &str) -> LyricsFacts {
    LyricsFacts {
        at,
        language: Language::Unstated,
        timing: lyrics::timing(&lyrics::clean_lrc(text)),
        stated_ms: lyrics::stated_length(text),
    }
}

fn mimetype_of(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/unknown",
    }
    .to_string()
}

fn measure_image(pgm: &Path) -> Option<ImageQuality> {
    let bytes = std::fs::read(pgm).ok()?;
    let (w, h, pixels) = ffmpeg::read_pgm(&bytes)?;
    quality::image(pixels, w, h)
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

/// What one ffmpeg output measures, to read back once it ran.
enum Measured {
    Print,
    Packets,
    Segment(PathBuf),
    Subtitle(u32, Language),
    Cover(CoverAt, String),
}

#[allow(clippy::too_many_lines)]
fn media<R: Runner>(
    runner: &R,
    located: &Located,
    scratch: &Path,
    facts: &mut Facts,
) -> Result<()> {
    let probed = probe::parse(&runner.output(&probe::ffprobe_command(&located.path))?)?;
    facts.duration = probed.duration;
    let info = dump_attachments(runner, located, &probed, scratch, true);
    facts.tags = tags_of(info.as_ref(), &probed);
    facts.release = info.as_ref().is_some_and(VideoInfo::is_release);
    let info = info.unwrap_or_default();

    let mut inputs: Vec<PathBuf> = vec![located.path.clone()];
    let mut outputs: Vec<(Output, Measured)> = Vec::new();
    if let Some(a) = &probed.audio {
        outputs.push((
            Output::new(fingerprint::output(0, a.index), &scratch.join("print.pcm")),
            Measured::Print,
        ));
        outputs.push((
            Output::new(packets_output(0, a.index), &scratch.join("packets.crc")),
            Measured::Packets,
        ));
        for (n, start) in segment_starts(probed.duration).into_iter().enumerate() {
            let path = scratch.join(format!("segment{n}"));
            outputs.push((
                Output::new(quality::segment_output(0, a.index, start), &path),
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
    for p in &probed.pictures {
        outputs.push((
            Output::new(
                quality::gray_output(0, &p.index.to_string()),
                &scratch.join(format!("pic{}.pgm", p.index)),
            ),
            Measured::Cover(CoverAt::Picture { index: p.index }, p.mimetype.clone()),
        ));
    }
    for a in probed.image_attachments() {
        let dumped = scratch.join(format!("att{}", a.ordinal));
        if !dumped.exists() {
            continue;
        }
        inputs.push(dumped);
        outputs.push((
            Output::new(
                quality::gray_output(inputs.len() - 1, "v:0"),
                &scratch.join(format!("att{}.pgm", a.ordinal)),
            ),
            Measured::Cover(
                CoverAt::Attachment { ordinal: a.ordinal },
                a.mimetype.clone(),
            ),
        ));
    }
    for (n, cover) in located.covers.iter().enumerate() {
        inputs.push(cover.clone());
        outputs.push((
            Output::new(
                quality::gray_output(inputs.len() - 1, "v:0"),
                &scratch.join(format!("side{n}.pgm")),
            ),
            Measured::Cover(CoverAt::Sidecar(n), mimetype_of(cover)),
        ));
    }

    let input_refs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
    let plain: Vec<Output> = outputs.iter().map(|(o, _)| o.clone()).collect();
    let results = ffmpeg::run_outputs(runner, &input_refs, &plain);
    let mut segments = Vec::new();
    let mut bytes = None;
    for ((output, measured), result) in outputs.into_iter().zip(results) {
        if result.is_err() {
            continue;
        }
        match measured {
            Measured::Print => {
                facts.print = runner.fingerprint(&output.path).ok().map(Print);
            }
            Measured::Packets => bytes = read_packets(&output.path),
            Measured::Segment(path) => segments.push(quality::read_segment(&path)),
            Measured::Subtitle(index, language) => {
                if let Ok(text) = read_text(&output.path) {
                    facts.lyrics.push(LyricsFacts {
                        at: LyricsAt::Stream { index },
                        language,
                        timing: lyrics::timing(&lyrics::clean_lrc(&text)),
                        stated_ms: None,
                    });
                }
            }
            Measured::Cover(at, mimetype) => facts.covers.push(CoverFacts {
                at,
                mimetype,
                quality: measure_image(&output.path),
            }),
        }
    }
    if let Some(lrc) = &located.lyrics {
        facts
            .lyrics
            .push(lyrics_of(LyricsAt::Sidecar, &read_text(lrc)?));
    }
    facts.audio = probed.audio.map(|a| AudioFacts {
        quality: quality::audio(&segments, a.sample_rate),
        index: a.index,
        codec: a.codec,
        channels: a.channels,
        bytes,
    });
    Ok(())
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
