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
//! fetched or dropped in belongs to one song only. A record a lookup
//! keeps, an LRCLIB record or a MusicBrainz recording, is data any song
//! of that recording may list, as two releases of one recording share
//! its lyrics; and a picture of the user's is any song's it looks like,
//! as an album's songs share its cover. A key that does not parse, a
//! file two songs list, and a pin outside the song's sources stop the
//! run before anything is written, rather than a song being silently
//! dropped and its file deleted; so does a tag set under two spellings
//! of its name, as `tags.title` and `tags.Title`, rather than one value
//! silently winning.
//!
//! A list in which songs share a picture of the user's is format version
//! 3, and one in which they share only records a lookup keeps version 2:
//! an older muman refuses either rather than misreads it, saying it is
//! too old where it would otherwise call the list broken. Any other is
//! written as version 1, which this one reads as well.
//!
//! muman changes the file only by recording edits and applying them to
//! the file as it is when saved, under the folder's lock and through a
//! renamed part file, so two runs keep each other's songs and a crash
//! leaves the old list or the new. An edit made from a song as it was
//! read, by `remove`, `set` or `edit`, is saved only while that song is
//! still listed as it was: a song changed meanwhile, by hand or by
//! another run, refuses the save and nothing is written.
//!
//! Every save takes a [`Recorder`], so `undo` can take it back: the run
//! the save is part of, begun before it, told of the list once it is
//! written. The engine's run is that recorder, and as a run records
//! itself as it begins, a save stopped anywhere is one a record covers;
//! this crate, below the engine, names only the trait. [`Unrecorded`]
//! records nothing, for the list a new home starts with, before its
//! first run and holding nothing to take back, and for tests.
//!
//! A byte-order mark and CRLF line endings, as Windows editors may
//! write, are read past and written back as the file had them when
//! saved: CRLF when most of its lines end so, so a file an editor wrote
//! whole keeps its form and one muman never saw stays LF.
//!
//! Every write puts the usage comment atop the file, replacing one an
//! earlier version wrote, while the user's own comments below it stay;
//! and it gives each song the common `tags.<name>` keys it lacks, under
//! any spelling, added empty, so the file shows what can be set. A tag
//! muman sets is written under the one name the song list gives it
//! ([`tags::song_list_name`]), over its other spellings. Tags are
//! dotted keys last in their song, not a `[song.tags]` table, so each
//! song reads as one
//! block between blank lines; a `[song.tags]` written by hand is read
//! the same and written back dotted. Anything a song gains later takes
//! a dotted prefix of its own in the same block, never a table. A key under
//! `[tags.clean]` or `[lyrics.clean]` naming no rule is kept and warned
//! about, so a misspelled switch is never silently ignored.
//!
//! A `[[removed]]` key no song lists is a tombstone: a playlist or album
//! listing skips it, a manual file under it is not listed, and naming its
//! video alone lists it again. A key a song lists wins over a tombstone
//! that also names it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Key, Table, TableLike, Value, value};

use crate::atomic::{self, Lock};
use crate::clean::{self, Settings};
use crate::dirs::MANIFEST;
use crate::hooks::{self, Hook};
use crate::provider::Provider;
use crate::providers;
use crate::source::SourceKey;
use crate::tags;
pub use crate::torrent::Torrent;

/// The newest format this version reads, written only when a song lists
/// a torrent's file, a key no muman before it reads. Adding a key is not
/// a new version; changing what an existing key means is.
pub const VERSION: i64 = 4;

/// The format written when songs share a picture of the user's, and no
/// song lists a torrent's file.
const PICTURES: i64 = 3;

/// The format written when songs share a record a lookup keeps, and no
/// picture of the user's.
const SHARED: i64 = 2;

/// The format written when no source is shared, which every version
/// since 0.1 reads.
const PLAIN: i64 = 1;

/// Whether songs may share `key`: a record a lookup keeps, or a picture
/// of the user's, as one album's songs share its cover; not a song's own
/// file.
#[must_use]
pub fn shareable(key: &SourceKey) -> bool {
    crate::provider::is_kept(key) || is_picture(key)
}

fn is_picture(key: &SourceKey) -> bool {
    matches!(key, SourceKey::Manual(m)
        if crate::store::kind_of(m.path()) == Some(crate::store::Kind::Image))
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
#             `musicbrainz:<id>` or `manual:<path>` under sources/manual.
#             muman picks the best audio, cover, lyrics and tags by
#             measuring each.
#   Pin one:  audio = \"<source>\", cover = \"<source>\", lyrics = \"<source>\";
#             lyrics = false for none. lyrics_offset = \"120 ms\" moves
#             them later, a negative time earlier. release = \"<ID>\"
#             puts it on that MusicBrainz release of its recording.
#   Tags:     fill in tags.<name>; an empty value keeps what the sources
#             offer. Any other Vorbis comment name works too; a list sets
#             several. `muman status` shows what was picked and why.
#   Cleaning: [tags.clean.*] turns each rule cleaning what the sources
#             offer on or off, [lyrics.clean.*] each cleaning the lyrics;
#             `muman status` names the rules that changed a tag. Tags set
#             by hand are never cleaned. [[tags.from]] says what one
#             torrent's or channel's sources are not believed for.
#   Held:     held.\"<source>\" records the audio, cover, lyrics and tags
#             each source held when the song was built. A fetched source
#             that holds otherwise after a fetch again leaves the song as
#             built until `muman sync --accept`; a file of your own is
#             followed.
#   Removed:  `muman remove` keeps a song under [[removed]], so nothing
#             lists it again; delete the entry to let it back.
#   Lookups:  [providers.<name>] says what a song asks each provider
#             `until` it has, which songs it `skip`s, and how the service
#             is asked. A song sets lookups = false to ask none, or
#             lookups.<provider> = false to ask that one nothing.
#   Hooks:    [[hook]] runs a command, on = \"changed\", once a run wrote,
#             moved or removed any file.
#   Settings: [library], [tags], [release], [lyrics], [audio],
#             [loudness], [cover], [ytdlp], [torrent], [tools], [history]
#             and [sites.*] lay out and name the library, spell quotes
#             and hyphens in tags, choose releases, set encoding,
#             loudness, how [audio.pick] and [cover.pick] rank sources,
#             fetching and the programs run, size `undo` and describe
#             the sites fetched from; sizes, bitrates, times and shares
#             take a unit, as \"2 GiB\" or \"1 %\". `edition`
#             names the settings' names and defaults the file was written
#             to: muman renames old names itself, and `muman sync
#             --update-defaults` moves settings still at an older default
#             to the current.
#
# Other keys muman does not know are kept. Leave `version` as it is.
";

/// How the last line of every version's header begins.
const HEADER_END: &str = "# Other keys muman does not know are kept.";

/// The first line every version writes atop the file.
const HEADER_START: &str = "# muman's song list";

pub use crate::defaults::NEW;

/// The tags every song's `tags.<name>` keys offer to fill in.
pub const TAG_TEMPLATE: [&str; 6] = ["title", "artist", "album", "album_artist", "genre", "date"];

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
    /// The providers its `lookups` turns off.
    pub lookups_off: LookupsOff,
    /// The MusicBrainz release its `release` pins it to, among those its
    /// recording is on.
    pub release: Option<String>,
}

/// The providers a song's `lookups` turns off: `lookups = false` all of
/// them, `lookups.<provider> = false` each named. A song can only turn
/// lookups off, so none asks a provider the song list turns off.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LookupsOff {
    #[default]
    None,
    All,
    Some(Vec<Provider>),
}

impl LookupsOff {
    /// Whether it turns `p`'s lookups off.
    #[must_use]
    pub fn covers(&self, p: Provider) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Some(off) => off.contains(&p),
        }
    }
}

impl Song {
    #[must_use]
    pub fn has(&self, key: &SourceKey) -> bool {
        self.sources.contains(key)
    }

