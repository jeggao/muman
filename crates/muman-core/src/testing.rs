//! A runner that answers ffprobe from canned JSON and writes, for each
//! ffmpeg output, what that output would have written: the Opus, FLAC,
//! Vorbis, MP3 or M4A fixture, a picture, a line of LRC, a print, noise
//! to measure. Tags, covers and renames then run against real files
//! offline.
//!
//! The audio it writes is the fixture's whatever the encode was asked
//! for, so a song encoded with another gain or bitrate would be the same
//! bytes. A FLAC output alone is stamped with its own arguments, in the
//! MD5 its STREAMINFO holds: a change that should encode a song again,
//! and did not, shows offline as FLAC bytes that differ from a fresh
//! build's. Its inputs' paths and other outputs are left out of the
//! stamp, so one song builds the same bytes in any folder.
//!
//! [`Digest`] pins each measure to its method. A measure's test hashes
//! what it makes of fixed samples, and [`check`] holds that digest to the
//! one recorded beside the method: a change to what a measure makes fails
//! until its method is raised, so every result saved by the old one is
//! made again. Floats are hashed to 9 significant digits, since a
//! platform's maths library may differ from another's in the last bit.
//!
//! A dependency bump that changes a digest raises the method when what
//! the old version saved is stale, and otherwise re-records the digest
//! alone, its pull request saying why. Dependabot brings the crates the
//! measures are made with in a pull request of their own, so such a
//! failure holds up no other update.

// The fake writes what each program it stands in for would write.
#![expect(clippy::disallowed_methods)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Result, anyhow};
use sha2::{Digest as _, Sha256};

use crate::runner::{Line, Runner};

pub const SILENCE_OPUS: &[u8] = include_bytes!("../../../testdata/silence.opus");
pub const SILENCE_FLAC: &[u8] = include_bytes!("../../../testdata/silence.flac");
/// Where a FLAC's STREAMINFO holds the MD5 of its audio: after `fLaC`,
/// the block's header and 18 bytes of the block.
pub const STREAMINFO_MD5: std::ops::Range<usize> = 26..42;
pub const SILENCE_VORBIS: &[u8] = include_bytes!("../../../testdata/silence.ogg");
pub const SILENCE_MP3: &[u8] = include_bytes!("../../../testdata/silence.mp3");
pub const SILENCE_M4A: &[u8] = include_bytes!("../../../testdata/silence.m4a");
pub const SILENCE_WV: &[u8] = include_bytes!("../../../testdata/silence.wv");
pub const PIXEL: &[u8] = include_bytes!("../../../testdata/pixel.png");
pub const PIXEL_JPEG: &[u8] = include_bytes!("../../../testdata/pixel.jpg");
pub const PIXEL_WEBP: &[u8] = include_bytes!("../../../testdata/pixel.webp");

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

