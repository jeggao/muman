//! Where sources come from, and what a song looks for: each provider's
//! settings, as `[providers.*]` in the song list sets them over the
//! defaults `new.toml` writes.
//!
//! A song asks a provider `until` it has what the provider's conditions
//! name, every one: each is a closed query term ([`Closed`]) the provider
//! can be judged to give by what it finds ([`Provider::gives`]), so a
//! provider is never asked for what it cannot answer, and a record it
//! found is judged by itself rather than by what its song resolves to.
//! `skip` lists queries naming songs it leaves out, `[providers] skip`
//! those every provider does. Each key is read alone over its default,
//! so a song list that sets one keeps every other default, and a later
//! one reaches it. A YouTube provider is looked up from a YouTube video
//! alone, so a file of the user's own never searches YouTube.
//!
//! `youtube` and `youtube-music` share one site, `youtube.com:<id>`: a
//! video is a release when its info names a track or its channel is an
//! artist's " - Topic", read with its tags, so no key changes when a
//! video is told apart. A video not measured yet is neither.
//!
//! `acoustid` finds MusicBrainz records by a song's fingerprint, so what
//! it finds is a `musicbrainz:` source ([`Provider::yields`]): a song with
//! one has what either would find, and no source is from `acoustid`.
//!
//! A provider not `enabled` is asked nothing: no lookup, no record of
//! its fetched again, no request made for another provider's sake.
//! `acoustid` fetches each recording it finds from MusicBrainz, so it is
//! off while `musicbrainz` is ([`Config::on`]); `coverart` then asks the
//! Cover Art Archive by the IDs a song's tags name alone, searching
//! MusicBrainz for no album; and `add` looks no video's YouTube Music
//! track up while `youtube-music` is off.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::facts::Facts;
use crate::source::{SourceKey, YOUTUBE};
use crate::tags::{Field, RECORD_FIELDS};
use crate::terms::{Closed, Have};

use crate::provider::Provider;

const LRCLIB_URL: &str = "https://lrclib.net";
const MUSICBRAINZ_URL: &str = "https://musicbrainz.org";
const COVERART_URL: &str = "https://coverartarchive.org";
const ACOUSTID_URL: &str = "https://api.acoustid.org";

/// What a provider is in the song list's terms: which provider a source
/// is from, the defaults its `[providers.*]` table is read over, and the
/// conditions of `until` it can meet. A trait, as [`Provider`] is the
/// lowest layer's and these read the song list's terms.
pub trait ProviderRules: Sized {
    /// The provider a source is from, a torrent's file read as a file of
    /// the user's is: a YouTube video is a release when
    /// its facts say so. `None` for a source of another extractor, or a
    /// video not measured yet.
    #[must_use]
    fn of(key: &SourceKey, facts: Option<&Facts>) -> Option<Self>;

    /// The defaults its `[providers.*]` table is read over.
    #[must_use]
    fn defaults(self) -> Settings;

    /// Whether a song may ask this provider until it meets `c`: whether
    /// what the provider finds can be judged to give it.
    #[must_use]
    fn gives(self, c: &Closed) -> bool;

    /// The conditions [`Self::gives`] takes, for a message.
    #[must_use]
    fn conditions(self) -> String;
}

impl ProviderRules for Provider {
    fn of(key: &SourceKey, facts: Option<&Facts>) -> Option<Self> {
        match key {
            SourceKey::Manual(_) | SourceKey::Torrent(_) => Some(Self::Manual),
            SourceKey::Remote { site, .. } if site == YOUTUBE => Some(if facts?.release {
                Self::YouTubeMusic
            } else {
                Self::YouTube
            }),
            SourceKey::Remote { site, .. } => crate::store::kept(site).map(|k| k.provider),
        }
    }