    /// Each key `moved` names written as the key it names, all at once,
    /// wherever the song names it, its record kept: files that moved.
    pub fn rekey(&mut self, moved: &BTreeMap<SourceKey, SourceKey>) {
        let follow = |k: &mut SourceKey| {
            if let Some(to) = moved.get(k) {
                k.clone_from(to);
            }
        };
        self.sources.iter_mut().for_each(follow);
        self.album.iter_mut().for_each(follow);
        self.audio.iter_mut().for_each(follow);
        self.cover.iter_mut().for_each(follow);
        if let Some(LyricsPin::From(k)) = &mut self.lyrics {
            follow(k);
        }
        self.held = rekeyed(std::mem::take(&mut self.held), moved);
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
    /// The title and artist it resolved to when it was removed, which
    /// name it once its sources are gone, as after `remove --purge`.
    pub title: Option<String>,
    pub artist: Option<String>,
    /// The tags it was given by hand.
    pub tags: Tags,
    /// What each source held when the song was last built.
    pub held: BTreeMap<SourceKey, crate::held::Held>,
    /// The files of the user's own `remove --purge` moved to the trash,
    /// each by its key in the manual folder, its `.lrc` and pictures
    /// with it: `restore` waits for each to be back.
    pub trashed: Vec<SourceKey>,
}

/// `moved`, with each of a removed song's `trashed` files that went to
/// the trash beside a song file `moved` follows following it: an `.lrc`
/// or picture named as the song file is, renamed as the file it is
/// named after is, and a picture named as none, the folder's, by its own
/// name, into the folder that file went to. A sidecar serves its song
/// only beside it, so beside it is where `restore` and `undo` wait for
/// it. One named as more than one song file, or a folder's picture whose
/// song files did not all go to one folder, is left where it was.
#[must_use]
pub fn moved_with(
    sources: &[SourceKey],
    trashed: &[SourceKey],
    moved: &BTreeMap<SourceKey, SourceKey>,
) -> BTreeMap<SourceKey, SourceKey> {
    let manual = |k: &SourceKey| match k {
        SourceKey::Manual(m) => Some(m.path().to_path_buf()),
        _ => None,
    };
    let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().to_lowercase());
    let to_of = |k: &SourceKey| moved.get(k).and_then(manual);
    let mut out = moved.clone();
    for key in trashed.iter().filter(|k| !moved.contains_key(k)) {
        let Some(file) = manual(key) else { continue };
        let (Some(name), Some(folder)) = (file.file_name(), file.parent()) else {
            continue;
        };
        let name = name.to_string_lossy();
        let lower = name.to_lowercase();
        let beside: Vec<(&SourceKey, PathBuf)> = (sources.iter())
            .filter(|k| *k != key)
            .filter_map(|k| Some((k, manual(k)?)))
            .filter(|(_, p)| p.parent() == Some(folder))
            .collect();
        let named_as: BTreeSet<&SourceKey> = (beside.iter())
            .filter(|(_, p)| stem(p).is_some_and(|s| lower.starts_with(&format!("{s}."))))
            .map(|(k, _)| *k)
            .collect();
        let to = if named_as.is_empty() {
            if crate::store::kind_of(&file) != Some(crate::store::Kind::Image) {
                continue;
            }
            // A folder's picture, which every song file there must go with.
            let gone: BTreeSet<Option<PathBuf>> = (beside.iter())
                .map(|(k, _)| to_of(k).and_then(|t| t.parent().map(Path::to_path_buf)))
                .collect();
            match gone.into_iter().collect::<Vec<_>>().as_slice() {
                [Some(to)] => to.join(name.as_ref()),
                _ => continue,
            }
        } else {
            let [one] = named_as.into_iter().collect::<Vec<_>>()[..] else {
                continue;
            };
            let (Some(old), Some(to)) = (manual(one), to_of(one)) else {
                continue;
            };
            let (Some(old), Some(new)) = (old.file_stem(), to.file_stem()) else {
                continue;
            };
            let rest: String = name
                .chars()
                .skip(old.to_string_lossy().chars().count())
                .collect();
            to.with_file_name(format!("{}{rest}", new.to_string_lossy()))
        };
        out.insert(key.clone(), SourceKey::Manual(to.into()));
    }
    out
}

/// `held` with each key `moved` names under the key it names.
fn rekeyed<T>(
    held: BTreeMap<SourceKey, T>,
    moved: &BTreeMap<SourceKey, SourceKey>,
) -> BTreeMap<SourceKey, T> {
    held.into_iter()
        .map(|(k, h)| (moved.get(&k).cloned().unwrap_or(k), h))
        .collect()
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
    /// A torrent recorded, unless it is.
    AddTorrent { hash: String, torrent: Torrent },
    /// A new album, unless listed.
    AddAlbum {
        source: SourceKey,
        tracks: Option<u32>,
    },
    /// One key in place of another wherever it is listed.
    Rename { from: SourceKey, to: SourceKey },
    /// A key dropped from the song listing it.
    Drop(SourceKey),
    /// In the song listing `song`, `new` in place of `old`, or `old`
    /// dropped where there is no `new` or it lists `new` already; another
    /// song listing `old` keeps it.
    Supersede {
        song: SourceKey,
        old: SourceKey,
        new: Option<SourceKey>,
    },
    /// Tags set in the `tags` of the song listing `key`, over any it
    /// sets already under another spelling of their names.
    Tag { key: SourceKey, tags: Tags },
    /// Tags set in the `tags` of the album `source` names, as
    /// [`Edit::Tag`] sets a song's.
    AlbumTags { source: SourceKey, tags: Tags },
    /// The song listing `key` pinned to the MusicBrainz release `release`,
    /// or no longer pinned to one.
    Release {
        key: SourceKey,
        release: Option<String>,
    },
    /// `drop` added to what the `[[tags.from]]` of the torrent or channel
    /// named leaves out, a new one where none names it.
    TagsFrom {
        torrent: Option<String>,
        channel: Option<String>,
        drop: Vec<String>,
    },
    /// The song listing `key` moved to `[[removed]]`, named by `note`.
    Remove { key: SourceKey, note: Note },
    /// The files of the user's own `remove --purge` moves to the trash
    /// noted in the removed song listing `key`, each by its key in the
    /// manual folder.
    Trashed {
        key: SourceKey,
        files: Vec<SourceKey>,
    },
    /// `key`, which no song lists, kept under `[[removed]]`, named by
    /// `note`, as a song taken out by hand.
    Tombstone { key: SourceKey, note: String },
    /// `key`, a file no song lists that holds what the removed song
    /// listing `with` held, added to its sources: kept out with it, and
    /// listed with it again by `restore`.
    KeptOut { with: SourceKey, key: SourceKey },
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
    /// What `key` holds recorded in the song named `song` that lists it.
    Hold {
        song: SourceKey,
        key: SourceKey,
        held: crate::held::Held,
    },
    /// What `key` held forgotten by every song listing it, so each is
    /// built from what it holds now: a record a lookup kept anew.
    Unhold(SourceKey),
    /// Each key named written as the key it names, all at once, wherever
    /// a song, an album or a removed song names it, its record kept:
    /// manual files followed to where they moved, two of which may have
    /// traded names.
    Moved(BTreeMap<SourceKey, SourceKey>),
}

/// What an edited song became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewritten {
    Table(SongTable),
    /// Removed, named by the note.
    Removed(Note),
}

/// How a song removed is named under `[[removed]]`: the text said, and
/// the title and artist it was made of, each kept apart so a query
/// still reads them as fields once its sources are gone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Note {
    pub text: String,
    pub title: Option<String>,
    pub artist: Option<String>,
}

impl From<&str> for Note {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_string(),
            ..Self::default()
        }
    }
}

impl From<String> for Note {
    fn from(text: String) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }
}

/// The keys under `[[removed]]` that name the song, which a song listed
/// again does not keep.
const NAMING: [&str; 3] = ["note", "title", "artist"];

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
    /// `[torrents]`, by hash.
    pub torrents: BTreeMap<String, Torrent>,
    /// Which cleaning rules are switched off.
    pub clean: Settings,
    pub removed: Vec<Removed>,
    /// What `[providers]` sets over the defaults.
    pub providers: providers::Config,
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
    /// What the file holds that is read as nothing, each said: a value an
    /// earlier muman took that this one refuses.
    pub left_out: Vec<String>,
    edits: Vec<Edit>,
    stale: bool,
    /// The file's text as this was read from it or last wrote it, with
    /// LF endings and no byte-order mark; `None` for a home without one.
    text: Option<String>,
}

/// What a song list's text holds besides its TOML, kept by a write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Form {
    bom: bool,
    crlf: bool,
}

impl Form {
    fn of(raw: &str) -> Self {
        let crlf = raw.matches("\r\n").count();
        Self {
            bom: raw.starts_with('\u{feff}'),
            crlf: crlf > 0 && crlf * 2 >= raw.matches('\n').count(),
        }
    }

    /// `text`, with LF endings and no mark, in this form.
    fn apply(self, text: &str) -> String {
        let text = if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text.to_string()
        };
        if self.bom {
            format!("\u{feff}{text}")
        } else {
            text
        }
    }
}

