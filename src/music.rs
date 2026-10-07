//! A video's YouTube Music track: the same song as released, with its
//! album art and release tags, found by searching YouTube Music's songs.
//!
//! An upload often has a released counterpart: an auto-generated track
//! whose info carries the track name, every credited artist and the
//! album, and whose square album art the upload lacks. yt-dlp's
//! `music.youtube.com/search?q=…#songs` extractor is asked for the
//! upload's channel and title; its top songs are extracted whole, and
//! those named as the upload is, and close enough in length, are
//! candidates:
//!
//! - the track's name inside the upload's title, by letters and digits
//!   alone, case-folded, and without the featured artists a release
//!   credits in its name, which an upload credits in its own words and
//!   script, or not at all;
//! - an artist, or the release's " - Topic" channel, in the upload's
//!   channel or title, which covers a label channel's `Artist - Song`
//!   uploads.
//!
//! Their audio decides, in search order (`align`); only where it cannot
//! be had does a candidate of the same length stand. A playlist's flat
//! entry names no track, so one from an artist's " - Topic" channel,
//! where only releases are, counts as one.
//!
//! The reverse search, for a release's upload with subtitles, puts the
//! artist's own channel first and compares only videos with a person's
//! subtitles: a cover sung over the official instrumental may sound
//! close enough to pass.
//!
//! YouTube Music names every album's playlist `OLAK5uy_…`; any other
//! playlist is a list of separate songs. A track's number is its place in
//! the album's full listing, counting entries no longer available, since
//! a track fetched alone never says it.
//!
//! A track's own thumbnails are 16:9 frames with the art pillarboxed in
//! the middle. Only the `web_music` player client lists the square art,
//! on `googleusercontent.com`, ranked last and sized to 544 pixels, so
//! yt-dlp is asked to use that client too, and the `AlbumArt`
//! postprocessor appends the art at its uploaded size, as JPEG (`=s0-rj`),
//! as the last thumbnail, the one yt-dlp writes and embeds. Without it
//! the frame's bars are cropped off when the song is written (`quality`).
//!
//! Listing a URL takes about 2 s, the flat search about 1.6 s, extracting
//! its songs at once about 1.8 s more, and each audio download about 1 s.
//! More lookups at once than `parallel::LOOKUPS` trip YouTube's rate
//! limit sooner.

use std::collections::BTreeMap;
use std::ffi::OsString;

use serde::Deserialize;

use crate::clean;
use crate::sites::Sites;
use crate::source::SourceKey;

/// How far apart a video and its track may run, in seconds; an intro or
/// outro on the video makes the two different cuts.
const DURATION_SLACK: f64 = 3.0;

/// How far apart they may run when the audio is compared instead: the
/// longest intro or outro a music video adds that alignment searches.
const COMPARED_SLACK: f64 = 60.0;

/// How many of the search's songs are extracted whole to compare.
const CANDIDATES: &str = "1:3";

/// How many videos a search for a track's upload lists.
const UPLOAD_CANDIDATES: usize = 5;

/// A video or track as yt-dlp's JSON describes it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub artists: Option<Vec<String>>,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub uploader_id: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    /// Absent from a playlist's flat entries.
    #[serde(default)]
    pub subtitles: Option<BTreeMap<String, serde_json::Value>>,
    /// The extractor that read a whole video, as `Youtube`.
    #[serde(default)]
    pub extractor_key: Option<String>,
    /// The extractor a playlist's flat entry is for.
    #[serde(default)]
    pub ie_key: Option<String>,
    /// `playlist` for a flat entry that lists more.
    #[serde(rename = "_type", default)]
    pub kind: Option<String>,
}

impl Entry {
    /// A track is already the music release; nothing to switch to. A
    /// playlist's flat entry names no track, but only releases come from
    /// an artist's " - Topic" channel.
    #[must_use]
    pub fn is_track(&self) -> bool {
        self.track.as_deref().is_some_and(|t| !t.trim().is_empty())
            || self
                .channel
                .as_deref()
                .is_some_and(|c| c.ends_with(" - Topic"))
    }

    /// Whether yt-dlp's YouTube extractor read it, so YouTube Music may
    /// have it as a track. An entry that does not say is taken for one.
    #[must_use]
    pub fn is_youtube(&self) -> bool {
        self.extractor_key
            .as_deref()
            .or(self.ie_key.as_deref())
            .is_none_or(|k| k == "Youtube")
    }

