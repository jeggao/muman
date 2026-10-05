//! The song list, `songs.toml`: every song the library holds, each with
//! the sources it may be made from, the tags set over what they offer,
//! and any aspect pinned to one source.
//!
//! The file is edited in place, the way Cargo keeps its lock file:
//! comments, order and keys this version does not know survive a write.
//! `version` names the format. A file of another version is refused,
//! never rewritten by a program that cannot read it.
//!
//! There is no song ID: a song is the sources it lists, and a key
//! belongs to one song only. A key that does not parse, one two songs
//! list, and a pin outside the song's sources stop the run before
//! anything is written, rather than a song being silently dropped and
//! its file deleted.
//!
//! muman changes the file only by recording edits and applying them to
//! the file as it is when saved, under the folder's lock and through a
//! renamed part file, so two runs keep each other's songs and a crash
//! leaves the old list or the new. An edit made from a song as it was
//! read, by `remove`, `set` or `edit`, is saved only while that song is
//! still listed as it was: a song changed meanwhile, by hand or by
//! another run, refuses the save and nothing is written.
//!
//! Every write puts the usage comment atop the file, replacing one an
//! earlier version wrote, while the user's own comments below it stay;
//! and it gives each song a `[song.tags]` table with the common keys it
//! lacks added empty, so the file shows what can be set. A key under
//! `[clean]` naming no rule is kept and warned about, so a misspelled
//! switch is never silently ignored.
//!
//! A `[[removed]]` key no song lists is a tombstone: a playlist or album
//! listing skips it, a manual file under it is not listed, and naming its
//! video alone lists it again. A key a song lists wins over a tombstone
//! that also names it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value, value};

use crate::atomic::{self, Lock};
use crate::clean::{self, Settings};
use crate::dirs::MANIFEST;
use crate::hooks::{self, Hook};
use crate::provider;
use crate::source::SourceKey;
use crate::tags;

/// The format this version reads and writes. Adding a key is not a new
/// version; changing what an existing key means is.
pub const VERSION: i64 = 1;

/// How to use the file, kept at its top; rewritten whenever it differs
/// from an earlier version's.
const HEADER: &str = "\
# muman's song list: every song in the library and what it is made from.
# muman adds a [[song]] for each song it keeps. Edit by hand, then run
# `muman sync` to apply: it fetches any missing source, rebuilds every
# changed song and deletes the library copy of anything no longer listed.
#
#   sources:  every file the song may be made from, as `youtube:<id>`,
#             `lrclib:<id>`, `musicbrainz:<id>` or `manual:<path>` under
#             sources/manual. muman picks the best audio, cover, lyrics and
#             tags by measuring each.
#   Pin one:  audio = \"<source>\", cover = \"<source>\", lyrics = \"<source>\";
#             lyrics = false for none. lyrics_offset_ms moves them later.
#   Tags:     fill in [song.tags]; an empty value keeps what the sources
#             offer. Any other Vorbis comment name works too; a list sets
#             several. `muman status` shows what was picked and why.
#   Cleaning: [clean.*] turns each rule cleaning what the sources offer on
#             or off; `muman status` names the rules that changed a tag.
#             Tags set by hand are never cleaned.
#   Removed:  `muman remove` keeps a song under [[removed]], so nothing
#             lists it again; delete the entry to let it back.
#   Lookups:  a song looks other sources up by [[trigger]] (from, find,
#             when), replacing the defaults; [providers.<name>] sets
#             enabled, concurrency, recheck_days, per_run, and the url
#             of lrclib and musicbrainz.
#   Hooks:    [[hook]] runs a command, on = \"written\" for each song file
#             ({path}), \"changed\" once a run wrote or removed any.
#   Settings: [library], [audio], [ytdlp] and [history] lay out and name
#             the library, set encoding and fetching, and size `undo`.
#
# Other keys muman does not know are kept. Leave `version` as it is.
";

/// The first line every version writes atop the file.
const HEADER_START: &str = "# muman's song list";

/// A new home's song list: the settings, each commented out at its
/// default, as the reference the docs point to.
pub const NEW: &str = include_str!("manifest/new.toml");

