//! yt-dlp's info JSON, embedded in each file it fetched.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideoInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub extractor_key: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub artists: Option<Vec<String>>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub creators: Option<Vec<String>>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_artists: Option<Vec<String>>,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub genres: Option<Vec<String>>,
    #[serde(default)]
    pub genre: Option<String>,
    /// `YYYYMMDD`.
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub release_year: Option<u32>,
    /// `YYYYMMDD`.
    #[serde(default)]
    pub upload_date: Option<String>,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    /// Language code to the formats it was offered in; only `name` is read.
    #[serde(default)]
    pub subtitles: BTreeMap<String, Vec<SubtitleFormat>>,
    /// The same, for captions the site generated.
    #[serde(default)]
    pub automatic_captions: BTreeMap<String, Vec<SubtitleFormat>>,
    /// The format fetched: one ID, or a video's and an audio's joined by
    /// `+`.
    #[serde(default)]
    pub format_id: Option<String>,
    /// Every format the site offered.
    #[serde(default)]
    pub formats: Vec<Format>,
    /// The page the video was fetched from.
    #[serde(default)]
    pub webpage_url: Option<String>,
    /// The address yt-dlp was given, which an info JSON it embeds leaves
    /// out.
    #[serde(default)]
    pub original_url: Option<String>,
    /// That address, copied where the embedded info JSON keeps it.
    #[serde(default)]
    pub muman_original_url: Option<String>,
}

/// One format a site offered, as yt-dlp lists it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct Format {
    #[serde(default)]
    pub format_id: String,
    /// Exact, where the site says it; `filesize_approx` is not read.
    #[serde(default)]
    pub filesize: Option<u64>,
    #[serde(default)]
    pub acodec: Option<String>,
    #[serde(default)]
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
    #[serde(default)]
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
/// the rest of it changes on every fetch, as its time and cookies do.
#[must_use]
pub fn tags_digest(json: &[u8]) -> Option<String> {
    let all: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(json).ok()?;
    let read: BTreeMap<String, serde_json::Value> = all
        .into_iter()
        .filter(|(k, v)| TAG_FIELDS.contains(&k.as_str()) && !v.is_null())
        .collect();
    Some(crate::facts::digest(&serde_json::to_vec(&read).ok()?))
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
    fn unknown_fields_are_ignored() {
        let i = info(r#"{"formats": [{"x": 1}], "title": "T", "release_year": 2024}"#);
        assert_eq!(i.title.as_deref(), Some("T"));
        assert_eq!(i.release_year, Some(2024));
    }
}