/// A v1 torrent's metainfo, of a folder `name` holding a byte-long
/// file at each of `files`, a path in it with `/` between its parts.
#[must_use]
pub fn metainfo(name: &str, files: &[&str]) -> Vec<u8> {
    let text = |t: &str| format!("{}:{t}", t.len());
    let list: Vec<String> = files
        .iter()
        .map(|f| {
            let parts: String = f.split('/').map(text).collect();
            format!("d6:lengthi1e4:pathl{parts}ee")
        })
        .collect();
    let list = list.concat();
    let info = format!(
        "d5:filesl{list}e4:name{}12:piece lengthi16384e6:pieces20:{}e",
        text(name),
        "0".repeat(20)
    );
    format!("d4:info{info}e").into_bytes()
}

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
    /// The WAV an analysis decodes to its pipe, by path fragment; a short
    /// quiet tone otherwise.
    pub wavs: Vec<(String, Vec<u8>)>,
    /// The size of every picture decoded to gray, by path fragment.
    pub pictures: Vec<(String, (u32, u32))>,
    /// The design a picture decodes to, by path fragment: two paths given
    /// one look alike; others look as their own.
    pub looks: Vec<(String, u64)>,
    pub lrc: Option<String>,
    /// The bytes an audio stream's packets add up to, by path fragment.
    pub packets: Vec<(String, u64)>,
    /// The whole seconds an audio stream's packets run to, by path fragment.
    pub held: Vec<(String, u32)>,
    /// A stream's packets as `framecrc` lists them, by path fragment, in
    /// place of one packet as long as its audio.
    pub timelines: Vec<(String, String)>,
    /// What the streams of a path holding the fragment hash as, in place
    /// of its bytes: a file fetched again holding other audio.
    pub hashes: Vec<(String, u64)>,
    /// Output formats that fail.
    pub failing: Vec<String>,
    /// Lines a streamed run writes, and what it does to the disk.
    pub lines: Vec<String>,
    // A hook one test installs; a named type would be used once.
    #[expect(clippy::type_complexity)]
    pub on_stream: Option<Box<dyn Fn(&[String]) -> bool + Sync>>,
    pub calls: Mutex<Vec<Vec<String>>>,
    /// The first file each run that attaches one attached, as text; the
    /// run copies its input whole.
    pub attached: Mutex<Vec<String>>,
}

fn find<'a, T>(table: &'a [(String, T)], path: &str) -> Option<&'a T> {
    table
        .iter()
        .find(|(f, _)| path.contains(f.as_str()))
        .map(|(_, v)| v)
}

/// Deterministic words, unique to their seed.
#[must_use]
/// `seconds` of a sine at `hz` and `amplitude` of full scale, the same in
/// each of `channels`, interleaved.
pub fn sine(rate: u32, channels: u16, hz: f64, amplitude: f32, seconds: f64) -> Vec<f32> {
    let frames = (f64::from(rate) * seconds).round() as usize;
    (0..frames)
        .flat_map(|n| {
            let x =
                amplitude * (std::f64::consts::TAU * hz * n as f64 / f64::from(rate)).sin() as f32;
            std::iter::repeat_n(x, usize::from(channels))
        })
        .collect()
}

/// `samples` as ffmpeg writes 32-bit float WAV on a pipe: sizes unknown,
/// and the speakers' mask where `mask` gives one.
#[must_use]
pub fn wav(rate: u32, channels: u16, mask: Option<u32>, samples: &[f32]) -> Vec<u8> {
    let mut fmt = Vec::new();
    let tag: u16 = if mask.is_some() { 0xFFFE } else { 3 };
    fmt.extend(tag.to_le_bytes());
    fmt.extend(channels.to_le_bytes());
    fmt.extend(rate.to_le_bytes());
    fmt.extend((rate * u32::from(channels) * 4).to_le_bytes());
    fmt.extend((channels * 4).to_le_bytes());
    fmt.extend(32u16.to_le_bytes());
    if let Some(mask) = mask {
        fmt.extend(22u16.to_le_bytes());
        fmt.extend(32u16.to_le_bytes());
        fmt.extend(mask.to_le_bytes());
        fmt.extend(3u16.to_le_bytes());
        fmt.extend([0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
    }
    let mut out = b"RIFF\xff\xff\xff\xffWAVEfmt ".to_vec();
    out.extend(u32::try_from(fmt.len()).unwrap().to_le_bytes());
    out.extend(fmt);
    out.extend(b"data\xff\xff\xff\xff");
    out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    out
}

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

/// [`seed_of`] a path below the test's temporary folder, so a fake's
/// pictures, prints and noise are the same on every run of a test, as a
/// real file's measures are.
fn seed_of_path(path: &str) -> u64 {
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    let temporary = |part: &&str| part.starts_with(".tmp") || part.starts_with(".muman-scratch-");
    match parts.iter().rposition(temporary) {
        Some(at) => seed_of(&parts[at + 1..].join("/")),
        None => seed_of(path),
    }
}

fn seed_of(path: &str) -> u64 {
    path.bytes().fold(1_469_598_103_934_665_603_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211)
    })
}