    /// Whether it is one video or track a source key can name, as
    /// `sites` takes its ID. A channel's tab or playlist is none.
    #[must_use]
    pub fn is_video(&self, sites: &Sites) -> bool {
        if self.kind.as_deref() == Some("playlist") {
            return false;
        }
        match self.extractor_key.as_deref().or(self.ie_key.as_deref()) {
            None | Some("Youtube") => is_video_id(&self.id, sites),
            Some(other) => {
                !other.starts_with("Youtube") && sites.fetched(other, None, &self.id).is_some()
            }
        }
    }

    /// Whether a person wrote subtitles for it; `None` when the entry
    /// does not say, as a playlist's flat one does not.
    #[must_use]
    pub fn has_subtitles(&self) -> Option<bool> {
        self.subtitles
            .as_ref()
            .map(|s| s.keys().any(|lang| lang != "live_chat"))
    }

    /// The originals folder yt-dlp's template would pick for this video,
    /// so its track is kept beside the channel's other videos.
    #[must_use]
    pub fn folder(&self) -> String {
        let handle = self
            .uploader_id
            .as_deref()
            .map(|h| h.trim_start_matches('@'));
        [handle, self.channel_id.as_deref(), self.uploader.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("unknown")
            .replace('/', "⧸")
    }
}

/// What a URL names, listed flat in one JSON object: a video whole, a
/// playlist or channel with its videos as entries.
#[must_use]
pub fn list_command(url: &str) -> Vec<OsString> {
    [
        "yt-dlp",
        "--ignore-config",
        "--flat-playlist",
        "--dump-single-json",
        "--",
        url,
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// The one video a URL names, whole, though it names a playlist too.
#[must_use]
pub fn one_video_command(url: &str) -> Vec<OsString> {
    [
        "yt-dlp",
        "--ignore-config",
        "--no-playlist",
        "--dump-json",
        "--",
        url,
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// What [`list_command`] found a URL to name.
#[derive(Debug)]
pub enum Listing {
    Video(Box<Entry>),
    /// A playlist, album or channel: its own address, which names
    /// nothing else, and the videos among its entries.
    Playlist {
        url: Option<String>,
        title: Option<String>,
        videos: Vec<Entry>,
        /// A YouTube Music album, with each video's place on it.
        album: Option<AlbumListing>,
    },
    /// A Mix, the radio YouTube plays on from a video without end. yt-dlp
    /// keeps it on the watch page, where a playlist moves to its own.
    Mix {
        title: Option<String>,
    },
}

/// An album's playlist ID, how many entries it lists, and each video's
/// place among them, counting entries no longer available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumListing {
    pub id: String,
    pub tracks: u32,
    pub places: BTreeMap<String, u32>,
}

/// YouTube Music names every album's playlist with this prefix; any
/// other playlist is a list of separate songs.
const ALBUM_PREFIX: &str = "OLAK5uy_";

#[derive(Deserialize)]
struct Listed {
    #[serde(rename = "_type", default)]
    kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    webpage_url: Option<String>,
    #[serde(default)]
    webpage_url_basename: Option<String>,
    #[serde(default)]
    entries: Vec<serde_json::Value>,
}

/// YouTube's address of a video.
#[must_use]
pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// Whether `id` is one of a YouTube video, as `sites` takes it.
fn is_video_id(id: &str, sites: &Sites) -> bool {
    !id.contains('/') && sites.accepts(&SourceKey::youtube(id))
}

/// Read a [`list_command`] run; `None` when it names nothing.
#[must_use]
pub fn listing(json: &[u8], sites: &Sites) -> Option<Listing> {
    let value: serde_json::Value = serde_json::from_slice(json).ok()?;
    let listed = Listed::deserialize(&value).ok()?;
    if listed.kind.as_deref() != Some("playlist") {
        let video = Entry::deserialize(&value).ok()?;
        return video
            .is_video(sites)
            .then(|| Listing::Video(Box::new(video)));
    }
    if listed.webpage_url_basename.as_deref() == Some("watch") {
        return Some(Listing::Mix {
            title: listed.title,
        });
    }
    // A channel's entries are its tabs, playlists themselves, which the
    // download expands; only videos are looked up.
    let all: Vec<Option<Entry>> = listed
        .entries
        .iter()
        .map(|e| Entry::deserialize(e).ok().filter(|e| e.is_video(sites)))
        .collect();
    let album = listed
        .id
        .filter(|id| id.starts_with(ALBUM_PREFIX))
        .map(|id| AlbumListing {
            id,
            tracks: u32::try_from(all.len()).unwrap_or(u32::MAX),
            places: all
                .iter()
                .zip(1..)
                .filter_map(|(e, n)| Some((e.as_ref()?.id.clone(), n)))
                .collect(),
        });
    Some(Listing::Playlist {
        url: listed.webpage_url,
        title: listed.title,
        videos: all.into_iter().flatten().collect(),
        album,
    })
}

/// YouTube Music's top songs for the video's channel and title, listed
/// flat: their IDs, to extract each whole in parallel with [`video_command`].
#[must_use]
pub fn search_command(video: &Entry) -> Option<Vec<OsString>> {
    let title = video.title.as_deref()?.trim();
    if title.is_empty() {
        return None;
    }
    let query = match video.uploader.as_deref().or(video.channel.as_deref()) {
        Some(by) => format!("{by} {title}"),
        None => title.to_string(),
    };
    let url = format!(
        "https://music.youtube.com/search?q={}#songs",
        percent_encode(&query)
    );
    Some(
        [
            "yt-dlp",
            "--ignore-config",
            "--flat-playlist",
            "--playlist-items",
            CANDIDATES,
            "--print",
            "id",
            "--",
            &url,
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
    )
}

/// One JSON object per line; a line that does not parse, or names no
/// video, is skipped.
#[must_use]
pub fn entries(jsonl: &[u8], sites: &Sites) -> Vec<Entry> {
    String::from_utf8_lossy(jsonl)
        .lines()
        .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
        .filter(|e| e.is_video(sites))
        .collect()
}

/// The video IDs a `--print id` run wrote, one per line.
#[must_use]
pub fn ids(printed: &[u8], sites: &Sites) -> Vec<String> {
    String::from_utf8_lossy(printed)
        .lines()
        .map(str::trim)
        .filter(|id| is_video_id(id, sites))
        .map(str::to_string)
        .collect()
}

/// The command extracting one video whole.
#[must_use]
pub fn video_command(id: &str) -> Vec<OsString> {
    one_video_command(&watch_url(id))
}

/// Whether a track runs exactly as long as the upload, to the second
/// YouTube reports: then the two are one recording, and the upload's
/// subtitles keep their timing on the track. Measured on four pairs,
/// equal lengths came within 20 ms of each other.
#[must_use]
pub fn same_length(a: &Entry, b: &Entry) -> bool {
    matches!((a.duration, b.duration), (Some(x), Some(y)) if (x.round() - y.round()).abs() < 0.5)
}

/// The first candidate that is the video's song by its metadata alone:
/// a track of the same length, whose name the video's title carries and
/// whose artist is the video's channel or named in its title. The
/// fallback when the audio cannot be compared.
#[must_use]
pub fn pick<'a>(video: &Entry, candidates: &'a [Entry]) -> Option<&'a Entry> {
    candidates
        .iter()
        .find(|c| is_named(video, c) && within(video, c, DURATION_SLACK))
}

/// The candidates that may be the video's song, in search order, for
/// their audio to decide: named as [`pick`] asks, and close enough in
/// length that the alignment's search reaches the difference.
#[must_use]
pub fn named<'a>(video: &Entry, candidates: &'a [Entry]) -> Vec<&'a Entry> {
    candidates
        .iter()
        .filter(|c| is_named(video, c) && within(video, c, COMPARED_SLACK))
        .collect()
}

fn within(a: &Entry, b: &Entry, slack: f64) -> bool {
    matches!((a.duration, b.duration), (Some(x), Some(y)) if (x - y).abs() <= slack)
}
/// YouTube's top five videos for a track's artist and name, listed flat
/// with their titles, channels and lengths: where its music video, and
/// with it the subtitles a track never has, may be.
#[must_use]
pub fn video_search_command(track: &Entry) -> Option<Vec<OsString>> {
    let name = track.track.as_deref()?.trim();
    if name.is_empty() {
        return None;
    }
    let artist = track
        .artists
        .iter()
        .flatten()
        .map(String::as_str)
        .chain(
            track
                .channel
                .as_deref()
                .map(|c| c.trim_end_matches(" - Topic")),
        )
        .map(str::trim)
        .find(|a| !a.is_empty());
    let query = match artist {
        Some(by) => format!("ytsearch{UPLOAD_CANDIDATES}:{by} {name}"),
        None => format!("ytsearch{UPLOAD_CANDIDATES}:{name}"),
    };
    Some(
        [
            "yt-dlp",
            "--ignore-config",
            "--flat-playlist",
            "--dump-json",
            "--",
            &query,
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
    )
}

/// The videos found that may be uploads of `track`, named as
/// [`named`] asks the other way round, the artist's own channel first:
/// a cover sung over the same instrumental is close enough in sound to
/// be taken for it.
#[must_use]
pub fn uploads_of<'a>(track: &Entry, found: &'a [Entry]) -> Vec<&'a Entry> {
    let artists: Vec<String> = track
        .artists
        .iter()
        .flatten()
        .map(|a| normalize(a))
        .filter(|a| !a.is_empty())
        .collect();
    let official = |v: &Entry| {
        let channel = normalize(
            v.channel
                .as_deref()
                .or(v.uploader.as_deref())
                .unwrap_or_default(),
        );
        !channel.is_empty() && artists.iter().any(|a| channel.contains(a.as_str()))
    };
    let mut named: Vec<&Entry> = found
        .iter()
        .filter(|v| is_named(v, track) && within(v, track, COMPARED_SLACK))
        .collect();
    named.sort_by_key(|v| !official(v));
    named
}

