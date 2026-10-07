//! Where sources come from, and which a song looks for: each provider's
//! settings, and the triggers by which a song with a source from one
//! looks another up, as `[providers.*]` and `[[trigger]]` in the song
//! list set them over the defaults.
//!
//! `youtube` and `youtube-music` share one key scheme, `youtube:<id>`: a
//! video is a release when its info names a track or its channel is an
//! artist's " - Topic", read with its tags, so no key changes when a
//! video is told apart. A video not measured yet is neither.
//!
//! Any `[[trigger]]` replaces every default trigger rather than adding to
//! them, so the triggers in the file are the whole of what runs. A YouTube
//! provider is looked up from a YouTube video alone, so a file of the
//! user's own never searches YouTube.
//!
//! `acoustid` finds MusicBrainz records by a song's fingerprint, so what
//! it finds is a `musicbrainz:` source ([`Provider::yields`]): a song with
//! one has what either would find, and no source is from `acoustid`.

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::facts::Facts;
use crate::source::SourceKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    /// Files of the user's own; never fetched, never looked up.
    Manual,
    /// A YouTube video that is not a release.
    YouTube,
    /// A YouTube Music release, a track with its album.
    YouTubeMusic,
    /// Lyrics from lrclib.net.
    Lrclib,
    /// Tags from musicbrainz.org.
    MusicBrainz,
    /// MusicBrainz recordings found by fingerprint on acoustid.org.
    AcoustId,
    /// Covers from the Cover Art Archive.
    CoverArt,
}

impl Provider {
    pub const ALL: [Self; 7] = [
        Self::Manual,
        Self::YouTube,
        Self::YouTubeMusic,
        Self::Lrclib,
        Self::MusicBrainz,
        Self::AcoustId,
        Self::CoverArt,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::YouTube => "youtube",
            Self::YouTubeMusic => "youtube-music",
            Self::Lrclib => "lrclib",
            Self::MusicBrainz => "musicbrainz",
            Self::AcoustId => "acoustid",
            Self::CoverArt => "coverart",
        }
    }

    pub fn named(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name() == name)
            .with_context(|| {
                format!(
                    "`{name}` is no provider: {}",
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }

    /// The provider a source is from: a YouTube video is a release when
    /// its facts say so. `None` for a source of another extractor, or a
    /// video not measured yet.
    #[must_use]
    pub fn of(key: &SourceKey, facts: Option<&Facts>) -> Option<Self> {
        match key {
            SourceKey::Manual(_) => Some(Self::Manual),
            SourceKey::Remote { extractor, .. } => match extractor.as_str() {
                "youtube" => Some(if facts?.release {
                    Self::YouTubeMusic
                } else {
                    Self::YouTube
                }),
                other => crate::store::kept(other).map(|k| k.provider),
            },
        }
    }

    /// Whether a song can look a source of this provider up.
    #[must_use]
    pub fn findable(self) -> bool {
        self != Self::Manual
    }

    /// The provider of the sources it finds.
    #[must_use]
    pub fn yields(self) -> Self {
        match self {
            Self::AcoustId => Self::MusicBrainz,
            p => p,
        }
    }

    /// Whether `[providers.*] url` names its server.
    fn served(self) -> bool {
        self.kept() || self == Self::AcoustId
    }

    /// The defaults its `[providers.*]` table is read over.
    #[must_use]
    pub fn defaults(self) -> Settings {
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
        }
    }

    /// Whether its sources are records muman keeps from a lookup, which
    /// `sync` fetches again by ID when gone.
    #[must_use]
    pub fn kept(self) -> bool {
        crate::store::KEPT.iter().any(|k| k.provider == self)
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The extractor name LRCLIB's keys carry: `lrclib:<id>`.
pub const LRCLIB: &str = "lrclib";
const LRCLIB_URL: &str = "https://lrclib.net";
/// The extractor name MusicBrainz's keys carry: `musicbrainz:<recording id>`.
pub const MUSICBRAINZ: &str = "musicbrainz";
const MUSICBRAINZ_URL: &str = "https://musicbrainz.org";
/// The extractor name the Cover Art Archive's keys carry:
/// `coverart:<release group or release id>`.
pub const COVERART: &str = "coverart";
const COVERART_URL: &str = "https://coverartarchive.org";
const ACOUSTID_URL: &str = "https://api.acoustid.org";

/// Whether `key` is a record muman keeps from a lookup, as
/// [`Provider::kept`] says.
#[must_use]
pub fn is_kept(key: &SourceKey) -> bool {
    matches!(key, SourceKey::Remote { extractor, .. } if crate::store::kept(extractor).is_some())
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
}

/// When a trigger looks a song up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Always,
    /// The song has no lyrics.
    NoLyrics,
    /// The song has no timed lyrics.
    NoTimedLyrics,
    /// No source names the song's album, nor does the song list.
    NoAlbum,
    /// The song has no cover, or one with less detail than
    /// [`crate::quality::SOFT_COVER`].
    SmallCover,
}

impl When {
    fn named(name: &str) -> Result<Self> {
        Ok(match name {
            "always" => Self::Always,
            "no-lyrics" => Self::NoLyrics,
            "no-timed-lyrics" => Self::NoTimedLyrics,
            "no-album" => Self::NoAlbum,
            "small-cover" => Self::SmallCover,
            _ => bail!(
                "`{name}` is no condition: always, no-lyrics, no-timed-lyrics, no-album, small-cover"
            ),
        })
    }
}

/// A song with a source from any of `from`, and none from `find`, looks
/// `find` up when the condition holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub from: Vec<Provider>,
    pub find: Provider,
    pub when: When,
}

