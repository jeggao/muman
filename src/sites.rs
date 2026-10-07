//! The sites yt-dlp fetches from, as the song list's `[sites]` describes
//! them: which of yt-dlp's extractors each names its sources by, the
//! kinds of source it holds, the shape of each kind's IDs and the address
//! one is fetched again from. Adding a site is a table in
//! `manifest/new.toml`, the defaults every song list starts from; no code
//! names a site but the YouTube Music lookups, which are YouTube's own.
//!
//! A source fetched by an extractor a site lists is named by that site,
//! whichever address it was found at, its ID led by the extractor's
//! prefix: YouTube's tab extractor fetches playlists, so its IDs read
//! `playlist/<id>`. A source of any other extractor is named by the
//! domain of its page ([`crate::source::page`]), or by the extractor
//! where it has no page. A kind is told by the longest prefix its key's
//! ID begins with; a key of a site that lists kinds, whose ID none of
//! them takes, is refused where the song list is read. A kind with a
//! `fetch` address is fetched again from it, `{id}` its ID without the
//! prefix; one without is fetched again from the page its song records.
//! A key of a kind without a prefix is archived for yt-dlp's
//! `--download-archive` by its site's extractor of no prefix.
//!
//! Extractors listed for a site name what they fetch by that site from
//! then on, where what they fetched before is named by each page's: list
//! one only where its pages are the site's own, or a source fetched again
//! is listed twice. The table is part of the song list, so one list names
//! its sources alike on every machine; the list's tables lie over the
//! built-in ones a site at a time, so a list naming one site keeps the
//! rest.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;

use crate::source::{SourceKey, domain_of};

/// One site as the song list writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    /// yt-dlp's extractors, lower-cased, each with the prefix its IDs
    /// take in a key.
    #[serde(default)]
    extractors: BTreeMap<String, String>,
    #[serde(default)]
    kinds: Vec<WrittenKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenKind {
    #[serde(default)]
    prefix: String,
    /// A regular expression the ID after the prefix matches.
    #[serde(default)]
    id: Option<String>,
    /// The address it is fetched again from, `{id}` its ID.
    #[serde(default)]
    fetch: Option<String>,
}

/// One kind of source a site holds.
#[derive(Debug, Clone)]
struct Kind {
    prefix: String,
    id: Option<Regex>,
    fetch: Option<String>,
}

#[derive(Debug, Clone)]
struct Site {
    extractors: BTreeMap<String, String>,
    kinds: Vec<Kind>,
}

/// Every site the song list describes, by domain.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "BTreeMap<String, Written>")]
pub struct Sites {
    written: BTreeMap<String, Written>,
    sites: BTreeMap<String, Site>,
}

impl PartialEq for Sites {
    fn eq(&self, other: &Self) -> bool {
        self.written == other.written
    }
}

impl TryFrom<BTreeMap<String, Written>> for Sites {
    type Error = anyhow::Error;

    /// The song list's tables over the built-in ones, a site at a time.
    fn try_from(own: BTreeMap<String, Written>) -> Result<Self> {
        let mut written = BUILT_IN.written.clone();
        written.extend(own);
        Self::build(written)
    }
}

impl Sites {
    fn build(written: BTreeMap<String, Written>) -> Result<Self> {
        let mut sites = BTreeMap::new();
        for (domain, w) in &written {
            if domain_of(domain).as_deref() != Some(domain.as_str()) {
                bail!("[sites] `{domain}` is no domain, lower-cased and without `www.`");
            }
            let mut kinds = Vec::new();
            for k in &w.kinds {
                if kinds.iter().any(|seen: &Kind| seen.prefix == k.prefix) {
                    bail!("[sites.\"{domain}\"] lists the prefix {:?} twice", k.prefix);
                }
                if k.fetch.as_deref().is_some_and(|f| !f.contains("{id}")) {
                    bail!("[sites.\"{domain}\"] a `fetch` address must hold {{id}}");
                }
                let id =
                    k.id.as_deref()
                        .map(Regex::new)
                        .transpose()
                        .with_context(|| format!("[sites.\"{domain}\"] `id`"))?;
                kinds.push(Kind {
                    prefix: k.prefix.clone(),
                    id,
                    fetch: k.fetch.clone(),
                });
            }
            for (extractor, prefix) in &w.extractors {
                if *extractor != extractor.to_ascii_lowercase() {
                    bail!("[sites.\"{domain}\"] extractor `{extractor}` must be lower-case");
                }
                if !kinds.is_empty() && !kinds.iter().any(|k| k.prefix == *prefix) {
                    bail!(
                        "[sites.\"{domain}\"] extractor `{extractor}` takes a prefix no kind has"
                    );
                }
            }
            sites.insert(
                domain.clone(),
                Site {
                    extractors: w.extractors.clone(),
                    kinds,
                },
            );
        }
        let mut owners = BTreeMap::new();
        for (domain, site) in &sites {
            for extractor in site.extractors.keys() {
                if let Some(other) = owners.insert(extractor.clone(), domain.clone()) {
                    bail!("[sites] extractor `{extractor}` is listed by `{other}` and `{domain}`");
                }
            }
        }
        Ok(Self { written, sites })
    }
}