/// The tags every song's `[song.tags]` offers to fill in.
const TAG_TEMPLATE: [&str; 6] = ["title", "artist", "album", "album_artist", "genre", "date"];

/// Tags by name, each with its values.
pub type Tags = Vec<(String, Vec<String>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LyricsPin {
    None,
    From(SourceKey),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Song {
    pub sources: Vec<SourceKey>,
    /// The `[[album]]` it belongs to, and its place there.
    pub album: Option<SourceKey>,
    pub track: Option<u32>,
    pub audio: Option<SourceKey>,
    pub cover: Option<SourceKey>,
    pub lyrics: Option<LyricsPin>,
    pub lyrics_offset_ms: i64,
    pub tags: Tags,
}

impl Song {
    #[must_use]
    pub fn has(&self, key: &SourceKey) -> bool {
        self.sources.contains(key)
    }
}

/// A song removed from the list, kept whole under `[[removed]]` so its
/// sources are never listed again unasked and `restore` brings it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub sources: Vec<SourceKey>,
    /// How the song was named when it was removed.
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub source: SourceKey,
    pub tracks: Option<u32>,
    pub tags: Tags,
}

/// A change to apply to the file as it is when saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// A new song, or more sources for the one already listing any key
    /// of `sources`; its album and place are set when it has none.
    Add {
        sources: Vec<SourceKey>,
        album: Option<(SourceKey, u32)>,
    },
    /// A new album, unless listed.
    AddAlbum {
        source: SourceKey,
        tracks: Option<u32>,
    },
    /// One key in place of another wherever it is listed.
    Rename { from: SourceKey, to: SourceKey },
    /// A key dropped from the song listing it.
    Drop(SourceKey),
    /// Tags set in `[song.tags]` of the song listing `key`, over any it
    /// sets already under another spelling of their names.
    Tag { key: SourceKey, tags: Tags },
    /// The song listing `key` moved to `[[removed]]`, named by `note`.
    Remove { key: SourceKey, note: String },
    /// The removed song listing `key` listed again, without any key a
    /// song lists meanwhile.
    Restore(SourceKey),
    /// Songs edited together: the song listing each key replaced or
    /// removed, every one found before any changes, then new songs.
    Rewrite {
        songs: Vec<(SourceKey, Rewritten)>,
        new: Vec<SongTable>,
    },
}

/// What an edited song became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewritten {
    Table(SongTable),
    /// Removed, named by the note.
    Removed(String),
}

/// A `[[song]]` table as written, compared by its text.
#[derive(Debug, Clone)]
pub struct SongTable(pub Table);

impl PartialEq for SongTable {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_string() == other.0.to_string()
    }
}

impl Eq for SongTable {}

#[derive(Debug)]
pub struct Manifest {
    dir: PathBuf,
    pub songs: Vec<Song>,
    pub albums: Vec<Album>,
    /// Subtitle languages lyrics are taken in, most preferred first.
    pub lyrics: Vec<String>,
    /// Which cleaning rules are switched off.
    pub clean: Settings,
    pub removed: Vec<Removed>,
    /// What `[providers.*]` and `[[trigger]]` set over the defaults.
    pub providers: provider::Config,
    pub hooks: Vec<Hook>,
    /// `[library]`, `[audio]`, `[ytdlp]` and `[history]`.
    pub settings: crate::settings::Settings,
    edits: Vec<Edit>,
    stale: bool,
}

impl Manifest {
    /// The list in `home`; an empty one when there is none yet.
    pub fn load(home: &Path) -> Result<Self> {
        let doc = read(home)?;
        let file = home.join(MANIFEST);
        let parsed = parse(&doc).with_context(|| format!("reading {}", file.display()))?;
        Ok(Self {
            dir: home.to_path_buf(),
            songs: parsed.songs,
            albums: parsed.albums,
            lyrics: parsed.lyrics,
            removed: parsed.removed,
            providers: parsed.providers,
            hooks: parsed.hooks,
            clean: parsed.clean,
            settings: parsed.settings,
            edits: Vec::new(),
            stale: file.exists()
                && (with_header(&doc.to_string()).is_some() || normalize(&mut doc.clone())),
        })
    }

