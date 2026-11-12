//! What a source file holds, read from ffprobe's JSON.
//!
//! A length ffprobe gives is not always one the file states. A raw
//! elementary stream, as an ADTS `.aac`, an AC-3 or DTS file, or an MP3
//! with no Xing, Info or VBRI header, holds no length; ffmpeg then guesses
//! one from the file's size and the bitrate of its first frames
//! ("Estimating duration from bitrate", a warning `-v error` hides), and
//! gives the guess as the stream's and the file's `duration` alike, with
//! nothing in the JSON to tell it apart. A VBR MP3 of 40 s with no header
//! read as 62 s, and an ADTS file of 44 s as 123 s, where its first frames
//! were loud. [`read`] drops the stream's length where it is such a guess
//! ([`Probed::guessed`]), so that only a length the file states is held
//! against its packets and its decode, and the length a song plays is
//! read from its packets instead.
//!
//! A length counts as stated only for a format known to state one
//! ([`STATED`]), and for MP3, WAVE, CAF and ASF only where muman reads it
//! in the file's own header; any other is taken for a guess. A format that
//! states its length but is not known so is then judged by its packets
//! and its decode alone, and a file of it cut short is not found by its
//! length; one taken to state a length it guesses was judged cut off
//! whole, every file of its kind, as VBR MP3 and MP3 in WAVE each were,
//! and encoded rather than copied for it.
//!
//! [`STATED`] holds what ffmpeg 6.1 reads from a header that still gives
//! the whole length once the file is cut: FLAC's STREAMINFO, MP4's `mvhd`
//! and `mdhd`, Matroska's segment duration and `DURATION` tags, and the
//! headers of WavPack, Monkey's Audio, TTA, AIFF and Sun AU. Ogg's is its
//! last page's granule, read from the end, so it never shows a cut and
//! never makes one up. A FLAC whose STREAMINFO counts no samples, as one
//! encoded into a pipe, has no length at all, and ffmpeg guesses none. An
//! MP3's own header says whether it states its frames, as ffmpeg reads
//! it: the Xing or Info tag, with its frame count, after the first frame's
//! side information, or the VBRI tag 32 bytes in.
//!
//! WAVE, CAF and ASF headers state a length too, but ffmpeg gives it only
//! while the file holds all the header says: past that, as for a WAVE
//! whose `data` runs past the file's end, an ASF file more than 5% off the
//! size its header states, and a CAF of PCM always, it estimates one from
//! the bytes there are, which a cut never shows. A PCM WAVE of 40 s cut to
//! 17 s read as 17 s, where the same cut of an AIFF read as 40 s. muman
//! reads these headers itself ([`Stated::Read`]). A WAVE file states its
//! length for PCM and float audio, whose data size over its block size is
//! exact, but for compressed audio held in it, as MP3, only in a `fact`
//! chunk counting its samples: without one, ffmpeg divides the data's size
//! by the header's average byte rate, which a muxer may write as anything.
//! An MP3 in WAVE of 90 s whose header said 27 kb/s for its 112 read as
//! 359 s. A file of these four whose header cannot be read states nothing
//! muman can confirm.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::chapters::Chapter;
use crate::runner::Runner;

/// Names which audio stream is taken and what is read of it and of the
/// file; facts read by another are made again, so changing any means
/// changing this. What only a versioned analysis reads, as the integrity
/// measure reads [`Audio::duration`], [`Probed::format`] and
/// [`Probed::guessed`], is versioned by that analysis instead, and caught
/// up without measuring every source again.
pub const METHOD: &str = "probe/3";

#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    /// Absolute stream index, as `-map 0:<index>` takes it.
    pub index: u32,
    pub codec: String,
    /// The codec's profile, as `DTS-HD MA`.
    pub profile: Option<String>,
    pub channels: u32,
    /// The speakers in ffmpeg's name, as `5.1(side)`; `None` when the file
    /// names none.
    pub layout: Option<String>,
    pub sample_rate: u32,
    /// Bits a decoded sample keeps; 0 when unknown.
    pub bits: u32,
    /// Whether samples decode as floating point.
    pub float: bool,
    /// Seconds, the stream's own length: its `duration`, else the
    /// `DURATION` tag Matroska keeps for each stream. The file's length
    /// counts every stream, a subtitle running past the audio too.
    pub duration: Option<f64>,
}

