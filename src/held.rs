//! What each source held when its song was built, recorded in the song
//! list beside the song, and the sources that hold something else now.
//!
//! The record is a digest of each part of a built file the source may
//! give it, as [`crate::facts`] makes them: `audio`, `cover` of all its
//! covers, `lyrics` of all its lyrics and `tags` of its tags as written
//! in it; and for what yt-dlp fetched, the `format` the site served and
//! that format's `size`. Two of them:
//!
//! ```toml
//! held."youtube.com:vid00000001" = { audio = "7c1cfa040b3b82c0", tags = "9a3c0e5f7d1b2468" }
//! ```
//!
//! Each digest survives a fetch again of the same thing: audio and
//! subtitles by their packets, which no remux changes, a picture by its
//! bytes, and tags by the fields of the info JSON they are read from,
//! where the rest of it holds the time and cookies of each fetch. A
//! source's covers, or lyrics, are one digest of all it offers. A part
//! the record lacks is recorded rather than called a change, so a
//! record written before muman recorded that part says nothing false.
//! Sixteen hex digits, 64 bits, keep the line short; two different
//! contents share them by chance once in about 10^19 comparisons.
//!
//! Who changes a source decides what a difference means. A file in the
//! manual folder changes when you change it, so its song follows, and
//! its record is written anew. What muman fetched changes only when it
//! is fetched again, its file lost, and what a site serves then may not
//! be what it served: a video re-encoded, its audio edited, another
//! format preferred, its thumbnail, subtitles or title changed. Such a
//! song keeps the file built from what was recorded until `sync
//! --accept` takes what the source holds now, and is built
//! from what is there when no file of it is left, its record kept for
//! the next sync to say so again. The format lets a fetch again ask a
//! site for the same stream, and the page, kept for a site whose keys
//! name none, lets a home made from the song list alone fetch it.
//!
//! The size lets `check --upstream` ask a site whether it still serves
//! a source as fetched, before anything is lost: YouTube gives each
//! format's exact length, so a format of another size is other audio,
//! and a size under another ID is taken for the same file renumbered,
//! as a site that numbers its formats by place does when it lists a new
//! one. The listing holds the tag fields too, so other tags are found
//! the same way.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use toml_edit::{InlineTable, Item, Table, Value, value};

use crate::facts::Facts;
use crate::manifest::{Edit, Manifest};
use crate::sites::Sites;
use crate::source::SourceKey;

/// The hex digits of a digest the song list records.
pub const DIGITS: usize = 16;

/// A part of a built file a source may give it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    Audio,
    Cover,
    Lyrics,
    Tags,
}

impl Aspect {
    pub const ALL: [Self; 4] = [Self::Audio, Self::Cover, Self::Lyrics, Self::Tags];

    /// Its key in the record, and its name in what muman says.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Cover => "cover",
            Self::Lyrics => "lyrics",
            Self::Tags => "tags",
        }
    }
}

/// What one source held when its song was built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    /// The first [`DIGITS`] of each part's digest, where it offers one.
    pub audio: Option<String>,
    pub cover: Option<String>,
    pub lyrics: Option<String>,
    pub tags: Option<String>,
    /// The format a site served it in.
    pub format: Option<String>,
    /// The size the site gave that format's audio.
    pub size: Option<u64>,
    /// The page it was fetched from, for a site whose keys name none.
    pub url: Option<String>,
}

/// The first [`DIGITS`] of a digest.
fn cut(digest: &str) -> Option<String> {
    digest.get(..DIGITS).map(str::to_string)
}

/// One digest of several parts, each by where it sits; none of none.
fn of_all(parts: &[(String, &String)]) -> Option<String> {
    use std::fmt::Write as _;
    if parts.is_empty() {
        return None;
    }
    let text = parts.iter().fold(String::new(), |mut text, (at, d)| {
        let _ = writeln!(text, "{at}={d}");
        text
    });
    cut(&crate::facts::digest(text.as_bytes()))
}

/// Whether two digests agree as far as both go.
fn agree(a: &str, b: &str) -> bool {
    let n = a.len().min(b.len());
    a[..n] == b[..n]
}