/// What a song list sets of providers and triggers, over the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub providers: BTreeMap<Provider, Settings>,
    pub triggers: Vec<Trigger>,
}

impl Default for Config {
    fn default() -> Self {
        use Provider::{AcoustId, CoverArt, Lrclib, Manual, MusicBrainz, YouTube, YouTubeMusic};
        Self {
            providers: Provider::ALL
                .into_iter()
                .map(|p| (p, p.defaults()))
                .collect(),
            triggers: vec![
                Trigger {
                    from: vec![YouTube],
                    find: YouTubeMusic,
                    when: When::Always,
                },
                Trigger {
                    from: vec![YouTubeMusic],
                    find: YouTube,
                    when: When::NoTimedLyrics,
                },
                Trigger {
                    from: vec![Manual, YouTube, YouTubeMusic],
                    find: Lrclib,
                    when: When::NoTimedLyrics,
                },
                // A release or a tagged file offers what a record would, and at
                // a request a second, 3600 songs looked up take an hour.
                Trigger {
                    from: vec![Manual, YouTube, YouTubeMusic],
                    find: AcoustId,
                    when: When::NoAlbum,
                },
                Trigger {
                    from: vec![Manual, YouTube, YouTubeMusic],
                    find: MusicBrainz,
                    when: When::NoAlbum,
                },
                Trigger {
                    from: vec![Manual, YouTube, YouTubeMusic],
                    find: CoverArt,
                    when: When::SmallCover,
                },
            ],
        }
    }
}

impl Config {
    #[must_use]
    pub fn settings(&self, p: Provider) -> &Settings {
        &self.providers[&p]
    }

    /// The triggers whose provider looked up is on.
    pub fn active(&self) -> impl Iterator<Item = &Trigger> {
        self.triggers
            .iter()
            .filter(|t| self.settings(t.find).enabled)
    }
}

fn names(item: &Item, what: &str) -> Result<Vec<Provider>> {
    match item {
        Item::Value(Value::String(s)) => Ok(vec![Provider::named(s.value())?]),
        Item::Value(Value::Array(a)) => a
            .iter()
            .map(|v| {
                Provider::named(
                    v.as_str()
                        .with_context(|| format!("{what}: names a provider"))?,
                )
            })
            .collect(),
        _ => bail!("{what} must name a provider or a list of them"),
    }
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
            "key" if p == Provider::AcoustId => {
                let k = item
                    .as_str()
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .with_context(|| format!("{what}: `key` must be an application key"))?;
                s.key = Some(k.to_string());
            }
            _ => bail!("{what}: `{key}` is no setting"),
        }
    }
    Ok(s)
}