/// The top-level keys the song list reads besides the settings'
/// [`crate::settings::TABLES`].
/// The tables that list songs: one misspelled would take what it lists
/// out of the library, so it is refused rather than warned of.
const LISTED: [&str; 3] = ["song", "album", "removed"];

/// `doc`, refused when a table in it is a letter or two from one of
/// [`LISTED`].
fn no_misspelled_list(doc: &DocumentMut) -> Result<&DocumentMut> {
    if let Some((key, near)) = doc
        .iter()
        .find_map(|(key, _)| Some((key, near_miss(key).filter(|n| LISTED.contains(n))?)))
    {
        bail!(
            "`{key}` in the song list is nothing muman reads; did you mean `{near}`? Read \
             as it is, what it lists would leave the library, so nothing is done until it \
             is spelled `{near}`"
        );
    }
    Ok(doc)
}

/// The key muman reads that `key`, which it does not, is a letter or
/// two from, as a misspelling.
fn near_miss(key: &str) -> Option<&'static str> {
    let mut known = KEYS.iter().chain(&crate::settings::TABLES).copied();
    if known.any(|k| k == key) {
        return None;
    }
    let near = KEYS
        .iter()
        .chain(&crate::settings::TABLES)
        .copied()
        .min_by_key(|k| strsim::osa_distance(k, key))?;
    let most = if key.chars().count() > 4 { 2 } else { 1 };
    (strsim::osa_distance(near, key) <= most).then_some(near)
}

const KEYS: [&str; 10] = [
    "version",
    "edition",
    "song",
    "album",
    "torrents",
    "removed",
    "providers",
    "hook",
    "tags",
    "defaults",
];

/// The oldest format version that reads the songs of `parsed`: whether
/// any lists a torrent's file, then what they share.
fn needed_version(parsed: &Parsed) -> i64 {
    let torrent = parsed
        .songs
        .iter()
        .flat_map(|s| &s.sources)
        .chain(parsed.removed.iter().flat_map(|r| &r.sources))
        .any(|k| matches!(k, SourceKey::Torrent(_)));
    if torrent {
        return VERSION;
    }
    let mut seen = BTreeSet::new();
    // Each song's keys once: a song naming one twice shares it with no one.
    let shared: Vec<&SourceKey> = parsed
        .songs
        .iter()
        .flat_map(|s| s.sources.iter().collect::<BTreeSet<_>>())
        .filter(|k| shareable(k) && !seen.insert(*k))
        .collect();
    if shared.iter().any(|k| is_picture(k)) {
        PICTURES
    } else if shared.is_empty() {
        PLAIN
    } else {
        SHARED
    }
}

/// What records a save of the song list: a run begun before the save,
/// so a save stopped anywhere is one its record covers, told of the list
/// once it is written.
pub trait Recorder {
    /// Whether the save may go on, asked before anything is read; a
    /// recorder of nothing, as a dry run's, refuses.
    fn ahead(&self) -> Result<()> {
        Ok(())
    }

    /// Note `manifest` as just written, the folder's lock still held: it
    /// takes no lock of the folder.
    fn saved(&mut self, manifest: &Manifest) -> Result<()>;
}

/// The recorder of a save no run records: the list a new home starts
/// with, and tests.
#[derive(Debug, Default)]
pub struct Unrecorded;

impl Recorder for Unrecorded {
    fn saved(&mut self, _: &Manifest) -> Result<()> {
        Ok(())
    }
}

/// Write the song list a new home starts with, every setting written
/// out to edit, when `home` has none: `sync` on a new home makes it, so
/// lookups can be turned off before the first `add`. A list gone missing
/// from a home muman wrote songs from is refused before this, by
/// `state::present`. No run records it: it comes before the first, which
/// takes it as its list before, and holds nothing to take back.
pub fn start(home: &Path) -> Result<()> {
    if home.join(MANIFEST).exists() {
        return Ok(());
    }
    let mut m = Manifest::load(home)?;
    m.stale = true;
    m.save(&mut Unrecorded)
}

impl Manifest {
    /// A warning for each top-level key muman does not read that is a
    /// letter or two from one it does. Unknown keys are kept for a newer
    /// muman, but a misspelled `[audio]` would leave its settings doing
    /// nothing, unnoticed.
    #[must_use]
    pub fn misspelled(&self) -> Vec<String> {
        let Some(doc) = self
            .text
            .as_deref()
            .and_then(|t| t.parse::<DocumentMut>().ok())
        else {
            return Vec::new();
        };
        doc.iter()
            .filter_map(|(key, _)| Some((key, near_miss(key)?)))
            .map(|(key, near)| {
                format!(
                    "`{key}` in the song list is nothing muman reads, so what it sets does \
                     nothing; did you mean `{near}`?"
                )
            })
            .collect()
    }

    /// The list in `home`; an empty one when there is none yet. A list
    /// stating a newer version than its songs need is stale, so the next
    /// save writes it down and an older muman reads it again.
    pub fn load(home: &Path) -> Result<Self> {
        let (doc, text, _) = read_text(home)?;
        let file = home.join(MANIFEST);
        let parsed = parse(&doc).with_context(|| format!("reading {}", file.display()))?;
        let stale = file.exists()
            && (with_header(&doc.to_string()).is_some()
                || doc.get("version").and_then(Item::as_integer) > Some(needed_version(&parsed))
                || normalize(&mut doc.clone())
                || !parsed.respelled.is_empty());
        Ok(Self {
            dir: home.to_path_buf(),
            songs: parsed.songs,
            albums: parsed.albums,
            torrents: parsed.torrents,
            removed: parsed.removed,
            providers: parsed.providers,
            hooks: parsed.hooks,
            clean: parsed.clean,
            settings: parsed.settings,
            stale_defaults: parsed.stale_defaults,
            renamed: parsed.renamed,
            respelled: parsed.respelled,
            left_out: parsed.left_out,
            edits: Vec::new(),
            stale,
            text,
        })
    }

    /// The file's text as this was read from it or last wrote it: what a
    /// run that read or wrote the song list left of it.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
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

