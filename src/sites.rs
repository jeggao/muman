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
//! prefix, percent-encoded but for `/` and the characters RFC 3986
//! leaves unreserved: archive.org reads its path as a form does, so a
//! `+` in a file's name left as it is reads as a space and names no
//! file. One without is fetched again from the page its song records.
//! A key of a kind without a prefix is archived for yt-dlp's
//! `--download-archive` once by each of its site's extractors of no
//! prefix, as a playlist's entries may be another extractor's than a
//! page of one: an Audius playlist lists its tracks by `AudiusTrack`,
//! where a track's own page is read by `Audius`.
//!
//! Extractors listed for a site name what they fetch by that site from
//! then on, where what they fetched before is named by each page's: list
//! one only where its pages are the site's own, or a source fetched again
//! is listed twice. The table is part of the song list, so one list names
//! its sources alike on every machine; the list's tables lie over the
//! built-in ones a site at a time, so a list naming one site keeps the
//! rest.
//!
//! What a page's domain named before is named by its site where the site
//! lists that domain among its `aliases`: a fetch from it names the site,
//! and a sync renames a key of it ([`Sites::renamed`]). The built-in
//! table alone, [`built_in`], reads an extractor's key wherever a key is
//! read, without the song list.
//!
//! A source of yt-dlp's generic extractor, a file at any address, is
//! named by its page's domain only where no table describes the domain:
//! a described site's kinds would read its ID, a file's name, as theirs.

use std::collections::BTreeMap;
use std::fmt::Write as _;
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
    /// Other domains its sources were named by, as the address they were
    /// found at once named them.
    #[serde(default)]
    aliases: Vec<String>,
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
    aliases: Vec<String>,
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
            for alias in &w.aliases {
                if domain_of(alias).as_deref() != Some(alias.as_str()) || alias == domain {
                    bail!(
                        "[sites.\"{domain}\"] alias `{alias}` is no other domain, lower-cased and without `www.`"
                    );
                }
            }
            sites.insert(
                domain.clone(),
                Site {
                    extractors: w.extractors.clone(),
                    kinds,
                    aliases: w.aliases.clone(),
                },
            );
        }
        let mut named = BTreeMap::new();
        for (domain, site) in &sites {
            for alias in &site.aliases {
                if sites.contains_key(alias) {
                    bail!("[sites] `{alias}` is a site of its own and an alias of `{domain}`");
                }
                if let Some(other) = named.insert(alias.clone(), domain.clone()) {
                    bail!("[sites] alias `{alias}` is listed by `{other}` and `{domain}`");
                }
            }
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

/// The built-in table, which reads the keys an earlier muman named by an
/// extractor it lists wherever a key is read, without a song list's.
#[must_use]
pub fn built_in() -> &'static Sites {
    &BUILT_IN
}

impl Sites {
    /// The site that lists `extractor`, lower-cased, and the prefix its
    /// IDs take.
    #[must_use]
    pub fn of_extractor(&self, extractor: &str) -> Option<(&str, &str)> {
        self.sites.iter().find_map(|(domain, site)| {
            site.extractors
                .get(extractor)
                .map(|prefix| (domain.as_str(), prefix.as_str()))
        })
    }

    /// The site a domain names: the one listing it as an alias, else itself.
    fn site_of_domain(&self, domain: String) -> String {
        self.sites
            .iter()
            .find(|(_, site)| site.aliases.contains(&domain))
            .map_or(domain, |(d, _)| d.clone())
    }

    /// How `key` is named now, where the table names it otherwise: a key
    /// of an extractor it lists, as an earlier muman named sources, by
    /// that extractor's site, and one of an alias by the site that lists it.
    #[must_use]
    pub fn renamed(&self, key: &SourceKey) -> Option<SourceKey> {
        let SourceKey::Remote { site, id } = key else {
            return None;
        };
        let now = match self.of_extractor(site) {
            Some((domain, prefix)) if !site.contains('.') => format!("{domain}:{prefix}{id}"),
            _ => format!("{}:{id}", self.site_of_domain(site.clone())),
        };
        SourceKey::parse(&now).ok().filter(|now| now != key)
    }

    /// The extractor an earlier muman named `key` by, and its ID then: the
    /// first of its site's extractors whose prefix its ID begins with.
    #[must_use]
    pub fn extractor_of(&self, key: &SourceKey) -> Option<(&str, String)> {
        let SourceKey::Remote { site, id } = key else {
            return None;
        };
        let site = self.sites.get(site)?;
        site.extractors
            .iter()
            .filter(|(_, p)| id.starts_with(p.as_str()))
            .max_by_key(|(_, p)| p.len())
            .map(|(e, p)| (e.as_str(), id[p.len()..].to_string()))
    }

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
    /// lists the extractor, else the site of the page's domain, else the
    /// extractor. The one place a fetched source is named.
    #[must_use]
    pub fn fetched(&self, extractor: &str, page: Option<&str>, id: &str) -> Option<SourceKey> {
        let extractor = extractor.to_ascii_lowercase();
        let listed = self
            .of_extractor(&extractor)
            .map(|(domain, prefix)| (domain.to_string(), prefix.to_string()));
        let paged = page.and_then(domain_of).map(|d| self.site_of_domain(d));
        let (site, prefix) = match (listed, paged) {
            (Some(listed), _) => listed,
            // A file of any address, read by yt-dlp's generic extractor, is
            // none of a described site's kinds, which would misread its ID.
            (None, Some(d)) if extractor == "generic" && self.sites.contains_key(&d) => {
                (extractor, String::new())
            }
            (None, Some(d)) => (d, String::new()),
            (None, None) => (extractor, String::new()),
        };
        let key = SourceKey::parse(&format!("{site}:{prefix}{id}")).ok()?;
        self.accepts(&key).then_some(key)
    }

