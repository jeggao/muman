//! yt-dlp's info JSON, embedded in each file it fetched.
//!
//! Each of yt-dlp's extractors fills it its own way, and one may write a
//! number as text, a count as a float or a `null` in a list. Every field
//! is read leniently: a value of a type a field does not hold loses that
//! field alone, never the file, whose title, IDs and page say which
//! source it is.
//!
//! As yt-dlp writes it, it also holds much muman never reads, some of it
//! the user's: what each request of the fetch sent, the cookies, a login
//! among them when `[ytdlp] args` passes `--cookies` or
//! `--cookies-from-browser`, and the HTTP headers, at the top and again
//! in each format and thumbnail; and the address of every format,
//! fragment, thumbnail and subtitle, which a site may sign for the
//! address that asked, as YouTube's name its IP address and carry a PO
//! token. A source keeps only what muman reads ([`only_read`]): the
//! fields [`VideoInfo`], [`Format`] and [`SubtitleFormat`] read, as their
//! own `Deserialize` names them, so a field read later is kept with no
//! list to change, and [`TAG_FIELDS`]. Each is kept as it is, so the
//! tags' digest does not change. What a source still holds of the user's
//! is what they gave: the page, and the address given to `add`, which
//! muman reads to name and fetch the source again.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer};

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideoInfo {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub extractor_key: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub track: Option<String>,
    #[serde(default, deserialize_with = "texts")]
    pub artists: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient")]
    pub artist: Option<String>,
    #[serde(default, deserialize_with = "texts")]
    pub creators: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient")]
    pub creator: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub album: Option<String>,
    #[serde(default, deserialize_with = "texts")]
    pub album_artists: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient")]
    pub album_artist: Option<String>,
    #[serde(default, deserialize_with = "whole")]
    pub track_number: Option<u32>,
    #[serde(default, deserialize_with = "whole")]
    pub disc_number: Option<u32>,
    #[serde(default, deserialize_with = "texts")]
    pub genres: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient")]
    pub genre: Option<String>,
    /// `YYYYMMDD`.
    #[serde(default, deserialize_with = "lenient")]
    pub release_date: Option<String>,
    #[serde(default, deserialize_with = "whole")]
    pub release_year: Option<u32>,
    /// `YYYYMMDD`.
    #[serde(default, deserialize_with = "lenient")]
    pub upload_date: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub uploader: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub channel: Option<String>,
    /// Language code to the formats it was offered in; only `name` is read.
    #[serde(default, deserialize_with = "lenient")]
    pub subtitles: BTreeMap<String, Vec<SubtitleFormat>>,
    /// The same, for captions the site generated.
    #[serde(default, deserialize_with = "lenient")]
    pub automatic_captions: BTreeMap<String, Vec<SubtitleFormat>>,
    /// The format fetched: one ID, or a video's and an audio's joined by
    /// `+`.
    #[serde(default, deserialize_with = "lenient")]
    pub format_id: Option<String>,
    /// Seconds the site says it plays.
    #[serde(default, deserialize_with = "lenient")]
    pub duration: Option<f64>,
    /// Every format the site offered.
    #[serde(default, deserialize_with = "each")]
    pub formats: Vec<Format>,
    /// The page the video was fetched from.
    #[serde(default, deserialize_with = "lenient")]
    pub webpage_url: Option<String>,
    /// The address yt-dlp was given, which an info JSON it embeds leaves
    /// out.
    #[serde(default, deserialize_with = "lenient")]
    pub original_url: Option<String>,
    /// That address, copied where the embedded info JSON keeps it.
    #[serde(default, deserialize_with = "lenient")]
    pub muman_original_url: Option<String>,
}

/// One format a site offered, as yt-dlp lists it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct Format {
    #[serde(default, deserialize_with = "lenient")]
    pub format_id: String,
    /// Exact, where the site says it; `filesize_approx` is not read.
    #[serde(default, deserialize_with = "whole")]
    pub filesize: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    pub acodec: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub vcodec: Option<String>,
}

impl Format {
    pub(crate) fn has_audio(&self) -> bool {
        self.acodec.as_deref().is_none_or(|c| c != "none")
    }