    fn defaults(self) -> Settings {
        let (days, concurrency, per_run) = match self {
            Self::Manual => (0, 1, 0),
            Self::YouTube | Self::YouTubeMusic => (14, 4, 50),
            Self::Lrclib => (7, 4, 300),
            // Its requests go a second apart whatever runs them (`musicbrainz`),
            // and each recording AcoustID finds is fetched from MusicBrainz.
            Self::MusicBrainz | Self::AcoustId => (30, 1, 200),
            // One lookup an album, its searches on MusicBrainz's pace.
            Self::CoverArt => (30, 1, 500),
        };
        Settings {
            enabled: true,
            concurrency,
            recheck: crate::units::Time::days(days).duration(),
            per_run,
            url: match self {
                Self::Lrclib => Some(LRCLIB_URL.to_string()),
                Self::MusicBrainz => Some(MUSICBRAINZ_URL.to_string()),
                Self::AcoustId => Some(ACOUSTID_URL.to_string()),
                Self::CoverArt => Some(COVERART_URL.to_string()),
                Self::Manual | Self::YouTube | Self::YouTubeMusic => None,
            },
            key: (self == Self::AcoustId).then(|| crate::acoustid::KEY.to_string()),
            until: match self {
                Self::Manual => Vec::new(),
                Self::YouTubeMusic => vec![Closed::From(Self::YouTubeMusic)],
                Self::YouTube | Self::Lrclib => vec![Closed::Lyrics(Have::Better)],
                Self::AcoustId | Self::MusicBrainz => [
                    Field::Album,
                    Field::AlbumArtist,
                    Field::Date,
                    Field::Track,
                    Field::Disc,
                    Field::Isrc,
                ]
                .map(|f| Closed::Has(f.vorbis().to_string()))
                .into(),
                Self::CoverArt => vec![Closed::Cover(Have::Better)],
            },
            skip: Vec::new(),
        }
    }

    fn gives(self, c: &Closed) -> bool {
        let found = |h: &Have| matches!(h, Have::Some | Have::Better);
        match (self, c) {
            (Self::YouTube | Self::Lrclib, Closed::Lyrics(h))
            | (Self::CoverArt, Closed::Cover(h)) => found(h),
            (Self::AcoustId | Self::MusicBrainz, Closed::Has(tag)) => {
                RECORD_FIELDS.iter().any(|f| f.vorbis() == tag)
            }
            (p, Closed::From(q)) => p.findable() && *q == p.yields(),
            _ => false,
        }
    }

    fn conditions(self) -> String {
        let mut all: Vec<String> = [Have::Better, Have::Some]
            .into_iter()
            .flat_map(|h| [Closed::Lyrics(h), Closed::Cover(h)])
            .chain(Self::ALL.map(Closed::From))
            .chain(RECORD_FIELDS.map(|f| Closed::Has(f.vorbis().to_string())))
            .filter(|c| self.gives(c))
            .map(|c| c.to_string())
            .collect();
        all.dedup();
        all.join(", ")
    }
}

/// One provider's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    /// Lookups at once.
    pub concurrency: usize,
    /// How long a lookup that found nothing waits to be made again.
    pub recheck: std::time::Duration,
    /// Lookups one run makes at most, so a backlog drains over several
    /// runs rather than at once on a free service; 0 for no limit.
    pub per_run: usize,
    /// Where an LRCLIB, MusicBrainz, AcoustID or Cover Art Archive server
    /// answers.
    pub url: Option<String>,
    /// The application key AcoustID is asked with.
    pub key: Option<String>,
    /// What a song asks this provider until it has, every one of them.
    pub until: Vec<Closed>,
    /// Queries, as written, naming songs that ask this provider nothing.
    pub skip: Vec<String>,
}

/// What a song list sets of providers, over the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub providers: BTreeMap<Provider, Settings>,
    /// Queries, as written, naming songs that ask no provider anything.
    pub skip: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            providers: Provider::ALL
                .into_iter()
                .map(|p| (p, p.defaults()))
                .collect(),
            skip: Vec::new(),
        }
    }
}

