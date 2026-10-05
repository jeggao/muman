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
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SubtitleFormat {
    #[serde(default)]
    pub name: Option<String>,
}

pub fn parse(json: &[u8]) -> Result<VideoInfo> {
    serde_json::from_slice(json).context("parsing yt-dlp's info JSON")
}

impl VideoInfo {
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
        assert!(i.subtitle_names("ja").is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let i = info(r#"{"formats": [{"x": 1}], "title": "T", "release_year": 2024}"#);
        assert_eq!(i.title.as_deref(), Some("T"));
        assert_eq!(i.release_year, Some(2024));
    }
}