    pub(crate) fn has_video(&self) -> bool {
        self.vcodec.as_deref().is_some_and(|c| c != "none")
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SubtitleFormat {
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
}

/// The fields of the info JSON a song's tags are read from.
pub const TAG_FIELDS: [&str; 19] = [
    "title",
    "track",
    "artists",
    "artist",
    "creators",
    "creator",
    "album",
    "album_artists",
    "album_artist",
    "track_number",
    "disc_number",
    "genres",
    "genre",
    "release_date",
    "release_year",
    "upload_date",
    "uploader",
    "channel",
    "id",
];

/// The digest of [`TAG_FIELDS`] in an info JSON, as they are there:
/// the rest of it changes on every fetch, as its time does.
#[must_use]
pub fn tags_digest(json: &[u8]) -> Option<String> {
    let all: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(json).ok()?;
    let read: BTreeMap<String, serde_json::Value> = all
        .into_iter()
        .filter(|(k, v)| TAG_FIELDS.contains(&k.as_str()) && !v.is_null())
        .collect();
    Some(crate::facts::digest(&serde_json::to_vec(&read).ok()?))
}

/// `json` with only the fields muman reads, or `None` when it holds
/// nothing else or does not read: of the whole, the fields [`VideoInfo`]
/// reads and [`TAG_FIELDS`]; of each format, those [`Format`] reads; of
/// each subtitle or caption, those [`SubtitleFormat`] reads. A list item
/// or table of another type, which reads as nothing, becomes `null`,
/// which reads as nothing alike.
#[must_use]
pub fn only_read(json: &[u8]) -> Option<Vec<u8>> {
    use serde_json::Value;
    let mut value: Value = serde_json::from_slice(json).ok()?;
    let before = value.clone();
    let Value::Object(top) = &mut value else {
        return None;
    };
    let read = fields::<VideoInfo>();
    top.retain(|field, _| read.contains(&field.as_str()) || TAG_FIELDS.contains(&field.as_str()));
    if let Some(formats) = top.get_mut("formats") {
        keep_in_each(formats, fields::<Format>());
    }
    for captions in ["subtitles", "automatic_captions"] {
        match top.get_mut(captions) {
            Some(Value::Object(languages)) => {
                for formats in languages.values_mut() {
                    keep_in_each(formats, fields::<SubtitleFormat>());
                }
            }
            Some(other) => *other = Value::Null,
            None => {}
        }
    }
    (value != before)
        .then(|| serde_json::to_vec(&value).ok())
        .flatten()
}

/// Of each object in the list `items`, the fields of `read` alone; any
/// other item, or `items` that is no list, `null`.
fn keep_in_each(items: &mut serde_json::Value, read: &[&str]) {
    let serde_json::Value::Array(items) = items else {
        *items = serde_json::Value::Null;
        return;
    };
    for item in items {
        match item {
            serde_json::Value::Object(fields) => fields.retain(|f, _| read.contains(&f.as_str())),
            other => *other = serde_json::Value::Null,
        }
    }
}

/// The fields `T`'s derived `Deserialize` reads, as it names them to
/// serde: a struct is asked of its deserializer with its field names
/// before any value, so one that stops there reads them off. A field
/// added to `T` is kept by [`only_read`] with no list to change.
fn fields<'de, T: Deserialize<'de>>() -> Fields {
    let names = std::cell::Cell::new(&[][..]);
    let _ = T::deserialize(FieldNames(&names));
    names.get()
}

/// A deserializer that notes the field names of the struct asked of it,
/// and gives nothing.
struct FieldNames<'a>(&'a std::cell::Cell<Fields>);

/// A struct's field names, as serde is given them.
type Fields = &'static [&'static str];

impl<'de> Deserializer<'de> for FieldNames<'_> {
    type Error = serde::de::value::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(serde::de::Error::custom(
            "only a struct's field names are read",
        ))
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.set(fields);
        self.deserialize_any(visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
        byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct map
        enum identifier ignored_any
    }
}

/// A field's value, or its default when it holds another type.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// A list's items of the type it holds, the others dropped: a `null`
/// among a video's artists loses that one.
fn each<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        _ => Vec::new(),
    })
}

fn texts<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value.is_array().then(|| each(value).unwrap_or_default()))
}

/// A whole number, written as one, as a float or as text.
fn whole<'de, D: Deserializer<'de>, T: TryFrom<u64>>(d: D) -> Result<Option<T>, D::Error> {
    let n = match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_u64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && *f >= 0.0 && f.fract() == 0.0)
                .map(|f| f as u64)
        }),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    };
    Ok(n.and_then(|n| T::try_from(n).ok()))
}

pub fn parse(json: &[u8]) -> Result<VideoInfo> {
    serde_json::from_slice(json).context("parsing yt-dlp's info JSON")
}

impl VideoInfo {
    /// The key it is a source under, by its extractor, page and ID, as
    /// `sites` names it.
    #[must_use]
    pub fn key(&self, sites: &crate::sites::Sites) -> Option<crate::source::SourceKey> {
        sites.fetched(
            self.extractor_key.as_deref()?,
            self.page(),
            self.id.as_deref()?,
        )
    }