    pub fn edit(&mut self, edit: Edit) {
        self.edits.push(edit);
    }

    #[must_use]
    pub fn has_edits(&self) -> bool {
        !self.edits.is_empty()
    }

    /// Every key some song lists.
    #[must_use]
    pub fn keys(&self) -> BTreeSet<SourceKey> {
        self.songs
            .iter()
            .flat_map(|s| s.sources.iter().cloned())
            .collect()
    }

    /// Every key a removed song lists that no song lists.
    #[must_use]
    pub fn removed_keys(&self) -> BTreeSet<SourceKey> {
        let listed = self.keys();
        self.removed
            .iter()
            .flat_map(|r| r.sources.iter().cloned())
            .filter(|k| !listed.contains(k))
            .collect()
    }

    /// The song tables listing any of `keys`, as the file holds them now.
    pub fn tables_of(&self, keys: &[SourceKey]) -> Result<Vec<Table>> {
        let wanted: Vec<String> = keys.iter().map(ToString::to_string).collect();
        let doc = read(&self.dir)?;
        Ok(tables(&doc, "song")
            .filter(|t| listed_keys(t).iter().any(|k| wanted.contains(k)))
            .cloned()
            .collect())
    }

    #[must_use]
    pub fn song_with(&self, key: &SourceKey) -> Option<&Song> {
        self.songs.iter().find(|s| s.has(key))
    }

    #[must_use]
    pub fn album(&self, key: &SourceKey) -> Option<&Album> {
        self.albums.iter().find(|a| &a.source == key)
    }

    /// Apply what was recorded to the file as it is now, so a run that
    /// finished meanwhile keeps its own changes. The folder is locked for
    /// the read and the write, and the file replaced through a rename.
    pub fn save(&mut self) -> Result<()> {
        if self.edits.is_empty() && !self.stale {
            return Ok(());
        }
        let lock = Lock::folder(&self.dir)?;
        self.save_locked(&lock)
    }

    /// [`Self::save`] under a lock the caller already holds.
    pub fn save_locked(&mut self, lock: &Lock) -> Result<()> {
        self.save_expecting(lock, &[]).map(drop)
    }

    /// Apply what was recorded only while each song in `expected` is
    /// still listed as it was, by any of its keys. Returns the first key
    /// of each that changed meanwhile; when any did, nothing is written
    /// and the edits are kept.
    pub fn save_expecting(&mut self, _lock: &Lock, expected: &[Song]) -> Result<Vec<SourceKey>> {
        if self.edits.is_empty() && !self.stale {
            return Ok(Vec::new());
        }
        let mut doc = read(&self.dir)?;
        let now = parse(&doc)?;
        let changed: Vec<SourceKey> = expected
            .iter()
            .filter(|e| {
                !e.sources
                    .iter()
                    .any(|k| now.songs.iter().find(|s| s.has(k)).is_some_and(|s| s == *e))
            })
            .filter_map(|e| e.sources.first().cloned())
            .collect();
        if !changed.is_empty() {
            return Ok(changed);
        }
        for edit in &self.edits {
            apply(&mut doc, edit)?;
        }
        normalize(&mut doc);
        let parsed = parse(&doc)?;
        let text = doc.to_string();
        let text = with_header(&text).unwrap_or(text);
        atomic::write(&self.dir, MANIFEST, text.as_bytes())?;
        self.edits.clear();
        (self.songs, self.albums, self.lyrics, self.removed) =
            (parsed.songs, parsed.albums, parsed.lyrics, parsed.removed);
        (self.providers, self.hooks, self.clean) = (parsed.providers, parsed.hooks, parsed.clean);
        self.settings = parsed.settings;
        self.stale = false;
        Ok(Vec::new())
    }

    /// [`Self::save_expecting`] under a lock of its own.
    pub fn save_if_unchanged(&mut self, expected: &[Song]) -> Result<Vec<SourceKey>> {
        let lock = Lock::folder(&self.dir)?;
        self.save_expecting(&lock, expected)
    }