impl Config {
    #[must_use]
    pub fn settings(&self, p: Provider) -> &Settings {
        &self.providers[&p]
    }

    /// Whether `p` may be asked anything: it is enabled, and so is the
    /// provider it fetches what it finds from, as `acoustid` fetches
    /// from `musicbrainz`.
    #[must_use]
    pub fn on(&self, p: Provider) -> bool {
        self.settings(p).enabled && self.settings(p.yields()).enabled
    }

    /// The queries naming songs that ask `p` nothing: `[providers] skip`
    /// and the provider's own.
    pub fn skips(&self, p: Provider) -> impl Iterator<Item = &str> {
        self.skip
            .iter()
            .chain(&self.settings(p).skip)
            .map(String::as_str)
    }
}

/// The conditions `until` names: one, or a list of them, each a closed
/// field's term `p` can be judged to give.
fn read_until(p: Provider, item: &Item, what: &str) -> Result<Vec<Closed>> {
    let texts: Vec<&str> = match item {
        Item::Value(Value::String(s)) => vec![s.value().as_str()],
        Item::Value(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .with_context(|| format!("{what}: `until` lists conditions, as \"has:album\""))
            })
            .collect::<Result<_>>()?,
        _ => bail!("{what}: `until` must be a condition or a list of them"),
    };
    if texts.is_empty() {
        bail!(
            "{what}: `until` lists no condition, so no song would ever ask; \
             set `enabled = false` to ask {p} nothing"
        );
    }
    let mut until = Vec::new();
    for text in texts {
        let text = text.trim();
        let c = (|| {
            if text.starts_with('^') {
                bail!("a song asks until it has something, so no condition is negated");
            }
            let (field, value) = text
                .split_once(':')
                .context("a condition is a field and a value, as \"has:album\"")?;
            let c = Closed::parse(field, value.strip_prefix('=').unwrap_or(value))?;
            if !p.gives(&c) {
                bail!("{p} cannot give it; it takes {}", p.conditions());
            }
            if until.contains(&c) {
                bail!("it is listed twice");
            }
            Ok(c)
        })()
        .with_context(|| format!("{what}: `until`: `{text}`"))?;
        until.push(c);
    }
    Ok(until)
}

/// The queries a `skip` lists, each checked to read as one.
fn read_skip(item: &Item, what: &str) -> Result<Vec<String>> {
    let list = item
        .as_array()
        .with_context(|| format!("{what} must be a list of queries"))?;
    let schemes: std::collections::BTreeSet<String> = crate::source::schemes()
        .chain(crate::sites::built_in().schemes())
        .map(str::to_string)
        .collect();
    list.iter()
        .map(|v| {
            let text = v.as_str().with_context(|| {
                format!("{what}: each entry is a query, as 'album:=\"Some Album\"'")
            })?;
            crate::terms::parse_text(text, &schemes)
                .map_err(|e| anyhow::anyhow!("{what}: `{text}`: {e:#}"))?;
            Ok(text.to_string())
        })
        .collect()
}