    /// Each key `moved` names written as the key it names, all at once,
    /// wherever a song, an album or a removed song names it, here alone;
    /// [`Edit::Moved`] writes it in the file.
    pub fn rekey(&mut self, moved: &BTreeMap<SourceKey, SourceKey>) {
        let mut with = moved.clone();
        for removed in &self.removed {
            with.extend(moved_with(&removed.sources, &removed.trashed, moved));
        }
        let moved = &with;
        for song in &mut self.songs {
            song.rekey(moved);
        }
        for album in &mut self.albums {
            if let Some(to) = moved.get(&album.source) {
                album.source.clone_from(to);
            }
        }
        for removed in &mut self.removed {
            for k in removed.sources.iter_mut().chain(&mut removed.trashed) {
                if let Some(to) = moved.get(k) {
                    k.clone_from(to);
                }
            }
            removed.held = rekeyed(std::mem::take(&mut removed.held), moved);
        }
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
    /// finished meanwhile keeps its own changes, `run` told once it is
    /// written. The folder is locked for the read and the write, and the
    /// file replaced through a rename.
    pub fn save(&mut self, run: &mut dyn Recorder) -> Result<()> {
        if self.edits.is_empty() && !self.stale {
            return Ok(());
        }
        let lock = Lock::folder(&self.dir)?;
        self.save_locked(&lock, run)
    }

    /// [`Self::save`] under a lock the caller already holds.
    pub fn save_locked(&mut self, lock: &Lock, run: &mut dyn Recorder) -> Result<()> {
        self.save_expecting(lock, &[], &[], run).map(drop)
    }

    /// Apply what was recorded only while each song in `expected` is
    /// still listed as it was, by any of its keys, and each album in
    /// `albums` as it was. Returns the first key of each song, and the
    /// source of each album, that changed meanwhile; when any did,
    /// nothing is written, the edits are kept and `run` is told nothing.
    fn save_expecting(
        &mut self,
        _lock: &Lock,
        expected: &[Song],
        albums: &[Album],
        run: &mut dyn Recorder,
    ) -> Result<Vec<SourceKey>> {
        if self.edits.is_empty() && !self.stale {
            return Ok(Vec::new());
        }
        run.ahead()?;
        let file = self.dir.join(MANIFEST);
        if self.text.is_some() && !file.exists() {
            return Err(crate::errors::Refused(format!(
                "{} was removed while muman ran: put it back, or `muman undo` the run that \
                 removed it",
                file.display()
            ))
            .into());
        }
        let (mut doc, _, form) = read_text(&self.dir)?;
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
            .chain(
                (albums.iter())
                    .filter(|e| !now.albums.iter().any(|a| a == *e))
                    .map(|e| e.source.clone()),
            )
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
        let version = needed_version(&parsed);
        if doc.get("version").and_then(Item::as_integer) != Some(version) {
            doc["version"] = value(version);
        }
        let text = doc.to_string();
        let text = with_header(&text).unwrap_or(text);
        let written = form.apply(&text);
        atomic::write(&self.dir, &atomic::Name::new(MANIFEST)?, written.as_bytes())?;
        self.text = Some(text);
        self.edits.clear();
        (self.songs, self.albums, self.removed) = (parsed.songs, parsed.albums, parsed.removed);
        (self.providers, self.hooks, self.clean) = (parsed.providers, parsed.hooks, parsed.clean);
        (self.settings, self.stale_defaults) = (parsed.settings, parsed.stale_defaults);
        self.torrents = parsed.torrents;
        (self.renamed, self.respelled) = (parsed.renamed, parsed.respelled);
        self.left_out = parsed.left_out;
        self.stale = false;
        run.saved(self)?;
        Ok(Vec::new())
    }

    /// [`Self::save_expecting`] under a lock of its own.
    pub fn save_if_unchanged(
        &mut self,
        expected: &[Song],
        run: &mut dyn Recorder,
    ) -> Result<Vec<SourceKey>> {
        self.save_if_unchanged_with(expected, &[], run)
    }

    /// [`Self::save_expecting`] under a lock of its own, albums too.
    pub fn save_if_unchanged_with(
        &mut self,
        expected: &[Song],
        albums: &[Album],
        run: &mut dyn Recorder,
    ) -> Result<Vec<SourceKey>> {
        let lock = Lock::folder(&self.dir)?;
        self.save_expecting(&lock, expected, albums, run)
    }

    /// Whether `edits` would leave a song list that reads, applied to the
    /// file as it is now; nothing is written.
    pub fn trial(&self, edits: &[Edit]) -> Result<()> {
        let mut doc = read(&self.dir)?;
        for edit in edits {
            apply(&mut doc, edit)?;
        }
        normalize(&mut doc);
        let parsed = parse(&doc)?;
        // What the file held before is left out as it reads; a value the
        // edits bring is refused as `set` refuses it.
        match parsed.left_out.iter().find(|l| !self.left_out.contains(l)) {
            Some(line) => bail!("{line}"),
            None => Ok(()),
        }
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

/// The song list `text` with each key `moved` names written as the key it
/// names, all at once, as [`Edit::Moved`] writes them, and nothing else
/// changed; `text` as it is when it does not read.
#[must_use]
pub fn moved_in(text: &str, moved: &BTreeMap<SourceKey, SourceKey>) -> String {
    let Ok(mut doc) = text.parse::<DocumentMut>() else {
        return text.to_string();
    };
    if moved.is_empty() || apply(&mut doc, &Edit::Moved(moved.clone())).is_err() {
        return text.to_string();
    }
    doc.to_string()
}

/// Every key a song or a removed song lists in the song list `text`;
/// none when it does not read.
#[must_use]
pub fn keys_in(text: &str) -> Option<BTreeSet<SourceKey>> {
    let parsed = parse(&text.parse().ok()?).ok()?;
    Some(
        (parsed.songs.iter().flat_map(|s| &s.sources))
            .chain(parsed.removed.iter().flat_map(|r| &r.sources))
            .cloned()
            .collect(),
    )
}

fn read(dir: &Path) -> Result<DocumentMut> {
    read_text(dir).map(|(doc, ..)| doc)
}

/// The song list in `dir`, its text as read with LF endings and no mark,
/// `None` for none, and the form it was read in.
fn read_text(dir: &Path) -> Result<(DocumentMut, Option<String>, Form)> {
    let file = dir.join(MANIFEST);
    let raw = match std::fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let form = raw.as_deref().map(Form::of).unwrap_or_default();
    let raw = raw.map(|t| t.trim_start_matches('\u{feff}').replace("\r\n", "\n"));
    let doc: DocumentMut = raw
        .as_deref()
        .unwrap_or(NEW)
        .parse()
        .with_context(|| format!("{} is not valid TOML", file.display()))?;
    match doc.get("version").and_then(Item::as_integer) {
        Some(PLAIN..=VERSION) => {}
        Some(v) if v > VERSION => bail!(
            "{} is format version {v}, newer than this muman's {VERSION}; \
             update muman before it changes the list",
            file.display()
        ),
        None => bail!(
            "{} has no `version`: start it with `version = 1`, or delete it and \
             `muman sync` writes a new one",
            file.display()
        ),
        Some(v) => bail!(
            "{} is format version {v}, which no muman wrote",
            file.display()
        ),
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
    Ok((doc, raw, form))
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
    torrents: BTreeMap<String, Torrent>,
    removed: Vec<Removed>,
    providers: providers::Config,
    hooks: Vec<Hook>,
    clean: Settings,
    settings: crate::settings::Settings,
    stale_defaults: Vec<crate::settings::Stale>,
    renamed: Vec<crate::migrate::Renamed>,
    respelled: BTreeMap<String, String>,
    left_out: Vec<String>,
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

/// The whole number of 1 or more `key` sets in `t`, if it sets one: a
/// song's place on its album, or how many songs the album holds.
fn numbered(t: &Table, key: &str, what: &str) -> Result<Option<u32>> {
    t.get(key)
        .map(|item| {
            item.as_integer()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .with_context(|| format!("{what}: `{key}` must be a whole number of 1 or more"))
        })
        .transpose()
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
            if other == n {
                bail!("{key} is listed twice by song {}", n + 1);
            }
            bail!(
                "{key} is listed by both song {} and song {}",
                other + 1,
                n + 1
            );
        }
    }
    Ok(keys)
}

/// Refuse a song on an album no `[[album]]` lists.
fn refuse_albumless(songs: &[Song], albums: &[Album]) -> Result<()> {
    for (n, song) in songs.iter().enumerate() {
        if let Some(album) = &song.album
            && !albums.iter().any(|a| &a.source == album)
        {
            bail!(
                "song {} ({}) is on album {album}, which no [[album]] lists as its `source`",
                n + 1,
                song.sources[0]
            );
        }
    }
    Ok(())
}

/// Refuse a song whose every key another song lists too: a song is named
/// by a key only it lists, and such a song would be taken for that other
/// song by every edit.
fn refuse_unowned(songs: &[Song]) -> Result<()> {
    for (n, song) in songs.iter().enumerate() {
        if song.sources.iter().all(shareable)
            && let Some(other) = songs
                .iter()
                .enumerate()
                .find(|(m, o)| *m != n && song.sources.iter().any(|k| o.has(k)))
        {
            bail!(
                "song {} lists no source of its own: song {} lists its {} too; \
                 give it a file of its own, or delete it",
                n + 1,
                other.0 + 1,
                song.sources
                    .iter()
                    .find(|k| other.1.has(k))
                    .map(ToString::to_string)
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
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
    if let Some(why) = offset_refused(time.0) {
        bail!("{what}: `lyrics_offset` {why}");
    }
    Ok(time.0)
}

/// Why a lyrics offset of `ms` is no song's, when it is none: what every
/// command that sets one asks before it shows the change.
#[must_use]
pub fn offset_refused(ms: i64) -> Option<&'static str> {
    (ms.unsigned_abs() > MAX_OFFSET_MS)
        .then_some("moves lyrics more than an hour, longer than any song")
}

/// A song's `lookups`: false, or a table of providers each set false.
fn lookups_off(t: &Table, what: &str) -> Result<LookupsOff> {
    let Some(item) = t.get("lookups") else {
        return Ok(LookupsOff::None);
    };
    let only_off = || {
        anyhow::anyhow!(
            "{what}: `lookups` only turns lookups off: `lookups = false`, or \
             `lookups.<provider> = false`"
        )
    };
    if let Some(on) = item.as_bool() {
        return if on {
            Err(only_off())
        } else {
            Ok(LookupsOff::All)
        };
    }
    let table = item.as_table_like().ok_or_else(only_off)?;
    let mut off = Vec::new();
    for (name, value) in table.iter() {
        let p = Provider::named(name).with_context(|| format!("{what}: `lookups`"))?;
        if !p.findable() {
            bail!("{what}: `lookups.{name}`: {p} is never looked up");
        }
        if value.as_bool() != Some(false) {
            return Err(only_off());
        }
        off.push(p);
    }
    Ok(LookupsOff::Some(off))
}

/// The most lyrics are moved, an hour either way.
const MAX_OFFSET_MS: u64 = 3_600_000;

/// What `doc` holds, read as the current edition writes it.
fn parse(doc: &DocumentMut) -> Result<Parsed> {
    let mut current = doc.clone();
    let renamed = crate::migrate::rename_all(&mut current, &crate::migrate::EDITIONS)
        .map_err(crate::errors::invalid)?;
    let respelled = respell(&mut current, &spelled_now);
    let doc = no_misspelled_list(&current)?;
    let mut songs = Vec::new();
    let mut left_out = Vec::new();
    let mut owner: BTreeMap<SourceKey, usize> = BTreeMap::new();
    for (n, t) in tables(doc, "song").enumerate() {
        let what = format!("song {}", n + 1);
        let sources = owned_keys(t, &what, n, &mut owner)?;
        let Some(first) = sources.first() else {
            bail!("{what} lists no source: give it one, or delete it");
        };
        let what = format!("{what} ({first})");
        let lyrics = match t.get("lyrics") {
            None => None,
            Some(item) if item.as_bool() == Some(false) => Some(LyricsPin::None),
            Some(_) => key_at(t, "lyrics", &what)?.map(LyricsPin::From),
        };
        let song = Song {
            album: key_at(t, "album", &what)?,
            track: numbered(t, "track", &what)?,
            audio: key_at(t, "audio", &what)?,
            cover: key_at(t, "cover", &what)?,
            lyrics,
            lyrics_offset_ms: lyrics_offset(t, &what)?,
            tags: tags_of(t.get("tags"), &what, &mut left_out)?,
            held: crate::held::read(t, &sources, &what)?,
            lookups_off: lookups_off(t, &what)?,
            release: match t.get("release") {
                None => None,
                Some(item) => Some(
                    item.as_str()
                        .filter(|r| !r.trim().is_empty())
                        .with_context(|| format!("{what}: `release` must be a release's ID"))?
                        .trim()
                        .to_string(),
                ),
            },
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
    refuse_unowned(&songs)?;
    let mut albums = Vec::new();
    for (n, t) in tables(doc, "album").enumerate() {
        let what = format!("album {}", n + 1);
        let Some(source) = key_at(t, "source", &what)? else {
            bail!("{what} names no source");
        };
        albums.push(Album {
            source,
            tracks: numbered(t, "tracks", &what)?,
            tags: tags_of(t.get("tags"), &what, &mut left_out)?,
        });
    }
    refuse_albumless(&songs, &albums)?;
    let removed = removed_of(doc, &mut left_out)?;
    let settings = crate::settings::read(doc)?;
    let providers = providers::read(doc)?;
    // A song's tags are those muman knows and those the song list gives
    // it or its album, as a query reads them.
    providers.check_fields(|name| {
        (songs.iter().map(|s| &s.tags))
            .chain(albums.iter().map(|a| &a.tags))
            .any(|t| t.iter().any(|(k, _)| tags::vorbis_key(k) == name))
    })?;
    let clean = clean_of(doc).map_err(crate::errors::invalid)?;
    refuse_unsited(&settings.sites, &songs, &albums, &removed)?;
    Ok(Parsed {
        songs,
        albums,
        torrents: torrents_of(doc)?,
        removed,
        providers,
        hooks: hooks::read(doc),
        clean,
        settings,
        stale_defaults: crate::settings::stale(doc, &crate::settings::EDITIONS),
        renamed,
        respelled,
        left_out,
    })
}

/// The torrents `[torrents]` records, by hash.
fn torrents_of(doc: &DocumentMut) -> Result<BTreeMap<String, Torrent>> {
    let mut torrents = BTreeMap::new();
    let Some(item) = doc.get("torrents") else {
        return Ok(torrents);
    };
    let table = item
        .as_table_like()
        .context("`torrents` must be a table of torrents by hash")?;
    for (hash, item) in table.iter() {
        let what = format!("[torrents.\"{hash}\"]");
        if !crate::torrent::is_hash(hash) {
            bail!("{what}: a torrent is named by its hash, in lower-case hex");
        }
        let t = item
            .as_table_like()
            .with_context(|| format!("{what} must be a table"))?;
        let text = |key: &str| -> Result<Option<String>> {
            t.get(key)
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .with_context(|| format!("{what} {key} must be text"))
                })
                .transpose()
        };
        let trackers = match t.get("trackers") {
            None => Vec::new(),
            Some(v) => v
                .as_array()
                .with_context(|| format!("{what} trackers must be a list"))?
                .iter()
                .map(|u| {
                    u.as_str()
                        .map(str::to_string)
                        .with_context(|| format!("{what} trackers must be addresses"))
                })
                .collect::<Result<_>>()?,
        };
        let v2 = text("v2")?;
        if v2
            .as_deref()
            .is_some_and(|h| h.len() != crate::torrent::V2_HEX || !crate::torrent::is_hash(h))
        {
            bail!("{what} v2 must be a v2 hash, 64 lower-case hex digits");
        }
        torrents.insert(
            hash.to_string(),
            Torrent {
                name: text("name")?,
                trackers,
                v2,
            },
        );
    }
    Ok(torrents)
}

/// The text of a removed song's `key`, as its title, when it has some.
fn named_by(t: &Table, key: &str) -> Option<String> {
    (t.get(key).and_then(Item::as_str))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The songs `[[removed]]` lists, each kept to be restored.
fn removed_of(doc: &DocumentMut, left_out: &mut Vec<String>) -> Result<Vec<Removed>> {
    let mut removed = Vec::new();
    for (n, t) in tables(doc, "removed").enumerate() {
        let what = format!("removed song {}", n + 1);
        removed.push(Removed {
            sources: keys_of(t, &what)?,
            note: t
                .get("note")
                .and_then(Item::as_str)
                .unwrap_or_default()
                .to_string(),
            title: named_by(t, "title"),
            artist: named_by(t, "artist"),
            tags: tags_of(t.get("tags"), &what, left_out)?,
            held: crate::held::read(t, &keys_of(t, &what)?, &what)?,
            trashed: trashed_of(t, &what)?,
        });
    }
    Ok(removed)
}

/// The files a removed song's `trashed` names, each a manual key.
fn trashed_of(t: &Table, what: &str) -> Result<Vec<SourceKey>> {
    let Some(item) = t.get("trashed") else {
        return Ok(Vec::new());
    };
    let list = item
        .as_array()
        .with_context(|| format!("{what}: `trashed` must be a list of manual keys"))?;
    list.iter()
        .map(|v| {
            v.as_str()
                .and_then(|text| SourceKey::parse(text).ok())
                .filter(|k| matches!(k, SourceKey::Manual(_)))
                .with_context(|| format!("{what}: `trashed` must be a list of manual keys"))
        })
        .collect()
}

/// Refuse a key whose ID its site's `[sites]` table takes in no kind.
fn refuse_unsited(
    sites: &crate::sites::Sites,
    songs: &[Song],
    albums: &[Album],
    removed: &[Removed],
) -> Result<()> {
    let place = |kind: &str, n: usize| format!("{kind} {}", n + 1);
    let listed = (songs.iter().enumerate())
        .flat_map(|(n, s)| s.sources.iter().map(move |k| (place("song", n), k)))
        .chain((albums.iter().enumerate()).map(|(n, a)| (place("album", n), &a.source)))
        .chain(
            (removed.iter().enumerate())
                .flat_map(|(n, r)| r.sources.iter().map(move |k| (place("removed song", n), k))),
        );
    if let Some((what, key)) = listed.into_iter().find(|(_, k)| !sites.accepts(k)) {
        bail!("{what}: `{key}` is not a source key: its site's `[sites]` table takes no such ID");
    }
    Ok(())
}

/// How a key written as `text` is written now, where that differs: an
/// extractor's name for a site the built-in `[sites]` lists, as
/// `youtube:<id>`, is its domain's, and a torrent's hash is lower-case.
fn spelled_now(text: &str) -> Option<String> {
    let key = SourceKey::parse(text)
        .ok()
        .filter(|k| k.site().is_some() || matches!(k, SourceKey::Torrent(_)))?;
    Some(key.to_string()).filter(|now| now != text)
}

/// Every field of a song, an album or a removed song that names sources,
/// a key or a list of keys; with the `held` record, by key, every place a
/// key stands. A file followed where it moved, as a key spelled anew, is
/// followed in each, in the file ([`respell`]) as in memory
/// ([`Manifest::rekey`]), which a test holds to the same.
const KEYED: [&str; 9] = [
    "sources", "trashed", "source", "album", "audio", "cover", "lyrics", "next", "prev",
];

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
            for name in KEYED {
                match t.get_mut(name).and_then(Item::as_value_mut) {
                    Some(Value::Array(keys)) => {
                        keys.iter_mut().for_each(|v| string(v, &mut written));
                    }
                    Some(v) => string(v, &mut written),
                    None => {}
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

pub fn key_array(keys: &[SourceKey]) -> Item {
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

// One match over every kind of edit.
#[expect(clippy::too_many_lines)]
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
        Edit::AddTorrent { hash, torrent } => {
            let all = doc
                .entry("torrents")
                .or_insert_with(|| {
                    let mut t = Table::new();
                    t.set_implicit(true);
                    Item::Table(t)
                })
                .as_table_mut()
                .context("`torrents` is not a table")?;
            if !all.contains_key(hash) {
                let mut t = Table::new();
                if let Some(name) = &torrent.name {
                    t.insert("name", value(name.as_str()));
                }
                if !torrent.trackers.is_empty() {
                    t.insert(
                        "trackers",
                        value(
                            torrent
                                .trackers
                                .iter()
                                .map(String::as_str)
                                .collect::<Array>(),
                        ),
                    );
                }
                if let Some(v2) = &torrent.v2 {
                    t.insert("v2", value(v2.as_str()));
                }
                all.insert(hash, Item::Table(t));
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
        Edit::Supersede { song, old, new } => {
            let (text, old_text) = (song.to_string(), old.to_string());
            let new_text = new.as_ref().map(ToString::to_string);
            let song = doc
                .get_mut("song")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|list| list.iter_mut().find(|t| listed_keys(t).contains(&text)));
            if let Some(song) = song {
                let keys = listed_keys(song);
                if keys.contains(&old_text) {
                    let gone = new_text.as_ref().is_none_or(|n| keys.contains(n));
                    let keys: Vec<SourceKey> = keys
                        .iter()
                        .filter(|k| !(gone && **k == old_text))
                        .map(|k| match &new_text {
                            Some(n) if *k == old_text => n.clone(),
                            _ => k.clone(),
                        })
                        .filter_map(|k| SourceKey::parse(&k).ok())
                        .collect();
                    song.insert("sources", key_array(&keys));
                    for pin in ["audio", "cover", "lyrics"] {
                        if song.get(pin).and_then(Item::as_str) == Some(old_text.as_str()) {
                            song.remove(pin);
                        }
                    }
                    crate::held::unset(song, old);
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
        Edit::Release { key, release } => {
            let text = key.to_string();
            let song = doc
                .get_mut("song")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|list| list.iter_mut().find(|t| listed_keys(t).contains(&text)))
                .with_context(|| format!("no song lists {text}"))?;
            match release {
                Some(id) => {
                    song.insert("release", value(id.as_str()));
                }
                None => {
                    song.remove("release");
                }
            }
        }
        Edit::TagsFrom {
            torrent,
            channel,
            drop,
        } => tags_from(doc, torrent.as_deref(), channel.as_deref(), drop)?,
        Edit::AlbumTags { source, tags } => {
            let text = source.to_string();
            let album = doc
                .get_mut("album")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|list| {
                    list.iter_mut()
                        .find(|t| t.get("source").and_then(Item::as_str) == Some(text.as_str()))
                })
                .with_context(|| format!("no album is listed by {text}"))?;
            set_tags(album, tags);
        }
        Edit::Remove { key, note } => {
            let at = tables(doc, "song").position(|t| listed_keys(t).contains(&key.to_string()));
            let near = at.map(|at| neighbours(doc, at)).unwrap_or_default();
            let Some(song) = take(doc, "song", key) else {
                return Ok(());
            };
            tables_mut(doc, "removed")?.push(tombstone(song, note, near));
        }
        Edit::Trashed { key, files } => {
            let text = key.to_string();
            if let Some(gone) = doc
                .get_mut("removed")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|l| l.iter_mut().find(|t| listed_keys(t).contains(&text)))
                .filter(|_| !files.is_empty())
            {
                gone.insert("trashed", key_array(files));
            }
        }
        Edit::KeptOut { with, key } => {
            let (with, key) = (with.to_string(), key.to_string());
            let sources = doc
                .get_mut("removed")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|l| l.iter_mut().find(|t| listed_keys(t).contains(&with)))
                .and_then(|t| t.get_mut("sources"))
                .and_then(Item::as_array_mut)
                .with_context(|| format!("no removed song lists {with}"))?;
            if !sources.iter().any(|v| v.as_str() == Some(key.as_str())) {
                sources.push(key.as_str());
            }
        }
        Edit::Tombstone { key, note } => {
            let mut song = Table::new();
            song.insert("sources", key_array(std::slice::from_ref(key)));
            let note = Note::from(note.as_str());
            tables_mut(doc, "removed")?.push(tombstone(song, &note, Near::default()));
        }
        Edit::Restore(key) => {
            let Some(mut song) = take(doc, "removed", key) else {
                return Ok(());
            };
            let removed = song.clone();
            for key in NAMING {
                song.remove(key);
            }
            song.remove("trashed");
            unplace(&mut song);
            let listed: BTreeSet<String> = tables(doc, "song").flat_map(listed_keys).collect();
            let taken =
                |k: &str| listed.contains(k) && SourceKey::parse(k).is_ok_and(|k| !shareable(&k));
            let keys: Vec<SourceKey> = listed_keys(&song)
                .iter()
                .filter(|k| !taken(k))
                .filter_map(|k| SourceKey::parse(k).ok())
                .collect();
            let shared = |k: &SourceKey| {
                tables(doc, "song").any(|t| listed_keys(t).contains(&k.to_string()))
            };
            if keys.is_empty() || keys.iter().all(|k| shareable(k) && shared(k)) {
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
            let at = place(doc, &removed);
            for near in ["next", "prev"] {
                song.remove(near);
            }
            let list = tables_mut(doc, "song")?;
            match at {
                Some(at) => list.insert(at, song),
                None => list.push(song),
            }
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
            let near: Vec<Near> = (0..list.len()).map(|at| neighbours_in(list, at)).collect();
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
                    removed.push(tombstone(song, &note, near[at].clone()));
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
        Edit::Moved(moved) => {
            let mut with = moved.clone();
            for t in tables(doc, "removed") {
                let sources: Vec<SourceKey> = (listed_keys(t).iter())
                    .filter_map(|k| SourceKey::parse(k).ok())
                    .collect();
                let trashed = trashed_of(t, "").unwrap_or_default();
                with.extend(moved_with(&sources, &trashed, moved));
            }
            let moved: BTreeMap<String, String> = (with.iter())
                .map(|(from, to)| (from.to_string(), to.to_string()))
                .collect();
            respell(doc, &|k| moved.get(k).cloned());
        }
        Edit::Unhold(key) => {
            let text = key.to_string();
            for song in doc
                .get_mut("song")
                .and_then(Item::as_array_of_tables_mut)
                .into_iter()
                .flat_map(|l| l.iter_mut())
                .filter(|t| listed_keys(t).contains(&text))
            {
                crate::held::unset(song, key);
            }
        }
        Edit::Hold { song, key, held } => {
            let (named, text) = (song.to_string(), key.to_string());
            // A key songs share is held by each apart: the song is found
            // by its id, which only it lists.
            if let Some(song) = doc
                .get_mut("song")
                .and_then(Item::as_array_of_tables_mut)
                .and_then(|l| {
                    l.iter_mut().find(|t| {
                        let listed = listed_keys(t);
                        listed.contains(&named) && listed.contains(&text)
                    })
                })
            {
                crate::held::set(song, key, held);
            }
        }
    }
    Ok(())
}

/// A song's table as `[[removed]]` keeps it, its note, title and artist
/// first.
fn tombstone(mut song: Table, note: &Note, near: Near) -> Table {
    let mut gone = Table::new();
    gone.insert("note", value(note.text.as_str()));
    for (key, named) in [("title", &note.title), ("artist", &note.artist)] {
        if let Some(named) = named.as_deref().filter(|n| !n.is_empty()) {
            gone.insert(key, value(named));
        }
    }
    for key in NAMING {
        song.remove(key);
    }
    gone.extend(song.iter().map(|(k, v)| (k.to_string(), v.clone())));
    for (name, key) in [("next", near.next), ("prev", near.prev)] {
        match key {
            Some(key) => gone.insert(name, value(key)),
            None => gone.remove(name),
        };
    }
    unplace(&mut gone);
    gone
}

/// The songs a removed song stood between, each named by its first
/// source, so `restore` puts it back where it was: list order says which
/// of two songs at one path keeps it plain, so a song put back elsewhere
/// would trade paths with another. A neighbour removed too is followed
/// through the removed songs to one listed.
#[derive(Debug, Clone, Default)]
struct Near {
    next: Option<String>,
    prev: Option<String>,
}

/// [`Near`] of the song at `at` among the listed songs of `doc`, a song
/// removed beside it before taken for the neighbour it was.
fn neighbours(doc: &DocumentMut, at: usize) -> Near {
    let Some(list) = doc.get("song").and_then(Item::as_array_of_tables) else {
        return Near::default();
    };
    let me = list.get(at).and_then(first_key);
    let gone_beside = |side: &str| {
        let me = me.as_deref()?;
        tables(doc, "removed")
            .filter(|t| t.get(side).and_then(Item::as_str) == Some(me))
            .last()
            .and_then(first_key)
    };
    let listed = neighbours_in(list, at);
    Near {
        next: gone_beside("prev").or(listed.next),
        prev: gone_beside("next").or(listed.prev),
    }
}

/// [`Near`] of the song at `at` in `list`, as it stands.
fn neighbours_in(list: &ArrayOfTables, at: usize) -> Near {
    Near {
        next: list.get(at + 1).and_then(first_key),
        prev: at
            .checked_sub(1)
            .and_then(|p| list.get(p))
            .and_then(first_key),
    }
}

fn first_key(t: &Table) -> Option<String> {
    listed_keys(t).into_iter().next()
}

/// Where the removed song `gone` goes back among the listed songs: before
/// the first listed song its `next` leads to, through songs removed
/// beside it, else after the first its `prev` does; none when neither
/// leads to one, and it goes last.
fn place(doc: &DocumentMut, gone: &Table) -> Option<usize> {
    let listed: Vec<Vec<String>> = tables(doc, "song").map(listed_keys).collect();
    let follow = |side: &str| {
        let mut key = gone.get(side).and_then(Item::as_str).map(str::to_string);
        for _ in 0..=tables(doc, "removed").count() {
            let k = key?;
            if let Some(at) = listed.iter().position(|keys| keys.contains(&k)) {
                return Some(at);
            }
            key = tables(doc, "removed")
                .find(|t| listed_keys(t).contains(&k))
                .and_then(|t| t.get(side).and_then(Item::as_str).map(str::to_string));
        }
        None
    };
    follow("next").or_else(|| follow("prev").map(|at| at + 1))
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
    // A song left with no file of its own is gone, whatever records it lists.
    list.retain(|t| {
        listed_keys(t)
            .iter()
            .any(|k| SourceKey::parse(k).is_ok_and(|k| !shareable(&k)))
    });
    if list.is_empty() {
        doc.remove("removed");
    }
}

/// `drop` added to the `[[tags.from]]` naming `torrent` or `channel`, or
/// to a new one after the others.
fn tags_from(
    doc: &mut DocumentMut,
    torrent: Option<&str>,
    channel: Option<&str>,
    drop: &[String],
) -> Result<()> {
    let tags = doc
        .entry("tags")
        .or_insert(toml_edit::table())
        .as_table_mut()
        .context("`tags` in the song list is no table")?;
    let list = tags
        .entry("from")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .context("`tags.from` must be tables, each `[[tags.from]]`")?;
    let names = |t: &Table, key: &str, want: Option<&str>| {
        want.is_some_and(|w| t.get(key).and_then(Item::as_str) == Some(w))
    };
    let at = list
        .iter()
        .position(|t| names(t, "torrent", torrent) || names(t, "channel", channel));
    let at = at.unwrap_or_else(|| {
        let mut t = Table::new();
        if let Some(hash) = torrent {
            t.insert("torrent", value(hash));
        }
        if let Some(name) = channel {
            t.insert("channel", value(name));
        }
        list.push(t);
        list.len() - 1
    });
    let table = list.get_mut(at).context("a `[[tags.from]]` went missing")?;
    let dropped = table
        .entry("drop")
        .or_insert(value(Array::new()))
        .as_array_mut()
        .context("`[[tags.from]] drop` must be a list of tag names")?;
    for name in drop {
        if !dropped.iter().any(|v| v.as_str() == Some(name.as_str())) {
            dropped.push(name.as_str());
        }
    }
    Ok(())
}

/// Set `tags` in a song table's `tags`, each under the name the song list
/// gives it ([`tags::song_list_name`]) and over any it sets under another
/// spelling of it; a tag with no values is cleared.
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
        let name = tags::song_list_name(name);
        let field = tags::vorbis_key(&name);
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
        table.insert(&name, item);
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
    // Filling renames first, so the switches of an older `[clean]` are
    // moved before any tier is added in their new place.
    let mut changed = crate::settings::fill(doc, &crate::settings::EDITIONS);
    changed |= clean_template(doc);
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
    changed | drop_listed_removed(doc)
}

/// A removed song's sources a song lists again taken out of its
/// `[[removed]]` table, and the table with them when none of its own is
/// left: a song joined to another in `edit` is listed, not removed, and
/// `restore` could list none of it again. Whether any was.
fn drop_listed_removed(doc: &mut DocumentMut) -> bool {
    let own = |k: &str| SourceKey::parse(k).ok().filter(|k| !shareable(k));
    let listed: BTreeSet<SourceKey> = doc
        .get("song")
        .and_then(Item::as_array_of_tables)
        .into_iter()
        .flatten()
        .flat_map(listed_keys)
        .filter_map(|k| own(&k))
        .collect();
    let Some(removed) = doc
        .get_mut("removed")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return false;
    };
    let mut changed = false;
    for table in removed.iter_mut() {
        let Some(sources) = table.get_mut("sources").and_then(Item::as_array_mut) else {
            continue;
        };
        let before = sources.len();
        sources.retain(|v| {
            !v.as_str()
                .and_then(own)
                .is_some_and(|k| listed.contains(&k))
        });
        changed |= sources.len() != before;
    }
    let before = removed.len();
    removed.retain(|t| listed_keys(t).iter().any(|k| own(k).is_some()));
    changed | (removed.len() != before)
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
            let field = tags::vorbis_key(name);
            if !tags.iter().any(|(k, _)| tags::vorbis_key(k) == field) {
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

/// The cleaning switches the file sets, the tags' under
/// `[tags.clean.<tier>]` and the lyrics' under `[lyrics.clean.<tier>]`. A
/// key naming no rule is kept, and reported, so a misspelled one is not
/// silently ignored.
fn clean_of(doc: &DocumentMut) -> Result<Settings> {
    let mut settings = Settings::default();
    for (aspect, rules_of) in [("tags", ""), ("lyrics", "lyrics.")] {
        let Some(tiers) = doc
            .get(aspect)
            .and_then(|t| t.get("clean"))
            .and_then(Item::as_table_like)
        else {
            continue;
        };
        for (tier, item) in tiers.iter() {
            let table = format!("{aspect}.clean.{tier}");
            match item.as_table_like() {
                Some(rules) => switch(&mut settings, &table, &format!("{rules_of}{tier}"), rules)?,
                None => settings.unknown.push(table),
            }
        }
    }
    settings.from = overrides_of(doc)?;
    Ok(settings)
}

/// Each `[[tags.from]]`: what the song list says of one torrent's or
/// channel's sources ([`crate::overrides`]).
fn overrides_of(doc: &DocumentMut) -> Result<Vec<crate::overrides::Override>> {
    let Some(item) = doc.get("tags").and_then(|t| t.get("from")) else {
        return Ok(Vec::new());
    };
    let tables = item
        .as_array_of_tables()
        .context("`tags.from` must be tables, each `[[tags.from]]`")?;
    tables.iter().map(override_of).collect()
}

fn override_of(table: &Table) -> Result<crate::overrides::Override> {
    let text = |key: &str| -> Result<Option<String>> {
        table
            .get(key)
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .with_context(|| format!("`[[tags.from]] {key}` must be text"))
            })
            .transpose()
    };
    for (key, _) in table {
        if !["torrent", "channel", "drop", "credits", "title"].contains(&key) {
            bail!("`[[tags.from]]` takes no key `{key}`");
        }
    }
    let drop = match table.get("drop") {
        None => Vec::new(),
        Some(item) => item
            .as_array()
            .context("`[[tags.from]] drop` must be a list of tag names")?
            .iter()
            .map(|v| {
                let name = v.as_str().unwrap_or_default();
                crate::tags::Field::named(name)
                    .with_context(|| format!("`[[tags.from]] drop`: no tag is named `{name}`"))
            })
            .collect::<Result<_>>()?,
    };
    let credits = match table.get("credits") {
        None => true,
        Some(item) => item
            .as_bool()
            .context("`[[tags.from]] credits` must be true or false")?,
    };
    let title = text("title")?
        .map(|t| {
            crate::overrides::Pattern::parse(&t)
                .map_err(|e| anyhow::anyhow!("`[[tags.from]] title` {e}"))
        })
        .transpose()?;
    let (torrent, channel) = (text("torrent")?, text("channel")?);
    match (&torrent, &channel) {
        (None, None) => bail!("a `[[tags.from]]` names no `torrent` or `channel`"),
        (Some(_), Some(_)) => bail!(
            "a `[[tags.from]]` names both a `torrent` and a `channel`; give each a table of its own"
        ),
        _ => {}
    }
    if let Some(hash) = &torrent
        && !crate::torrent::is_hash(&hash.trim().to_ascii_lowercase())
    {
        bail!(
            "`[[tags.from]] torrent`: `{hash}` is no info hash, {} or {} hex digits",
            crate::torrent::V1_HEX,
            crate::torrent::V2_HEX
        );
    }
    if channel.as_deref().is_some_and(|c| c.trim().is_empty()) {
        bail!("`[[tags.from]] channel` names no channel");
    }
    Ok(crate::overrides::Override {
        torrent,
        channel,
        drop,
        credits,
        title,
    })
}

/// The switches `table` holds, of the rules of `tier`.
fn switch(settings: &mut Settings, table: &str, tier: &str, rules: &dyn TableLike) -> Result<()> {
    for (key, item) in rules.iter() {
        let on = item
            .as_bool()
            .with_context(|| format!("`{table}.{key}` must be true or false"))?;
        if !settings.set(&format!("{tier}.{key}"), on) {
            settings.unknown.push(format!("{table}.{key}"));
        }
    }
    Ok(())
}

/// The last place a table of `item` holds in the file, its own tables'
/// included.
fn last_position(item: &Item) -> Option<isize> {
    let t = item.as_table()?;
    t.iter()
        .filter_map(|(_, i)| last_position(i))
        .chain(t.position())
        .max()
}

/// Each tier's `[lyrics.clean.<tier>]` and `[tags.clean.<tier>]`, with
/// every switch it lacks added on, placed after the settings so it is
/// found at the top of the file. A table placed alike prints in the
/// order the file holds it, so the lyrics' tiers, under a settings table,
/// come first, and a song added after them prints after them.
fn clean_template(doc: &mut DocumentMut) -> bool {
    let after = crate::settings::WRITTEN
        .iter()
        .filter_map(|t| doc.get(t)?.as_table())
        .flat_map(|t| {
            t.iter()
                .filter(|(k, _)| *k != "clean")
                .filter_map(|(_, i)| last_position(i))
                .chain(t.position())
        })
        .max();
    let position = after.map(|p| p + 1);
    let root = doc.as_table_mut();
    let mut changed = false;
    // A `[tags]` that only held its tiers until `fill` gave it a setting
    // goes just above them.
    if let Some(tags) = root.get_mut("tags").and_then(Item::as_table_mut)
        && tags.position().is_none()
    {
        tags.set_position(position);
    }
    // Of tables placed alike, the one first in key order prints first, and
    // one placed nowhere takes the place before it; so `tags` goes after the
    // settings, and the songs after its tiers.
    let rank = |k: &str| match k {
        "version" | "edition" => 0,
        "tags" => 2,
        k if crate::settings::WRITTEN.contains(&k) => 1,
        _ => 3,
    };
    root.sort_values_by(|a, _, b, _| rank(a.get()).cmp(&rank(b.get())));
    let aspects: [(&str, Vec<_>); 2] = [
        ("lyrics", clean::lyrics::switches().collect()),
        ("tags", clean::switches().collect()),
    ];
    for (aspect, switches) in aspects {
        let Some(table) = root.get_mut(aspect).and_then(Item::as_table_mut) else {
            continue;
        };
        if !table.contains_key("clean") {
            let mut t = Table::new();
            t.set_implicit(true);
            t.set_position(position);
            table.insert("clean", Item::Table(t));
            changed = true;
        }
        let Some(clean) = table.get_mut("clean").and_then(Item::as_table_mut) else {
            continue;
        };
        changed |= switches_template(clean, position, switches.into_iter());
        let rank = |k: &str| {
            clean::TIERS
                .iter()
                .position(|(t, _)| *t == k)
                .unwrap_or(usize::MAX)
        };
        clean.sort_values_by(|a, _, b, _| rank(a.get()).cmp(&rank(b.get())));
    }
    changed
}

/// Each tier's table in `clean`, with every switch of `switches` it
/// lacks added on.
fn switches_template(
    clean: &mut Table,
    position: Option<isize>,
    switches: impl Iterator<Item = (&'static str, &'static str)>,
) -> bool {
    let switches: Vec<_> = switches.collect();
    let mut changed = false;
    for (tier, about) in clean::TIERS {
        if !switches.iter().any(|(t, _)| *t == tier) {
            continue;
        }
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
        for (_, key) in switches.iter().filter(|(t, _)| *t == tier) {
            if !t.contains_key(key) {
                t.insert(key, value(true));
                changed = true;
            }
        }
    }
    changed
}

/// Strings, numbers, booleans (as `1` and `0`), TOML dates, and lists of
/// them, each read as its field's kind reads it; an empty value sets
/// nothing. A value its field cannot hold, as a date that is none, or of
/// any other type, as a table, refuses the song list, naming the song as
/// `what`; but a language no ISO 639 table names, which an earlier muman
/// set, is left out and said in `left_out`, so the list still reads. Two
/// spellings of one tag's name both setting it, as `tags.title` and
/// `tags.Title`, refuse it too, as neither is the one meant; an empty one
/// beside another, which an earlier `set` wrote, sets nothing.
fn tags_of(item: Option<&Item>, what: &str, left_out: &mut Vec<String>) -> Result<Tags> {
    let Some(t) = item.and_then(Item::as_table_like) else {
        return Ok(Vec::new());
    };
    let scalar = |key: &str, v: &Value| match v {
        Value::String(s) => Ok(Some(s.value().trim().to_string()).filter(|s| !s.is_empty())),
        Value::Integer(i) => Ok(Some(i.value().to_string())),
        Value::Float(f) => Ok(Some(f.value().to_string())),
        Value::Boolean(b) => Ok(Some(if *b.value() { "1" } else { "0" }.to_string())),
        Value::Datetime(d) => Ok(Some(d.value().to_string())),
        _ => Err(anyhow::anyhow!(
            "{what}: tags.{key} is no text, number, yes or no, date or list of them"
        )),
    };
    let mut tags: Tags = Vec::new();
    for (key, item) in t.iter() {
        let Some(value) = item.as_value() else {
            bail!("{what}: tags.{key} is no text, number, yes or no, date or list of them");
        };
        let raw: Vec<String> = match value {
            Value::Array(list) => list
                .iter()
                .map(|v| scalar(key, v))
                .filter_map(Result::transpose)
                .collect::<Result<_>>()?,
            v => scalar(key, v)?.into_iter().collect(),
        };
        let mut values = Vec::new();
        for v in &raw {
            match tags::by_hand(key, v) {
                // A value read as nothing, as one of marks alone, sets nothing.
                Ok(v) if v.is_empty() => {}
                Ok(v) => values.push(v),
                Err(_) if tags::unnamed_language(key, v) => left_out.push(format!(
                    "{what}: tags.{key} = {v:?} names no language ISO 639 has a code for, so \
                     it is left out; set the language by its code, as jpn or ja"
                )),
                Err(why) => bail!("{what}: tags.{why}"),
            }
        }
        if values.is_empty() {
            continue;
        }
        let field = tags::vorbis_key(key);
        if let Some((twin, _)) = tags.iter().find(|(k, _)| tags::vorbis_key(k) == field) {
            let name = tags::song_list_name(key);
            let why = if twin.eq_ignore_ascii_case(key) {
                "as a tag's name is read whatever its case".to_string()
            } else {
                format!("{twin} and {key} being two names of one tag")
            };
            let text = tags::Field::named(key).is_none_or(|f| f.spec().kind == tags::Kind::Text);
            let both = if text {
                ", listing both values there to set both"
            } else {
                ""
            };
            bail!("{what}: tags.{twin} and tags.{key} both set {name}, {why}; keep one{both}");
        }
        tags.push((key.to_string(), values));
    }
    Ok(tags)
}

#[cfg(test)]
mod tests;