    /// Whether `edits` would leave a song list that reads, applied to the
    /// file as it is now; nothing is written.
    pub fn trial(&self, edits: &[Edit]) -> Result<()> {
        let mut doc = read(&self.dir)?;
        for edit in edits {
            apply(&mut doc, edit)?;
        }
        normalize(&mut doc);
        parse(&doc).map(drop)
    }

    /// Drop what was recorded and not saved.
    pub fn discard(&mut self) {
        self.edits.clear();
    }
}

/// `text` with the current header in place of its leading comments when
/// those are no header or an earlier one; `None` when already current.
/// Leading comments of the user's own are kept below it.
fn with_header(text: &str) -> Option<String> {
    let lead = text.lines().take_while(|l| l.starts_with('#')).count();
    let ours = text
        .lines()
        .next()
        .is_some_and(|l| l.starts_with(HEADER_START));
    let rest = if ours {
        text.lines().skip(lead).fold(String::new(), |mut acc, l| {
            acc.push_str(l);
            acc.push('\n');
            acc
        })
    } else {
        text.to_string()
    };
    let new = format!("{HEADER}{rest}");
    (new != text).then_some(new)
}

fn read(dir: &Path) -> Result<DocumentMut> {
    let file = dir.join(MANIFEST);
    let text = match std::fs::read_to_string(&file) {
        // Windows editors may add a byte-order mark and CRLF endings; the
        // file is written back with neither, rather than with both mixed.
        Ok(text) => text.trim_start_matches('\u{feff}').replace("\r\n", "\n"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => NEW.to_string(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let doc: DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML", file.display()))?;
    match doc.get("version").and_then(Item::as_integer) {
        Some(VERSION) => {}
        Some(v) if v > VERSION => bail!(
            "{} is format version {v}, newer than this muman's {VERSION}; \
             update muman before it changes the list",
            file.display()
        ),
        _ => bail!("{} has no format version muman knows", file.display()),
    }
    for key in ["song", "album", "removed"] {
        if doc
            .get(key)
            .is_some_and(|s| s.as_array_of_tables().is_none())
        {
            bail!(
                "{}: `{key}` must be a list of [[{key}]] tables",
                file.display()
            );
        }
    }
    Ok(doc)
}

fn key_at(t: &Table, name: &str, what: &str) -> Result<Option<SourceKey>> {
    match t.get(name) {
        None => Ok(None),
        Some(item) => {
            let text = item
                .as_str()
                .with_context(|| format!("{what}: `{name}` must be a source key"))?;
            SourceKey::parse(text)
                .map(Some)
                .with_context(|| format!("{what}: `{name}`"))
        }
    }
}

struct Parsed {
    songs: Vec<Song>,
    albums: Vec<Album>,
    lyrics: Vec<String>,
    removed: Vec<Removed>,
    providers: provider::Config,
    hooks: Vec<Hook>,
    clean: Settings,
    settings: crate::settings::Settings,
}

/// The keys a table lists, each parsed.
fn keys_of(t: &Table, what: &str) -> Result<Vec<SourceKey>> {
    t.get("sources")
        .and_then(Item::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let text = item
                .as_str()
                .with_context(|| format!("{what}: sources must be strings"))?;
            SourceKey::parse(text).with_context(|| what.to_string())
        })
        .collect()
}

fn parse(doc: &DocumentMut) -> Result<Parsed> {
    let lyrics = doc
        .get("defaults")
        .and_then(|d| d.get("lyrics"))
        .and_then(Item::as_array)
        .map_or_else(
            || vec!["en".to_string()],
            |a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            },
        );
    let mut songs = Vec::new();
    let mut owner: BTreeMap<SourceKey, usize> = BTreeMap::new();
    for (n, t) in tables(doc, "song").enumerate() {
        let what = format!("song {}", n + 1);
        let mut sources = Vec::new();
        for key in keys_of(t, &what)? {
            if let Some(other) = owner.insert(key.clone(), n) {
                bail!(
                    "{key} is listed by both song {} and song {}",
                    other + 1,
                    n + 1
                );
            }
            sources.push(key);
        }
        let lyrics = match t.get("lyrics") {
            None => None,
            Some(item) if item.as_bool() == Some(false) => Some(LyricsPin::None),
            Some(_) => key_at(t, "lyrics", &what)?.map(LyricsPin::From),
        };
        let song = Song {
            album: key_at(t, "album", &what)?,
            track: t
                .get("track")
                .and_then(Item::as_integer)
                .and_then(|i| u32::try_from(i).ok()),
            audio: key_at(t, "audio", &what)?,
            cover: key_at(t, "cover", &what)?,
            lyrics,
            lyrics_offset_ms: t
                .get("lyrics_offset_ms")
                .and_then(Item::as_integer)
                .unwrap_or(0),
            tags: tags_of(t.get("tags")),
            sources,
        };
        let pinned = [
            song.audio.as_ref(),
            song.cover.as_ref(),
            match &song.lyrics {
                Some(LyricsPin::From(k)) => Some(k),
                _ => None,
            },
        ];
        if let Some(stray) = pinned.into_iter().flatten().find(|k| !song.has(k)) {
            bail!("{what} pins {stray}, which its sources do not list");
        }
        songs.push(song);
    }
    let mut albums = Vec::new();
    for (n, t) in tables(doc, "album").enumerate() {
        let what = format!("album {}", n + 1);
        let Some(source) = key_at(t, "source", &what)? else {
            bail!("{what} names no source");
        };
        albums.push(Album {
            source,
            tracks: t
                .get("tracks")
                .and_then(Item::as_integer)
                .and_then(|i| u32::try_from(i).ok()),
            tags: tags_of(t.get("tags")),
        });
    }
    let mut removed = Vec::new();
    for (n, t) in tables(doc, "removed").enumerate() {
        removed.push(Removed {
            sources: keys_of(t, &format!("removed song {}", n + 1))?,
            note: t
                .get("note")
                .and_then(Item::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(Parsed {
        songs,
        albums,
        lyrics,
        removed,
        providers: provider::read(doc)?,
        hooks: hooks::read(doc)?,
        clean: clean_of(doc)?,
        settings: crate::settings::read(doc)?,
    })
}

fn tables<'a>(doc: &'a DocumentMut, key: &str) -> impl Iterator<Item = &'a Table> {
    doc.get(key)
        .and_then(Item::as_array_of_tables)
        .into_iter()
        .flat_map(ArrayOfTables::iter)
}

fn key_array(keys: &[SourceKey]) -> Item {
    value(keys.iter().map(ToString::to_string).collect::<Array>())
}

fn listed_keys(t: &Table) -> Vec<String> {
    t.get("sources")
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[allow(clippy::too_many_lines)]
fn apply(doc: &mut DocumentMut, edit: &Edit) -> Result<()> {
    match edit {
        Edit::Add { sources, album } => {
            let wanted: Vec<String> = sources.iter().map(ToString::to_string).collect();
            lift(doc, &wanted);
            let list = doc
                .entry("song")
                .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
                .as_array_of_tables_mut()
                .context("`song` is not a list of tables")?;
            let found = list
                .iter_mut()
                .find(|t| listed_keys(t).iter().any(|k| wanted.contains(k)));
            let song = if let Some(song) = found {
                let mut keys = listed_keys(song);
                for k in &wanted {
                    if !keys.contains(k) {
                        keys.push(k.clone());
                    }
                }
                let keys: Vec<SourceKey> = keys
                    .iter()
                    .filter_map(|k| SourceKey::parse(k).ok())
                    .collect();
                song.insert("sources", key_array(&keys));
                song
            } else {
                let mut song = Table::new();
                song.insert("sources", key_array(sources));
                list.push(song);
                list.iter_mut().last().context("a song was just added")?
            };
            if let Some((key, track)) = album
                && song.get("album").is_none()
            {
                song.insert("album", value(key.to_string()));
                song.insert("track", value(i64::from(*track)));
            }
        }
        Edit::AddAlbum { source, tracks } => {
            let list = doc
                .entry("album")
                .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
                .as_array_of_tables_mut()
                .context("`album` is not a list of tables")?;
            let text = source.to_string();
            if !list
                .iter()
                .any(|t| t.get("source").and_then(Item::as_str) == Some(text.as_str()))
            {
                let mut album = Table::new();
                album.insert("source", value(text));
                if let Some(n) = tracks {
                    album.insert("tracks", value(i64::from(*n)));
                }
                let mut tags = Table::new();
                for key in ["album", "album_artist", "date", "genre"] {
                    tags.insert(key, value(""));
                }
                album.insert("tags", Item::Table(tags));
                list.push(album);
            }
        }
        Edit::Rename { from, to } => {
            let (from, to) = (from.to_string(), to.to_string());
            if let Some(list) = doc.get_mut("song").and_then(Item::as_array_of_tables_mut) {
                for song in list.iter_mut() {
                    let keys = listed_keys(song);
                    if keys.contains(&from) {
                        let keys: Vec<SourceKey> = keys
                            .iter()
                            .map(|k| if *k == from { to.clone() } else { k.clone() })
                            .filter_map(|k| SourceKey::parse(&k).ok())
                            .collect();
                        song.insert("sources", key_array(&keys));
                        for pin in ["audio", "cover", "lyrics"] {
                            if song.get(pin).and_then(Item::as_str) == Some(from.as_str()) {
                                song.insert(pin, value(to.clone()));
                            }
                        }
                    }
                }
            }
        }
        Edit::Drop(key) => {
            let text = key.to_string();
            if let Some(list) = doc.get_mut("song").and_then(Item::as_array_of_tables_mut) {
                for song in list.iter_mut() {
                    let keys = listed_keys(song);
                    if keys.contains(&text) {
                        let keys: Vec<SourceKey> = keys
                            .iter()
                            .filter(|k| **k != text)
                            .filter_map(|k| SourceKey::parse(k).ok())
                            .collect();
                        song.insert("sources", key_array(&keys));
                        for pin in ["audio", "cover", "lyrics"] {
                            if song.get(pin).and_then(Item::as_str) == Some(text.as_str()) {
                                song.remove(pin);
                            }
                        }
                    }
                }
            }
        }
        Edit::Tag { key, tags } => {
            let text = key.to_string();
            let song = doc
                .get_mut("song")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|list| list.iter_mut().find(|t| listed_keys(t).contains(&text)));
            if let Some(song) = song {
                set_tags(song, tags);
            }
        }
        Edit::Remove { key, note } => {
            let Some(song) = take(doc, "song", key) else {
                return Ok(());
            };
            tables_mut(doc, "removed")?.push(tombstone(song, note));
        }
        Edit::Restore(key) => {
            let Some(mut song) = take(doc, "removed", key) else {
                return Ok(());
            };
            song.remove("note");
            unplace(&mut song);
            let listed: BTreeSet<String> = tables(doc, "song").flat_map(listed_keys).collect();
            let keys: Vec<SourceKey> = listed_keys(&song)
                .iter()
                .filter(|k| !listed.contains(*k))
                .filter_map(|k| SourceKey::parse(k).ok())
                .collect();
            song.insert("sources", key_array(&keys));
            for pin in ["audio", "cover", "lyrics"] {
                let stray = song
                    .get(pin)
                    .and_then(Item::as_str)
                    .is_some_and(|k| listed.contains(k));
                if stray {
                    song.remove(pin);
                }
            }
            tables_mut(doc, "song")?.push(song);
        }
        Edit::Rewrite { songs, new } => {
            let list = tables_mut(doc, "song")?;
            let found: Vec<(usize, &Rewritten)> = songs
                .iter()
                .filter_map(|(key, to)| {
                    let text = key.to_string();
                    let at = list.iter().position(|t| listed_keys(t).contains(&text))?;
                    Some((at, to))
                })
                .collect();
            let mut gone = Vec::new();
            for (at, to) in &found {
                let Some(song) = list.get_mut(*at) else {
                    continue;
                };
                match to {
                    Rewritten::Table(table) => {
                        let (decor, at) = (song.decor().clone(), song.position());
                        *song = table.0.clone();
                        unplace(song);
                        song.set_position(at);
                        *song.decor_mut() = decor;
                    }
                    Rewritten::Removed(note) => gone.push((*at, note.clone())),
                }
            }
            gone.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
            let mut removed = Vec::new();
            for (at, note) in gone {
                if let Some(song) = list.get(at).cloned() {
                    list.remove(at);
                    removed.push(tombstone(song, &note));
                }
            }
            for table in new {
                let mut table = table.0.clone();
                table.decor_mut().clear();
                unplace(&mut table);
                list.push(table);
            }
            if !removed.is_empty() {
                let list = tables_mut(doc, "removed")?;
                for t in removed.into_iter().rev() {
                    list.push(t);
                }
            }
        }
    }
    Ok(())
}

/// A song's table as `[[removed]]` keeps it, its note first.
fn tombstone(mut song: Table, note: &str) -> Table {
    let mut gone = Table::new();
    gone.insert("note", value(note));
    song.remove("note");
    gone.extend(song.iter().map(|(k, v)| (k.to_string(), v.clone())));
    unplace(&mut gone);
    gone
}

/// A table moved to another place in the file, written where it lands
/// rather than where it was read: kept positions would split an array
/// of tables in two.
fn unplace(table: &mut Table) {
    table.set_position(None);
    for (_, item) in table.iter_mut() {
        if let Some(t) = item.as_table_mut() {
            unplace(t);
        }
    }
}

fn tables_mut<'a>(doc: &'a mut DocumentMut, key: &str) -> Result<&'a mut ArrayOfTables> {
    doc.entry(key)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .with_context(|| format!("`{key}` is not a list of tables"))
}

/// The table of the `list` listing `key`, taken out of it.
fn take(doc: &mut DocumentMut, list: &str, key: &SourceKey) -> Option<Table> {
    let text = key.to_string();
    let tables = doc.get_mut(list)?.as_array_of_tables_mut()?;
    let at = tables.iter().position(|t| listed_keys(t).contains(&text))?;
    let table = tables.get(at)?.clone();
    tables.remove(at);
    if tables.is_empty() {
        doc.remove(list);
    }
    Some(table)
}

/// Each key in `keys` taken out of the removed songs listing it; a
/// removed song left with no key goes.
fn lift(doc: &mut DocumentMut, keys: &[String]) {
    let Some(list) = doc
        .get_mut("removed")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return;
    };
    for gone in list.iter_mut() {
        let listed = listed_keys(gone);
        if listed.iter().any(|k| keys.contains(k)) {
            let left: Vec<SourceKey> = listed
                .iter()
                .filter(|k| !keys.contains(k))
                .filter_map(|k| SourceKey::parse(k).ok())
                .collect();
            gone.insert("sources", key_array(&left));
        }
    }
    list.retain(|t| !listed_keys(t).is_empty());
    if list.is_empty() {
        doc.remove("removed");
    }
}