fn read_settings(p: Provider, t: &Table) -> Result<Settings> {
    let what = format!("[providers.{p}]");
    let mut s = p.defaults();
    for (key, item) in t {
        let number = || {
            item.as_integer()
                .and_then(|n| u64::try_from(n).ok())
                .with_context(|| format!("{what}: `{key}` must be a whole number"))
        };
        match key {
            "enabled" => {
                s.enabled = item
                    .as_bool()
                    .with_context(|| format!("{what}: `enabled` must be true or false"))?;
            }
            "concurrency" => {
                s.concurrency = usize::try_from(number()?.max(1)).unwrap_or(1);
            }
            "recheck" => {
                let t: crate::units::Time = item
                    .as_str()
                    .with_context(|| {
                        format!("{what}: `recheck` must be a length of time, such as \"30 days\"")
                    })?
                    .parse()
                    .map_err(|e| anyhow::anyhow!("{what}: `recheck`: {e}"))?;
                s.recheck = t.duration();
            }
            "recheck_days" => {
                let days = i64::try_from(number()?).unwrap_or(i64::MAX);
                s.recheck = crate::units::Time::days(days).duration();
            }
            "per_run" => s.per_run = usize::try_from(number()?).unwrap_or(usize::MAX),
            "url" if p.served() => {
                let url = item
                    .as_str()
                    .filter(|u| !u.trim().is_empty())
                    .with_context(|| format!("{what}: `url` must name a server"))?;
                s.url = Some(
                    crate::http::normalize_base(url)
                        .map_err(|e| anyhow::anyhow!("{what}: `url`: {e}"))?,
                );
            }
            "until" | "skip" if !p.findable() => {
                bail!("{what}: `{key}`: {p} is never looked up");
            }
            "until" => s.until = read_until(p, item, &what)?,
            "skip" => s.skip = read_skip(item, &format!("{what} skip"))?,
            "key" if p == Provider::AcoustId => {
                let k = item
                    .as_str()
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .with_context(|| format!("{what}: `key` must be an application key"))?;
                s.key = Some(k.to_string());
            }
            _ => bail!("{what}: `{key}` is no setting: {}", p.keys().join(", ")),
        }
    }
    Ok(s)
}

