//! The song list, `songs.toml`: every song the library holds, each with
//! the sources it may be made from, the tags set over what they offer,
//! and any aspect pinned to one source.
//!
//! The file is edited in place, the way Cargo keeps its lock file:
//! comments, order and keys this version does not know survive a write.
//! `version` names the format. A file of another version is refused,
//! never rewritten by a program that cannot read it.
//!
//! There is no song ID: a song is the sources it lists, and is named by
//! the first of them that belongs to it alone ([`Song::id`]). A file
//! fetched or dropped in belongs to one song only; a record a lookup
//! keeps, an LRCLIB record or a MusicBrainz recording, is data any song
//! of that recording may list, as two releases of one recording share
//! its lyrics. A key that does not parse, a file two songs list, and a
//! pin outside the song's sources stop the run before anything is
//! written, rather than a song being silently dropped and its file
//! deleted.
//!
//! A list in which songs share a record is format version 2, which an
//! older muman refuses rather than misreads; any other is written as
//! version 1, which this one reads as well.
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
//! and it gives each song the common `tags.<name>` keys it lacks, added
//! empty, so the file shows what can be set. Tags are dotted keys last
//! in their song, not a `[song.tags]` table, so each song reads as one
//! block between blank lines; a `[song.tags]` written by hand is read
//! the same and written back dotted. Anything a song gains later takes
//! a dotted prefix of its own in the same block, never a table. A key under
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
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Key, Table, Value, value};

use crate::atomic::{self, Lock};
use crate::clean::{self, Settings};
use crate::dirs::MANIFEST;
use crate::hooks::{self, Hook};
use crate::provider;
use crate::source::SourceKey;
use crate::tags;

/// The newest format this version reads, written only when songs share
/// a record. Adding a key is not a new version; changing what an
/// existing key means is.
pub const VERSION: i64 = 2;

/// The format written when no record is shared, which every version
/// since 0.1 reads.
const PLAIN: i64 = 1;

/// Whether songs may share `key`: a record a lookup keeps, not a file of
/// one song's.
#[must_use]
pub fn shareable(key: &SourceKey) -> bool {
    provider::is_kept(key)
}

/// The key that names a song listing `keys`: the first only it lists,
/// else the first.
#[must_use]
pub fn id_of(keys: &[SourceKey]) -> Option<&SourceKey> {
    keys.iter().find(|k| !shareable(k)).or_else(|| keys.first())
}

/// How to use the file, kept at its top; rewritten whenever it differs
/// from an earlier version's.
const HEADER: &str = "\
# muman's song list: every song in the library and what it is made from.
# muman adds a [[song]] for each song it keeps. Edit by hand, then run
# `muman sync` to apply: it fetches any missing source, rebuilds every
# changed song and deletes the library copy of anything no longer listed;
# a song you delete here goes under [[removed]], as `muman remove` puts it.
#
#   sources:  every file the song may be made from: `<site>:<id>` for
#             what yt-dlp fetched, as `youtube.com:<id>`, or `lrclib:<id>`,
#             `musicbrainz:<id>` or `manual:<path>` under sources/manual. muman picks the best audio, cover, lyrics and
#             tags by measuring each.
#   Pin one:  audio = \"<source>\", cover = \"<source>\", lyrics = \"<source>\";
#             lyrics = false for none. lyrics_offset = \"120 ms\" moves
#             them later, a negative time earlier.
#   Tags:     fill in tags.<name>; an empty value keeps what the sources
#             offer. Any other Vorbis comment name works too; a list sets
#             several. `muman status` shows what was picked and why.
#   Cleaning: [clean.*] turns each rule cleaning what the sources offer on
#             or off; `muman status` names the rules that changed a tag.
#             Tags set by hand are never cleaned.
#   Held:     held.\"<source>\" records the audio, cover, lyrics and tags
#             each source held when the song was built. A fetched source
#             that holds otherwise after a fetch again leaves the song as
#             built until `muman sync --accept`; a file of your own is
#             followed.
#   Removed:  `muman remove` keeps a song under [[removed]], so nothing
#             lists it again; delete the entry to let it back.
#   Lookups:  a song looks other sources up by [[trigger]] (from, find,
#             when), replacing the defaults; [providers.<name>] sets
#             enabled, concurrency, recheck = \"30 days\", per_run, and the url
#             of lrclib and musicbrainz.
#   Hooks:    [[hook]] runs a command, on = \"written\" for each song file
#             ({path}), \"changed\" once a run wrote or removed any.
#   Settings: [library], [audio], [quality.*], [ytdlp] and [history] lay
#             out and name the library, set encoding, ranking and fetching,
#             and size `undo`; sizes, bitrates, times and shares take a
#             unit, as \"2 GiB\" or \"1 %\". `edition` names the settings'
#             names and defaults the file was written to: muman renames
#             old names itself, and `muman sync --update-defaults` moves
#             settings still at an older default to the current.
#
# Other keys muman does not know are kept. Leave `version` as it is.
";