    /// The page it names its site by and is fetched again from, as
    /// [`crate::source::page`] picks it.
    #[must_use]
    pub fn page(&self) -> Option<&str> {
        crate::source::page(
            self.extractor_key.as_deref().unwrap_or_default(),
            self.webpage_url.as_deref(),
            self.original_url
                .as_deref()
                .or(self.muman_original_url.as_deref()),
        )
    }

    /// Whether it is a YouTube Music release: it names its track, or its
    /// channel is an artist's " - Topic", where only releases are.
    #[must_use]
    pub fn is_release(&self) -> bool {
        self.track.as_deref().is_some_and(|t| !t.trim().is_empty())
            || self
                .channel
                .as_deref()
                .is_some_and(|c| c.ends_with(" - Topic"))
    }

    /// The channel's display name; a release's channel without the
    /// " - Topic" YouTube appends to an artist's.
    #[must_use]
    pub fn channel_name(&self) -> Option<String> {
        [self.channel.as_ref(), self.uploader.as_ref()]
            .into_iter()
            .flatten()
            .map(|c| c.trim().trim_end_matches(" - Topic").trim())
            .find(|c| !c.is_empty())
            .map(str::to_string)
    }

    /// Whether the info JSON lists any subtitles or captions at all, and
    /// so can tell a person's subtitle from a generated one.
    #[must_use]
    pub fn knows_subtitles(&self) -> bool {
        !self.subtitles.is_empty() || !self.automatic_captions.is_empty()
    }

    /// The format of the audio fetched: of a video's and an audio's
    /// joined, the one without video.
    #[must_use]
    pub fn audio_format(&self) -> Option<&Format> {
        let parts: Vec<&Format> = self
            .format_id
            .as_deref()?
            .split('+')
            .filter_map(|id| self.formats.iter().find(|f| f.format_id == id))
            .collect();
        parts
            .iter()
            .find(|f| f.has_audio() && !f.has_video())
            .or_else(|| parts.iter().find(|f| f.has_audio()))
            .copied()
    }

    /// The display names yt-dlp gave the subtitles in `code`, which it
    /// also writes as each embedded stream's title.
    #[must_use]
    pub fn subtitle_names(&self, code: &str) -> Vec<&str> {
        self.subtitles
            .get(code)
            .into_iter()
            .flatten()
            .filter_map(|f| f.name.as_deref())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(json: &str) -> VideoInfo {
        parse(json.as_bytes()).unwrap()
    }

    #[test]
    fn only_what_muman_reads_is_kept_and_reads_the_same() {
        let fetched = r#"{"id": "vid00000001", "title": "Lantern Weather", "epoch": 1,
            "extractor_key": "Youtube", "track_number": "3", "artists": ["Marlo Venn", null],
            "duration": 241.5, "format_id": "251", "release_year": 2024,
            "cookies": "sess=TOKEN-0001; Domain=.video.test",
            "http_headers": {"User-Agent": "Agent/1", "Authorization": "TOKEN-0002"},
            "url": "https://media.video.test/v?ip=203.0.113.7&pot=TOKEN-0003",
            "manifest_url": "https://media.video.test/m?ip=203.0.113.7",
            "webpage_url": "https://video.test/v/vid00000001",
            "muman_original_url": "https://video.test/v/vid00000001",
            "requested_downloads": [{"filepath": "/home/marlo/partial/vid00000001.webm"}],
            "formats": [{"format_id": "251", "filesize": 100, "acodec": "opus", "vcodec": "none",
                "url": "https://media.video.test/251?ip=203.0.113.7&pot=TOKEN-0004",
                "fragment_base_url": "https://media.video.test/f?ip=203.0.113.7",
                "fragments": [{"url": "https://media.video.test/f/1?ip=203.0.113.7"}],
                "cookies": "sess=TOKEN-0001", "http_headers": {"Referer": "https://video.test/"}},
                "not a format"],
            "subtitles": {"en": [{"ext": "vtt", "name": "English",
                "url": "https://video.test/sub?ip=203.0.113.7&pot=TOKEN-0005"}]},
            "automatic_captions": {"ja": [{"url": "https://video.test/cap?ip=203.0.113.7"}]},
            "thumbnails": [{"url": "https://video.test/t.jpg", "http_headers": {"Cookie": "TOKEN-0006"}}]}"#;
        let kept = only_read(fetched.as_bytes()).unwrap();
        let text = String::from_utf8(kept.clone()).unwrap();
        for sent in [
            "TOKEN-",
            "203.0.113.7",
            "cookies",
            "http_headers",
            "/home/marlo",
            "epoch",
        ] {
            assert!(!text.contains(sent), "{sent} in {text}");
        }
        assert_eq!(tags_digest(&kept), tags_digest(fetched.as_bytes()));
        assert_eq!(
            format!("{:?}", parse(&kept).unwrap()),
            format!("{:?}", parse(fetched.as_bytes()).unwrap()),
            "read the same"
        );
        let i = parse(&kept).unwrap();
        assert_eq!(i.subtitle_names("en"), ["English"]);
        assert!(i.knows_subtitles());
        assert_eq!(i.audio_format().and_then(|f| f.filesize), Some(100));
        assert_eq!(only_read(&kept), None, "nothing left to drop");
        assert_eq!(only_read(b"not json"), None);
        let odd = r#"{"id": "a", "subtitles": ["https://video.test/sub?pot=TOKEN-0007"]}"#;
        let kept = only_read(odd.as_bytes()).unwrap();
        assert!(!String::from_utf8_lossy(&kept).contains("TOKEN"));
        assert_eq!(
            format!("{:?}", parse(&kept).unwrap()),
            format!("{:?}", parse(odd.as_bytes()).unwrap())
        );
    }