impl Held {
    /// What `key`, measured as `facts`, holds now; none with nothing
    /// hashed. The page is recorded only for a key `sites` gives no
    /// address of its own.
    #[must_use]
    pub fn now(key: &SourceKey, facts: &Facts, sites: &Sites) -> Option<Self> {
        let served = facts.served.as_ref();
        let held = Self {
            audio: facts
                .audio
                .as_ref()
                .and_then(|a| a.digest.as_deref())
                .and_then(cut),
            cover: of_all(
                &facts
                    .covers
                    .iter()
                    .filter_map(|c| Some((format!("{:?}", c.at), c.digest.as_ref()?)))
                    .collect::<Vec<_>>(),
            ),
            lyrics: of_all(
                &facts
                    .lyrics
                    .iter()
                    .filter_map(|l| Some((format!("{:?}", l.at), l.digest.as_ref()?)))
                    .collect::<Vec<_>>(),
            ),
            tags: facts.tags_digest.as_deref().and_then(cut),
            format: served.map(|s| s.format.clone()),
            size: served.and_then(|s| s.size),
            url: served
                .and_then(|s| s.url.clone())
                .filter(|_| sites.fetch_url(key).is_none()),
        };
        Aspect::ALL
            .iter()
            .any(|a| held.digest(*a).is_some())
            .then_some(held)
    }

    /// The digest recorded of `aspect`.
    #[must_use]
    pub fn digest(&self, aspect: Aspect) -> Option<&str> {
        match aspect {
            Aspect::Audio => self.audio.as_deref(),
            Aspect::Cover => self.cover.as_deref(),
            Aspect::Lyrics => self.lyrics.as_deref(),
            Aspect::Tags => self.tags.as_deref(),
        }
    }

    /// Each part this record names that `now` holds otherwise, or no
    /// longer at all; a part it does not name is no change.
    #[must_use]
    pub fn changed(&self, now: &Self) -> Vec<Aspect> {
        Aspect::ALL
            .into_iter()
            .filter(|a| match (self.digest(*a), now.digest(*a)) {
                (Some(was), Some(is)) => !agree(was, is),
                (Some(_), None) => true,
                (None, _) => false,
            })
            .collect()
    }

    /// Whether this records tags other than those `digest` is of, as a
    /// site's listing gives them; recording none, it cannot tell.
    #[must_use]
    pub fn tags_differ(&self, digest: &str) -> bool {
        self.tags.as_deref().is_some_and(|t| !agree(t, digest))
    }

    /// `now`, keeping what this records of the fetch where `now` knows
    /// none of it, as a source measured without its info JSON.
    #[must_use]
    pub fn kept_in(&self, mut now: Self) -> Self {
        if now.format.is_none() {
            now.format.clone_from(&self.format);
            now.size = self.size;
        }
        if now.url.is_none() {
            now.url.clone_from(&self.url);
        }
        now
    }

    /// The format to ask for when `key` is fetched again: the one it was
    /// fetched in, then its audio alone, then `configured`. Only a
    /// YouTube ID names one stream for good; other sites number their
    /// formats by their place in a list, which a new file shifts.
    #[must_use]
    pub fn asked(&self, key: &SourceKey, configured: &str) -> Option<String> {
        let format = self.format.as_ref()?;
        if !key.is_youtube() {
            return None;
        }
        Some(match format.split_once('+') {
            Some((_, audio)) => format!("{format}/bv*+{audio}/{audio}/{configured}"),
            None => format!("{format}/{configured}"),
        })
    }

    /// The song list's inline table.
    #[must_use]
    pub fn item(&self) -> Item {
        let mut t = InlineTable::new();
        for aspect in Aspect::ALL {
            if let Some(d) = self.digest(aspect) {
                t.insert(aspect.name(), d.into());
            }
        }
        if let Some(f) = &self.format {
            t.insert("format", f.as_str().into());
        }
        if let Some(s) = self.size.and_then(|s| i64::try_from(s).ok()) {
            t.insert("size", s.into());
        }
        if let Some(u) = &self.url {
            t.insert("url", u.as_str().into());
        }
        value(t)
    }