impl Audio {
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        crate::codec::is_lossless_name(&self.codec, self.profile.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtitle {
    /// Absolute stream index, as `-map 0:<index>` takes it.
    pub index: u32,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Position among the attachments, as `-dump_attachment:t:<n>` takes it.
    pub ordinal: usize,
    pub filename: String,
    pub mimetype: String,
}

/// An attached image ffmpeg reads as a picture stream rather than an
/// attachment: a JPEG or PNG one in Matroska, any cover in FLAC or MP3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// Absolute stream index, as `-map 0:<index>` takes it.
    pub index: u32,
    pub mimetype: String,
    pub width: u32,
    pub height: u32,
    /// Whether its picture type says it shows no front cover.
    pub other: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probed {
    pub audio: Option<Audio>,
    pub subtitles: Vec<Subtitle>,
    pub attachments: Vec<Attachment>,
    pub pictures: Vec<Picture>,
    /// ffprobe's names for the container's format, as `mp3` or
    /// `mov,mp4,m4a,3gp,3g2,mj2`.
    pub format: String,
    /// Seconds, from the container, or as [`read`] reads its header;
    /// ffmpeg's guess where [`Self::guessed`].
    pub duration: Option<f64>,
    /// Whether the file states no length, so that ffprobe's lengths are
    /// guessed from its size and bitrate; [`read`] then drops the audio
    /// stream's.
    pub guessed: bool,
    /// The container's tags, then the audio stream's where the container
    /// has none by that name, as Ogg keeps them; keys lower-cased.
    pub tags: BTreeMap<String, String>,
    pub chapters: Vec<Chapter>,
}

impl Probed {
    #[must_use]
    pub fn attachment_named(&self, filename: &str) -> Option<&Attachment> {
        self.attachments
            .iter()
            .find(|a| a.filename.eq_ignore_ascii_case(filename))
    }

    /// The image attachments, which ffmpeg cannot read as streams.
    pub fn image_attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.attachments
            .iter()
            .filter(|a| a.mimetype.starts_with("image/"))
    }
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    format: Format,
    #[serde(default)]
    chapters: Vec<ProbedChapter>,
}

#[derive(Deserialize)]
struct ProbedChapter {
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    end_time: Option<String>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

impl ProbedChapter {
    /// The chapter, where its times read and it ends after it starts.
    fn chapter(&self) -> Option<Chapter> {
        let ms = |t: &Option<String>| -> Option<u64> {
            let s: f64 = t.as_deref()?.trim().parse().ok()?;
            (s.is_finite() && s >= 0.0).then(|| (s * 1000.0).round() as u64)
        };
        let (start_ms, end_ms) = (ms(&self.start_time)?, ms(&self.end_time)?);
        let title = self
            .tags
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("title"))
            .map(|(_, v)| title(v))
            .unwrap_or_default();
        (end_ms > start_ms).then_some(Chapter {
            start_ms,
            end_ms,
            title,
        })
    }
}

/// A chapter's title as a song keeps it: a control character, which no
/// title means and which a chapter list a title is written into can
/// read as its own, is dropped, a line break or a tab becoming a space.
fn title(tag: &str) -> String {
    let kept: String = tag
        .chars()
        .filter_map(|c| match c {
            _ if !c.is_control() => Some(c),
            _ if c.is_whitespace() => Some(' '),
            _ => None,
        })
        .collect();
    kept.trim().to_string()
}

#[derive(Deserialize, Default)]
struct Format {
    #[serde(default, rename = "format_name")]
    name: String,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Stream {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    channels: u32,
    #[serde(default)]
    channel_layout: Option<String>,
    #[serde(default)]
    sample_rate: Option<String>,
    #[serde(default)]
    sample_fmt: String,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    bits_per_sample: u32,
    #[serde(default)]
    bits_per_raw_sample: Option<String>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    disposition: BTreeMap<String, i64>,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

impl Stream {
    fn audio(&self) -> Audio {
        let float = matches!(self.sample_fmt.as_str(), "flt" | "fltp" | "dbl" | "dblp");
        let raw = self
            .bits_per_raw_sample
            .as_deref()
            .and_then(|b| b.parse().ok())
            .filter(|b| *b > 0);
        let bits = match self.sample_fmt.as_str() {
            "dbl" | "dblp" => 64,
            _ if float => 32,
            _ => raw.unwrap_or(self.bits_per_sample),
        };
        Audio {
            index: self.index,
            codec: self.codec_name.clone(),
            profile: self.profile.clone().filter(|p| !p.is_empty()),
            channels: self.channels.max(1),
            layout: self
                .channel_layout
                .clone()
                .filter(|l| !l.is_empty() && l != "unknown"),
            sample_rate: self
                .sample_rate
                .as_deref()
                .and_then(|r| r.parse().ok())
                .unwrap_or(48_000),
            bits,
            float,
            duration: self
                .duration
                .as_deref()
                .and_then(|d| d.parse().ok())
                .or_else(|| self.tag("DURATION").as_deref().and_then(clock_seconds))
                .filter(|d: &f64| d.is_finite() && *d >= 0.0),
        }
    }

    fn tag(&self, key: &str) -> Option<String> {
        self.tags
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    }

    /// The image type of a picture stream: by the codec ffmpeg reads its
    /// bytes as, over the type its tag says, which a file may get wrong.
    /// A TIFF is named so it is read as ffmpeg can read one
    /// ([`crate::quality::read_whole`]).
    fn picture_type(&self) -> Option<String> {
        let read = crate::picture::ImageFormat::of_codec(&self.codec_name)
            .map(crate::picture::ImageFormat::mimetype)
            .or((self.codec_name == "tiff").then_some("image/tiff"));
        let said = self.tag("mimetype").filter(|m| m.starts_with("image/"));
        if said.is_none() && self.disposition.get("attached_pic") != Some(&1) {
            return None;
        }
        Some(
            read.map(str::to_string)
                .or(said)
                .unwrap_or_else(|| "image/unknown".into()),
        )
    }
}

/// Seconds of a clock time as Matroska writes a stream's length,
/// `HH:MM:SS.nnnnnnnnn`.
fn clock_seconds(text: &str) -> Option<f64> {
    let mut parts = text.trim().split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    let s: f64 = s.parse().ok()?;
    (m < 60 && (0.0..60.0).contains(&s)).then(|| f64::from(h) * 3600.0 + f64::from(m) * 60.0 + s)
}

#[must_use]
pub fn ffprobe_command(path: &Path) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = [
        "ffprobe",
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_streams",
        "-show_format",
        "-show_chapters",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.extend(crate::ffmpeg::Input::from(path).args());
    cmd
}

/// What `path` holds, by one ffprobe run, its audio stream's length
/// the one its header states, as muman reads it where ffmpeg would not
/// give it once the file is cut ([`Stated::Read`]), and dropped where the
/// file states none ([`Probed::guessed`]).
pub fn read<R: Runner>(runner: &R, path: &Path) -> Result<Probed> {
    let mut probed = parse(&runner.output(&ffprobe_command(path))?)?;
    match stated(&probed.format, || std::fs::File::open(path)) {
        Stated::Probed => {}
        Stated::Read(seconds) => {
            probed.duration = Some(seconds);
            if let Some(audio) = &mut probed.audio {
                audio.duration = Some(seconds);
            }
        }
        Stated::Unstated => {
            probed.guessed = true;
            if let Some(audio) = &mut probed.audio {
                audio.duration = None;
            }
        }
    }
    Ok(probed)
}

/// ffprobe's names of the formats whose header states the audio's length,
/// as ffmpeg reads it, besides MP3, WAVE, CAF and ASF, which muman reads
/// itself.
const STATED: [&str; 9] = [
    "flac",
    "ogg",
    "mov,mp4,m4a,3gp,3g2,mj2",
    "matroska,webm",
    "wv",
    "ape",
    "tta",
    "aiff",
    "au",
];

/// What a file's header states of its audio's length.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stated {
    /// The length ffprobe gives, which ffmpeg reads from the header.
    Probed,
    /// Seconds, as muman reads them from the header: ffmpeg gives what is
    /// there instead once the file is cut.
    Read(f64),
    /// None muman can confirm: ffprobe's length is a guess.
    Unstated,
}

/// What a file of `format` states of its length: nothing for one not of
/// [`STATED`] or an MP3 whose header, read from what `open` gives, counts
/// no frames; a WAVE, CAF or ASF file's as its header says.
fn stated<F: Read + Seek>(format: &str, open: impl FnOnce() -> std::io::Result<F>) -> Stated {
    let reading = |read: fn(F) -> std::io::Result<Stated>| open().and_then(read);
    let header = match format {
        "mp3" => reading(|file| {
            Ok(if mp3_counts_frames(file)? {
                Stated::Probed
            } else {
                Stated::Unstated
            })
        }),
        "wav" => reading(wave_length),
        "caf" => reading(caf_length),
        "asf" => reading(asf_length),
        _ if STATED.contains(&format) => Ok(Stated::Probed),
        _ => Ok(Stated::Unstated),
    };
    match header {
        Ok(Stated::Read(seconds)) if !(seconds.is_finite() && seconds > 0.0) => Stated::Unstated,
        Ok(stated) => stated,
        Err(_) => Stated::Unstated,
    }
}

/// The WAVE format tags whose length the data's size states: PCM and IEEE
/// float.
const WAVE_EXACT: [u16; 2] = [0x0001, 0x0003];

/// The format tag of a WAVE file whose real one is in its extension.
const WAVE_EXTENSIBLE: u16 = 0xfffe;

/// The length a WAVE file states before its `data`: of PCM or float audio,
/// the data's size over the block size, of other audio only a `fact`
/// chunk's samples. An RF64 or BW64 file's sizes are in its `ds64` chunk.
/// A RIFF file's data of no size, or of the largest, is written as one
/// streamed, its end unknown, and ffmpeg reads it to the end of the file.
/// A file not shaped as one states none muman can confirm.
fn wave_length<F: Read + Seek>(mut file: F) -> std::io::Result<Stated> {
    let mut head = [0_u8; 12];
    file.read_exact(&mut head)?;
    let wide = matches!(&head[..4], b"RF64" | b"BW64");
    if !(wide || &head[..4] == b"RIFF") || &head[8..] != b"WAVE" {
        return Ok(Stated::Unstated);
    }
    let (mut tag, mut rate, mut align) = (None, 0_u32, 0_u16);
    let (mut samples, mut wide_data) = (0_u64, None);
    loop {
        let mut chunk = [0_u8; 8];
        if file.read_exact(&mut chunk).is_err() {
            return Ok(Stated::Unstated);
        }
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        match &chunk[..4] {
            b"data" => {
                let data = if wide {
                    wide_data
                } else {
                    (size != 0 && size != u32::MAX).then_some(u64::from(size))
                };
                let frames = match data {
                    Some(bytes) if tag.is_some_and(|t| WAVE_EXACT.contains(&t)) && align > 0 => {
                        bytes / u64::from(align)
                    }
                    _ => samples,
                };
                return Ok(if frames > 0 && rate > 0 {
                    Stated::Read(frames as f64 / f64::from(rate))
                } else {
                    Stated::Unstated
                });
            }
            b"fmt " => {
                let mut fmt = vec![0_u8; size.min(64) as usize];
                file.read_exact(&mut fmt)?;
                let word = |at: usize| {
                    fmt.get(at..at + 2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                };
                tag = match word(0) {
                    Some(WAVE_EXTENSIBLE) => word(24),
                    other => other,
                };
                rate = fmt
                    .get(4..8)
                    .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                align = word(12).unwrap_or(0);
                past(&mut file, u64::from(size), fmt.len())?;
            }
            b"fact" => {
                let mut counted = [0_u8; 4];
                file.read_exact(&mut counted)?;
                samples = samples.max(u64::from(u32::from_le_bytes(counted)));
                past(&mut file, u64::from(size), 4)?;
            }
            b"ds64" => {
                let mut sizes = [0_u8; 24];
                file.read_exact(&mut sizes)?;
                let long = |at: usize| {
                    let mut b = [0_u8; 8];
                    b.copy_from_slice(&sizes[at..at + 8]);
                    u64::from_le_bytes(b)
                };
                wide_data = Some(long(8)).filter(|bytes| *bytes > 0);
                samples = samples.max(long(16));
                past(&mut file, u64::from(size), 24)?;
            }
            _ => past(&mut file, u64::from(size), 0)?,
        }
    }
}

/// Seek past the rest of a chunk of `size` bytes, padded to an even
/// length, `read` of them read already.
fn past<F: Seek>(file: &mut F, size: u64, read: usize) -> std::io::Result<()> {
    let rest = (size + (size & 1)).saturating_sub(read as u64);
    file.seek(SeekFrom::Current(i64::try_from(rest).unwrap_or(i64::MAX)))
        .map(drop)
}

/// The length a CAF file states: of audio in packets of one size, as PCM,
/// its `data` chunk's size in packets, over its `desc` chunk's rate; none
/// where that size is unknown, the file written as a stream. Packets of
/// many sizes, as ALAC's or AAC's, its `pakt` chunk lists, and ffmpeg
/// reads its length from them.
fn caf_length<F: Read + Seek>(mut file: F) -> std::io::Result<Stated> {
    let mut head = [0_u8; 8];
    file.read_exact(&mut head)?;
    if &head[..4] != b"caff" {
        return Ok(Stated::Unstated);
    }
    let (mut rate, mut packet_bytes, mut packet_frames) = (0.0, 0_u64, 0_u64);
    loop {
        let mut chunk = [0_u8; 12];
        if file.read_exact(&mut chunk).is_err() {
            return Ok(Stated::Unstated);
        }
        let mut size = [0_u8; 8];
        size.copy_from_slice(&chunk[4..]);
        let size = i64::from_be_bytes(size);
        match &chunk[..4] {
            b"desc" => {
                let mut desc = [0_u8; 32];
                file.read_exact(&mut desc)?;
                let word = |at: usize| {
                    u64::from(u32::from_be_bytes([
                        desc[at],
                        desc[at + 1],
                        desc[at + 2],
                        desc[at + 3],
                    ]))
                };
                let mut hz = [0_u8; 8];
                hz.copy_from_slice(&desc[..8]);
                (rate, packet_bytes, packet_frames) = (f64::from_be_bytes(hz), word(16), word(20));
                file.seek(SeekFrom::Current(size.saturating_sub(32)))?;
            }
            b"data" if packet_bytes == 0 || packet_frames == 0 => return Ok(Stated::Probed),
            b"data" => {
                // The data begins with an edit count; -1 is a size unknown.
                let Some(bytes) = u64::try_from(size).ok().and_then(|s| s.checked_sub(4)) else {
                    return Ok(Stated::Unstated);
                };
                let frames = bytes / packet_bytes * packet_frames;
                return Ok(Stated::Read(frames as f64 / rate));
            }
            _ if size < 0 => return Ok(Stated::Unstated),
            _ => {
                file.seek(SeekFrom::Current(size))?;
            }
        }
    }
}

/// The GUID an ASF file's header object begins with, as its bytes lie.
const ASF_HEADER: [u8; 16] = [
    0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce, 0x6c,
];

/// The GUID of an ASF file's properties object, as its bytes lie.
const ASF_PROPERTIES: [u8; 16] = [
    0xa1, 0xdc, 0xab, 0x8c, 0x47, 0xa9, 0xcf, 0x11, 0x8e, 0xe4, 0x00, 0xc0, 0x0c, 0x20, 0x53, 0x65,
];

/// The length an ASF file states, as ffmpeg reads it: its file properties'
/// play duration, in 100 ns, less its preroll, in ms; none for a broadcast,
/// whose properties hold no length. ffmpeg keeps that length only while
/// the file is within 5% of the size its properties state.
fn asf_length<F: Read + Seek>(mut file: F) -> std::io::Result<Stated> {
    let mut head = [0_u8; 30];
    file.read_exact(&mut head)?;
    if head[..16] != ASF_HEADER {
        return Ok(Stated::Unstated);
    }
    let objects = u32::from_le_bytes([head[24], head[25], head[26], head[27]]);
    let long = |b: &[u8], at: usize| {
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&b[at..at + 8]);
        u64::from_le_bytes(bytes)
    };
    for _ in 0..objects {
        let mut object = [0_u8; 24];
        file.read_exact(&mut object)?;
        let size = long(&object, 16);
        if object[..16] == ASF_PROPERTIES {
            let mut properties = [0_u8; 68];
            file.read_exact(&mut properties)?;
            let (play, preroll) = (long(&properties, 40), long(&properties, 56));
            let broadcast = properties[64] & 1 == 1;
            return Ok(if broadcast || play == 0 {
                Stated::Unstated
            } else {
                Stated::Read(play as f64 / 1e7 - preroll as f64 / 1e3)
            });
        }
        let Some(rest) = size.checked_sub(24).and_then(|r| i64::try_from(r).ok()) else {
            break;
        };
        file.seek(SeekFrom::Current(rest))?;
    }
    Ok(Stated::Unstated)
}

/// Bytes read for an MP3's first frame, past its `ID3v2` tag: enough for
/// junk before the frame and the largest header after it.
const MP3_HEAD: u64 = 8192;

/// Whether an MP3's first frame carries a Xing or Info tag with its frame
/// count, or a VBRI tag, as ffmpeg's demuxer finds them.
fn mp3_counts_frames(mut file: impl Read + Seek) -> std::io::Result<bool> {
    let mut id3 = [0u8; 10];
    let mut skip = 0;
    if file.read_exact(&mut id3).is_ok() && id3.starts_with(b"ID3") {
        let size = id3[6..10]
            .iter()
            .fold(0u64, |size, b| (size << 7) | u64::from(b & 0x7f));
        let footer = if id3[5] & 0x10 == 0 { 0 } else { 10 };
        skip = 10 + size + footer;
    }
    file.seek(SeekFrom::Start(skip))?;
    let mut head = Vec::new();
    file.take(MP3_HEAD).read_to_end(&mut head)?;
    let Some(at) = (0..head.len().saturating_sub(3)).find(|&i| mp3_frame(&head[i..i + 4])) else {
        return Ok(false);
    };
    let frame = &head[at..];
    let (mpeg1, mono) = (frame[1] & 0x18 == 0x18, frame[3] >> 6 == 3);
    let side = match (mpeg1, mono) {
        (true, false) => 32,
        (true, true) | (false, false) => 17,
        (false, true) => 9,
    };
    let tagged = |at: usize, tag: &[u8]| frame.get(at..at + 4) == Some(tag);
    let word = |at: usize| {
        frame
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let xing = 4 + side;
    if (tagged(xing, b"Xing") || tagged(xing, b"Info"))
        && word(xing + 4).is_some_and(|f| f & 1 == 1)
    {
        return Ok(word(xing + 8).is_some_and(|frames| frames > 0));
    }
    Ok(tagged(36, b"VBRI") && word(36 + 14).is_some_and(|frames| frames > 0))
}

/// Whether `b` begins an MPEG audio frame: its sync, and a version, layer,
/// bitrate and sample rate that are not reserved.
fn mp3_frame(b: &[u8]) -> bool {
    b[0] == 0xff
        && b[1] & 0xe0 == 0xe0
        && b[1] & 0x18 != 0x08
        && b[1] & 0x06 != 0
        && !matches!(b[2] >> 4, 0 | 15)
        && b[2] & 0x0c != 0x0c
}

/// Parse `ffprobe -show_streams -show_format -show_chapters`. The audio stream a song is
/// made from is the best: lossless before lossy, then the one with the
/// most channels, then the highest rate, then the first.
pub fn parse(json: &[u8]) -> Result<Probed> {
    let probe: Probe = serde_json::from_slice(json).context("parsing ffprobe JSON")?;
    let mut probed = Probed {
        format: probe.format.name,
        duration: probe
            .format
            .duration
            .as_deref()
            .and_then(|d| d.parse().ok()),
        tags: probe
            .format
            .tags
            .into_iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v))
            .collect(),
        chapters: probe
            .chapters
            .iter()
            .filter_map(ProbedChapter::chapter)
            .collect(),
        ..Probed::default()
    };
    let mut unreadable = None;
    let mut audio_tags = BTreeMap::new();
    let mut fronts = Vec::new();
    for stream in probe.streams {
        match stream.codec_type.as_str() {
            // ffprobe gives a sample rate of 0 to a stream it could not
            // decode a frame of, as a file cut off or misnamed holds.
            "audio" if stream.sample_rate.as_deref() == Some("0") => {
                unreadable.get_or_insert(stream.codec_name);
            }
            "audio" => {
                let audio = stream.audio();
                let rank = |a: &Audio| (a.is_lossless(), a.channels, a.sample_rate);
                if probed
                    .audio
                    .as_ref()
                    .is_none_or(|best| rank(&audio) > rank(best))
                {
                    probed.audio = Some(audio);
                    audio_tags = stream.tags;
                }
            }
            "subtitle" => probed.subtitles.push(Subtitle {
                index: stream.index,
                language: stream.tag("language"),
                title: stream.tag("title"),
            }),
            "video" => {
                if let Some(mimetype) = stream.picture_type() {
                    let kind = stream.tag("comment");
                    fronts.push(kind.as_deref().map(str::trim) == Some("Cover (front)"));
                    probed.pictures.push(Picture {
                        index: stream.index,
                        mimetype,
                        width: stream.width,
                        height: stream.height,
                        other: crate::picture::typed_other(kind.as_deref()),
                    });
                }
            }
            "attachment" => {
                let ordinal = probed.attachments.len();
                probed.attachments.push(Attachment {
                    ordinal,
                    filename: stream.tag("filename").unwrap_or_default(),
                    mimetype: stream.tag("mimetype").unwrap_or_default(),
                });
            }
            _ => {}
        }
    }
    // A picture typed `Other` may be the cover, unless one beside it is
    // typed the front cover.
    if fronts.contains(&true) {
        for (picture, front) in probed.pictures.iter_mut().zip(fronts) {
            picture.other |= !front;
        }
    }
    for (k, v) in audio_tags {
        probed.tags.entry(k.to_ascii_lowercase()).or_insert(v);
    }
    if let (None, Some(codec)) = (&probed.audio, unreadable) {
        bail!("its {codec} audio could not be decoded: ffprobe found no sample rate");
    }
    Ok(probed)
}

/// The start of an MP3: an `ID3v2` tag of 300 bytes where `id3`, then the
/// first frame of an MPEG-1 (else MPEG-2) Layer III stream, `stereo` or
/// mono, holding `tag` at `at` bytes into the frame, with its flags and
/// frame count after it, and a frame's worth of silence.
#[cfg(test)]
pub(crate) fn mp3_head(
    id3: bool,
    mpeg1: bool,
    stereo: bool,
    tag: Option<(&[u8; 4], usize, u32, u32)>,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    if id3 {
        bytes.extend_from_slice(b"ID3\x04\x00\x00\x00\x00\x02\x2c");
        bytes.resize(bytes.len() + 300, 0);
    }
    let start = bytes.len();
    let version = if mpeg1 { 0xfb } else { 0xf3 };
    let mode = if stereo { 0x64 } else { 0xc4 };
    bytes.extend_from_slice(&[0xff, version, 0x90, mode]);
    bytes.resize(start + 417, 0);
    if let Some((name, at, flags, frames)) = tag {
        let at = start + at;
        bytes[at..at + 4].copy_from_slice(name);
        let (flags_at, frames_at) = if name == b"VBRI" {
            (at + 10, at + 14)
        } else {
            (at + 4, at + 8)
        };
        bytes[flags_at..flags_at + 4].copy_from_slice(&flags.to_be_bytes());
        bytes[frames_at..frames_at + 4].copy_from_slice(&frames.to_be_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn guessed<F: Read + Seek>(format: &str, open: impl FnOnce() -> std::io::Result<F>) -> bool {
        stated(format, open) == Stated::Unstated
    }

    fn mp3_guessed(bytes: Vec<u8>) -> bool {
        guessed("mp3", || Ok(Cursor::new(bytes)))
    }

    #[test]
    fn an_mp3_states_its_length_only_by_a_header_counting_its_frames() {
        let xing = Some((b"Xing", 36, 0x0f, 1500));
        assert!(!mp3_guessed(mp3_head(true, true, true, xing)));
        assert!(!mp3_guessed(mp3_head(false, true, true, xing)));
        let info = Some((b"Info", 21, 0x01, 1500));
        assert!(!mp3_guessed(mp3_head(true, true, false, info)), "mono");
        let mpeg2 = Some((b"Xing", 13, 0x0f, 1500));
        assert!(!mp3_guessed(mp3_head(true, false, false, mpeg2)));
        let vbri = Some((b"VBRI", 36, 0, 1500));
        assert!(!mp3_guessed(mp3_head(true, true, true, vbri)));

        assert!(mp3_guessed(mp3_head(true, true, true, None)), "no header");
        let uncounted = Some((b"Xing", 36, 0x0e, 0));
        assert!(mp3_guessed(mp3_head(true, true, true, uncounted)));
        let misplaced = Some((b"Xing", 21, 0x0f, 1500));
        assert!(
            mp3_guessed(mp3_head(true, true, true, misplaced)),
            "a stereo frame's side information is 32 bytes"
        );
        assert!(
            mp3_guessed(b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec()),
            "no frame"
        );
        assert!(
            guessed::<Cursor<Vec<u8>>>("mp3", || Err(std::io::ErrorKind::NotFound.into())),
            "a file whose header cannot be read states nothing muman confirms"
        );
    }

    #[test]
    fn only_a_format_known_to_state_its_length_states_one() {
        let open = || Ok(Cursor::new(Vec::new()));
        for format in ["aac", "ac3", "eac3", "dts", "dtshd", "shn", "amr", ""] {
            assert!(guessed(format, open), "{format:?}");
        }
        for format in STATED {
            assert!(!guessed(format, open), "{format}");
        }
    }

    /// A WAVE file's header: its format tag, an extension's subformat tag
    /// where given, and a `fact` chunk's sample count where given.
    fn wave(tag: u16, sub: Option<u16>, fact: Option<u32>) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend(tag.to_le_bytes());
        fmt.extend(2_u16.to_le_bytes());
        fmt.extend(44_100_u32.to_le_bytes());
        fmt.extend(3375_u32.to_le_bytes());
        fmt.extend(1_u16.to_le_bytes());
        fmt.extend(0_u16.to_le_bytes());
        if let Some(sub) = sub {
            fmt.extend(22_u16.to_le_bytes());
            fmt.extend([0; 6]);
            fmt.extend(sub.to_le_bytes());
            fmt.extend([0; 14]);
        }
        let mut body = b"WAVE".to_vec();
        let mut chunk = |id: &[u8], data: &[u8]| {
            body.extend(id);
            body.extend((data.len() as u32).to_le_bytes());
            body.extend(data);
        };
        chunk(b"fmt ", &fmt);
        if let Some(samples) = fact {
            chunk(b"fact", &samples.to_le_bytes());
        }
        chunk(b"LIST", b"INFOjunk");
        chunk(b"data", &[0; 16]);
        let mut file = b"RIFF".to_vec();
        file.extend((body.len() as u32).to_le_bytes());
        file.extend(body);
        file
    }

    #[test]
    fn a_wave_of_compressed_audio_states_its_length_only_by_its_samples() {
        let wav = |bytes: Vec<u8>| guessed("wav", || Ok(Cursor::new(bytes)));
        assert!(!wav(wave(0x0001, None, None)), "PCM");
        assert!(!wav(wave(0x0003, None, None)), "float");
        assert!(!wav(wave(0xfffe, Some(0x0001), None)), "extensible PCM");
        assert!(wav(wave(0x0055, None, None)), "MP3 with no fact chunk");
        assert!(!wav(wave(0x0055, None, Some(3_969_000))), "MP3 counted");
        assert!(wav(wave(0xfffe, Some(0x0055), None)), "extensible MP3");
        assert!(wav(b"not a wave at all".to_vec()));
    }

    fn header(format: &str, bytes: Vec<u8>) -> Stated {
        stated(format, || Ok(Cursor::new(bytes)))
    }

    /// [`wave`] of PCM whose `data` chunk says it holds `bytes`.
    fn cut_wave(bytes: u32) -> Vec<u8> {
        let mut cut = wave(0x0001, None, None);
        let at = cut.len() - 20;
        cut[at..at + 4].copy_from_slice(&bytes.to_le_bytes());
        cut
    }

    #[test]
    fn a_wave_cut_off_states_the_length_its_header_does() {
        let forty = 44_100 * 40;
        assert_eq!(
            header("wav", cut_wave(forty)),
            Stated::Read(40.0),
            "not the 16 bytes held"
        );
        for streamed in [0, u32::MAX] {
            assert_eq!(
                header("wav", cut_wave(streamed)),
                Stated::Unstated,
                "{streamed}"
            );
        }
        let mut rf64 = cut_wave(u32::MAX);
        rf64[..4].copy_from_slice(b"RF64");
        let mut ds64 = b"ds64".to_vec();
        ds64.extend(28_u32.to_le_bytes());
        for long in [0_u64, 44_100 * 50, 0] {
            ds64.extend(long.to_le_bytes());
        }
        ds64.extend(0_u32.to_le_bytes());
        rf64.splice(12..12, ds64);
        assert_eq!(header("wav", rf64), Stated::Read(50.0), "its sizes in ds64");
    }

    /// A CAF file's header: packets of `packet_bytes`, one frame each, at
    /// 44.1 kHz, its data chunk `data` bytes long.
    fn caf(packet_bytes: u32, data: i64) -> Vec<u8> {
        let mut file = b"caff\0\x01\0\0desc".to_vec();
        file.extend(32_i64.to_be_bytes());
        file.extend(44_100.0_f64.to_be_bytes());
        file.extend(b"lpcm");
        for word in [0, packet_bytes, 1, 2, 16] {
            file.extend(word.to_be_bytes());
        }
        file.extend(b"data");
        file.extend(data.to_be_bytes());
        file.extend([0; 12]);
        file
    }

    #[test]
    fn a_caf_of_pcm_states_the_length_of_its_data() {
        let forty = 4 + 4 * 44_100 * 40;
        assert_eq!(header("caf", caf(4, forty)), Stated::Read(40.0));
        assert_eq!(header("caf", caf(4, -1)), Stated::Unstated, "streamed");
        assert_eq!(
            header("caf", caf(0, forty)),
            Stated::Probed,
            "packets listed"
        );
        assert_eq!(header("caf", b"caff".to_vec()), Stated::Unstated);
    }

    /// An ASF header holding file properties of `play` 100 ns, `preroll`
    /// ms and `flags`.
    fn asf(play: u64, preroll: u64, flags: u32) -> Vec<u8> {
        let mut file = ASF_HEADER.to_vec();
        file.extend(134_u64.to_le_bytes());
        file.extend(1_u32.to_le_bytes());
        file.extend([1, 2]);
        file.extend(ASF_PROPERTIES);
        file.extend(104_u64.to_le_bytes());
        file.extend([0; 40]);
        for long in [play, 0, preroll] {
            file.extend(long.to_le_bytes());
        }
        file.extend(flags.to_le_bytes());
        file.extend([0; 12]);
        file
    }

    #[test]
    fn an_asf_file_states_its_play_duration_less_its_preroll() {
        assert_eq!(header("asf", asf(71_000_000, 100, 2)), Stated::Read(7.0));
        let broadcast = header("asf", asf(71_000_000, 100, 1));
        assert_eq!(broadcast, Stated::Unstated);
        let mut other = asf(71_000_000, 100, 2);
        other[0] = 0;
        assert_eq!(header("asf", other), Stated::Unstated);
    }

    #[test]
    fn a_length_ffmpeg_estimates_gives_way_to_the_one_the_header_states() {
        use crate::testing::Fake;

        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "pcm_s16le",
            "sample_rate": "44100", "channels": 2, "duration": "17.0"}],
            "format": {"format_name": "wav", "duration": "17.0"}}"#;
        let fake = Fake::default().probe(".wav", json);
        let path = dir.path().join("Cut.wav");
        std::fs::write(&path, cut_wave(44_100 * 40)).unwrap();
        let probed = read(&fake, &path).unwrap();
        assert!(!probed.guessed);
        assert_eq!(probed.audio.unwrap().duration, Some(40.0));
        assert_eq!(probed.duration, Some(40.0));
    }

    #[test]
    fn a_guessed_length_is_dropped_from_the_stream_and_kept_for_the_file() {
        use crate::testing::Fake;

        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "mp3",
            "sample_rate": "44100", "channels": 2, "duration": "62.0"}],
            "format": {"format_name": "mp3", "duration": "62.0"}}"#;
        let fake = Fake::default().probe(".mp3", json);
        let bare = dir.path().join("Bare.mp3");
        std::fs::write(&bare, mp3_head(true, true, true, None)).unwrap();
        let probed = read(&fake, &bare).unwrap();
        assert!(probed.guessed);
        assert_eq!(probed.audio.unwrap().duration, None);
        assert_eq!(probed.duration, Some(62.0), "where excerpts are placed");
        let counted = dir.path().join("Counted.mp3");
        let xing = Some((b"Xing", 36, 0x0f, 2374));
        std::fs::write(&counted, mp3_head(true, true, true, xing)).unwrap();
        let probed = read(&fake, &counted).unwrap();
        assert!(!probed.guessed);
        assert_eq!(probed.audio.unwrap().duration, Some(62.0));
    }

    // Trimmed from ffprobe on a real yt-dlp original.
    const FIXTURE: &str = r#"{"streams": [
        {"index": 0, "codec_name": "vp9", "codec_type": "video", "width": 1920, "height": 1080},
        {"index": 1, "codec_name": "opus", "codec_type": "audio", "channels": 2,
         "sample_rate": "48000",
         "tags": {"language": "eng", "DURATION": "00:03:44.261000000"}},
        {"index": 2, "codec_name": "webvtt", "codec_type": "subtitle",
         "tags": {"language": "zho", "title": "Chinese (Traditional)"}},
        {"index": 3, "codec_name": "webvtt", "codec_type": "subtitle",
         "tags": {"language": "eng", "title": "English"}},
        {"index": 7, "codec_type": "attachment",
         "tags": {"filename": "info.json", "mimetype": "application/json"}},
        {"index": 8, "codec_type": "attachment",
         "tags": {"filename": "cover.webp", "mimetype": "image/webp"}}
    ], "format": {"duration": "226.400000", "tags": {"TITLE": "Song"}}}"#;

    #[test]
    fn reads_audio_subtitles_attachments_and_format() {
        let o = parse(FIXTURE.as_bytes()).unwrap();
        assert_eq!(
            o.audio,
            Some(Audio {
                index: 1,
                codec: "opus".into(),
                profile: None,
                channels: 2,
                layout: None,
                sample_rate: 48_000,
                bits: 0,
                float: false,
                duration: Some(224.261),
            })
        );
        assert_eq!(o.subtitles.len(), 2);
        assert_eq!(o.subtitles[1].index, 3);
        assert_eq!(o.subtitles[1].title.as_deref(), Some("English"));
        assert_eq!(o.attachment_named("INFO.JSON").map(|a| a.ordinal), Some(0));
        assert_eq!(
            o.image_attachments().map(|a| a.ordinal).collect::<Vec<_>>(),
            [1]
        );
        assert!(o.pictures.is_empty(), "the video stream is no picture");
        assert_eq!(
            o.duration,
            Some(226.4),
            "the file's length, its subtitles' too"
        );
        assert_eq!(o.tags.get("title").map(String::as_str), Some("Song"));
    }

    #[test]
    fn a_stream_says_its_own_length_by_its_duration_or_its_tag() {
        let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "mp3",
            "sample_rate": "44100", "channels": 2, "duration": "358.516463"}],
            "format": {"duration": "358.516463"}}"#;
        let o = parse(json.as_bytes()).unwrap();
        assert_eq!(o.audio.unwrap().duration, Some(358.516_463));
        assert_eq!(clock_seconds("01:02:03.5"), Some(3723.5));
        for bad in ["", "3:04", "00:61:00.0", "00:00:75.0", "a:b:c", "1:2:3:4"] {
            assert_eq!(clock_seconds(bad), None, "{bad}");
        }
        let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "opus",
            "tags": {"DURATION": "soon"}}]}"#;
        assert_eq!(
            parse(json.as_bytes()).unwrap().audio.unwrap().duration,
            None
        );
    }

    #[test]
    fn an_attached_jpeg_is_a_picture_not_a_video() {
        let json = r#"{"streams": [
            {"index": 0, "codec_type": "video", "codec_name": "vp9"},
            {"index": 2, "codec_type": "video", "codec_name": "mjpeg", "width": 1400, "height": 1400,
             "tags": {"filename": "cover.jpg", "mimetype": "image/jpeg"}}]}"#;
        let o = parse(json.as_bytes()).unwrap();
        assert_eq!(
            o.pictures,
            vec![Picture {
                index: 2,
                mimetype: "image/jpeg".into(),
                width: 1400,
                height: 1400,
                other: false,
            }]
        );
        assert_eq!(o.image_attachments().count(), 0);
    }

    #[test]
    fn a_flac_cover_is_known_by_its_disposition() {
        let json = r#"{"streams": [
            {"index": 0, "codec_type": "audio", "codec_name": "flac", "channels": 2},
            {"index": 1, "codec_type": "video", "codec_name": "png", "width": 600, "height": 600,
             "disposition": {"attached_pic": 1}}]}"#;
        let o = parse(json.as_bytes()).unwrap();
        assert_eq!(o.pictures[0].mimetype, "image/png");
        let tiff = json.replace(r#""codec_name": "png""#, r#""codec_name": "tiff""#);
        let o = parse(tiff.as_bytes()).unwrap();
        assert_eq!(o.pictures[0].mimetype, "image/tiff");
    }

    #[test]
    fn a_picture_typed_other_is_the_cover_only_beside_no_front() {
        let picture = |index: u32, kind: &str| {
            format!(
                r#"{{"index": {index}, "codec_type": "video", "codec_name": "mjpeg",
                    "width": 600, "height": 600, "disposition": {{"attached_pic": 1}},
                    "tags": {{"comment": "{kind}"}}}}"#
            )
        };
        let probed = |kinds: &[&str]| {
            let streams: Vec<String> = kinds
                .iter()
                .zip(1..)
                .map(|(kind, index)| picture(index, kind))
                .collect();
            let json = format!(
                r#"{{"streams": [{{"index": 0, "codec_type": "audio", "codec_name": "mp3",
                    "channels": 2}}, {}]}}"#,
                streams.join(",")
            );
            let o = parse(json.as_bytes()).unwrap();
            o.pictures.iter().map(|p| p.other).collect::<Vec<_>>()
        };
        assert_eq!(probed(&["Other"]), [false]);
        assert_eq!(probed(&["Other", "Cover (front)"]), [true, false]);
        assert_eq!(probed(&["Cover (back)", "Other"]), [true, false]);
    }

    #[test]
    fn chapters_are_read_with_their_titles() {
        let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "aac",
            "channels": 2}],
          "chapters": [
            {"id": 0, "time_base": "1/44100", "start": 0, "start_time": "0.000000",
             "end": 14509414, "end_time": "329.011655", "tags": {"title": "Part One"}},
            {"id": 1, "start_time": "329.011655", "end_time": "466.017392"},
            {"id": 2, "start_time": "500.0", "end_time": "500.0"}]}"#;
        let o = parse(json.as_bytes()).unwrap();
        assert_eq!(
            o.chapters,
            [
                Chapter {
                    start_ms: 0,
                    end_ms: 329_012,
                    title: "Part One".into()
                },
                Chapter {
                    start_ms: 329_012,
                    end_ms: 466_017,
                    title: String::new()
                },
            ],
            "one of no length is none"
        );
    }

    #[test]
    fn a_chapter_title_keeps_no_control_character() {
        let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "aac",
            "channels": 2}],
          "chapters": [{"start_time": "0.0", "end_time": "1.0",
            "tags": {"title": " Harbor\r[CHAPTER]\u001b]0;X\u0007\tEnd\u0000 "}}]}"#;
        let o = parse(json.as_bytes()).unwrap();
        assert_eq!(o.chapters[0].title, "Harbor [CHAPTER]]0;X End");
    }

    #[test]
    fn upper_case_tag_keys_are_read() {
        let json = r#"{"streams": [{"index": 2, "codec_type": "subtitle",
            "tags": {"LANGUAGE": "jpn", "TITLE": "Japanese"}}]}"#;
        let o = parse(json.as_bytes()).unwrap();
        assert_eq!(o.subtitles[0].language.as_deref(), Some("jpn"));
    }

    // Trimmed from ffprobe on a text file named `.flac`.
    #[test]
    fn audio_ffprobe_could_not_decode_is_an_error() {
        let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "flac",
            "sample_rate": "0", "channels": 0}], "format": {}}"#;
        assert!(parse(json.as_bytes()).is_err());
        let json = r#"{"streams": [
            {"index": 0, "codec_type": "audio", "codec_name": "flac", "sample_rate": "0"},
            {"index": 1, "codec_type": "audio", "codec_name": "opus", "sample_rate": "48000",
             "channels": 2}]}"#;
        assert_eq!(parse(json.as_bytes()).unwrap().audio.unwrap().index, 1);
    }

    #[test]
    fn a_file_with_no_audio_has_none() {
        let json = r#"{"streams": [{"index": 0, "codec_type": "video"}]}"#;
        assert_eq!(parse(json.as_bytes()).unwrap().audio, None);
    }

    // Trimmed from ffprobe on an Ogg Opus file, whose comments are the
    // stream's.
    #[test]
    fn the_audio_streams_tags_fill_the_containers() {
        let json = r#"{"streams": [
            {"index": 0, "codec_type": "audio", "codec_name": "opus", "channels": 2,
             "tags": {"title": "Velvet Ladder", "ARTIST": "Marlo Venn", "album": "Stream"}},
            {"index": 1, "codec_type": "audio", "codec_name": "opus",
             "tags": {"genre": "Second stream"}}],
            "format": {"tags": {"ALBUM": "Low Orchard"}}}"#;
        let o = parse(json.as_bytes()).unwrap();
        assert_eq!(o.tags["title"], "Velvet Ladder");
        assert_eq!(o.tags["artist"], "Marlo Venn");
        assert_eq!(o.tags["album"], "Low Orchard");
        assert!(!o.tags.contains_key("genre"));
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn the_best_audio_stream_is_taken() {
        let json = r#"{"streams": [
            {"index": 0, "codec_name": "aac", "codec_type": "audio", "channels": 2,
             "sample_rate": "48000", "sample_fmt": "fltp", "tags": {"title": "Downmix"}},
            {"index": 1, "codec_name": "flac", "codec_type": "audio", "channels": 6,
             "channel_layout": "5.1(side)", "sample_rate": "96000", "sample_fmt": "s32",
             "bits_per_raw_sample": "24", "tags": {"title": "Master"}},
            {"index": 2, "codec_name": "flac", "codec_type": "audio", "channels": 2,
             "sample_rate": "192000", "sample_fmt": "s16"}
        ], "format": {}}"#;
        let o = parse(json.as_bytes()).unwrap();
        let audio = o.audio.unwrap();
        assert_eq!(audio.index, 1);
        assert_eq!(audio.layout.as_deref(), Some("5.1(side)"));
        assert_eq!((audio.bits, audio.float), (24, false));
        assert_eq!(o.tags["title"], "Master");
    }

    #[test]
    fn float_and_wide_samples_are_told_apart() {
        let audio = |fmt: &str, bits: &str| {
            let json = format!(
                r#"{{"streams": [{{"index": 0, "codec_name": "pcm_x", "codec_type": "audio",
                    "channels": 2, "sample_rate": "44100", "sample_fmt": "{fmt}",
                    "bits_per_sample": 32, "bits_per_raw_sample": "{bits}"}}], "format": {{}}}}"#
            );
            let a = parse(json.as_bytes()).unwrap().audio.unwrap();
            (a.bits, a.float)
        };
        assert_eq!(audio("s32", "32"), (32, false));
        assert_eq!(audio("s32", "N/A"), (32, false));
        assert_eq!(audio("flt", "N/A"), (32, true));
        assert_eq!(audio("dbl", "N/A"), (64, true));
    }

    /// Files that take every path [`parse`] reads by: a picture typed
    /// beside a front cover, a stream's own length in its tag, chapters,
    /// a control character, audio no frame of decoded, and no lengths.
    const SAMPLES: [&str; 5] = [
        r#"{"streams": [
            {"index": 0, "codec_type": "audio", "codec_name": "dts", "profile": "DTS-HD MA",
             "channels": 6, "channel_layout": "5.1(side)", "sample_rate": "96000",
             "sample_fmt": "s32p", "bits_per_raw_sample": "24", "duration": "241.5"},
            {"index": 1, "codec_type": "audio", "codec_name": "aac", "channels": 2,
             "channel_layout": "unknown", "sample_rate": "48000", "sample_fmt": "fltp"},
            {"index": 2, "codec_type": "video", "codec_name": "mjpeg", "width": 600,
             "height": 600, "disposition": {"attached_pic": 1},
             "tags": {"comment": "Cover (front)"}},
            {"index": 3, "codec_type": "video", "codec_name": "png", "width": 300,
             "height": 200, "disposition": {"attached_pic": 1}, "tags": {"comment": "Other"}}
        ], "format": {"duration": "241.52", "tags": {"ARTIST": "Marlo Venn"}}}"#,
        r#"{"streams": [
            {"index": 0, "codec_type": "audio", "codec_name": "vorbis", "channels": 1,
             "sample_rate": "22050", "sample_fmt": "dbl",
             "tags": {"TITLE": "Paper Comets", "DURATION": "00:02:05.250000000"}}
        ], "format": {"tags": {"title": "The Glass Orchards"}},
        "chapters": [
            {"start_time": "0.000000", "end_time": "61.5", "tags": {"title": "Part\u0007 One\n"}},
            {"start_time": "61.5", "end_time": "61.5", "tags": {"title": "Empty"}},
            {"start_time": "61.5", "end_time": "125.25", "tags": {"TITLE": "Part Two"}}
        ]}"#,
        r#"{"streams": [
            {"index": 0, "codec_type": "audio", "codec_name": "flac", "channels": 2,
             "bits_per_sample": 16},
            {"index": 1, "codec_type": "subtitle", "codec_name": "ass"},
            {"index": 2, "codec_type": "attachment", "tags": {"filename": "back.jpg",
             "mimetype": "image/jpeg"}},
            {"index": 3, "codec_type": "video", "codec_name": "webp", "width": 80, "height": 80,
             "tags": {"mimetype": "image/png"}}
        ]}"#,
        r#"{"streams": [
            {"index": 0, "codec_type": "audio", "codec_name": "mp3", "sample_rate": "0"}
        ], "format": {"duration": "12.0"}}"#,
        r#"{"streams": []}"#,
    ];

    /// What probing reads of [`SAMPLES`] and the shared fixtures, with
    /// [`METHOD`].
    const WRITTEN: (&str, &str) = (
        "probe/3",
        "156e8f078cc6f4601b01c564a6fcfea2e984247747ba318a80e1c9e35af8434a",
    );

    #[test]
    fn the_method_rises_with_what_it_makes() {
        let mut digest = crate::testing::Digest::default();
        let fixtures = [crate::testing::ORIGINAL, crate::testing::FLAC, FIXTURE];
        for json in fixtures.into_iter().chain(SAMPLES) {
            match parse(json.as_bytes()) {
                Ok(probed) => digest.debug(&probed),
                Err(_) => digest.text("unreadable"),
            };
        }
        crate::testing::check(WRITTEN, METHOD, &digest.hex());
    }
}