/// How the last line of every version's header begins.
const HEADER_END: &str = "# Other keys muman does not know are kept.";

/// The first line every version writes atop the file.
const HEADER_START: &str = "# muman's song list";

/// A new home's song list: the settings, each written out at its
/// default, as the reference the docs point to.
pub const NEW: &str = include_str!("manifest/new.toml");

/// The tags every song's `tags.<name>` keys offer to fill in.
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
    /// What each source held when the song was built.
    pub held: BTreeMap<SourceKey, crate::held::Held>,
}

impl Song {
    #[must_use]
    pub fn has(&self, key: &SourceKey) -> bool {
        self.sources.contains(key)
    }

    /// `from` named `to` wherever the song names it, as a file that moved.
    pub fn rename(&mut self, from: &SourceKey, to: &SourceKey) {
        let follow = |k: &mut SourceKey| {
            if k == from {
                k.clone_from(to);
            }
        };
        self.sources.iter_mut().for_each(follow);
        self.audio.iter_mut().for_each(follow);
        self.cover.iter_mut().for_each(follow);
        if let Some(LyricsPin::From(k)) = &mut self.lyrics {
            follow(k);
        }
        if let Some(held) = self.held.remove(from) {
            self.held.insert(to.clone(), held);
        }
    }

    /// The key that names this song alone, by [`id_of`].
    #[must_use]
    pub fn id(&self) -> Option<&SourceKey> {
        id_of(&self.sources)
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
    /// of `sources` no other song may share; its album and place are set
    /// when it has none.
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
    /// Tags set in the `tags` of the song listing `key`, over any it
    /// sets already under another spelling of their names.
    Tag { key: SourceKey, tags: Tags },
    /// The song listing `key` moved to `[[removed]]`, named by `note`.
    Remove { key: SourceKey, note: String },
    /// `key`, which no song lists, kept under `[[removed]]`, named by
    /// `note`, as a song taken out by hand.
    Tombstone { key: SourceKey, note: String },
    /// The removed song listing `key` listed again, without any file a
    /// song lists meanwhile.
    Restore(SourceKey),
    /// Songs edited together: the song listing each key replaced or
    /// removed, every one found before any changes, then new songs.
    Rewrite {
        songs: Vec<(SourceKey, Rewritten)>,
        new: Vec<SongTable>,
    },
    /// Every setting still at an earlier edition's default moved to the
    /// current one; see [`crate::settings::update`].
    UpdateDefaults,
    /// `from`, an extractor's name for a source, named `to` wherever it
    /// is written, its record kept: one key, renamed.
    Respell { from: SourceKey, to: SourceKey },
    /// What `key` holds recorded in the song listing it.
    Hold {
        key: SourceKey,
        held: crate::held::Held,
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
    /// Settings at the default of an earlier edition, which this one
    /// changed.
    pub stale_defaults: Vec<crate::settings::Stale>,
    /// The settings the file names by an old name, renamed when read and
    /// by the next write.
    pub renamed: Vec<crate::migrate::Renamed>,
    /// Each source key the file names by yt-dlp's extractor, as
    /// `youtube:<id>`, by the key the next write names by its site.
    pub respelled: BTreeMap<String, String>,
    edits: Vec<Edit>,
    stale: bool,
}

/// Refuse a home whose song list is gone while the library holds songs
/// muman wrote: read as listing nothing, it would remove every one, and a
/// list saved from it would list only what a command added. A song list
/// that lists no song removes them.
pub fn present(home: &Path) -> Result<()> {
    let file = home.join(MANIFEST);
    if file.exists() {
        return Ok(());
    }
    let written = crate::state::State::load(home).map_or(0, |s| s.outputs.len());
    if written == 0 {
        return Ok(());
    }
    Err(crate::change::Refused(format!(
        "{} is missing while the library holds {written} song(s) muman wrote: put it back, \
         or `muman undo` the run that removed it. A song list that lists no song removes them",
        file.display()
    ))
    .into())
}

impl Manifest {
    /// The list in `home`; an empty one when there is none yet.
    pub fn load(home: &Path) -> Result<Self> {
        let doc = read(home)?;
        let file = home.join(MANIFEST);
        let parsed = parse(&doc).with_context(|| format!("reading {}", file.display()))?;
        let stale = file.exists()
            && (with_header(&doc.to_string()).is_some()
                || normalize(&mut doc.clone())
                || !parsed.respelled.is_empty());
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
            stale_defaults: parsed.stale_defaults,
            renamed: parsed.renamed,
            respelled: parsed.respelled,
            edits: Vec::new(),
            stale,
        })
    }