fn is_named(video: &Entry, track: &Entry) -> bool {
    if track.id == video.id || !track.is_track() {
        return false;
    }
    let title = normalize(video.title.as_deref().unwrap_or_default());
    let name = normalize(clean::without_credits(
        track.track.as_deref().unwrap_or_default(),
    ));
    if name.is_empty() || !title.contains(&name) {
        return false;
    }
    let channels: Vec<String> = [video.uploader.as_deref(), video.channel.as_deref()]
        .into_iter()
        .flatten()
        .map(normalize)
        .filter(|c| !c.is_empty())
        .collect();
    let topic = track
        .channel
        .as_deref()
        .map(|c| c.trim_end_matches(" - Topic"));
    track
        .artists
        .iter()
        .flatten()
        .map(String::as_str)
        .chain(topic)
        .map(normalize)
        .filter(|a| !a.is_empty())
        .any(|a| title.contains(&a) || channels.iter().any(|c| c.contains(&a) || a.contains(c)))
}

/// Letters and digits only, lower-cased and in NFKC, so punctuation,
/// spacing, case and full-width forms never decide a match.
pub(crate) fn normalize(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfkc()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// How closely one name holds another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Exactness {
    /// One holds the other, as `Rain` and `Rain (Live)` do.
    Holds,
    Exact,
}