    fn read(item: &Item, what: &str) -> Result<Self> {
        let Some(t) = item.as_table_like() else {
            bail!("{what} must be a table, such as {{ audio = \"7c1cfa040b3b82c0\" }}");
        };
        let digest = |aspect: Aspect| -> Result<Option<String>> {
            let Some(d) = t.get(aspect.name()).and_then(Item::as_str) else {
                return Ok(None);
            };
            if d.len() < DIGITS || d.len() > 64 || !d.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!(
                    "{what}: `{}` must be at least {DIGITS} hex digits",
                    aspect.name()
                );
            }
            Ok(Some(d.to_ascii_lowercase()))
        };
        let held = Self {
            audio: digest(Aspect::Audio)?,
            cover: digest(Aspect::Cover)?,
            lyrics: digest(Aspect::Lyrics)?,
            tags: digest(Aspect::Tags)?,
            format: t.get("format").and_then(Item::as_str).map(str::to_string),
            size: t
                .get("size")
                .and_then(Item::as_integer)
                .and_then(|s| u64::try_from(s).ok()),
            url: t.get("url").and_then(Item::as_str).map(str::to_string),
        };
        if Aspect::ALL.iter().all(|a| held.digest(*a).is_none()) {
            bail!("{what} records no digest: `audio`, `cover`, `lyrics` or `tags`");
        }
        Ok(held)
    }
}

/// A song table's `held`, for the keys in `sources`; a record of a key
/// the song does not list is ignored, and dropped when muman next
/// records one.
pub fn read(song: &Table, sources: &[SourceKey], what: &str) -> Result<BTreeMap<SourceKey, Held>> {
    let Some(item) = song.get("held") else {
        return Ok(BTreeMap::new());
    };
    let Some(table) = item.as_table_like() else {
        bail!("{what}: `held` must be a table of source keys");
    };
    let mut held = BTreeMap::new();
    for (key, item) in table.iter() {
        let Some(source) = sources.iter().find(|s| s.to_string() == key) else {
            continue;
        };
        held.insert(
            source.clone(),
            Held::read(item, &format!("{what}: held.\"{key}\""))?,
        );
    }
    Ok(held)
}

/// Set `key`'s record in a song table, as a dotted `held."<key>"` line,
/// dropping any of a key the song no longer lists.
pub fn set(song: &mut Table, key: &SourceKey, held: &Held) {
    let listed: Vec<String> = song
        .get("sources")
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let entry = song.entry("held").or_insert_with(|| {
        let mut t = Table::new();
        t.set_dotted(true);
        Item::Table(t)
    });
    // A `held` that is no table is the user's to mend; reading it says so.
    let Some(table) = entry.as_table_like_mut() else {
        return;
    };
    let stale: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !listed.contains(k))
        .collect();
    for k in stale {
        table.remove(&k);
    }
    table.insert(&key.to_string(), held.item());
}

/// Drop `key`'s record from a song table, and `held` once empty.
pub fn unset(song: &mut Table, key: &SourceKey) {
    if let Some(table) = song.get_mut("held").and_then(Item::as_table_like_mut) {
        table.remove(&key.to_string());
        if table.is_empty() {
            song.remove("held");
        }
    }
}

/// What a site serves now of a source, against what was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upstream {
    /// The audio format recorded, at the size recorded.
    Same,
    /// The size recorded, under another format ID.
    Renumbered(String),
    /// The format recorded, at another size: other audio.
    Resized { format: String, was: u64, now: u64 },
    /// The format recorded, the site giving no size to compare.
    Unsized(String),
    /// Neither the format nor the size recorded.
    Withdrawn(String),
}