/// The table `manifest/new.toml` writes, read once; a test reads it too,
/// so a table that does not read fails the build's tests.
static BUILT_IN: LazyLock<Sites> = LazyLock::new(|| {
    let doc: toml::Table = toml::from_str(crate::manifest::NEW).unwrap_or_default();
    doc.get("sites")
        .cloned()
        .and_then(|t| t.try_into::<BTreeMap<String, Written>>().ok())
        .and_then(|w| Sites::build(w).ok())
        .unwrap_or_else(|| Sites {
            written: BTreeMap::new(),
            sites: BTreeMap::new(),
        })
});

impl Default for Sites {
    fn default() -> Self {
        BUILT_IN.clone()
    }
}

impl Sites {
    /// The site and the rest of its ID for a key, with the kind it is.
    fn kind_of<'s>(&'s self, key: &SourceKey) -> Option<(&'s Site, Option<&'s Kind>, String)> {
        let SourceKey::Remote { site, id } = key else {
            return None;
        };
        let site = self.sites.get(site)?;
        let kind = site
            .kinds
            .iter()
            .filter(|k| id.starts_with(&k.prefix))
            .max_by_key(|k| k.prefix.len());
        let rest = kind.map_or(id.as_str(), |k| &id[k.prefix.len()..]);
        Some((site, kind, rest.to_string()))
    }

    /// Whether the site a key names takes its ID: any ID of a site the
    /// table does not describe, or of one listing no kinds.
    #[must_use]
    pub fn accepts(&self, key: &SourceKey) -> bool {
        match self.kind_of(key) {
            None => true,
            Some((site, _, _)) if site.kinds.is_empty() => true,
            Some((_, None, _)) => false,
            Some((_, Some(kind), rest)) => kind.id.as_ref().is_none_or(|re| re.is_match(&rest)),
        }
    }

    /// What yt-dlp fetched, by the extractor key it printed, the page
    /// [`crate::source::page`] picks and the ID: named by the site that
    /// lists the extractor, else the page's domain, else the extractor.
    #[must_use]
    pub fn fetched(&self, extractor: &str, page: Option<&str>, id: &str) -> Option<SourceKey> {
        let extractor = extractor.to_ascii_lowercase();
        let listed = self.sites.iter().find_map(|(domain, site)| {
            site.extractors
                .get(&extractor)
                .map(|prefix| (domain.clone(), prefix.clone()))
        });
        let (site, prefix) = listed
            .or_else(|| page.and_then(domain_of).map(|d| (d, String::new())))
            .unwrap_or((extractor, String::new()));
        let key = SourceKey::parse(&format!("{site}:{prefix}{id}")).ok()?;
        self.accepts(&key).then_some(key)
    }

    /// The address a key's kind is fetched again from, if it has one.
    #[must_use]
    pub fn fetch_url(&self, key: &SourceKey) -> Option<String> {
        let (_, kind, rest) = self.kind_of(key)?;
        Some(kind?.fetch.as_deref()?.replace("{id}", &rest))
    }

    /// Every domain the table describes and every extractor it lists.
    pub fn schemes(&self) -> impl Iterator<Item = &str> {
        self.sites.iter().flat_map(|(domain, site)| {
            std::iter::once(domain.as_str()).chain(site.extractors.keys().map(String::as_str))
        })
    }

    /// yt-dlp's `--download-archive` line for a key, `<extractor> <id>`:
    /// a key of a kind without a prefix, by its site's extractor of no
    /// prefix, or one an earlier muman named by its extractor, as is.
    #[must_use]
    pub fn archive_line(&self, key: &SourceKey) -> Option<String> {
        let SourceKey::Remote { site, id } = key else {
            return None;
        };
        key.site()?;
        if !site.contains('.') {
            return Some(format!("{site} {id}\n"));
        }
        let (described, kind, _) = self.kind_of(key)?;
        if kind.is_some_and(|k| !k.prefix.is_empty()) {
            return None;
        }
        let extractor = described.extractors.iter().find(|(_, p)| p.is_empty())?.0;
        Some(format!("{extractor} {id}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> SourceKey {
        SourceKey::parse(text).unwrap()
    }

    #[test]
    fn the_built_in_table_reads() {
        let doc: toml::Table = toml::from_str(crate::manifest::NEW).unwrap();
        let written: BTreeMap<String, Written> = doc["sites"].clone().try_into().unwrap();
        Sites::build(written).unwrap();
        let sites = Sites::default();
        assert!(sites.sites.contains_key(crate::source::YOUTUBE));
        assert!(sites.sites.contains_key("archive.org"));
    }

    #[test]
    fn a_youtube_key_needs_a_video_or_playlist_id() {
        let sites = Sites::default();
        assert!(sites.accepts(&key("youtube.com:-dashid_123")));
        assert!(sites.accepts(&key("youtube.com:playlist/OLAK5uy_x")));
        assert!(!sites.accepts(&key("youtube.com:short")));
        assert!(!sites.accepts(&key("youtube.com:playlist/a.b")));
        assert!(sites.accepts(&key("files.example:anything")));
    }

    #[test]
    fn a_fetched_source_is_named_by_one_site_per_extractor_else_its_page() {
        let sites = Sites::default();
        let at = |extractor, page, id| sites.fetched(extractor, page, id).unwrap().to_string();
        assert_eq!(
            at(
                "Youtube",
                Some("https://music.youtube.com/watch?v=vid00000001"),
                "vid00000001"
            ),
            "youtube.com:vid00000001"
        );
        assert_eq!(
            at("YoutubeTab", None, "OLAK5uy_x"),
            "youtube.com:playlist/OLAK5uy_x"
        );
        assert_eq!(
            at("Bandcamp", Some("https://marlo.bandcamp.com/track/a"), "1"),
            "bandcamp.com:1"
        );
        assert_eq!(
            at("Generic", Some("https://www.files.example/a.mp3"), "a"),
            "files.example:a"
        );
        assert_eq!(at("Funkwhale", None, "b"), "funkwhale:b");
        assert!(sites.fetched("Youtube", None, "short").is_none());
    }

    #[test]
    fn a_kind_with_an_address_is_fetched_again_from_it() {
        let sites = Sites::default();
        let url = |k: &str| sites.fetch_url(&key(k));
        assert_eq!(
            url("youtube.com:vid00000001").as_deref(),
            Some("https://www.youtube.com/watch?v=vid00000001")
        );
        assert_eq!(
            url("youtube.com:playlist/OLAK5uy_x").as_deref(),
            Some("https://www.youtube.com/playlist?list=OLAK5uy_x")
        );
        assert_eq!(
            url("archive.org:item-1/Part_1.mp3").as_deref(),
            Some("https://archive.org/details/item-1/Part_1.mp3")
        );
        assert_eq!(url("soundcloud.com:1"), None);
        assert_eq!(url("files.example:a"), None);
    }

    #[test]
    fn yt_dlp_archives_a_source_by_its_extractor_where_known() {
        let sites = Sites::default();
        let line = |k: &str| sites.archive_line(&key(k));
        assert_eq!(
            line("youtube.com:vid00000001").as_deref(),
            Some("youtube vid00000001\n")
        );
        assert_eq!(line("soundcloud.com:1").as_deref(), Some("soundcloud 1\n"));
        assert_eq!(line("funkwhale:b").as_deref(), Some("funkwhale b\n"));
        assert_eq!(line("files.example:a"), None);
        assert_eq!(line("youtube.com:playlist/OLAK5uy_x"), None);
        assert_eq!(line("lrclib:7"), None);
    }

    #[test]
    fn a_site_is_added_as_a_table() {
        let written: BTreeMap<String, Written> = toml::from_str(
            r#"
            ["tunes.example"]
            extractors = { tunes = "", tunesset = "set/" }
            kinds = [
                { prefix = "", id = "^[0-9]+$", fetch = "https://tunes.example/t/{id}" },
                { prefix = "set/", fetch = "https://tunes.example/s/{id}" },
            ]
            "#,
        )
        .unwrap();
        let sites = Sites::try_from(written).unwrap();
        assert!(sites.fetch_url(&key("youtube.com:vid00000001")).is_some());
        let k = sites.fetched("Tunes", None, "42").unwrap();
        assert_eq!(k.to_string(), "tunes.example:42");
        assert_eq!(
            sites.fetch_url(&k).as_deref(),
            Some("https://tunes.example/t/42")
        );
        assert!(sites.fetched("Tunes", None, "x1").is_none());
        let set = sites.fetched("TunesSet", None, "a").unwrap();
        assert_eq!(set.to_string(), "tunes.example:set/a");
        assert_eq!(sites.archive_line(&k).as_deref(), Some("tunes 42\n"));
    }

    #[test]
    fn a_table_that_contradicts_itself_is_refused() {
        for bad in [
            r#"["Tunes.example"]"#,
            "[\"tunes.example\"]\nkinds = [{ prefix = \"\" }, { prefix = \"\" }]",
            "[\"tunes.example\"]\nkinds = [{ fetch = \"https://tunes.example/\" }]",
            "[\"tunes.example\"]\nkinds = [{ id = \"(\" }]",
            "[\"tunes.example\"]\nextractors = { Tunes = \"\" }",
            "[\"tunes.example\"]\nextractors = { tunes = \"x/\" }\nkinds = [{}]",
            "[\"a.example\"]\nextractors = { t = \"\" }\n[\"b.example\"]\nextractors = { t = \"\" }",
        ] {
            let written: BTreeMap<String, Written> = toml::from_str(bad).unwrap();
            assert!(Sites::build(written).is_err(), "{bad}");
        }
    }
}
