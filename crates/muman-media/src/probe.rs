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
//! read from its packets instead. An MP3's own header says whether it
//! states its frames, as ffmpeg reads it: the Xing or Info tag, with its
//! frame count, after the first frame's side information, or the VBRI tag
//! 32 bytes in. A file that cannot be read keeps ffprobe's lengths.

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
    /// Seconds, from the container; ffmpeg's guess where [`Self::guessed`].
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
    fn picture_type(&self) -> Option<String> {
        let read = crate::picture::ImageFormat::of_codec(&self.codec_name)
            .map(crate::picture::ImageFormat::mimetype);
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
/// dropped where the file states none ([`Probed::guessed`]).
pub fn read<R: Runner>(runner: &R, path: &Path) -> Result<Probed> {
    let mut probed = parse(&runner.output(&ffprobe_command(path))?)?;
    probed.guessed = guessed(&probed.format, || std::fs::File::open(path));
    if probed.guessed
        && let Some(audio) = &mut probed.audio
    {
        audio.duration = None;
    }
    Ok(probed)
}

/// ffprobe's names of the formats that state no length: raw elementary
/// streams, whose demuxer leaves ffmpeg to guess one from the bitrate.
const UNSTATED: [&str; 6] = ["aac", "ac3", "eac3", "dts", "truehd", "mlp"];

/// Whether a file of `format` states no length: one of [`UNSTATED`], or an
/// MP3 whose header, read from what `open` gives, counts no frames.
fn guessed<F: Read + Seek>(format: &str, open: impl FnOnce() -> std::io::Result<F>) -> bool {
    match format {
        "mp3" => open()
            .and_then(mp3_counts_frames)
            .is_ok_and(|counts| !counts),
        _ => UNSTATED.contains(&format),
    }
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
            !guessed::<Cursor<Vec<u8>>>("mp3", || Err(std::io::ErrorKind::NotFound.into())),
            "a file that cannot be read keeps ffprobe's lengths"
        );
    }

    #[test]
    fn a_raw_stream_states_no_length_and_a_container_does() {
        let open = || Ok(Cursor::new(Vec::new()));
        for format in ["aac", "ac3", "eac3", "dts"] {
            assert!(guessed(format, open), "{format}");
        }
        for format in [
            "flac",
            "ogg",
            "wav",
            "matroska,webm",
            "mov,mp4,m4a,3gp,3g2,mj2",
        ] {
            assert!(!guessed(format, open), "{format}");
        }
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
        "8f725d4bed9c52ee0f23d973b54cda1f3de6b907c9a860e178ba18cf8a664eea",
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