/// Stereo noise, full band, as f32 bytes.
fn noise_bytes(seed: u64) -> Vec<u8> {
    words(8192 * 2, seed)
        .into_iter()
        .flat_map(|w| ((w as f32 / u32::MAX as f32 - 0.5) * 0.5).to_le_bytes())
        .collect()
}

/// An opaque gray picture with detail at every pixel, as a PAM.
fn pam(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut out = format!(
        "P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH 2\nMAXVAL 255\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n"
    )
    .into_bytes();
    out.extend(
        words((w * h) as usize, seed)
            .into_iter()
            .flat_map(|v| [(v >> 24) as u8, u8::MAX]),
    );
    out
}

impl std::fmt::Debug for Fake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fake").finish_non_exhaustive()
    }
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

    /// A picture stream `map` names of `input` copied out: JPEG for one
    /// its probe says is, else PNG.
    fn copied_picture(&self, input: &str, map: Option<&str>) -> &'static [u8] {
        let probe = find(&self.probes, input).map_or(ORIGINAL, String::as_str);
        let index = map.and_then(|m| m.split(':').nth(1));
        let streams: serde_json::Value = serde_json::from_str(probe).unwrap_or_default();
        let jpeg = streams["streams"].as_array().is_some_and(|all| {
            all.iter().any(|s| {
                Some(s["index"].to_string().as_str()) == index && s["codec_name"] == "mjpeg"
            })
        });
        if jpeg { PIXEL_JPEG } else { PIXEL }
    }

    fn write_output(&self, args: &[String], at: usize, inputs: &[String]) -> Result<()> {
        let format = args[at + 1].as_str();
        let path = PathBuf::from(&args[at + 2]);
        let mapped = args[..at].iter().rposition(|a| a == "-map");
        let map = mapped.map(|i| args[i + 1].as_str());
        let encoder = |name: &str| args[mapped.unwrap_or(0)..at].iter().any(|a| a == name);
        let input = map
            .and_then(|m| m.split(':').next())
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| inputs.get(n))
            .cloned()
            .unwrap_or_default();
        if self.failing.iter().any(|f| f == format) {
            if path != Path::new(crate::runner::PIPE) {
                fs::write(&path, b"partial")?;
            }
            return Err(anyhow!("ffmpeg: {format} failed"));
        }
        let bytes = match format {
            // The fake's decoded audio is the print itself, which its
            // `fingerprint` hands back as it is.
            "s16le" => find(&self.prints, &input)
                .cloned()
                .unwrap_or_else(|| words(1600, seed_of_path(&input)))
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect(),
            "f32le" => find(&self.segments, &input)
                .cloned()
                .unwrap_or_else(|| noise_bytes(seed_of_path(&input))),
            "lrc" => self
                .lrc
                .clone()
                .unwrap_or_else(|| "[00:01.00]line\n[00:30.00]more\n".into())
                .into_bytes(),
            "image2" if args[..at].iter().any(|a| a == "pam") => {
                let (w, h) = find(&self.pictures, &input).copied().unwrap_or((64, 64));
                let seed = find(&self.looks, &input)
                    .copied()
                    .unwrap_or_else(|| seed_of_path(&input));
                pam(w, h, seed)
            }
            "image2" if encoder("copy") => self.copied_picture(&input, map).to_vec(),
            "image2" if encoder("mjpeg") => PIXEL_JPEG.to_vec(),
            "image2" if encoder("libwebp") => PIXEL_WEBP.to_vec(),
            "image2" => PIXEL.to_vec(),
            "opus" => SILENCE_OPUS.to_vec(),
            // Stamped with the audio output's own arguments, so a gain, a
            // filter or a bitrate the encode was given changes its bytes.
            "flac" => {
                let mut flac = SILENCE_FLAC.to_vec();
                let stamp = <sha2::Sha256 as sha2::Digest>::digest(
                    args[mapped.unwrap_or(0)..at].join("\0"),
                );
                flac[STREAMINFO_MD5].copy_from_slice(&stamp[..16]);
                flac
            }
            // A stream's packets, as many bytes as 200 s at 128 kbit/s,
            // running as long as the audio its analysis decodes, unless the
            // tables say otherwise.
            "framecrc" => {
                if let Some(list) = find(&self.timelines, &input) {
                    list.clone().into_bytes()
                } else {
                    let bytes = find(&self.packets, &input).copied().unwrap_or(3_200_000);
                    let ticks = find(&self.held, &input).map_or_else(
                        || {
                            let wav = find(&self.wavs, &input).cloned().unwrap_or_else(quiet_wav);
                            (wav_seconds(&wav) * 48_000.0).round() as i64
                        },
                        |s| i64::from(*s) * 48_000,
                    );
                    format!("#tb 0: 1/48000\n0, 0, 0, {ticks}, {bytes}, 0x00000000\n").into_bytes()
                }
            }
            // A stream hashes as its file's bytes and the stream mapped.
            "hash" => {
                let seed = find(&self.hashes, &input).copied().unwrap_or_else(|| {
                    seed_of(&String::from_utf8_lossy(
                        &fs::read(&input).unwrap_or_default(),
                    ))
                });
                let bytes: Vec<u8> = words(8, seed ^ seed_of(map.unwrap_or_default()))
                    .into_iter()
                    .flat_map(u32::to_be_bytes)
                    .collect();
                format!("SHA256={}\n", crate::hash::hex(&bytes)).into_bytes()
            }
            "ogg" => SILENCE_VORBIS.to_vec(),
            "mp3" => SILENCE_MP3.to_vec(),
            "ipod" => SILENCE_M4A.to_vec(),
            "wv" => SILENCE_WV.to_vec(),
            _ => return Ok(()),
        };
        fs::write(&path, bytes)?;
        Ok(())
    }
}