/// Set `tags` in a song table's `[song.tags]`, over any it sets under
/// another spelling of their names; a tag with no values is cleared.
pub fn set_tags(song: &mut Table, tags: &[(String, Vec<String>)]) {
    let table = song
        .entry("tags")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_like_mut();
    // A `tags` that is no table at all is left as the user wrote it.
    let Some(table) = table else {
        return;
    };
    for (name, values) in tags {
        let field = tags::vorbis_key(name);
        let spellings: Vec<String> = table
            .iter()
            .map(|(k, _)| k.to_string())
            .filter(|k| tags::vorbis_key(k) == field)
            .collect();
        for k in spellings {
            table.remove(&k);
        }
        let item = match values.as_slice() {
            [] if !TAG_TEMPLATE.contains(&name.as_str()) => continue,
            [] => value(""),
            [one] => value(one.as_str()),
            many => value(many.iter().map(String::as_str).collect::<Array>()),
        };
        table.insert(name, item);
    }
}

/// Every song's `tags` as a table of its own, never inline, with the
/// template's tags it lacks added empty, and every cleaning switch the
/// file lacks added on. Whether anything changed.
fn normalize(doc: &mut DocumentMut) -> bool {
    let mut changed = clean_template(doc);
    let Some(list) = doc.get_mut("song").and_then(Item::as_array_of_tables_mut) else {
        return changed;
    };
    for song in list.iter_mut() {
        let mut tags = match song.remove("tags") {
            Some(Item::Table(t)) => t,
            Some(item) => {
                changed = true;
                match item.into_table() {
                    Ok(t) => t,
                    // Not a table at all: left as the user wrote it.
                    Err(item) => {
                        song.insert("tags", item);
                        continue;
                    }
                }
            }
            None => {
                changed = true;
                Table::new()
            }
        };
        for key in TAG_TEMPLATE {
            if !tags.contains_key(key) {
                tags.insert(key, value(""));
                changed = true;
            }
        }
        song.insert("tags", Item::Table(tags));
    }
    changed
}