/// Whether two names are one, by their letters and digits without
/// featured artists, or one holds the other; `None` when neither.
pub(crate) fn names_match(a: &str, b: &str) -> Option<Exactness> {
    let (a, b) = (
        normalize(clean::without_credits(a)),
        normalize(clean::without_credits(b)),
    );
    if a.is_empty() || b.is_empty() {
        None
    } else if a == b {
        Some(Exactness::Exact)
    } else {
        (a.contains(&b) || b.contains(&a)).then_some(Exactness::Holds)
    }
}

/// Every byte but the characters RFC 3986 leaves unreserved.
const RESERVED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) fn percent_encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, RESERVED).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_percent_encoded_but_for_unreserved_characters() {
        assert_eq!(
            percent_encode("Marlo Venn & Co. ~ 雨/a_b-c?"),
            "Marlo%20Venn%20%26%20Co.%20~%20%E9%9B%A8%2Fa_b-c%3F"
        );
    }

    #[test]
    fn names_match_exactly_before_one_holds_the_other() {
        assert_eq!(
            names_match("Lantern Weather", "lantern weather!"),
            Some(Exactness::Exact)
        );
        assert_eq!(
            names_match("Ｌａｎｔｅｒｎ　Ｗｅａｔｈｅｒ", "Lantern Weather"),
            Some(Exactness::Exact),
            "full-width forms are the same letters"
        );
        assert_eq!(
            names_match("Lantern Weather (Live)", "Lantern Weather"),
            Some(Exactness::Holds)
        );
        assert_eq!(names_match("Second Tune", "Lantern Weather"), None);
        assert_eq!(names_match("", "Lantern Weather"), None);
    }

    fn video() -> Entry {
        Entry {
            id: "vid00000002".into(),
            title: Some("ガラス果樹園 / 星屑ラジオSV".into()),
            uploader: Some("Marlo Venn".into()),
            uploader_id: Some("@marlo_ven".into()),
            channel: Some("Marlo Venn".into()),
            duration: Some(143.0),
            ..Entry::default()
        }
    }

    fn track(id: &str, name: &str, artists: &[&str], duration: f64) -> Entry {
        Entry {
            id: id.into(),
            title: Some(name.into()),
            track: Some(name.into()),
            artists: Some(artists.iter().map(|a| (*a).to_string()).collect()),
            channel: Some(format!("{} - Topic", artists[0])),
            duration: Some(duration),
            ..Entry::default()
        }
    }

    #[test]
    fn the_released_track_of_an_upload_is_picked() {
        let candidates = [
            track("vid00000003", "Glimmerquay", &["星屑ラジオ"], 189.0),
            track("vid00000001", "ガラス果樹園", &["Marlo Venn"], 143.0),
        ];
        assert_eq!(
            pick(&video(), &candidates).map(|t| t.id.as_str()),
            Some("vid00000001")
        );
    }

    #[test]
    fn a_label_upload_matches_by_the_artist_in_its_title() {
        let v = Entry {
            title: Some("Artist - Song (Official Video)".into()),
            uploader: Some("Label Records".into()),
            ..video()
        };
        let t = track("aaaaaaaaaaa", "Song", &["Artist"], 144.5);
        assert!(pick(&v, std::slice::from_ref(&t)).is_some());
    }

    #[test]
    fn another_cut_is_not_the_same_song() {
        let t = track("vid00000001", "ガラス果樹園", &["Marlo Venn"], 150.0);
        assert!(pick(&video(), &[t]).is_none());
    }

    #[test]
    fn another_artist_is_not_the_same_song() {
        let t = track("vid00000001", "ガラス果樹園", &["Someone Else"], 143.0);
        assert!(pick(&video(), &[t]).is_none());
    }

    #[test]
    fn a_longer_cut_is_left_to_the_audio() {
        let t = track("vid00000001", "ガラス果樹園", &["Marlo Venn"], 175.0);
        let far = track("vid00000003", "ガラス果樹園", &["Marlo Venn"], 240.0);
        let candidates = [t, far];
        assert!(pick(&video(), &candidates).is_none());
        let named = named(&video(), &candidates);
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].id, "vid00000001");
    }

    #[test]
    fn a_track_s_uploads_put_the_artist_s_channel_first() {
        let t = Entry {
            artists: Some(vec!["MOTHlamp".into(), "Kiri Hoshino".into()]),
            ..track("vid00000004", "PAPER ORRERY", &["MOTHlamp"], 178.0)
        };
        let v = |id: &str, channel: &str, title: &str, duration: f64| Entry {
            id: id.into(),
            title: Some(title.into()),
            channel: Some(channel.into()),
            duration: Some(duration),
            ..Entry::default()
        };
        let found = [
            v(
                "vid00000005",
                "Nimbleweft",
                "PAPER ORRERY| Mothlamp - COVER",
                182.0,
            ),
            v(
                "vid00000006",
                "MOTHlamp",
                "【Official MV】 PAPER ORRERY ft. 星野キリ",
                182.0,
            ),
            v("aaaaaaaaaaa", "Other", "Something else", 182.0),
            v("bbbbbbbbbbb", "MOTHlamp", "PAPER ORRERY (Extended)", 400.0),
        ];
        let ids: Vec<&str> = uploads_of(&t, &found)
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, vec!["vid00000006", "vid00000005"]);
        let cmd = video_search_command(&t).unwrap();
        assert_eq!(cmd.last().unwrap(), "ytsearch5:MOTHlamp PAPER ORRERY");
    }

    #[test]
    fn a_featuring_credit_is_no_part_of_the_name() {
        assert_eq!(
            clean::without_credits("ペリペロ (feat. Kiri Hoshino)"),
            "ペリペロ"
        );
        assert_eq!(clean::without_credits("Song ft. B"), "Song");
        assert_eq!(clean::without_credits("Feat. (feat. X)"), "Feat.");
        assert_eq!(clean::without_credits("(feat. X)"), "(feat. X)");
        let t = Entry {
            artists: Some(vec!["WhateverBloomsBright".into(), "Kiri Hoshino".into()]),
            ..track(
                "vid00000007",
                "ペリペロ (feat. Kiri Hoshino)",
                &["WhateverBloomsBright"],
                147.0,
            )
        };
        let mv = Entry {
            id: "vid00000008".into(),
            title: Some("[MV] WBB - 'ペリペロ' (PERIPERO) feat. 星野キリ".into()),
            channel: Some("WhateverBloomsBright and PINWHEELO".into()),
            duration: Some(157.0),
            ..Entry::default()
        };
        assert_eq!(uploads_of(&t, std::slice::from_ref(&mv)).len(), 1);
    }

    #[test]
    fn a_video_without_a_length_is_never_switched() {
        let v = Entry {
            duration: None,
            ..video()
        };
        let t = track("vid00000001", "ガラス果樹園", &["Marlo Venn"], 143.0);
        assert!(pick(&v, &[t]).is_none());
    }

    #[test]
    fn a_track_is_not_its_own_match() {
        let t = track("vid00000002", "ガラス果樹園", &["Marlo Venn"], 143.0);
        assert!(pick(&video(), &[t]).is_none());
    }

    #[test]
    fn the_search_names_channel_and_title_encoded() {
        let cmd = search_command(&video()).unwrap();
        let url = cmd.last().unwrap().to_string_lossy();
        assert!(url.starts_with("https://music.youtube.com/search?q=Marlo%20Venn%20%E3%82%AC"));
        assert!(url.ends_with("#songs"));
    }

    #[test]
    fn subtitles_are_known_only_when_listed() {
        assert_eq!(video().has_subtitles(), None);
        let e = |json: &str| entries(json.as_bytes(), &Sites::default()).remove(0);
        let live = e(r#"{"id": "vid00000002", "subtitles": {"live_chat": []}}"#);
        assert_eq!(live.has_subtitles(), Some(false));
        let en = e(r#"{"id": "vid00000002", "subtitles": {"en": [{"name": "English"}]}}"#);
        assert_eq!(en.has_subtitles(), Some(true));
    }

    #[test]
    fn lengths_match_to_the_reported_second() {
        let t = track("vid00000001", "ガラス果樹園", &["Marlo Venn"], 143.0);
        assert!(same_length(&video(), &t));
        let longer = track("vid00000001", "ガラス果樹園", &["Marlo Venn"], 144.0);
        assert!(!same_length(&video(), &longer));
    }

    #[test]
    fn printed_ids_skip_anything_else() {
        assert_eq!(
            ids(b"vid00000001\nNA\n\nvid00000003\n", &Sites::default()),
            vec!["vid00000001", "vid00000003"]
        );
    }

    #[test]
    fn entries_skip_channels_tabs_and_garbage() {
        let jsonl = br#"{"id": "vid00000002", "title": "T", "duration": 143}
not json
{"id": "UC0000000000000000000001", "title": "Videos"}
"#;
        let e = entries(jsonl, &Sites::default());
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].duration, Some(143.0));
    }

    #[test]
    fn a_playlist_lists_its_videos_at_its_own_address() {
        let json = br#"{"_type": "playlist", "title": "Songs",
            "webpage_url": "https://www.youtube.com/playlist?list=PLx",
            "webpage_url_basename": "playlist",
            "entries": [{"_type": "url", "id": "vid00000009", "duration": 171},
                        {"_type": "url", "id": "vid00000010", "duration": 183}]}"#;
        let Some(Listing::Playlist {
            url,
            title,
            videos,
            album,
        }) = listing(json, &Sites::default())
        else {
            panic!("not a playlist");
        };
        assert_eq!(album, None, "a playlist of the user's is no album");
        assert_eq!(
            url.as_deref(),
            Some("https://www.youtube.com/playlist?list=PLx")
        );
        assert_eq!(title.as_deref(), Some("Songs"));
        let ids: Vec<_> = videos.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["vid00000009", "vid00000010"]);
    }

    #[test]
    fn an_album_numbers_its_tracks_counting_the_unavailable() {
        let json = br#"{"_type": "playlist", "id": "OLAK5uy_abc", "title": "Album - Record",
            "webpage_url_basename": "playlist",
            "entries": [{"id": "vid00000009"}, {"id": "[Private video]"}, {"id": "vid00000010"}]}"#;
        let Some(Listing::Playlist {
            album: Some(album), ..
        }) = listing(json, &Sites::default())
        else {
            panic!("not an album");
        };
        assert_eq!(album.id, "OLAK5uy_abc");
        assert_eq!(album.tracks, 3);
        assert_eq!(album.places["vid00000010"], 3);
    }

    #[test]
    fn an_item_of_another_site_lists_its_files() {
        let json = br#"{"_type": "playlist", "id": "item-1", "title": "Item",
            "webpage_url": "https://archive.org/details/item-1",
            "entries": [{"id": "item-1/Part_1.mp3", "extractor_key": "ArchiveOrg"},
                        {"id": "item-1/Part 2.mp3", "extractor_key": "ArchiveOrg"},
                        {"_type": "playlist", "id": "more", "ie_key": "ArchiveOrg"},
                        {"_type": "url", "id": "UC0000000000000000000002", "ie_key": "YoutubeTab"}]}"#;
        let Some(Listing::Playlist { videos, .. }) = listing(json, &Sites::default()) else {
            panic!("not a playlist");
        };
        let ids: Vec<_> = videos.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["item-1/Part_1.mp3"], "a space no key takes");
        let one = br#"{"id": "item-2", "extractor_key": "ArchiveOrg"}"#;
        assert!(
            matches!(listing(one, &Sites::default()), Some(Listing::Video(v)) if v.id == "item-2")
        );
    }

    #[test]
    fn a_channel_lists_no_videos_of_its_own() {
        let json = br#"{"_type": "playlist", "webpage_url_basename": "@pellucidfox",
            "entries": [{"_type": "playlist", "id": "UC0000000000000000000002"}]}"#;
        assert!(
            matches!(listing(json, &Sites::default()), Some(Listing::Playlist { videos, .. }) if videos.is_empty())
        );
    }

    #[test]
    fn a_playlist_left_on_the_watch_page_is_a_mix() {
        let json = br#"{"_type": "playlist", "id": "RDvid00000009", "title": "Mix - M",
            "webpage_url_basename": "watch", "entries": [{"id": "vid00000009"}]}"#;
        assert!(matches!(
            listing(json, &Sites::default()),
            Some(Listing::Mix { .. })
        ));
    }

    #[test]
    fn a_video_is_listed_whole() {
        let json = br#"{"_type": "video", "id": "vid00000009", "duration": 171.0}"#;
        assert!(
            matches!(listing(json, &Sites::default()), Some(Listing::Video(v)) if v.duration == Some(171.0))
        );
        assert!(listing(b"not json", &Sites::default()).is_none());
    }

    #[test]
    fn only_a_youtube_entry_is_looked_up() {
        let flat: Entry = serde_json::from_str(r#"{"id": "x", "ie_key": "Youtube"}"#).unwrap();
        let other: Entry =
            serde_json::from_str(r#"{"id": "1", "extractor_key": "Soundcloud"}"#).unwrap();
        assert!(flat.is_youtube() && video().is_youtube());
        assert!(!other.is_youtube());
    }

    #[test]
    fn a_topic_channel_entry_is_a_track() {
        let flat = Entry {
            channel: Some("Pellucid Fox - Topic".into()),
            ..Entry::default()
        };
        assert!(flat.is_track());
        assert!(!video().is_track());
    }

    #[test]
    fn the_folder_is_the_handle_without_its_at() {
        assert_eq!(video().folder(), "marlo_ven");
        let none = Entry {
            uploader_id: None,
            channel_id: Some("UCx".into()),
            ..video()
        };
        assert_eq!(none.folder(), "UCx");
    }
}
