//! What a source file holds, read from ffprobe's JSON.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::chapters::Chapter;

/// Names which audio stream is taken and what is read of it and of the
/// file; facts read by another are made again, so changing any means
/// changing this. What only a versioned analysis reads, as the integrity
/// measure reads [`Audio::duration`], is versioned by that analysis
/// instead, and caught up without measuring every source again.
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
    /// Seconds, from the container.
    pub duration: Option<f64>,
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
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default();
        (end_ms > start_ms).then_some(Chapter {
            start_ms,
            end_ms,
            title,
        })
    }
}

#[derive(Deserialize, Default)]
struct Format {
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
        let read = match self.codec_name.as_str() {
            "mjpeg" => Some("image/jpeg"),
            "png" => Some("image/png"),
            "webp" => Some("image/webp"),
            _ => None,
        };
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
        "--",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.push(path.as_os_str().to_os_string());
    cmd
}

/// Parse `ffprobe -show_streams -show_format -show_chapters`. The audio stream a song is
/// made from is the best: lossless before lossy, then the one with the
/// most channels, then the highest rate, then the first.
pub fn parse(json: &[u8]) -> Result<Probed> {
    let probe: Probe = serde_json::from_slice(json).context("parsing ffprobe JSON")?;
    let mut probed = Probed {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