/// The cleaning switches the file sets. A key naming no rule is kept,
/// and reported, so a misspelled one is not silently ignored.
fn clean_of(doc: &DocumentMut) -> Result<Settings> {
    let mut settings = Settings::default();
    let Some(tiers) = doc.get("clean").and_then(Item::as_table_like) else {
        return Ok(settings);
    };
    for (tier, item) in tiers.iter() {
        let Some(rules) = item.as_table_like() else {
            settings.unknown.push(format!("clean.{tier}"));
            continue;
        };
        for (key, item) in rules.iter() {
            let name = format!("{tier}.{key}");
            let on = item
                .as_bool()
                .with_context(|| format!("`clean.{name}` must be true or false"))?;
            if !settings.set(&name, on) {
                settings.unknown.push(format!("clean.{name}"));
            }
        }
    }
    Ok(settings)
}

/// Each tier's `[clean.<tier>]`, with every switch it lacks added on,
/// placed after `[defaults]` so it is found at the top of the file.
fn clean_template(doc: &mut DocumentMut) -> bool {
    let position = doc
        .get("defaults")
        .and_then(Item::as_table)
        .and_then(Table::position);
    let root = doc.as_table_mut();
    let mut changed = false;
    if !root.contains_key("clean") {
        let mut t = Table::new();
        t.set_implicit(true);
        t.set_position(position);
        root.insert("clean", Item::Table(t));
        // A new table without a position prints after the one before it
        // in key order, so `clean` moves up beside `defaults`.
        let rank = |k: &str| match k {
            "version" => 0,
            "defaults" => 1,
            "clean" => 2,
            _ => 3,
        };
        root.sort_values_by(|a, _, b, _| rank(a.get()).cmp(&rank(b.get())));
        changed = true;
    }
    let Some(clean) = root.get_mut("clean").and_then(Item::as_table_mut) else {
        return changed;
    };
    for (tier, about) in clean::TIERS {
        if !clean.contains_key(tier) {
            let mut t = Table::new();
            t.set_position(position);
            t.decor_mut().set_prefix(format!("\n# {about}.\n"));
            clean.insert(tier, Item::Table(t));
            changed = true;
        }
        let Some(t) = clean.get_mut(tier).and_then(Item::as_table_mut) else {
            continue;
        };
        for (_, key) in clean::switches().filter(|(t, _)| *t == tier) {
            if !t.contains_key(key) {
                t.insert(key, value(true));
                changed = true;
            }
        }
    }
    changed
}

/// Strings, numbers, and lists of either; anything else is skipped, and
/// an empty value sets nothing.
fn tags_of(item: Option<&Item>) -> Tags {
    let Some(t) = item.and_then(Item::as_table_like) else {
        return Vec::new();
    };
    let scalar = |v: &Value| match v {
        Value::String(s) => Some(s.value().trim().to_string()).filter(|s| !s.is_empty()),
        Value::Integer(i) => Some(i.value().to_string()),
        Value::Float(f) => Some(f.value().to_string()),
        _ => None,
    };
    t.iter()
        .filter_map(|(key, item)| {
            let values: Vec<String> = match item.as_value()? {
                Value::Array(list) => list.iter().filter_map(scalar).collect(),
                v => scalar(v).into_iter().collect(),
            };
            (!values.is_empty()).then(|| (key.to_string(), values))
        })
        .collect()
}

#[cfg(test)]
mod tests;