/// Seconds of audio a WAV as [`wav`] writes holds: its 32-bit samples
/// after its `data` chunk, at the rate and channels its `fmt ` chunk
/// names.
#[must_use]
pub fn wav_seconds(bytes: &[u8]) -> f64 {
    let u16_at = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (mut at, mut format) = (12, None);
    while let Some(id) = bytes.get(at..at + 4) {
        let body = at + 8;
        if id == b"data" {
            let Some((channels, rate)) = format else {
                break;
            };
            let frames = (bytes.len() - body) as f64 / f64::from(u32::from(channels) * 4);
            return frames / f64::from(rate);
        }
        if id == b"fmt " {
            format = u16_at(body + 2).zip(u32_at(body + 4));
        }
        let Some(size) = u32_at(at + 4) else { break };
        at = body + size as usize;
    }
    0.0
}

/// What a fake analysis decodes where no table names it: two seconds of
/// a tone at 8 kHz, cheap to measure.
#[must_use]
pub fn quiet_wav() -> Vec<u8> {
    wav(8000, 1, None, &sine(8000, 1, 1000.0, 0.1, 2.0))
}

/// A SHA-256 of what a measure makes, each item ended by a 0 byte.
#[derive(Default, Debug)]
pub struct Digest(Sha256);

impl Digest {
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.bytes(text.as_bytes())
    }

    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update(bytes);
        self.0.update([0]);
        self
    }

    /// `value` as `{:?}` writes it, each float in it rounded to 9
    /// significant digits; a string's text is kept as it is.
    pub fn debug(&mut self, value: &impl std::fmt::Debug) -> &mut Self {
        let float =
            regex::Regex::new(r#""(?:[^"\\]|\\.)*"|(-?\d+\.\d+(?:e-?\d+)?|-?\d+e-?\d+)"#).unwrap();
        let text = format!("{value:?}");
        let rounded = float.replace_all(&text, |c: &regex::Captures<'_>| match c.get(1) {
            None => c[0].to_string(),
            Some(x) => {
                let x: f64 = x.as_str().parse().unwrap();
                if x == 0.0 {
                    "0".to_string()
                } else {
                    format!("{x:.8e}")
                }
            }
        });
        self.text(&rounded)
    }

    #[must_use]
    pub fn hex(&self) -> String {
        crate::hash::hex(&self.0.clone().finalize())
    }
}