/// The providers and triggers a song list sets; the defaults where it
/// sets none. Any `[[trigger]]` replaces every default trigger.
pub fn read(doc: &DocumentMut) -> Result<Config> {
    let mut config = Config::default();
    if let Some(item) = doc.get("providers") {
        let table = item
            .as_table()
            .context("`providers` must be a table of [providers.<name>] tables")?;
        for (name, item) in table {
            let p = Provider::named(name)?;
            let t = item
                .as_table()
                .with_context(|| format!("`providers.{name}` must be a table"))?;
            config.providers.insert(p, read_settings(p, t)?);
        }
    }
    if let Some(item) = doc.get("trigger") {
        let list = item
            .as_array_of_tables()
            .context("`trigger` must be a list of [[trigger]] tables")?;
        config.triggers.clear();
        for (n, t) in list.iter().enumerate() {
            let what = format!("trigger {}", n + 1);
            let from = names(
                t.get("from")
                    .with_context(|| format!("{what} names no `from`"))?,
                &what,
            )?;
            let find = t
                .get("find")
                .and_then(Item::as_str)
                .with_context(|| format!("{what} names no `find`"))?;
            let find = Provider::named(find)?;
            if from.contains(&Provider::AcoustId) {
                bail!("{what}: no source is from acoustid; its finds are from musicbrainz");
            }
            if !find.findable() {
                bail!("{what}: {find} cannot be looked up");
            }
            let youtube = [Provider::YouTube, Provider::YouTubeMusic];
            if youtube.contains(&find) && from.iter().any(|p| !youtube.contains(p)) {
                bail!("{what}: {find} is looked up from a YouTube video alone");
            }
            let when = match t.get("when") {
                None => When::Always,
                Some(w) => When::named(
                    w.as_str()
                        .with_context(|| format!("{what}: `when` must be text"))?,
                )?,
            };
            for (key, _) in t {
                if !["from", "find", "when"].contains(&key) {
                    bail!("{what}: `{key}` is no setting");
                }
            }
            config.triggers.push(Trigger { from, find, when });
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
        let lrclib = c
            .triggers
            .iter()
            .find(|t| t.find == Provider::Lrclib)
            .unwrap();
        assert!(lrclib.from.contains(&Provider::Manual));
        assert!(
            !c.triggers.iter().any(|t| t.from.contains(&Provider::Manual)
                && [Provider::YouTube, Provider::YouTubeMusic].contains(&t.find)),
            "a file of the user's own looks nothing up on YouTube"
        );
        let mb = c.settings(Provider::MusicBrainz);
        assert_eq!(mb.url.as_deref(), Some("https://musicbrainz.org"));
        assert_eq!(mb.concurrency, 1);
        assert_eq!(mb.key, None);
        let acoustid = c.settings(Provider::AcoustId);
        assert_eq!(acoustid.key.as_deref(), Some(crate::acoustid::KEY));
        let order: Vec<Provider> = c.triggers.iter().map(|t| t.find).collect();
        let at = |p| order.iter().position(|f| *f == p).unwrap();
        assert!(at(Provider::AcoustId) < at(Provider::MusicBrainz));
    }

    #[test]
    fn tables_set_over_the_defaults_and_triggers_replace_them() {
        let c = config(
            "[providers.lrclib]\nrecheck = \"3 days\"\nurl = \"lrclib.example:8080\"\n\
             [providers.youtube-music]\nenabled = false\n\
             [providers.musicbrainz]\nurl = \"https://mb.example/\"\n\
             [providers.acoustid]\nkey = \"0wnK3y\"\nurl = \"https://aid.example\"\n\
             [[trigger]]\nfrom = \"manual\"\nfind = \"lrclib\"\nwhen = \"no-lyrics\"\n",
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
        assert_eq!(c.triggers.len(), 1);
        assert_eq!(c.triggers[0].when, When::NoLyrics);
        assert!(!c.settings(Provider::YouTubeMusic).enabled);
    }

    #[test]
    fn mistakes_are_refused() {
        assert!(config("[providers.spotify]\n").is_err());
        assert!(config("[providers.lrclib]\nrecheck = 3\n").is_err());
        assert!(config("[providers.youtube]\nurl = \"yt.example\"\n").is_err());
        assert!(config("[providers.musicbrainz]\nkey = \"k\"\n").is_err());
        assert!(config("[providers.acoustid]\nkey = \" \"\n").is_err());
        assert!(config("[[trigger]]\nfrom = \"acoustid\"\nfind = \"lrclib\"\n").is_err());
        assert!(config("[[trigger]]\nfrom = \"youtube\"\nfind = \"manual\"\n").is_err());
        assert!(config("[[trigger]]\nfrom = \"manual\"\nfind = \"youtube-music\"\n").is_err());
        assert!(
            config("[[trigger]]\nfrom = \"youtube\"\nfind = \"lrclib\"\nwhen = \"often\"\n")
                .is_err()
        );
    }
}