    /// What the next write renames of the source keys, if anything.
    #[must_use]
    pub fn respelled_notice(&self) -> Option<String> {
        let (old, new) = self.respelled.iter().next()?;
        Some(format!(
            "{} source key(s) by their site, as {old} → {new}",
            self.respelled.len()
        ))
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

    /// The song table each of `ids` names, as the file holds it now, read
    /// once for them all.
    pub fn tables_by_id(&self, ids: &[SourceKey]) -> Result<BTreeMap<SourceKey, Table>> {
        let wanted: BTreeMap<String, &SourceKey> = ids.iter().map(|k| (k.to_string(), k)).collect();
        let doc = read(&self.dir)?;
        let mut found = BTreeMap::new();
        for table in tables(&doc, "song") {
            for key in listed_keys(table) {
                if let Some(id) = wanted.get(&key) {
                    found.entry((*id).clone()).or_insert_with(|| table.clone());
                }
            }
        }
        Ok(found)
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
        present(&self.dir)?;
        let mut doc = read(&self.dir)?;
        let now = parse(&doc)?;
        let changed: Vec<SourceKey> = expected
            .iter()
            .filter(|e| {
                !now.songs
                    .iter()
                    .find(|s| s.id().is_some() && s.id() == e.id())
                    .is_some_and(|s| s == *e)
            })
            .filter_map(|e| e.id().cloned())
            .collect();
        if !changed.is_empty() {
            return Ok(changed);
        }
        crate::migrate::rename_all(&mut doc, &crate::migrate::EDITIONS)?;
        respell(&mut doc, &spelled_now);
        for edit in &self.edits {
            apply(&mut doc, edit)?;
        }
        normalize(&mut doc);
        let parsed = parse(&doc)?;
        let shared = parsed
            .songs
            .iter()
            .flat_map(|s| &s.sources)
            .filter(|k| shareable(k));
        let mut seen = BTreeSet::new();
        let version = if shared.into_iter().all(|k| seen.insert(k)) {
            PLAIN
        } else {
            VERSION
        };
        if doc.get("version").and_then(Item::as_integer) != Some(version) {
            doc["version"] = value(version);
        }
        let text = doc.to_string();
        let text = with_header(&text).unwrap_or(text);
        atomic::write(&self.dir, MANIFEST, text.as_bytes())?;
        saw(&self.dir, Some(&text));
        self.edits.clear();
        (self.songs, self.albums, self.lyrics, self.removed) =
            (parsed.songs, parsed.albums, parsed.lyrics, parsed.removed);
        (self.providers, self.hooks, self.clean) = (parsed.providers, parsed.hooks, parsed.clean);
        (self.settings, self.stale_defaults) = (parsed.settings, parsed.stale_defaults);
        (self.renamed, self.respelled) = (parsed.renamed, parsed.respelled);
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
    let comments = text.lines().take_while(|l| l.starts_with('#')).count();
    // Every version's header ends on this line; a comment below it is the
    // user's, even with no blank line between.
    let lead = text
        .lines()
        .take(comments)
        .position(|l| l.starts_with(HEADER_END))
        .map_or(comments, |at| at + 1);
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

/// The song list as this process last read or wrote it, by home, `None`
/// for one missing: what a run worked from, which [`crate::history`]
/// records as the list the run left, so a hand edit made while it ran
/// and never read by it is no part of it.
static SEEN: std::sync::Mutex<BTreeMap<PathBuf, Option<String>>> =
    std::sync::Mutex::new(BTreeMap::new());

/// Note the song list in `home` as read or written now.
pub fn saw(home: &Path, text: Option<&str>) {
    SEEN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(home.to_path_buf(), text.map(str::to_string));
}

/// The song list in `home` as last read or written since [`unsee`], if it
/// was.
#[must_use]
pub fn seen(home: &Path) -> Option<Option<String>> {
    SEEN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(home)
        .cloned()
}

/// Forget what was seen of the song list in `home`.
pub fn unsee(home: &Path) {
    SEEN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(home);
}

fn read(dir: &Path) -> Result<DocumentMut> {
    let file = dir.join(MANIFEST);
    let raw = match std::fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    saw(dir, raw.as_deref());
    // Windows editors may add a byte-order mark and CRLF endings; the
    // file is written back with neither, rather than with both mixed.
    let text = raw.map_or_else(
        || NEW.to_string(),
        |t| t.trim_start_matches('\u{feff}').replace("\r\n", "\n"),
    );
    let doc: DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML", file.display()))?;
    match doc.get("version").and_then(Item::as_integer) {
        Some(PLAIN..=VERSION) => {}
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
    stale_defaults: Vec<crate::settings::Stale>,
    renamed: Vec<crate::migrate::Renamed>,
    respelled: BTreeMap<String, String>,
}

/// The keys a table lists, each parsed.
fn keys_of(t: &Table, what: &str) -> Result<Vec<SourceKey>> {
    if t.get("sources").is_some_and(|s| s.as_array().is_none()) {
        bail!("{what}: sources must be a list, as sources = [\"manual:<path>\"]");
    }
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

/// The keys song `n`'s table lists, refused when another song lists one
/// of its files: `owner` has every file's song so far.
fn owned_keys(
    t: &Table,
    what: &str,
    n: usize,
    owner: &mut BTreeMap<SourceKey, usize>,
) -> Result<Vec<SourceKey>> {
    let keys = keys_of(t, what)?;
    for key in keys.iter().filter(|k| !shareable(k)) {
        if let Some(other) = owner.insert(key.clone(), n) {
            bail!(
                "{key} is listed by both song {} and song {}",
                other + 1,
                n + 1
            );
        }
    }
    Ok(keys)
}

/// A song's `lyrics_offset`, in milliseconds.
fn lyrics_offset(t: &Table, what: &str) -> Result<i64> {
    let Some(item) = t.get("lyrics_offset") else {
        return Ok(0);
    };
    let time: crate::units::Time = item
        .as_str()
        .with_context(|| format!("{what}: `lyrics_offset` must be a time, such as \"120 ms\""))?
        .parse()
        .map_err(|e| anyhow::anyhow!("{what}: `lyrics_offset`: {e}"))?;
    if time.0.unsigned_abs() > MAX_OFFSET_MS {
        bail!("{what}: `lyrics_offset` moves lyrics more than an hour, longer than any song");
    }
    Ok(time.0)
}

/// The most lyrics are moved, an hour either way.
const MAX_OFFSET_MS: u64 = 3_600_000;

/// What `doc` holds, read as the current edition writes it.
fn parse(doc: &DocumentMut) -> Result<Parsed> {
    let mut current = doc.clone();
    let renamed = crate::migrate::rename_all(&mut current, &crate::migrate::EDITIONS)?;
    let respelled = respell(&mut current, &spelled_now);
    let doc = &current;
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
        let sources = owned_keys(t, &what, n, &mut owner)?;
        if sources.is_empty() {
            bail!("{what} lists no source: give it one, or delete it");
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
            lyrics_offset_ms: lyrics_offset(t, &what)?,
            tags: tags_of(t.get("tags")),
            held: crate::held::read(t, &sources, &what)?,
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
        stale_defaults: crate::settings::stale(doc, &crate::settings::EDITIONS),
        renamed,
        respelled,
    })
}

/// How a key written as `text` is written now, where that differs: an
/// extractor's name for a site [`crate::source::SITES`] names, as
/// `youtube:<id>`, is its domain's.
fn spelled_now(text: &str) -> Option<String> {
    let key = SourceKey::parse(text).ok().filter(|k| k.site().is_some())?;
    Some(key.to_string()).filter(|now| now != text)
}

/// Write each source key in `doc` as `now` spells it, where it does:
/// in each song's, album's and removed song's keys, and its records.
/// Returns each key written otherwise, by the key written.
fn respell(
    doc: &mut DocumentMut,
    now: &dyn Fn(&str) -> Option<String>,
) -> BTreeMap<String, String> {
    let mut written = BTreeMap::new();
    let string = |v: &mut Value, written: &mut BTreeMap<String, String>| {
        let Some((old, new)) = v
            .as_str()
            .and_then(|old| Some((old.to_string(), now(old)?)))
        else {
            return;
        };
        let decor = v.decor().clone();
        *v = Value::from(new.as_str());
        *v.decor_mut() = decor;
        written.insert(old, new);
    };
    for list in ["song", "album", "removed"] {
        let Some(tables) = doc.get_mut(list).and_then(Item::as_array_of_tables_mut) else {
            continue;
        };
        for t in tables.iter_mut() {
            if let Some(keys) = t.get_mut("sources").and_then(Item::as_array_mut) {
                keys.iter_mut().for_each(|v| string(v, &mut written));
            }
            for name in ["source", "album", "audio", "cover", "lyrics"] {
                if let Some(v) = t.get_mut(name).and_then(Item::as_value_mut) {
                    string(v, &mut written);
                }
            }
            let Some(held) = t.get_mut("held").and_then(Item::as_table_like_mut) else {
                continue;
            };
            let entries: Vec<(String, Item)> = held
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect();
            if entries.iter().all(|(k, _)| now(k).is_none()) {
                continue;
            }
            held.clear();
            for (k, v) in entries {
                if let Some(new) = now(&k) {
                    held.insert(&new, v);
                    written.insert(k, new);
                } else {
                    held.insert(&k, v);
                }
            }
        }
    }
    written
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
            // A song is found by what only it lists; a record it shares
            // with another names neither.
            let own: Vec<String> = sources
                .iter()
                .filter(|k| !shareable(k))
                .map(ToString::to_string)
                .collect();
            lift(doc, &own);
            let list = doc
                .entry("song")
                .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
                .as_array_of_tables_mut()
                .context("`song` is not a list of tables")?;
            let found = list
                .iter_mut()
                .find(|t| listed_keys(t).iter().any(|k| own.contains(k)));
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
                        if let Ok(from) = SourceKey::parse(&from) {
                            crate::held::unset(song, &from);
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
                        crate::held::unset(song, key);
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
        Edit::Tombstone { key, note } => {
            let mut song = Table::new();
            song.insert("sources", key_array(std::slice::from_ref(key)));
            tables_mut(doc, "removed")?.push(tombstone(song, note));
        }
        Edit::Restore(key) => {
            let Some(mut song) = take(doc, "removed", key) else {
                return Ok(());
            };
            let removed = song.clone();
            song.remove("note");
            unplace(&mut song);
            let listed: BTreeSet<String> = tables(doc, "song").flat_map(listed_keys).collect();
            let taken =
                |k: &str| listed.contains(k) && SourceKey::parse(k).is_ok_and(|k| !shareable(&k));
            let keys: Vec<SourceKey> = listed_keys(&song)
                .iter()
                .filter(|k| !taken(k))
                .filter_map(|k| SourceKey::parse(k).ok())
                .collect();
            if keys.is_empty() {
                // Every source of it is another song's now: nothing to list.
                tables_mut(doc, "removed")?.push(removed);
                return Ok(());
            }
            song.insert("sources", key_array(&keys));
            for pin in ["audio", "cover", "lyrics"] {
                let stray = song.get(pin).and_then(Item::as_str).is_some_and(taken);
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
        Edit::UpdateDefaults => {
            crate::settings::update(doc, &crate::settings::EDITIONS);
        }
        Edit::Respell { from, to } => {
            let (from, to) = (from.to_string(), to.to_string());
            respell(doc, &|k| (k == from).then(|| to.clone()));
        }
        Edit::Hold { key, held } => {
            let text = key.to_string();
            if let Some(song) = doc
                .get_mut("song")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|l| l.iter_mut().find(|t| listed_keys(t).contains(&text)))
            {
                crate::held::set(song, key, held);
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

/// Set `tags` in a song table's `tags`, over any it sets under
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
        let item = match values.as_slice() {
            [] if !TAG_TEMPLATE.contains(&name.as_str()) => {
                for k in &spellings {
                    table.remove(k);
                }
                continue;
            }
            [] => value(""),
            [one] => value(one.as_str()),
            many => value(many.iter().map(String::as_str).collect::<Array>()),
        };
        // Setting what a tag holds already, however it is spelled, changes
        // nothing.
        if let [only] = spellings.as_slice()
            && table.get(only).and_then(item_values) == item_values(&item)
        {
            continue;
        }
        for k in &spellings {
            table.remove(k);
        }
        table.insert(name, item);
    }
}

/// The text values a tag's item holds, one or a list.
fn item_values(item: &Item) -> Option<Vec<&str>> {
    match item.as_value()? {
        Value::String(s) => Some(vec![s.value().as_str()]),
        Value::Array(a) => a.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

/// Every song's, album's and removed song's `tags` as dotted keys, last
/// in its table; each song's with the template's tags it lacks added
/// empty; every cleaning switch the file lacks added on; and every
/// setting it lacks added at its default. Whether anything changed.
fn normalize(doc: &mut DocumentMut) -> bool {
    let mut changed = clean_template(doc);
    changed |= crate::settings::fill(doc, &crate::settings::EDITIONS);
    for (list, template) in [("song", true), ("album", false), ("removed", false)] {
        let Some(list) = doc.get_mut(list).and_then(Item::as_array_of_tables_mut) else {
            continue;
        };
        for table in list.iter_mut() {
            changed |= dot_tags(table, template);
            if template {
                changed |= drop_stale_held(table);
            }
        }
    }
    changed
}

/// `held` records of sources the song no longer lists taken out, as a
/// source moved to another song leaves; a removed song keeps its own
/// for `restore`. Whether any was.
fn drop_stale_held(song: &mut Table) -> bool {
    let listed = listed_keys(song);
    let Some(held) = song.get_mut("held").and_then(Item::as_table_like_mut) else {
        return false;
    };
    let stale: Vec<String> = held
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !listed.contains(k))
        .collect();
    for key in &stale {
        held.remove(key);
    }
    let empty = held.is_empty();
    if empty && !stale.is_empty() {
        song.remove("held");
    }
    !stale.is_empty()
}

/// `table`'s `tags` as `tags.<name>` lines after its other keys, so a
/// song reads as one block; a comment above a `[song.tags]` header moves
/// above the first. With `template`, the template's tags it lacks are
/// added empty. Whether anything changed.
fn dot_tags(table: &mut Table, template: bool) -> bool {
    let last = table.iter().last().is_some_and(|(k, _)| k == "tags");
    let (key, mut tags) = match table.remove_entry("tags") {
        Some((key, Item::Table(t))) if t.is_dotted() => (key, t),
        Some((_, Item::Table(t))) => (Key::new("tags"), t),
        Some((key, item)) => match item.into_table() {
            Ok(t) => (Key::new("tags"), t),
            // Not a table at all: left as the user wrote it.
            Err(item) => {
                table.insert_formatted(&key, item);
                return false;
            }
        },
        None if template => (Key::new("tags"), Table::new()),
        None => return false,
    };
    let mut changed = !last || !tags.is_dotted();
    if !tags.is_dotted() {
        let above = decor_prefix(tags.decor())
            .trim_start_matches('\n')
            .to_string();
        tags.decor_mut().clear();
        tags.set_position(None);
        tags.set_dotted(true);
        let first = tags.iter().next().map(|(k, _)| k.to_string());
        if let Some(mut first) = first.and_then(|k| tags.key_mut(&k))
            && !above.is_empty()
        {
            let own = decor_prefix(first.leaf_decor())
                .trim_start_matches('\n')
                .to_string();
            first.leaf_decor_mut().set_prefix(format!("{above}{own}"));
        }
    }
    if template {
        for name in TAG_TEMPLATE {
            if !tags.contains_key(name) {
                tags.insert(name, value(""));
                changed = true;
            }
        }
    }
    table.insert_formatted(&key, Item::Table(tags));
    changed
}

fn decor_prefix(decor: &toml_edit::Decor) -> &str {
    decor.prefix().and_then(|p| p.as_str()).unwrap_or("")
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
            "version" | "edition" => 0,
            k if crate::settings::TABLES.contains(&k) => 1,
            "defaults" => 2,
            "clean" => 3,
            _ => 4,
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