/// Fails unless `written`, the method and digest a test recorded, is
/// `method` and `digest`.
#[track_caller]
pub fn check(written: (&str, &str), method: &str, digest: &str) {
    assert_eq!(
        written,
        (method, digest),
        "{method} writes otherwise: if the code changed what it makes, raise its version and \
         record {digest}; if only a dependency moved, decide whether saved results are stale, \
         and either raise it or re-record the hex and say why in the PR"
    );
}

impl Runner for Fake {
    fn pipe(&self, cmd: &[OsString], on_bytes: &mut dyn FnMut(&[u8])) -> Result<()> {
        let ran = self.run(cmd);
        let args: Vec<String> = cmd
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let inputs: Vec<String> = args
            .iter()
            .zip(args.iter().skip(1))
            .filter(|(f, _)| *f == "-i")
            .map(|(_, p)| p.clone())
            .collect();
        let Some(at) = args.iter().position(|a| a == crate::runner::PIPE) else {
            return ran;
        };
        let input = args[..at]
            .iter()
            .rposition(|a| a == "-map")
            .and_then(|i| args[i + 1].split(':').next())
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| inputs.get(n))
            .cloned()
            .unwrap_or_default();
        let bytes = find(&self.wavs, &input).cloned().unwrap_or_else(quiet_wav);
        let mut rest = bytes.as_slice();
        for size in [5, 40, 999].into_iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (piece, after) = rest.split_at(size.min(rest.len()));
            on_bytes(piece);
            rest = after;
        }
        ran
    }

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
        if let Some(at) = args.iter().position(|a| a == "-attach") {
            self.attached
                .lock()
                .unwrap()
                .push(fs::read_to_string(&args[at + 1])?);
            fs::copy(
                &inputs[0],
                args.last().map(String::as_str).unwrap_or_default(),
            )?;
            return Ok(());
        }
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

/// A link at `at` to `to`, a folder's link if `to` is a folder; `false`
/// where the system refuses one, as Windows does outside developer mode.
#[must_use]
pub fn link(to: &Path, at: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(to, at);
    #[cfg(windows)]
    let made = if to.is_dir() {
        std::os::windows::fs::symlink_dir(to, at)
    } else {
        std::os::windows::fs::symlink_file(to, at)
    };
    #[cfg(not(any(unix, windows)))]
    let made: std::io::Result<()> = Err(std::io::ErrorKind::Unsupported.into());
    made.is_ok()
}

#[test]
fn a_fake_flac_encode_is_stamped_with_its_own_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let encode = |out: &str, shaping: &[&str], cover: bool| {
        let out = dir.path().join(out);
        let mut cmd: Vec<OsString> = ["ffmpeg", "-i", "song.mkv", "-map", "0:a:0", "-c:a", "flac"]
            .iter()
            .chain(shaping)
            .map(OsString::from)
            .collect();
        cmd.extend(["-f".into(), "flac".into(), out.clone().into()]);
        if cover {
            let picture = dir.path().join("cover.png");
            cmd.extend(["-map", "0:v:0", "-f", "image2"].map(OsString::from));
            cmd.push(picture.into());
        }
        fake.run(&cmd).unwrap();
        fs::read(out).unwrap()
    };
    let plain = encode("a.flac", &[], false);
    assert_eq!(plain, encode("b.flac", &[], true), "another output aside");
    assert_ne!(plain, encode("c.flac", &["-af", "volume=-3dB"], false));
    assert_ne!(plain, encode("d.flac", &["-b:a", "900k"], false));
    assert_eq!(plain.len(), SILENCE_FLAC.len());
}