/// The providers a song list sets; the defaults where it sets none.
pub fn read(doc: &DocumentMut) -> Result<Config> {
    let mut config = Config::default();
    if let Some(item) = doc.get("providers") {
        let table = item
            .as_table()
            .context("`providers` must be a table of [providers.<name>] tables")?;
        for (name, item) in table {
            if name == "skip" {
                config.skip = read_skip(item, "[providers] skip")?;
                continue;
            }
            let p = Provider::named(name)?;
            let t = item
                .as_table()
                .with_context(|| format!("`providers.{name}` must be a table"))?;
            config.providers.insert(p, read_settings(p, t)?);
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Result<Config> {
        read(&text.parse().unwrap())
    }

    #[test]
    fn without_tables_the_defaults_hold() {
        let c = config("version = 1\n").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(
            c.settings(Provider::Lrclib).recheck,
            crate::units::Time::days(7).duration()
        );
        assert_eq!(
            c.settings(Provider::YouTubeMusic).recheck,
            crate::units::Time::days(14).duration()
        );
        assert_eq!(
            c.settings(Provider::Lrclib).until,
            [Closed::Lyrics(Have::Better)]
        );
        assert_eq!(c.settings(Provider::MusicBrainz).until.len(), 6);
        assert_eq!(c.skip, Vec::<String>::new());
        let mb = c.settings(Provider::MusicBrainz);
        assert_eq!(mb.url.as_deref(), Some("https://musicbrainz.org"));
        assert_eq!(mb.concurrency, 1);
        assert_eq!(mb.key, None);
        let acoustid = c.settings(Provider::AcoustId);
        assert_eq!(acoustid.key.as_deref(), Some(crate::acoustid::KEY));
    }

    #[test]
    fn tables_set_over_the_defaults_key_by_key() {
        let c = config(
            "[providers.lrclib]\nrecheck = \"3 days\"\nurl = \"lrclib.example:8080\"\n\
             [providers.youtube-music]\nenabled = false\n\
             [providers.musicbrainz]\nurl = \"https://mb.example/\"\n\
             [providers.acoustid]\nkey = \"0wnK3y\"\nurl = \"https://aid.example\"\n\
             until = [\"has:album\", \"has:label\"]\n\
             [providers.coverart]\nuntil = \"cover:yes\"\nskip = [\"from:manual\"]\n\
             [providers]\nskip = ['album:=\"The Glass Orchards\"']\n",
        )
        .unwrap();
        assert_eq!(
            c.settings(Provider::MusicBrainz).url.as_deref(),
            Some("https://mb.example")
        );
        let l = c.settings(Provider::Lrclib);
        assert_eq!(
            (l.recheck, l.concurrency, l.per_run),
            (crate::units::Time::days(3).duration(), 4, 300)
        );
        assert_eq!(l.url.as_deref(), Some("http://lrclib.example:8080"));
        let acoustid = c.settings(Provider::AcoustId);
        assert_eq!(acoustid.key.as_deref(), Some("0wnK3y"));
        assert_eq!(acoustid.url.as_deref(), Some("https://aid.example"));
        assert_eq!(
            acoustid.until,
            [Closed::Has("ALBUM".into()), Closed::Has("LABEL".into())]
        );
        assert_eq!(
            c.settings(Provider::MusicBrainz).until,
            Provider::MusicBrainz.defaults().until,
            "a provider's own `until` alone"
        );
        let cover = c.settings(Provider::CoverArt);
        assert_eq!(cover.until, [Closed::Cover(Have::Some)]);
        assert_eq!(
            c.skips(Provider::CoverArt).collect::<Vec<_>>(),
            ["album:=\"The Glass Orchards\"", "from:manual"]
        );
        assert!(!c.settings(Provider::YouTubeMusic).enabled);
    }

    #[test]
    fn acoustid_is_off_with_musicbrainz_which_it_fetches_from() {
        let c = config("[providers.musicbrainz]\nenabled = false\n").unwrap();
        assert!(c.settings(Provider::AcoustId).enabled);
        assert!(!c.on(Provider::AcoustId) && !c.on(Provider::MusicBrainz));
        assert!(c.on(Provider::CoverArt) && c.on(Provider::Lrclib));
    }

    #[test]
    fn mistakes_are_refused() {
        assert!(config("[providers.spotify]\n").is_err());
        assert!(config("[providers.lrclib]\nrecheck = 3\n").is_err());
        assert!(config("[providers.youtube]\nurl = \"yt.example\"\n").is_err());
        assert!(config("[providers.musicbrainz]\nkey = \"k\"\n").is_err());
        assert!(config("[providers.acoustid]\nkey = \" \"\n").is_err());
        for until in [
            "[]",
            "\"has:date\"",
            "\"^lyrics:timed\"",
            "\"lyrics:none\"",
            "\"from:musicbrainz\"",
            "[\"lyrics:timed\", \"lyrics:timed\"]",
            "\"timed\"",
        ] {
            let e = config(&format!("[providers.lrclib]\nuntil = {until}\n")).unwrap_err();
            assert!(format!("{e:#}").contains("until"), "{until}: {e:#}");
        }
        let e = config("[providers.lrclib]\nuntil = \"has:date\"\n").unwrap_err();
        assert!(
            format!("{e:#}").contains("lyrics:timed, lyrics:yes, from:lrclib"),
            "the error names what lrclib takes: {e:#}"
        );
        assert!(config("[providers.musicbrainz]\nuntil = \"has:genre\"\n").is_err());
        let e = config("[providers.musicbrainz]\nuntil = \"has:mood\"\n").unwrap_err();
        let said = format!("{e:#}");
        for name in [
            "has:isrc",
            "has:language",
            "has:musicbrainz_trackid",
            "has:label",
        ] {
            assert!(
                said.contains(name),
                "named as the docs name it, {name}: {said}"
            );
        }
        for alias in ["tsrc", "tlan", "track id", "publisher", "has:upc"] {
            assert!(!said.contains(alias), "no alias, {alias}: {said}");
        }
        for name in said.split("it takes ").nth(1).unwrap().split(", ") {
            config(&format!("[providers.musicbrainz]\nuntil = \"{name}\"\n"))
                .unwrap_or_else(|e| panic!("{name} as the message names it: {e:#}"));
        }
        assert!(config("[providers.manual]\nuntil = \"from:manual\"\n").is_err());
        assert!(config("[providers]\nskip = [\"\"]\n").is_err());
        assert!(config("[providers.lrclib]\nskip = \"genre:ambient\"\n").is_err());
    }
}