    #[test]
    fn the_fields_kept_are_those_each_struct_reads() {
        let read = fields::<VideoInfo>();
        for field in [
            "muman_original_url",
            "formats",
            "subtitles",
            "format_id",
            "duration",
        ] {
            assert!(read.contains(&field), "{field}");
        }
        assert!(TAG_FIELDS.iter().all(|f| read.contains(f)));
        assert_eq!(
            fields::<Format>(),
            ["format_id", "filesize", "acodec", "vcodec"]
        );
        assert_eq!(fields::<SubtitleFormat>(), ["name"]);
    }

    #[test]
    fn the_channel_name_drops_the_topic_suffix() {
        let i = info(r#"{"channel": "Kiri Hoshino - Topic", "uploader": "U"}"#);
        assert_eq!(i.channel_name().as_deref(), Some("Kiri Hoshino"));
        let i = info(r#"{"uploader": "Lu5ivo"}"#);
        assert_eq!(i.channel_name().as_deref(), Some("Lu5ivo"));
    }

    #[test]
    fn subtitle_names_come_from_each_format() {
        let i =
            info(r#"{"subtitles": {"en": [{"ext": "vtt", "name": "English"}, {"ext": "srt"}]}}"#);
        assert_eq!(i.subtitle_names("en"), vec!["English"]);
        assert_eq!(i.subtitle_names("ja"), Vec::<&str>::new());
    }

    #[test]
    fn a_file_fetched_by_its_address_is_named_by_it_not_its_mirror() {
        let embedded = info(
            r#"{"id": "a", "extractor_key": "Generic",
                "webpage_url": "https://mirror7.files.example/0/a.mp3",
                "muman_original_url": "https://files.example/download/a.mp3"}"#,
        );
        assert_eq!(
            embedded.page(),
            Some("https://files.example/download/a.mp3")
        );
        assert_eq!(
            embedded
                .key(&crate::sites::Sites::default())
                .unwrap()
                .to_string(),
            "files.example:a"
        );
        let site = info(
            r#"{"id": "a", "extractor_key": "Funkwhale",
                "webpage_url": "https://tunes.example/t/a",
                "original_url": "https://m.tunes.example/t/a"}"#,
        );
        assert_eq!(
            site.key(&crate::sites::Sites::default())
                .unwrap()
                .to_string(),
            "tunes.example:a"
        );
    }

    #[test]
    fn a_field_of_another_type_loses_that_field_alone() {
        let i = info(
            r#"{"id": "vid00000001", "title": "Lantern Weather", "track_number": "3",
                "disc_number": 2.0, "release_year": -1, "artists": ["Marlo Venn", null],
                "album": 7, "formats": [{"format_id": "251", "filesize": 1.5e6}, 3]}"#,
        );
        assert_eq!(i.id.as_deref(), Some("vid00000001"));
        assert_eq!(i.title.as_deref(), Some("Lantern Weather"));
        assert_eq!(i.track_number, Some(3));
        assert_eq!(i.disc_number, Some(2));
        assert_eq!(i.release_year, None);
        assert_eq!(i.artists, Some(vec!["Marlo Venn".to_string()]));
        assert_eq!(i.album, None);
        assert_eq!(i.formats.len(), 1);
        assert_eq!(i.formats[0].filesize, Some(1_500_000));
        let i = info(r#"{"title": "T", "track_number": "three", "filesize": 0.5}"#);
        assert_eq!((i.title.as_deref(), i.track_number), (Some("T"), None));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let i = info(r#"{"formats": [{"x": 1}], "title": "T", "release_year": 2024}"#);
        assert_eq!(i.title.as_deref(), Some("T"));
        assert_eq!(i.release_year, Some(2024));
    }
}