impl Held {
    /// What `info`, a site's listing now, serves against this record;
    /// none when it records no format.
    #[must_use]
    pub fn upstream(&self, info: &crate::info::VideoInfo) -> Option<Upstream> {
        let recorded = self.format.as_deref()?;
        let parts: Vec<&str> = recorded.split('+').collect();
        let offered = |id: &str| info.formats.iter().find(|f| f.format_id == id);
        let audio = parts
            .iter()
            .copied()
            .find(|p| offered(p).is_some_and(|f| f.has_audio() && !f.has_video()))
            .or_else(|| parts.last().copied())?;
        let sized = |f: &crate::info::Format| f.filesize.zip(self.size);
        Some(match offered(audio) {
            Some(f) => match sized(f) {
                Some((now, was)) if now == was => Upstream::Same,
                Some((now, was)) => Upstream::Resized {
                    format: audio.to_string(),
                    was,
                    now,
                },
                None => Upstream::Unsized(audio.to_string()),
            },
            None => match info
                .formats
                .iter()
                .find(|f| self.size.is_some() && f.filesize == self.size)
            {
                Some(f) => Upstream::Renumbered(f.format_id.clone()),
                None => Upstream::Withdrawn(audio.to_string()),
            },
        })
    }
}

/// A source that holds otherwise than its song's record says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    /// The song's place in the list.
    pub song: usize,
    pub key: SourceKey,
    pub was: Held,
    pub now: Held,
    /// The parts it holds otherwise, or no longer.
    pub changed: Vec<Aspect>,
}

impl Drift {
    /// Whether its song follows it, being a file of your own.
    #[must_use]
    pub fn followed(&self) -> bool {
        matches!(self.key, SourceKey::Manual(_))
    }

    /// What changed, as a person reads it: `other audio and cover, no
    /// lyrics`, and the format, where that changed too.
    #[must_use]
    pub fn show(&self) -> String {
        let (gone, other): (Vec<Aspect>, Vec<Aspect>) = self
            .changed
            .iter()
            .partition(|a| self.now.digest(**a).is_none());
        let list = |aspects: &[Aspect]| {
            let names: Vec<&str> = aspects.iter().map(|a| a.name()).collect();
            match names.split_last() {
                Some((last, [])) => (*last).to_string(),
                Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
                None => String::new(),
            }
        };
        let mut what = Vec::new();
        if !other.is_empty() {
            what.push(format!("other {}", list(&other)));
        }
        if !gone.is_empty() {
            what.push(format!("no {}", list(&gone)));
        }
        let format = match (&self.was.format, &self.now.format) {
            (Some(a), Some(b)) if a != b => format!(" (format {a} → {b})"),
            _ => String::new(),
        };
        format!(
            "{} has changed since its song was built: {}{format}",
            self.key,
            what.join(", ")
        )
    }
}

/// What each listed source with a record holds otherwise now.
#[must_use]
pub fn drifts(manifest: &Manifest, facts: &BTreeMap<SourceKey, Facts>) -> Vec<Drift> {
    let mut found = Vec::new();
    for (n, song) in manifest.songs.iter().enumerate() {
        for (key, was) in &song.held {
            let Some(now) = facts
                .get(key)
                .and_then(|f| Held::now(key, f, &manifest.settings.sites))
            else {
                continue;
            };
            let changed = was.changed(&now);
            if !changed.is_empty() {
                found.push(Drift {
                    song: n,
                    key: key.clone(),
                    was: was.clone(),
                    now,
                    changed,
                });
            }
        }
    }
    found
}

/// The records to write: every listed source with something hashed and
/// no record, every record that holds the same but names a part, a
/// format or a page otherwise or not at all, and each drift in `taken`.
#[must_use]
pub fn records(
    manifest: &Manifest,
    facts: &BTreeMap<SourceKey, Facts>,
    taken: &[&Drift],
) -> Vec<Edit> {
    let mut edits = Vec::new();
    for song in &manifest.songs {
        for key in &song.sources {
            let Some(now) = facts
                .get(key)
                .and_then(|f| Held::now(key, f, &manifest.settings.sites))
            else {
                continue;
            };
            let held = match song.held.get(key) {
                None => Some(now),
                Some(was) if was.changed(&now).is_empty() => {
                    Some(was.kept_in(now)).filter(|n| n != was)
                }
                Some(was) => taken
                    .iter()
                    .any(|d| d.key == *key)
                    .then(|| was.kept_in(now)),
            };
            if let Some(held) = held {
                edits.push(Edit::Hold {
                    key: key.clone(),
                    held,
                });
            }
        }
    }
    edits
}

#[cfg(test)]
mod tests;
