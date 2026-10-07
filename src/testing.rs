//! A runner that answers ffprobe from canned JSON and writes, for each
//! ffmpeg output, what that output would have written: the Opus, FLAC,
//! Vorbis, MP3 or M4A fixture, a picture, a line of LRC, a print, noise
//! to measure. Tags, covers and renames then run against real files
//! offline.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::runner::{Line, Runner};

pub const SILENCE_OPUS: &[u8] = include_bytes!("../testdata/silence.opus");
pub const SILENCE_FLAC: &[u8] = include_bytes!("../testdata/silence.flac");
pub const SILENCE_VORBIS: &[u8] = include_bytes!("../testdata/silence.ogg");
pub const SILENCE_MP3: &[u8] = include_bytes!("../testdata/silence.mp3");
pub const SILENCE_M4A: &[u8] = include_bytes!("../testdata/silence.m4a");
pub const PIXEL: &[u8] = include_bytes!("../testdata/pixel.png");

/// The ffprobe JSON of a yt-dlp original: video, Opus audio, an English
/// subtitle, its info JSON and a WebP thumbnail as attachments.
pub const ORIGINAL: &str = r#"{"streams": [
    {"index": 0, "codec_type": "video", "codec_name": "vp9"},
    {"index": 1, "codec_type": "audio", "codec_name": "opus", "channels": 2, "sample_rate": "48000"},
    {"index": 2, "codec_type": "subtitle", "tags": {"language": "eng", "title": "English"}},
    {"index": 3, "codec_type": "attachment", "tags": {"filename": "info.json", "mimetype": "application/json"}},
    {"index": 4, "codec_type": "attachment", "tags": {"filename": "cover.webp", "mimetype": "image/webp"}}
], "format": {"duration": "200.0"}}"#;

/// A manual FLAC with an embedded square cover.
pub const FLAC: &str = r#"{"streams": [
    {"index": 0, "codec_type": "audio", "codec_name": "flac", "channels": 2, "sample_rate": "44100"},
    {"index": 1, "codec_type": "video", "codec_name": "mjpeg", "width": 64, "height": 64,
     "disposition": {"attached_pic": 1}}
], "format": {"duration": "200.0", "tags": {"TITLE": "Song", "ARTIST": "Artist", "ALBUM": "Record", "track": "2"}}}"#;

pub const INFO: &str = r#"{"id": "aaaaaaaaaaa", "title": "Song / Wren", "uploader": "Hoshi7ne",
    "upload_date": "20260527", "subtitles": {"en": [{"name": "English"}]}}"#;

#[derive(Default)]
pub struct Fake {
    /// ffprobe JSON by a fragment of the probed path; the first match
    /// answers, and a path no fragment matches has [`ORIGINAL`]'s.
    pub probes: Vec<(String, String)>,
    /// The info JSON dumped from a path holding the fragment.
    pub infos: Vec<(String, String)>,
    /// The print of a path holding the fragment; others are their own.
    pub prints: Vec<(String, Vec<u32>)>,
    /// Decoded segments as stereo f32 by path fragment; noise otherwise.
    pub segments: Vec<(String, Vec<u8>)>,
    /// The 8 kHz samples a comparison decodes, by path fragment.
    pub pcm: Vec<(String, Vec<u8>)>,
    /// The size of every picture decoded to gray, by path fragment.
    pub pictures: Vec<(String, (u32, u32))>,
    pub lrc: Option<String>,
    /// The bytes an audio stream's packets add up to, by path fragment.
    pub packets: Vec<(String, u64)>,
    /// The whole seconds an audio stream's packets run to, by path fragment.
    pub held: Vec<(String, u32)>,
    /// Output formats that fail.
    pub failing: Vec<String>,
    /// Lines a streamed run writes, and what it does to the disk.
    pub lines: Vec<String>,
    #[allow(clippy::type_complexity)]
    pub on_stream: Option<Box<dyn Fn(&[String]) -> bool + Sync>>,
    pub calls: Mutex<Vec<Vec<String>>>,
}

fn find<'a, T>(table: &'a [(String, T)], path: &str) -> Option<&'a T> {
    table
        .iter()
        .find(|(f, _)| path.contains(f.as_str()))
        .map(|(_, v)| v)
}

/// Deterministic words, unique to their seed.
#[must_use]
pub fn words(n: usize, seed: u64) -> Vec<u32> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            u32::try_from(state >> 32).unwrap_or(0)
        })
        .collect()
}

fn seed_of(path: &str) -> u64 {
    path.bytes().fold(1_469_598_103_934_665_603_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211)
    })
}

/// Stereo noise, full band, as f32 bytes.
#[allow(clippy::cast_precision_loss)]
fn noise_bytes(seed: u64) -> Vec<u8> {
    words(8192 * 2, seed)
        .into_iter()
        .flat_map(|w| ((w as f32 / u32::MAX as f32 - 0.5) * 0.5).to_le_bytes())
        .collect()
}

/// A gray picture with detail at every pixel.
fn pgm(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut out = format!("P5\n{w} {h}\n255\n").into_bytes();
    out.extend(
        words((w * h) as usize, seed)
            .into_iter()
            .map(|v| (v >> 24) as u8),
    );
    out
}

impl Fake {
    #[must_use]
    pub fn probe(mut self, fragment: &str, json: &str) -> Self {
        self.probes.push((fragment.into(), json.into()));
        self
    }

    #[must_use]
    pub fn info(mut self, fragment: &str, json: &str) -> Self {
        self.infos.push((fragment.into(), json.into()));
        self
    }

    #[must_use]
    pub fn print(mut self, fragment: &str, words: Vec<u32>) -> Self {
        self.prints.push((fragment.into(), words));
        self
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }

    /// Whether any run carried `needle` as a whole argument.
    pub fn ran(&self, needle: &str) -> bool {
        self.calls().iter().any(|c| c.iter().any(|a| a == needle))
    }

    fn record(&self, cmd: &[OsString]) -> Vec<String> {
        let args: Vec<String> = cmd
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        self.calls.lock().unwrap().push(args.clone());
        args
    }

    fn write_output(&self, args: &[String], at: usize, inputs: &[String]) -> Result<()> {
        let format = args[at + 1].as_str();
        let path = PathBuf::from(&args[at + 2]);
        let map = args[..at]
            .iter()
            .rposition(|a| a == "-map")
            .map(|i| args[i + 1].as_str());
        let input = map
            .and_then(|m| m.split(':').next())
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| inputs.get(n))
            .cloned()
            .unwrap_or_default();
        if self.failing.iter().any(|f| f == format) {
            fs::write(&path, b"partial")?;
            return Err(anyhow!("ffmpeg: {format} failed"));
        }
        let bytes = match format {
            // The fake's decoded audio is the print itself, which its
            // `fingerprint` hands back as it is.
            "s16le" => find(&self.prints, &input)
                .cloned()
                .unwrap_or_else(|| words(1600, seed_of(&input)))
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect(),
            "f32le" => find(&self.segments, &input)
                .cloned()
                .unwrap_or_else(|| noise_bytes(seed_of(&input))),
            "lrc" => self
                .lrc
                .clone()
                .unwrap_or_else(|| "[00:01.00]line\n[00:30.00]more\n".into())
                .into_bytes(),
            "image2" if args[..at].iter().any(|a| a == "pgm") => {
                let (w, h) = find(&self.pictures, &input).copied().unwrap_or((64, 64));
                pgm(w, h, seed_of(&input))
            }
            "image2" => PIXEL.to_vec(),
            "opus" => SILENCE_OPUS.to_vec(),
            "flac" => SILENCE_FLAC.to_vec(),
            // A stream's packets, as many bytes as 200 s at 128 kbit/s, over
            // ten hours, longer than any header, unless the tables say otherwise.
            "framecrc" => {
                let bytes = find(&self.packets, &input).copied().unwrap_or(3_200_000);
                let ticks =
                    find(&self.held, &input).map_or(1_728_000_000, |s| i64::from(*s) * 48_000);
                format!("#tb 0: 1/48000\n0, 0, 0, {ticks}, {bytes}, 0x00000000\n").into_bytes()
            }
            "ogg" => SILENCE_VORBIS.to_vec(),
            "mp3" => SILENCE_MP3.to_vec(),
            "ipod" => SILENCE_M4A.to_vec(),
            _ => return Ok(()),
        };
        fs::write(&path, bytes)?;
        Ok(())
    }
}

impl Runner for Fake {
    fn fingerprint(&self, pcm: &Path) -> Result<Vec<u32>> {
        Ok(crate::fingerprint::Print::from_raw(&fs::read(pcm)?)
            .map(|p| p.words().to_vec())
            .unwrap_or_default())
    }

    fn run(&self, cmd: &[OsString]) -> Result<()> {
        let args = self.record(cmd);
        let inputs: Vec<String> = args
            .iter()
            .zip(args.iter().skip(1))
            .filter(|(f, _)| *f == "-i")
            .map(|(_, p)| p.clone())
            .collect();
        if args.iter().any(|a| a.starts_with("-dump_attachment")) {
            for (i, a) in args.iter().enumerate() {
                if !a.starts_with("-dump_attachment") {
                    continue;
                }
                let to = Path::new(&args[i + 1]);
                let input = args[i..]
                    .iter()
                    .position(|a| a == "-i")
                    .map(|j| args[i + j + 1].clone())
                    .unwrap_or_default();
                if to.ends_with("info.json") {
                    if let Some(info) = find(&self.infos, &input).or(Some(&INFO.to_string())) {
                        fs::write(to, info)?;
                    }
                } else {
                    fs::write(to, PIXEL)?;
                }
            }
            return Ok(());
        }
        let mut failed = None;
        for at in 0..args.len().saturating_sub(2) {
            if args[at] == "-f"
                && args[at + 1] != "null"
                && let Err(e) = self.write_output(&args, at, &inputs)
            {
                failed = Some(e);
            }
        }
        failed.map_or(Ok(()), Err)
    }

    fn output(&self, cmd: &[OsString]) -> Result<Vec<u8>> {
        let args = self.record(cmd);
        match args.first().map(String::as_str) {
            Some("ffprobe") => {
                let path = args.last().cloned().unwrap_or_default();
                Ok(find(&self.probes, &path)
                    .cloned()
                    .unwrap_or_else(|| ORIGINAL.into())
                    .into_bytes())
            }
            Some("ffmpeg") if args.iter().any(|a| a == "-version") => {
                Ok(b"ffmpeg version fake\n".to_vec())
            }
            Some("ffmpeg") if args.iter().any(|a| a == "s16le") => {
                let path = args
                    .iter()
                    .position(|a| a == "-i")
                    .map(|i| args[i + 1].clone())
                    .unwrap_or_default();
                find(&self.pcm, &path)
                    .cloned()
                    .ok_or_else(|| anyhow!("no samples for {path}"))
            }
            _ => Err(anyhow!("offline")),
        }
    }

    fn stream(&self, cmd: &[OsString], on_line: &mut dyn FnMut(Line<'_>)) -> Result<bool> {
        let args = self.record(cmd);
        for line in &self.lines {
            on_line(Line::Out(line));
        }
        Ok(self.on_stream.as_ref().is_none_or(|f| f(&args)))
    }
}