    /// The address a key's kind is fetched again from, if it has one.
    #[must_use]
    pub fn fetch_url(&self, key: &SourceKey) -> Option<String> {
        let (_, kind, rest) = self.kind_of(key)?;
        let id: Vec<String> = rest.split('/').map(crate::music::percent_encode).collect();
        Some(kind?.fetch.as_deref()?.replace("{id}", &id.join("/")))
    }

    /// Each kind's `fetch` address, as its site's domain, the kind's
    /// prefix and the address.
    pub fn fetches(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.sites.iter().flat_map(|(domain, site)| {
            site.kinds.iter().filter_map(move |k| {
                Some((domain.as_str(), k.prefix.as_str(), k.fetch.as_deref()?))
            })
        })
    }

    /// Every domain the table describes, every alias and every extractor
    /// it lists.
    pub fn schemes(&self) -> impl Iterator<Item = &str> {
        self.sites.iter().flat_map(|(domain, site)| {
            std::iter::once(domain.as_str())
                .chain(site.extractors.keys().map(String::as_str))
                .chain(site.aliases.iter().map(String::as_str))
        })
    }

    /// yt-dlp's `--download-archive` lines for a key, `<extractor> <id>`:
    /// a key of a kind without a prefix, by each of its site's extractors
    /// of no prefix, or one an earlier muman named by its extractor, as is.
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
        let lines = described
            .extractors
            .iter()
            .filter(|(_, p)| p.is_empty())
            .fold(String::new(), |mut lines, (extractor, _)| {
                let _ = writeln!(lines, "{extractor} {id}");
                lines
            });
        (!lines.is_empty()).then_some(lines)
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
    fn every_key_an_earlier_muman_named_by_its_extractor_reads_as_before() {
        for (old, now) in [
            ("youtube:vid00000001", "youtube.com:vid00000001"),
            ("youtubetab:OLAK5uy_x", "youtube.com:playlist/OLAK5uy_x"),
            ("archiveorg:item0001", "archive.org:item0001"),
            ("soundcloud:1", "soundcloud.com:1"),
            ("bandcamp:1", "bandcamp.com:1"),
            ("vimeo:1", "vimeo.com:1"),
            ("dailymotion:x1", "dailymotion.com:x1"),
            ("mixcloud:a", "mixcloud.com:a"),
            ("audiomack:a", "audiomack.com:a"),
            ("niconico:sm1", "nicovideo.jp:sm1"),
            ("bilibili:BV1", "bilibili.com:BV1"),
            ("twitter:1", "x.com:1"),
            ("tiktok:1", "tiktok.com:1"),
            ("jamendo:1234567", "jamendo.com:1234567"),
            ("audiustrack:aBc12", "audius.co:aBc12"),
        ] {
            assert_eq!(key(old).to_string(), now);
        }
        assert_eq!(
            key("youtube.com:playlist/OLAK5uy_x").spelled_before(),
            Some(SourceKey::Remote {
                site: "youtubetab".into(),
                id: "OLAK5uy_x".into()
            })
        );
    }

    #[test]
    fn an_alias_names_its_sites_sources_and_a_generic_fetch_names_none() {
        let sites = Sites::default();
        let page = "https://licensing.jamendo.com/en/track/1234567/title";
        assert_eq!(
            sites.fetched("JamendoLicensing", Some(page), "1234567"),
            Some(key("jamendo.com:1234567"))
        );
        assert_eq!(
            sites.renamed(&key("licensing.jamendo.com:1234567")),
            Some(key("jamendo.com:1234567"))
        );
        assert_eq!(sites.renamed(&key("jamendo.com:1234567")), None);
        let mirror = "https://archive.org/download/item-1/01.mp3";
        assert_eq!(
            sites.fetched("Generic", Some(mirror), "01"),
            Some(key("generic:01")),
            "an archive.org kind would read `01` as an item"
        );
        assert_eq!(
            sites.fetched("Generic", Some("https://tunes.example/01.mp3"), "01"),
            Some(key("tunes.example:01"))
        );
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
        assert_eq!(
            at(
                "Jamendo",
                Some("https://licensing.jamendo.com/en/track/1000001/a"),
                "1000001"
            ),
            "jamendo.com:1000001"
        );
        assert_eq!(
            at("AudiusTrack", Some("audius:Abc12"), "Abc12"),
            "audius.co:Abc12"
        );
        assert!(sites.fetched("Niconico", None, "watch").is_none());
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
        assert_eq!(
            url("archive.org:item-1/Marlo+Venn_-_Lantern(1).mp3").as_deref(),
            Some("https://archive.org/details/item-1/Marlo%2BVenn_-_Lantern%281%29.mp3")
        );
        assert_eq!(
            url("nicovideo.jp:sm00000001").as_deref(),
            Some("https://www.nicovideo.jp/watch/sm00000001")
        );
        assert_eq!(
            url("jamendo.com:1000001").as_deref(),
            Some("https://www.jamendo.com/track/1000001")
        );
        assert_eq!(url("audius.co:Abc12").as_deref(), Some("audius:Abc12"));
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
        assert_eq!(
            line("audius.co:Abc12").as_deref(),
            Some("audius Abc12\naudiustrack Abc12\n")
        );
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
