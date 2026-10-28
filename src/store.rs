//! The sources on disk: what yt-dlp fetched, each at its key's place,
//! the records LRCLIB, MusicBrainz and Cover Art Archive lookups kept, by
//! their IDs, and the manual folder, where a song's file brings the
//! lyrics and pictures beside it.
//!
//! A fetched source is kept at a path of its key alone,
//! `sources/yt-dlp/<site>/<id>.<ext>` ([`fetched_path`]), its ID spelled
//! as every system takes a name: a key has one place, so it has one file, a download
//! resumes at one path whatever the uploader is called this time, and a
//! file anywhere else is not a source ([`Store::leftovers`]). yt-dlp
//! writes a run's finished files into a scratch folder of that run's in
//! the store, [`crate::atomic::Scratch`], and each is moved to its place
//! as the run ends ([`place`]): hidden, so no scan takes what is in it for
//! a source; in the store, so a file is renamed in, not copied, and kept
//! apart from the unfinished downloads, which are cleared by their age,
//! as a file yt-dlp dated by the server's clock would seem; and gone with
//! the run, with what yt-dlp could not embed, as a JSON subtitle, and a
//! file no place could take, so nothing a run fetched outlives it but at
//! its key's place. A file in the layout before,
//! named by the uploader and title as `<uploader>/<title> [<id>].mkv`,
//! stays where it is and is read by its ID; only what is fetched from now
//! on is kept by its key.
//!
//! What each kind of kept record is, its folder, its files and what it
//! offers, is said once, in [`KEPT`]: the store, `remove --purge`, the
//! providers and `info` all read it there, so a provider added is one
//! entry. A key of any other scheme is a file yt-dlp fetched.
//!
//! A manual song file brings the `.lrc` and pictures named as it is in
//! any case, as a folder's `COVER.JPG` covers it and as a filesystem
//! blind to case holds them: one in its own case first, and none named
//! exactly as another song file, which is that file's.
//!
//! Lyrics and pictures `add` gives to songs are copied into the manual
//! folder's `added/`, which holds no song file, so none of them becomes
//! a song's sidecar or a folder's cover by its name.
//!
//! A torrent's files are kept in a folder of its hash beside its
//! metainfo, `sources/torrent/<hash>.torrent`, each at the path
//! [`crate::torrent::stored`] gives it. The metainfo says which files a
//! torrent has, so a file is read as a source only while its torrent's
//! metainfo is there, and anything else in the folder is a leftover. A
//! torrent's song file brings its lyrics and pictures by the manual
//! folder's rules, from among its own torrent's files alone. A listed
//! song file whose torrent's metainfo is gone is read where its key puts
//! it, without what it brings, and nothing in its torrent's folder is
//! unused until the metainfo is back.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::dirs::Dirs;
use crate::provider::{COVERART, LRCLIB, MUSICBRAINZ, Provider};
use crate::source::{SourceKey, TorrentKey, id_of};
use crate::torrent::Metainfo;

/// The most bytes a part of a stored path takes, its extension aside:
/// under the 255 bytes or UTF-16 units a name may take on any filesystem
/// muman writes to, a character never taking fewer bytes in UTF-8 than
/// units in UTF-16, with room for an extension.
pub const PART_BYTES: usize = 200;

/// Where the file of the fetched source `key` is kept, of extension
/// `ext`: `<site>/<id>.<ext>`, its ID spelled by `stored_id`; a spelling
/// longer than [`PART_BYTES`] is cut into folders, each of the most that
/// fits, the last holding the file, and read back joined. `None` for a
/// key no fetch makes.
#[must_use]
pub fn fetched_path(root: &Path, key: &SourceKey, ext: &str) -> Option<PathBuf> {
    let SourceKey::Remote { site, id } = key else {
        return None;
    };
    if kept(site).is_some() {
        return None;
    }
    let mut parts = cut(&stored_id(id));
    let last = parts.pop()?;
    let mut path = root.join(site);
    path.extend(parts);
    Some(path.join(format!("{last}.{ext}")))
}

/// `name` in parts of [`PART_BYTES`] at most, whole characters each,
/// none beginning with a `.`, which would hide it: a `.` past the end
/// stays with the part before, short of a run of dots too long for any
/// name, which no ID holds.
fn cut(name: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    for c in name.chars() {
        let len = parts.last().map_or(0, String::len);
        let full = len + c.len_utf8() > PART_BYTES;
        if full && (c != '.' || len >= PART_BYTES + 40) {
            parts.push(String::new());
        }
        if let Some(part) = parts.last_mut() {
            part.push(c);
        }
    }
    parts
}

/// Whether `c` stands in a stored file's name as it is: letters, digits
/// and `-_.()+,=@!'&~`, and a letter outside ASCII but `⧸`.
fn kept_in_name(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || "-_.()+,=@!'&~".contains(c)
    } else {
        c != '⧸' && !c.is_whitespace()
    }
}

/// A key's ID as its file's name spells it: each `/` written `⧸`, as
/// yt-dlp writes one in a name, and each character [`kept_in_name`]
/// refuses written `%` and its UTF-8 bytes in hex, so a name holds
/// nothing Windows, FAT or a phone refuses and reads back as the one ID.
/// A torrent's file is named by lookalikes instead ([`crate::torrent::stored`]),
/// as its metainfo says what each is; this name is all a fetched file's
/// key is read from. An ID of letters, digits and `-_.()+,=@!'&~` alone,
/// as every ID was before others were taken, is named by itself.
fn stored_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for (n, part) in id.split('/').enumerate() {
        if n > 0 {
            out.push('⧸');
        }
        for c in part.chars() {
            if kept_in_name(c) {
                out.push(c);
            } else {
                for b in c.encode_utf8(&mut [0; 4]).bytes() {
                    let _ = write!(out, "%{b:02X}");
                }
            }
        }
    }
    out
}

/// The ID a stored file's name spells, where [`stored_id`] spells it so.
fn id_of_stored(name: &str) -> Option<String> {
    let id = name
        .split('⧸')
        .map(|part| {
            percent_encoding::percent_decode_str(part)
                .decode_utf8()
                .ok()
                .map(std::borrow::Cow::into_owned)
        })
        .collect::<Option<Vec<String>>>()?
        .join("/");
    (stored_id(&id) == name).then_some(id)
}

/// Where a torrent's metainfo is kept in `root`, the torrents' folder.
#[must_use]
pub fn metainfo_path(root: &Path, hash: &str) -> PathBuf {
    root.join(format!("{hash}.torrent"))
}

/// Where the torrent's file `key` is kept in `root`, the torrents' folder.
#[must_use]
pub fn torrent_path(root: &Path, key: &TorrentKey) -> PathBuf {
    root.join(key.hash())
        .join(crate::torrent::stored(key.path()))
}

/// Move `from`, a torrent's file aria2c finished, to its key's place in
/// `root`, the torrents' folder. Refused where a filesystem that ignores
/// case holds another file there, of a path differing only in case.
pub fn place_torrent(root: &Path, key: &TorrentKey, from: &Path) -> Result<PathBuf> {
    let to = torrent_path(root, key);
    let folder = to.parent().unwrap_or(root);
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let name = to.file_name().unwrap_or_default();
    let named = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .filter_map(|e| Some(e.ok()?.file_name()))
        .any(|n| n == name);
    if to.exists() && !named {
        anyhow::bail!(
            "{} holds another file of the torrent, its path differing from {}'s only in case, \
             which this filesystem does not tell apart",
            to.display(),
            key.path().display()
        );
    }
    crate::atomic::rename(from, &to)
        .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
    Ok(to)
}

/// The key of a file at its key's place in `root`, as [`fetched_path`]
/// names it.
fn key_at(root: &Path, file: &Path) -> Option<SourceKey> {
    let rel = file.strip_prefix(root).ok()?;
    let mut parts = rel.components().map(|c| c.as_os_str().to_str());
    let site = parts.next()??;
    let mut name = parts.collect::<Option<Vec<&str>>>()?;
    let last = Path::new(name.pop()?);
    let stem = last.file_stem()?.to_str()?;
    let id = id_of_stored(&format!("{}{stem}", name.concat()))?;
    let key = SourceKey::parse(&format!("{site}:{id}")).ok()?;
    let ext = last.extension()?.to_str()?;
    // One place per key: a name cut where it need not be is none.
    (fetched_path(root, &key, ext).as_deref() == Some(file)).then_some(key)
}

/// Move `from`, what yt-dlp fetched for `key`, to its key's place in
/// `root`, replacing the key's file there and dropping one of another
/// extension, so a key has one file. Refused where a filesystem that
/// ignores case holds another key's file there, of an ID differing from
/// this one's only in case.
pub fn place(root: &Path, key: &SourceKey, from: &Path) -> Result<PathBuf> {
    let ext = from.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
    let to =
        fetched_path(root, key, ext).with_context(|| format!("{key} is fetched by no site"))?;
    if to == from {
        return Ok(to);
    }
    let folder = to.parent().unwrap_or(root);
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let stem = to.file_stem().unwrap_or_default().to_os_string();
    let names: Vec<std::ffi::OsString> = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .filter_map(|e| Some(e.ok()?.file_name()))
        .collect();
    let to_name = to.file_name().unwrap_or_default();
    if to.exists() && !names.iter().any(|n| n == to_name) {
        anyhow::bail!(
            "{} holds another source's file, its ID differing from {key}'s only in case, \
             which this filesystem does not tell apart",
            to.display()
        );
    }
    crate::atomic::rename(from, &to)
        .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
    for name in &names {
        let other = folder.join(name);
        if name != to_name
            && other.file_stem() == Some(stem.as_os_str())
            && kind_of(&other) == Some(Kind::Media)
        {
            crate::atomic::remove(&other)
                .with_context(|| format!("deleting {}", other.display()))?;
        }
    }
    Ok(to)
}

/// A manual file this recent may still be copying in.
pub const SETTLING: Duration = Duration::from_secs(10);

pub(crate) const MEDIA: [&str; 37] = [
    "flac", "wav", "w64", "rf64", "aif", "aiff", "aifc", "caf", "m4a", "m4b", "mp3", "mp2", "ogg",
    "oga", "opus", "spx", "wv", "ape", "tta", "tak", "shn", "aac", "ac3", "eac3", "dts", "thd",
    "mlp", "mka", "mkv", "mp4", "webm", "mov", "alac", "dsf", "dff", "wma", "au",
];
pub(crate) const IMAGES: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
/// The names a picture covers every song in its folder by.
pub(crate) const FOLDER_COVERS: [&str; 4] = ["cover", "folder", "front", "album"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Audio, with or without video.
    Media,
    Lyrics,
    Image,
    /// A MusicBrainz record, which offers tags alone.
    Tags,
}

/// Whether two files are in one folder and named alike but for their
/// extensions, in any case: `Song.JPG` comes with `song.flac`, as a
/// folder's `COVER.JPG` covers it.
pub(crate) fn named_as(a: &Path, b: &Path) -> bool {
    let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().to_lowercase());
    a.parent() == b.parent() && stem(a) == stem(b)
}

pub(crate) fn kind_of(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if MEDIA.contains(&ext.as_str()) {
        Some(Kind::Media)
    } else if ext == "lrc" {
        Some(Kind::Lyrics)
    } else if IMAGES.contains(&ext.as_str()) {
        Some(Kind::Image)
    } else {
        None
    }
}

/// A kind of record a lookup keeps in the store, by the scheme of its
/// keys, `<provider>:<id>`.
#[derive(Debug, Clone, Copy)]
pub struct Kept {
    pub extractor: &'static str,
    pub provider: Provider,
    /// Its folder in `sources`.
    pub folder: &'static str,
    /// The extensions the file a key names may have, in order of
    /// preference.
    pub main: &'static [&'static str],
    /// Extensions of the files kept beside it.
    pub beside: &'static [&'static str],
    pub kind: Kind,
    /// Whether an ID has the shape this scheme's take.
    pub valid: fn(&str) -> bool,
}

/// Every kind of record a lookup keeps.
pub const KEPT: [Kept; 3] = [
    Kept {
        extractor: LRCLIB,
        provider: Provider::Lrclib,
        folder: LRCLIB,
        main: &["lrc"],
        beside: &["json"],
        kind: Kind::Lyrics,
        valid: |id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
    },
    Kept {
        extractor: MUSICBRAINZ,
        provider: Provider::MusicBrainz,
        folder: MUSICBRAINZ,
        main: &["json"],
        beside: &[],
        kind: Kind::Tags,
        valid: crate::musicbrainz::is_mbid,
    },
    Kept {
        extractor: COVERART,
        provider: Provider::CoverArt,
        folder: COVERART,
        main: &["jpg", "png", "webp"],
        beside: &[],
        kind: Kind::Image,
        valid: crate::musicbrainz::is_mbid,
    },
];

/// The kind of record a key of `extractor` is, if a lookup keeps it.
#[must_use]
pub fn kept(extractor: &str) -> Option<&'static Kept> {
    KEPT.iter().find(|k| k.extractor == extractor)
}

impl Kept {
    /// Its folder in the home.
    #[must_use]
    pub fn dir(&self, dirs: &Dirs) -> PathBuf {
        dirs.sources(self.folder)
    }

    /// The files kept for `id` that are on disk: the one a key names and
    /// those beside it.
    #[must_use]
    pub fn files(&self, dirs: &Dirs, id: &str) -> Vec<PathBuf> {
        self.main
            .iter()
            .chain(self.beside)
            .map(|ext| self.dir(dirs).join(format!("{id}.{ext}")))
            .filter(|p| p.exists())
            .collect()
    }
}

/// A source found on disk, with the manual files that come with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub key: SourceKey,
    pub path: PathBuf,
    pub kind: Kind,
    /// The `.lrc` beside a manual file, named as it is.
    pub lyrics: Option<PathBuf>,
    /// Pictures beside a manual file: its own name first, then the
    /// folder's cover.
    pub covers: Vec<PathBuf>,
}

impl Located {
    /// What changes whenever any of its files does: their sizes and
    /// modification times, and where the system keeps one, the time each
    /// last changed at all, which a tagger that puts the modification time
    /// back, or a copy that keeps it, cannot.
    #[must_use]
    pub fn rev(&self) -> String {
        std::iter::once(&self.path)
            .chain(&self.lyrics)
            .chain(&self.covers)
            .map(|p| {
                let Some((size, ns)) = stamp(p) else {
                    return "-".to_string();
                };
                match changed_ns(p) {
                    Some(changed) => format!("{size}:{ns}:{changed}"),
                    None => format!("{size}:{ns}"),
                }
            })
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// A revision as muman wrote it before revisions held change times.
#[must_use]
pub fn without_change_times(rev: &str) -> String {
    rev.split(';')
        .map(|part| part.splitn(3, ':').take(2).collect::<Vec<_>>().join(":"))
        .collect::<Vec<_>>()
        .join(";")
}

/// When `path` last changed in any way, data or name or mode, in
/// nanoseconds; Unix keeps it, other systems not as `std` reads them.
fn changed_ns(path: &Path) -> Option<u128> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        let secs = u128::try_from(meta.ctime()).ok()?;
        let ns = u128::try_from(meta.ctime_nsec()).ok()?;
        Some(secs * 1_000_000_000 + ns)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// A file's size and modification time in nanoseconds.
#[must_use]
pub fn stamp(path: &Path) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    let ns = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), ns))
}

/// [`stamp`] as text, as an output's is recorded.
#[must_use]
pub fn stamp_text(path: &Path) -> Option<String> {
    stamp(path).map(|(size, ns)| format!("{size}:{ns}"))
}

/// The size and time a revision recorded for its main file.
#[must_use]
pub fn stamp_of_rev(rev: &str) -> Option<(u64, u128)> {
    let mut fields = rev.split(';').next()?.split(':');
    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}

#[derive(Debug, Default)]
pub struct Store {
    manual_root: PathBuf,
    fetched_root: PathBuf,
    /// What yt-dlp fetched, each at its key's place.
    fetched: BTreeMap<SourceKey, PathBuf>,
    /// What yt-dlp fetched in the layout before, by the ID its name
    /// holds, a `.mkv` first.
    legacy: HashMap<String, Vec<PathBuf>>,
    /// Files in the store that are no source: a subtitle yt-dlp left
    /// beside its video, a second file of one key, a file of no key.
    leftovers: Vec<PathBuf>,
    /// Kept records by scheme, then ID.
    kept: HashMap<&'static str, HashMap<String, PathBuf>>,
    /// Manual files by their path in the manual folder.
    manual: BTreeMap<PathBuf, Kind>,
    /// The name on disk of each manual file whose key differs from it: a
    /// decomposed name is listed composed, and Linux and NTFS open only
    /// the bytes a name was written with.
    on_disk: HashMap<PathBuf, PathBuf>,
    /// Manual files passed over for another's name: the same once
    /// normalized, as `café` written composed and decomposed.
    twins: Vec<PathBuf>,
    torrent_root: PathBuf,
    /// Each torrent whose metainfo is kept, by hash: the paths in it of
    /// its files on disk that are sources, lyrics or pictures.
    torrents: BTreeMap<String, BTreeMap<PathBuf, Kind>>,
}

impl Store {
    /// Read both folders; a folder not made yet holds nothing.
    pub fn scan(dirs: &Dirs) -> Result<Self> {
        let mut store = Self {
            manual_root: dirs.manual(),
            fetched_root: dirs.ytdlp(),
            torrent_root: dirs.torrent(),
            ..Self::default()
        };
        let root = dirs.ytdlp();
        let mkv_first = |a: &PathBuf, b: &PathBuf| {
            let mkv = |p: &PathBuf| p.extension().is_none_or(|e| e != "mkv");
            mkv(a).cmp(&mkv(b)).then_with(|| a.cmp(b))
        };
        // Deep enough for an ID of thousands of bytes, cut into folders.
        for file in walk(&root, 32)? {
            let media = kind_of(&file) == Some(Kind::Media);
            let depth = file
                .strip_prefix(&root)
                .map_or(0, |r| r.components().count());
            if let (true, 2, Some(id)) = (media, depth, id_of(&file)) {
                store.legacy.entry(id).or_default().push(file);
                continue;
            }
            let Some(key) = key_at(&root, &file).filter(|_| media) else {
                store.leftovers.push(file);
                continue;
            };
            match store.fetched.remove(&key) {
                Some(have) => {
                    let mut both = [have, file];
                    both.sort_by(mkv_first);
                    let [keep, extra] = both;
                    store.leftovers.push(extra);
                    store.fetched.insert(key, keep);
                }
                None => {
                    store.fetched.insert(key, file);
                }
            }
        }
        for files in store.legacy.values_mut() {
            files.sort_by(mkv_first);
            store.leftovers.extend(files.drain(1..));
        }
        store.leftovers.sort();
        for kind in &KEPT {
            let mut found: HashMap<String, PathBuf> = HashMap::new();
            for file in walk(&kind.dir(dirs), 1)? {
                let (Some(id), Some(ext)) = (
                    file.file_stem().and_then(|s| s.to_str()),
                    file.extension().and_then(|e| e.to_str()),
                ) else {
                    continue;
                };
                let Some(rank) = kind.main.iter().position(|m| m.eq_ignore_ascii_case(ext)) else {
                    continue;
                };
                let better = found.get(id).is_none_or(|have| {
                    have.extension()
                        .and_then(|e| e.to_str())
                        .and_then(|e| kind.main.iter().position(|m| m.eq_ignore_ascii_case(e)))
                        .is_none_or(|r| rank < r)
                });
                if (kind.valid)(id) && better {
                    found.insert(id.to_string(), file.clone());
                }
            }
            store.kept.insert(kind.extractor, found);
        }
        store.scan_torrents()?;
        let root = dirs.manual();
        // A folder linked in from the library would bring muman's own songs
        // back as new ones, run after run.
        let library = dunce::canonicalize(&dirs.library).ok();
        let in_library = |file: &Path| {
            library.as_ref().is_some_and(|lib| {
                dunce::canonicalize(file).is_ok_and(|real| real.starts_with(lib))
            })
        };
        for file in walk(&root, usize::MAX)? {
            if in_library(&file) {
                continue;
            }
            if let (Some(kind), Ok(rel)) = (kind_of(&file), file.strip_prefix(&root)) {
                let key = crate::relpath::normalized(rel);
                // The first in path order is read, the same on every run.
                if store.manual.contains_key(&key) {
                    store.twins.push(rel.to_path_buf());
                    continue;
                }
                if key != rel {
                    store.on_disk.insert(key.clone(), rel.to_path_buf());
                }
                store.manual.insert(key, kind);
            }
        }
        Ok(store)
    }

    /// Read each torrent's metainfo and find which of its files are on
    /// disk; every other file in the torrents' folder is a leftover.
    fn scan_torrents(&mut self) -> Result<()> {
        let root = self.torrent_root.clone();
        let mut sources: BTreeSet<PathBuf> = BTreeSet::new();
        for file in walk(&root, 1)? {
            let hash = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let is_metainfo =
                file.extension().is_some_and(|e| e == "torrent") && crate::torrent::is_hash(&hash);
            let meta = is_metainfo
                .then(|| std::fs::read(&file).ok())
                .flatten()
                .and_then(|bytes| Metainfo::read(&bytes).ok())
                .filter(|m| m.hash() == hash);
            let Some(meta) = meta else {
                self.leftovers.push(file);
                continue;
            };
            sources.insert(file);
            let mut found = BTreeMap::new();
            let shadowed: BTreeSet<&Path> = crate::torrent::shadowed(&meta)
                .into_iter()
                .map(|(lost, _)| lost)
                .collect();
            for f in meta
                .files
                .iter()
                .filter(|f| !shadowed.contains(f.path.as_path()))
            {
                let key = TorrentKey::new(&hash, &f.path);
                let at = torrent_path(&root, &key);
                if let Some(kind) = kind_of(&f.path).filter(|_| at.is_file()) {
                    sources.insert(at);
                    found.insert(key.path().to_path_buf(), kind);
                }
            }
            self.torrents.insert(hash, found);
        }
        for file in walk(&root, usize::MAX)? {
            if !sources.contains(&file) {
                self.leftovers.push(file);
            }
        }
        self.leftovers.sort();
        self.leftovers.dedup();
        Ok(())
    }

    /// Manual files not read, for another's name, as [`Store`] keeps them.
    #[must_use]
    pub fn twins(&self) -> &[PathBuf] {
        &self.twins
    }

    /// Files in the store that are no source, which `purge` deletes.
    #[must_use]
    pub fn leftovers(&self) -> &[PathBuf] {
        &self.leftovers
    }

    #[must_use]
    pub fn has(&self, key: &SourceKey) -> bool {
        match key {
            SourceKey::Remote { site, id } => match kept(site) {
                Some(kind) => self.kept_file(kind, id).is_some(),
                None => self.fetched_file(key).is_some(),
            },
            SourceKey::Manual(rel) => self.manual.contains_key(rel.path()),
            SourceKey::Torrent(t) => match self.torrents.get(t.hash()) {
                Some(files) => files.contains_key(t.path()),
                None => self.unread_torrent_file(t).is_some(),
            },
        }
    }

    /// The file of `key` at its place, of a torrent whose metainfo is
    /// gone: a song file, read as one without what it brings, which only
    /// the metainfo tells.
    fn unread_torrent_file(&self, key: &TorrentKey) -> Option<PathBuf> {
        let at = torrent_path(&self.torrent_root, key);
        (kind_of(key.path()) == Some(Kind::Media) && at.is_file()).then_some(at)
    }

    /// Whether the metainfo of the torrent of `hash` is kept.
    #[must_use]
    pub fn has_metainfo(&self, hash: &str) -> bool {
        self.torrents.contains_key(hash)
    }

    /// The file of the fetched source `key`: at its place, else in the
    /// layout before, by its ID.
    fn fetched_file(&self, key: &SourceKey) -> Option<&PathBuf> {
        self.fetched.get(key).or_else(|| {
            let id = key.id()?;
            let bare = id.rsplit('/').next().unwrap_or(id);
            self.legacy
                .get(id)
                .or_else(|| self.legacy.get(bare))
                .and_then(|files| files.first())
        })
    }

    #[must_use]
    pub fn locate(&self, key: &SourceKey) -> Option<Located> {
        match key {
            SourceKey::Remote { site, id } if kept(site).is_some() => {
                let kind = kept(site)?;
                Some(Located {
                    key: key.clone(),
                    path: self.kept_file(kind, id)?.clone(),
                    kind: kind.kind,
                    lyrics: None,
                    covers: Vec::new(),
                })
            }
            SourceKey::Remote { .. } => Some(Located {
                key: key.clone(),
                path: self.fetched_file(key)?.clone(),
                kind: Kind::Media,
                lyrics: None,
                covers: Vec::new(),
            }),
            SourceKey::Torrent(t) if !self.torrents.contains_key(t.hash()) => Some(Located {
                key: key.clone(),
                path: self.unread_torrent_file(t)?,
                kind: Kind::Media,
                lyrics: None,
                covers: Vec::new(),
            }),
            SourceKey::Torrent(t) => {
                let files = self.torrents.get(t.hash())?;
                let kind = *files.get(t.path())?;
                let (lyrics, covers) = if kind == Kind::Media {
                    sidecars(files, t.path())
                } else {
                    (None, Vec::new())
                };
                let at = |p: &Path| torrent_path(&self.torrent_root, &TorrentKey::new(t.hash(), p));
                Some(Located {
                    key: key.clone(),
                    path: at(t.path()),
                    kind,
                    lyrics: lyrics.as_deref().map(at),
                    covers: covers.iter().map(|c| at(c)).collect(),
                })
            }
            SourceKey::Manual(manual) => {
                let rel = manual.path();
                let kind = *self.manual.get(rel)?;
                let (lyrics, covers) = if kind == Kind::Media {
                    sidecars(&self.manual, rel)
                } else {
                    (None, Vec::new())
                };
                Some(Located {
                    key: key.clone(),
                    path: self.manual_file(rel),
                    kind,
                    lyrics: lyrics.map(|l| self.manual_file(&l)),
                    covers: covers.iter().map(|c| self.manual_file(c)).collect(),
                })
            }
        }
    }

    /// Every file yt-dlp fetched for `key`: its file, and what it left
    /// beside it, as a subtitle or a file of another extension.
    #[must_use]
    pub fn fetched_files(&self, key: &SourceKey) -> Vec<PathBuf> {
        let Some(id) = key.id() else {
            return Vec::new();
        };
        let bare = id.rsplit('/').next().unwrap_or(id);
        let mut files: Vec<PathBuf> = self.fetched_file(key).into_iter().cloned().collect();
        let place = fetched_path(&self.fetched_root, key, "mkv");
        let stem = place.as_deref().and_then(Path::file_stem);
        let tagged = format!("[{bare}]");
        files.extend(
            self.leftovers
                .iter()
                .filter(|p| {
                    let at_place = place.as_deref().and_then(Path::parent) == p.parent()
                        && p.file_name().zip(stem).is_some_and(|(name, stem)| {
                            name.to_string_lossy()
                                .starts_with(&format!("{}.", stem.to_string_lossy()))
                        });
                    let named = p
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().contains(&tagged));
                    at_place || named
                })
                .cloned(),
        );
        files.sort();
        files.dedup();
        files
    }

    /// Every file yt-dlp fetched that comes with `file`, one of them: the
    /// files of its key, or of the ID its name holds in the layout before,
    /// or a file of no key alone.
    #[must_use]
    pub fn fetched_files_at(&self, file: &Path) -> Vec<PathBuf> {
        if let Some(key) = key_at(&self.fetched_root, file) {
            return self.fetched_files(&key);
        }
        let Some(id) = id_of(file) else {
            return vec![file.to_path_buf()];
        };
        let tagged = format!("[{id}]");
        let mut files: Vec<PathBuf> = self
            .legacy
            .get(&id)
            .into_iter()
            .flatten()
            .chain(self.leftovers.iter().filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().contains(&tagged))
            }))
            .cloned()
            .collect();
        files.push(file.to_path_buf());
        files.sort();
        files.dedup();
        files
    }

    /// Delete what yt-dlp fetched for each of `keys` no song in `listed`
    /// has, as an upload a release took the place of; returns what went.
    pub fn discard(
        &self,
        keys: &[SourceKey],
        listed: &BTreeSet<SourceKey>,
    ) -> Result<Vec<PathBuf>> {
        let mut gone = Vec::new();
        for key in keys.iter().filter(|k| !listed.contains(*k)) {
            let SourceKey::Remote { site, .. } = key else {
                continue;
            };
            if kept(site).is_some() {
                continue;
            }
            for file in self.fetched_files(key) {
                match crate::atomic::remove(&file) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(e).with_context(|| format!("deleting {}", file.display()));
                    }
                    _ => gone.push(file),
                }
            }
        }
        Ok(gone)
    }

    /// Where the manual file listed as `rel` is on disk.
    fn manual_file(&self, rel: &Path) -> PathBuf {
        self.manual_root
            .join(self.on_disk.get(rel).map_or(rel, PathBuf::as_path))
    }

    fn kept_file(&self, kind: &Kept, id: &str) -> Option<&PathBuf> {
        self.kept.get(kind.extractor)?.get(id)
    }

    /// Manual song files no song lists, in path order, and those
    /// still settling, which wait for a later run.
    #[must_use]
    pub fn unlisted(
        &self,
        listed: &BTreeSet<SourceKey>,
        now: SystemTime,
        wait: Duration,
    ) -> (Vec<SourceKey>, Vec<PathBuf>) {
        let (mut ready, mut settling) = (Vec::new(), Vec::new());
        for (rel, kind) in &self.manual {
            let key = SourceKey::Manual(rel.into());
            if *kind != Kind::Media || listed.contains(&key) {
                continue;
            }
            let fresh = std::fs::metadata(self.manual_file(rel))
                .ok()
                .and_then(|m| crate::platform::arrived(&m))
                .is_some_and(|t| {
                    // A time ahead of the clock is a skewed stamp, as a
                    // camera or a FAT stick leaves, not a copy under way.
                    now.duration_since(t)
                        .unwrap_or_else(|ahead| ahead.duration())
                        < wait
                });
            if fresh {
                settling.push(rel.clone());
            } else {
                ready.push(key);
            }
        }
        (ready, settling)
    }

    /// Every file in the store no song lists and nothing listed brings
    /// with it, and each torrent's metainfo no listed file is of: clutter
    /// a status run names, never deletes. A listed torrent whose metainfo
    /// is gone keeps its folder whole: without it, which of its files a
    /// song lists or brings cannot be told.
    #[must_use]
    pub fn unused(&self, listed: &BTreeSet<SourceKey>) -> Vec<PathBuf> {
        let mut used: BTreeSet<PathBuf> = BTreeSet::new();
        let mut unread: BTreeSet<PathBuf> = BTreeSet::new();
        for key in listed {
            if let SourceKey::Torrent(t) = key {
                used.insert(metainfo_path(&self.torrent_root, t.hash()));
                if !self.torrents.contains_key(t.hash()) {
                    unread.insert(self.torrent_root.join(t.hash()));
                }
            }
            if let Some(l) = self.locate(key) {
                used.insert(l.path);
                used.extend(l.lyrics);
                used.extend(l.covers);
            }
        }
        let torrents: Vec<PathBuf> = self
            .torrents
            .iter()
            .flat_map(|(hash, files)| {
                std::iter::once(metainfo_path(&self.torrent_root, hash)).chain(
                    files
                        .keys()
                        .map(|p| torrent_path(&self.torrent_root, &TorrentKey::new(hash, p))),
                )
            })
            .collect();
        let mut unused: Vec<PathBuf> = self
            .fetched
            .values()
            .chain(&torrents)
            .chain(self.legacy.values().flatten())
            .chain(&self.leftovers)
            .chain(self.kept.values().flat_map(HashMap::values))
            .chain(
                self.manual
                    .keys()
                    .map(|r| self.manual_file(r))
                    .collect::<Vec<_>>()
                    .iter(),
            )
            .filter(|p| !used.contains(*p) && !unread.iter().any(|d| p.starts_with(d)))
            .cloned()
            .collect();
        unused.sort();
        unused
    }
}

/// The `.lrc` and pictures that come with the song file `rel` among
/// `files`, a folder's or a torrent's: named as it is in any case, those
/// in its own case first, and none named in its case as another song
/// file is; then its folder's cover.
fn sidecars(files: &BTreeMap<PathBuf, Kind>, rel: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
    let stem = rel.with_extension("");
    let another_s = |p: &Path| {
        files.iter().any(|(m, k)| {
            *k == Kind::Media && m != rel && m.with_extension("") == p.with_extension("")
        })
    };
    let named = |kind: Kind| -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = files
            .iter()
            .filter(|(p, k)| **k == kind && named_as(p, rel))
            .filter(|(p, _)| p.with_extension("") == stem || !another_s(p))
            .map(|(p, _)| p.clone())
            .collect();
        found.sort_by_key(|p| p.with_extension("") != stem);
        found
    };
    let lyrics = named(Kind::Lyrics).into_iter().next();
    let mut covers = named(Kind::Image);
    let folder = rel.parent().unwrap_or(Path::new(""));
    covers.extend(
        files
            .iter()
            .filter(|(p, k)| {
                **k == Kind::Image
                    && p.parent().unwrap_or(Path::new("")) == folder
                    && p.file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| FOLDER_COVERS.contains(&s.to_ascii_lowercase().as_str()))
            })
            .map(|(p, _)| p.clone()),
    );
    (lyrics, covers)
}

/// Each listed manual file gone from its path that is one of `ready`,
/// the manual files no song lists, by its size and modification time,
/// which a move keeps: the file moved there, taken out of `ready`. A
/// listed `.lrc` or picture, which `ready` never holds, is followed to
/// one no song lists either.
pub fn moved_manual(
    listed: &BTreeSet<SourceKey>,
    state: &crate::state::State,
    store: &Store,
    ready: &mut Vec<SourceKey>,
) -> BTreeMap<SourceKey, SourceKey> {
    let mut moved = BTreeMap::new();
    let mut loose: Vec<SourceKey> = store
        .manual
        .iter()
        .filter(|(_, kind)| **kind != Kind::Media)
        .map(|(rel, _)| SourceKey::Manual(rel.into()))
        .filter(|k| !listed.contains(k))
        .collect();
    let gone = listed
        .iter()
        .filter(|k| matches!(k, SourceKey::Manual(_)) && !store.has(k));
    for gone in gone {
        let Some(was) = state.facts.get(gone).and_then(|f| stamp_of_rev(&f.rev)) else {
            continue;
        };
        let media = matches!(gone, SourceKey::Manual(m) if kind_of(m.path()) == Some(Kind::Media));
        let candidates = if media { &mut *ready } else { &mut loose };
        let at = candidates
            .iter()
            .position(|k| store.locate(k).and_then(|l| stamp(&l.path)) == Some(was));
        if let Some(at) = at {
            moved.insert(gone.clone(), candidates.remove(at));
        }
    }
    moved
}

/// Files below `dir`, at most `depth` folders down, without hidden ones
/// and without downloads not yet finished.
pub(crate) fn walk(dir: &Path, depth: usize) -> Result<Vec<PathBuf>> {
    files_below(dir, depth, visible)
}

/// Whether a walk reads the file or folder named `name`: one not hidden
/// and no download yet unfinished.
pub(crate) fn visible(name: &str) -> bool {
    let unfinished = [".part", ".crdownload", ".tmp", ".ytdl"]
        .iter()
        .any(|s| name.ends_with(s));
    !name.starts_with('.') && !unfinished
}

/// Files below `dir`, at most `depth` folders down, in path order, each
/// named, as is every folder between, as `keep` allows; a folder not
/// made yet holds nothing.
///
/// A linked file or folder is taken as what it links to, except a link
/// to a folder it is inside, which would hold the same files again
/// without end; a link that leads nowhere, to nothing or round to
/// itself, is passed over, as a file deleted during the walk is. It
/// walks muman's own folders, whose links the user made; a folder given
/// to `add` is walked by [`crate::given::Folder::walk`], which follows a
/// link only to a place inside it.
pub(crate) fn files_below(
    dir: &Path,
    depth: usize,
    keep: impl Fn(&str) -> bool,
) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let entries = walkdir::WalkDir::new(dir)
        .follow_links(true)
        .min_depth(1)
        .max_depth(depth)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || keep(&e.file_name().to_string_lossy()));
    for entry in entries {
        match entry {
            Ok(e) if e.file_type().is_file() => found.push(e.into_path()),
            Ok(_) => {}
            Err(e) if e.loop_ancestor().is_some() || e.path().is_some_and(is_link) => {}
            Err(e)
                if e.io_error()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
    }
    found.sort();
    Ok(found)
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str) -> PathBuf {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rel.as_bytes()).unwrap();
        path
    }

    fn dirs(root: &Path) -> Dirs {
        Dirs {
            home: root.to_path_buf(),
            library: root.join("lib"),
        }
    }

    fn manual(rel: &str) -> SourceKey {
        SourceKey::Manual(rel.into())
    }

    /// A link at `link` to the folder `to`; `false` where the system
    /// refuses one, as Windows does outside developer mode.
    fn link_folder(to: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(to, link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(to, link);
        #[cfg(not(any(unix, windows)))]
        let made: std::io::Result<()> = Err(std::io::ErrorKind::Unsupported.into());
        made.is_ok()
    }

    #[test]
    fn a_torrents_files_are_read_by_its_metainfo_with_their_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let files = [
            "CD1/01 Paper Comets.flac",
            "CD1/01 paper comets.lrc",
            "CD1/Cover.jpg",
            "CD1/02 Rooms of Salt?.flac",
            "Scans/front.jpg",
            "notes.nfo",
        ];
        let bytes = crate::testing::metainfo("Lantern Weather", &files);
        let hash = Metainfo::read(&bytes).unwrap().hash().to_string();
        let root = d.torrent();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(metainfo_path(&root, &hash), &bytes).unwrap();
        let key =
            |p: &str| SourceKey::Torrent(TorrentKey::new(&hash, &crate::relpath::from_portable(p)));
        let place = |p: &str| {
            let SourceKey::Torrent(t) = key(p) else {
                unreachable!()
            };
            let at = torrent_path(&root, &t);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(&at, b"x").unwrap();
            at
        };
        let song = place("CD1/01 Paper Comets.flac");
        let lyrics = place("CD1/01 paper comets.lrc");
        let cover = place("CD1/Cover.jpg");
        let salt = place("CD1/02 Rooms of Salt?.flac");
        assert!(salt.ends_with(Path::new("CD1").join("02 Rooms of Salt？.flac")));
        place("notes.nfo");
        let stray = touch(&root, &format!("{hash}/stray.flac"));
        let orphan = touch(&root, &format!("{}/x.flac", "2".repeat(40)));

        let store = Store::scan(&d).unwrap();
        assert!(store.has(&key("CD1/01 Paper Comets.flac")));
        assert!(store.has_metainfo(&hash));
        assert!(!store.has(&key("Scans/front.jpg")), "not on disk");
        assert!(!store.has(&key("notes.nfo")), "no song, lyrics or picture");
        let located = store.locate(&key("CD1/01 Paper Comets.flac")).unwrap();
        assert_eq!(located.path, song);
        assert_eq!(located.kind, Kind::Media);
        assert_eq!(located.lyrics, Some(lyrics));
        assert_eq!(located.covers, std::slice::from_ref(&cover));
        assert!(store.leftovers().contains(&stray));
        assert!(store.leftovers().contains(&orphan));

        let listed = BTreeSet::from([key("CD1/01 Paper Comets.flac")]);
        let unused = store.unused(&listed);
        assert!(unused.contains(&salt));
        assert!(!unused.contains(&cover));
        assert!(!unused.contains(&metainfo_path(&root, &hash)));
        assert!(
            store
                .unused(&BTreeSet::new())
                .contains(&metainfo_path(&root, &hash))
        );
    }

    #[test]
    fn a_linked_folder_is_walked_once_and_a_link_back_up_not_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let manual = d.manual();
        touch(&manual, "Marlo Venn/Lantern Weather.flac");
        touch(dir.path(), "elsewhere/Glass Orchards.flac");
        let linked = link_folder(&manual, &manual.join("Marlo Venn").join("up"))
            && link_folder(&dir.path().join("elsewhere"), &manual.join("more"))
            && link_folder(&dir.path().join("gone"), &manual.join("gone"))
            && link_folder(&manual.join("round"), &manual.join("round"));
        if !linked {
            return;
        }
        let store = Store::scan(&d).unwrap();
        let found: Vec<String> = store
            .manual
            .keys()
            .map(|k| crate::relpath::show(k))
            .collect();
        assert_eq!(
            found,
            [
                "Marlo Venn/Lantern Weather.flac",
                "more/Glass Orchards.flac"
            ]
        );
    }

    #[test]
    fn a_folder_linked_in_from_the_library_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "lib/Marlo Venn/Lantern Weather.flac");
        touch(&d.manual(), "Glass Orchards.flac");
        if !link_folder(&d.library, &d.manual().join("mirror")) {
            return;
        }
        let store = Store::scan(&d).unwrap();
        let found: Vec<String> = store
            .manual
            .keys()
            .map(|k| crate::relpath::show(k))
            .collect();
        assert_eq!(found, ["Glass Orchards.flac"]);
    }

    #[test]
    fn two_files_of_one_name_once_normalized_are_read_once_and_said() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "sources/manual/Noe\u{308}l.flac");
        touch(dir.path(), "sources/manual/No\u{eb}l.flac");
        let store = Store::scan(&d).unwrap();
        assert_eq!(store.manual.len(), 1);
        // A filesystem blind to normalization, as APFS is, holds one file.
        if std::fs::read_dir(d.manual()).unwrap().count() == 2 {
            assert_eq!(store.twins().len(), 1);
        }
    }

    #[test]
    fn fetched_files_are_found_by_id_an_mkv_first() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "sources/yt-dlp/chan/A [aaaaaaaaaaa].webm");
        let mkv = touch(dir.path(), "sources/yt-dlp/chan/A [aaaaaaaaaaa].mkv");
        touch(dir.path(), "sources/yt-dlp/chan/B [bbbbbbbbbbb].mkv.part");
        touch(dir.path(), "sources/yt-dlp/chan/.hidden [ccccccccccc].mkv");
        let store = Store::scan(&d).unwrap();
        assert_eq!(
            store
                .locate(&SourceKey::youtube("aaaaaaaaaaa"))
                .unwrap()
                .path,
            mkv
        );
        assert!(!store.has(&SourceKey::youtube("bbbbbbbbbbb")));
        assert!(!store.has(&SourceKey::youtube("ccccccccccc")));
    }

    #[test]
    fn a_fetched_source_is_at_its_key_s_place_and_the_rest_is_left_over() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let mkv = touch(dir.path(), "sources/yt-dlp/youtube.com/aaaaaaaaaaa.mkv");
        let webm = touch(dir.path(), "sources/yt-dlp/youtube.com/aaaaaaaaaaa.webm");
        let vtt = touch(dir.path(), "sources/yt-dlp/youtube.com/aaaaaaaaaaa.en.vtt");
        let other = touch(dir.path(), "sources/yt-dlp/soundcloud.com/aaaaaaaaaaa.mkv");
        let loose = touch(dir.path(), "sources/yt-dlp/notes.txt");
        let tube = SourceKey::youtube("aaaaaaaaaaa");
        let cloud = SourceKey::parse("soundcloud.com:aaaaaaaaaaa").unwrap();
        let store = Store::scan(&d).unwrap();
        assert_eq!(store.locate(&tube).unwrap().path, mkv);
        assert_eq!(store.locate(&cloud).unwrap().path, other);
        assert_eq!(
            store.leftovers(),
            [loose.clone(), vtt.clone(), webm.clone()]
        );
        assert_eq!(
            store.fetched_files(&tube),
            [vtt.clone(), mkv.clone(), webm.clone()]
        );
        assert_eq!(store.fetched_files(&cloud), std::slice::from_ref(&other));
        assert_eq!(
            store.unused(&BTreeSet::from([tube, cloud])),
            [loose, vtt, webm],
            "what is no source is clutter"
        );
    }

    #[test]
    fn every_key_has_one_place_of_names_any_system_takes_and_reads_back() {
        let root = Path::new("sources").join("yt-dlp");
        let long = |part: &str, n: usize| part.repeat(n);
        for id in [
            "vid00000001".to_string(),
            format!("item-1/{}.mp3", long("%E6%97%85", 30)),
            format!("item-1/{}", long("夜の歌 ", 60)),
            format!("item-1/{}.mp3", long("a", 199)),
            format!("item-1/{}.mp3", long("a", 600)),
            format!("item-1/{}{}b.flac", long("a", 188), long(".", 10)),
        ] {
            let key = SourceKey::parse(&format!("archive.org:{id}")).unwrap();
            let at = fetched_path(&root, &key, "mkv").unwrap();
            for part in at.iter().map(|p| p.to_str().unwrap()) {
                assert!(part.len() <= 255, "{} bytes: {part}", part.len());
                assert!(part.encode_utf16().count() <= 255, "{part}");
                assert!(!part.starts_with('.'), "{part}");
            }
            assert_eq!(key_at(&root, &at), Some(key), "{}", at.display());
        }
        let key = SourceKey::parse(&format!("archive.org:item-1/{}", long("a", 300))).unwrap();
        let at = fetched_path(&root, &key, "mkv").unwrap();
        let stem = at
            .strip_prefix(root.join("archive.org"))
            .unwrap()
            .to_string_lossy();
        let cut_otherwise = root.join("archive.org").join(stem.replacen('a', "a/", 1));
        assert_eq!(key_at(&root, &cut_otherwise), None, "one place per key");
    }

    #[test]
    fn a_file_fetched_is_placed_at_its_key_and_replaces_the_key_s_other() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sources/yt-dlp");
        let key = SourceKey::youtube("-bbbbbbbbbb");
        let first = touch(
            dir.path(),
            "sources/yt-dlp/.incoming/Youtube/-bbbbbbbbbb.webm",
        );
        let placed = place(&root, &key, &first).unwrap();
        assert_eq!(placed, root.join("youtube.com").join("-bbbbbbbbbb.webm"));
        assert!(!first.exists());
        let again = touch(
            dir.path(),
            "sources/yt-dlp/.incoming/Youtube/-bbbbbbbbbb.mkv",
        );
        let placed = place(&root, &key, &again).unwrap();
        assert_eq!(placed, root.join("youtube.com").join("-bbbbbbbbbb.mkv"));
        assert!(!root.join("youtube.com").join("-bbbbbbbbbb.webm").exists());
        let store = Store::scan(&dirs(dir.path())).unwrap();
        assert_eq!(store.locate(&key).unwrap().path, placed);
        assert!(store.leftovers().is_empty(), "{:?}", store.leftovers());
        let list = SourceKey::parse("youtube.com:playlist/PLabc").unwrap();
        assert_eq!(
            fetched_path(&root, &list, "mkv").unwrap(),
            root.join("youtube.com").join("playlist⧸PLabc.mkv")
        );
        assert_eq!(
            key_at(&root, &root.join("youtube.com/playlist⧸PLabc.mkv")),
            Some(list)
        );
        assert_eq!(fetched_path(&root, &manual("a.flac"), "mkv"), None);
        let file = SourceKey::parse("archive.org:item-1/01 Lantern #2?.mp3").unwrap();
        let staged = touch(
            dir.path(),
            "sources/yt-dlp/.incoming/ArchiveOrg/item-1⧸01 Lantern #2？.mp3.mp3",
        );
        let placed = place(&root, &file, &staged).unwrap();
        assert_eq!(
            placed,
            root.join("archive.org")
                .join("item-1⧸01%20Lantern%20%232%3F.mp3.mp3")
        );
        assert_eq!(key_at(&root, &placed), Some(file));
        let at = |name: &str| key_at(&root, &root.join("archive.org").join(name));
        assert_eq!(at("item-1⧸01 Lantern.mp3.mkv"), None, "spelled otherwise");
        assert_eq!(at("item-1⧸%41.mp3.mkv"), None, "spelled otherwise");
    }

    #[test]
    fn kept_records_are_found_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let id = "00000000-0000-0000-0000-000000000001";
        let json = touch(dir.path(), &format!("sources/musicbrainz/{id}.json"));
        touch(dir.path(), "sources/musicbrainz/not-an-id.json");
        let store = Store::scan(&d).unwrap();
        let key = SourceKey::parse(&format!("musicbrainz:{id}")).unwrap();
        let located = store.locate(&key).unwrap();
        assert_eq!((located.path, located.kind), (json, Kind::Tags));
        assert!(!store.has(&SourceKey::parse("musicbrainz:not-an-id").unwrap()));
    }

    #[test]
    fn a_decomposed_manual_name_is_listed_composed_and_opened_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let song = touch(dir.path(), "sources/manual/Cafe\u{301}/Noe\u{308}l.flac");
        let lrc = touch(dir.path(), "sources/manual/Cafe\u{301}/Noe\u{308}l.lrc");
        let store = Store::scan(&d).unwrap();
        let key = manual("Caf\u{e9}/No\u{eb}l.flac");
        let located = store.locate(&key).unwrap();
        assert!(located.path.is_file());
        assert_eq!((located.path, located.lyrics), (song, Some(lrc)));
        assert_eq!(store.unused(&BTreeSet::from([key])), [] as [PathBuf; 0]);
    }

    #[test]
    fn a_manual_song_brings_its_lyrics_and_covers() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "sources/manual/Album/01 Song.flac");
        touch(dir.path(), "sources/manual/Album/02 Other.flac");
        let lrc = touch(dir.path(), "sources/manual/Album/01 Song.lrc");
        let own = touch(dir.path(), "sources/manual/Album/01 Song.png");
        let folder = touch(dir.path(), "sources/manual/Album/Cover.JPG");
        let store = Store::scan(&d).unwrap();
        let song = store.locate(&manual("Album/01 Song.flac")).unwrap();
        assert_eq!(song.lyrics, Some(lrc));
        assert_eq!(song.covers, vec![own, folder.clone()]);
        let other = store.locate(&manual("Album/02 Other.flac")).unwrap();
        assert_eq!(other.lyrics, None);
        assert_eq!(other.covers, vec![folder]);
    }

    #[test]
    fn a_sidecar_named_as_its_song_in_another_case_comes_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "sources/manual/Album/marlo-venn-01.flac");
        let lrc = touch(dir.path(), "sources/manual/Album/Marlo-Venn-01.LRC");
        let other = touch(dir.path(), "sources/manual/Album/MARLO-VENN-01.jpg");
        let own = touch(dir.path(), "sources/manual/Album/marlo-venn-01.png");
        let store = Store::scan(&d).unwrap();
        let song = store.locate(&manual("Album/marlo-venn-01.flac")).unwrap();
        assert_eq!(song.lyrics, Some(lrc));
        assert_eq!(song.covers, vec![own, other]);

        touch(dir.path(), "sources/manual/Tide/Tide.flac");
        touch(dir.path(), "sources/manual/Tide/tide.flac");
        let lrc = touch(dir.path(), "sources/manual/Tide/tide.lrc");
        let store = Store::scan(&d).unwrap();
        let folder = dir.path().join("sources").join("manual").join("Tide");
        let folder = std::fs::read_dir(folder).unwrap();
        // A filesystem blind to case holds one song file and one name.
        if folder.count() == 3 {
            let tide = |name| store.locate(&manual(name)).unwrap().lyrics;
            assert_eq!(tide("Tide/tide.flac"), Some(lrc));
            assert_eq!(tide("Tide/Tide.flac"), None);
        }
    }

    #[test]
    fn unlisted_songs_wait_while_settling() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "sources/manual/a.flac");
        touch(dir.path(), "sources/manual/b.mp3");
        touch(dir.path(), "sources/manual/b.lrc");
        let store = Store::scan(&d).unwrap();
        let listed = BTreeSet::from([manual("a.flac")]);
        let later = SystemTime::now() + Duration::from_secs(60);
        assert_eq!(
            store.unlisted(&listed, later, SETTLING),
            (vec![manual("b.mp3")], vec![])
        );
        let (ready, settling) = store.unlisted(&listed, SystemTime::now(), SETTLING);
        assert_eq!(ready, []);
        assert_eq!(settling, vec![PathBuf::from("b.mp3")]);
    }

    #[test]
    fn a_file_stamped_hours_ahead_of_the_clock_is_not_left_settling() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let file = touch(dir.path(), "sources/manual/a.flac");
        let later = SystemTime::now() + Duration::from_secs(60);
        let ahead = filetime::FileTime::from_system_time(later + Duration::from_secs(7200));
        filetime::set_file_mtime(&file, ahead).unwrap();
        let store = Store::scan(&d).unwrap();
        assert_eq!(
            store.unlisted(&BTreeSet::new(), later, SETTLING),
            (vec![manual("a.flac")], vec![])
        );
    }

    #[test]
    fn an_upload_no_song_lists_is_discarded_whole() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let upload = [
            touch(dir.path(), "sources/yt-dlp/c/Old [ooooooooooo].mkv"),
            touch(dir.path(), "sources/yt-dlp/c/Old [ooooooooooo].webm"),
        ];
        let listed = touch(dir.path(), "sources/yt-dlp/c/Kept [kkkkkkkkkkk].mkv");
        let store = Store::scan(&d).unwrap();
        let keys = [
            SourceKey::youtube("ooooooooooo"),
            SourceKey::youtube("kkkkkkkkkkk"),
        ];
        let gone = store
            .discard(&keys, &BTreeSet::from([SourceKey::youtube("kkkkkkkkkkk")]))
            .unwrap();
        assert_eq!(gone.len(), 2);
        assert!(upload.iter().all(|p| !p.exists()));
        assert!(listed.exists());
    }

    #[test]
    fn unused_files_are_what_nothing_listed_brings() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "sources/manual/a.flac");
        touch(dir.path(), "sources/manual/a.lrc");
        let stray = touch(dir.path(), "sources/manual/stray.png");
        let old = touch(dir.path(), "sources/yt-dlp/c/Old [ooooooooooo].mkv");
        let store = Store::scan(&d).unwrap();
        let listed = BTreeSet::from([manual("a.flac")]);
        assert_eq!(store.unused(&listed), vec![stray, old]);
    }

    #[cfg(unix)]
    #[test]
    fn an_edit_that_puts_the_time_back_changes_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        let file = touch(dir.path(), "sources/manual/a.flac");
        let rev = |d: &Dirs| {
            Store::scan(d)
                .unwrap()
                .locate(&manual("a.flac"))
                .unwrap()
                .rev()
        };
        let before = rev(&d);
        let modified = filetime::FileTime::from_last_modification_time(&file.metadata().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, "sources/manual/A.flac").unwrap();
        filetime::set_file_mtime(&file, modified).unwrap();
        let after = rev(&d);
        assert_ne!(before, after);
        assert_eq!(stamp_of_rev(&before), stamp_of_rev(&after));
        assert_eq!(without_change_times(&after), without_change_times(&before));
    }

    #[test]
    fn a_revision_changes_with_a_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let d = dirs(dir.path());
        touch(dir.path(), "sources/manual/a.flac");
        let store = Store::scan(&d).unwrap();
        let before = store.locate(&manual("a.flac")).unwrap().rev();
        touch(dir.path(), "sources/manual/a.lrc");
        let store = Store::scan(&d).unwrap();
        let after = store.locate(&manual("a.flac")).unwrap().rev();
        assert_ne!(before, after);
        assert_eq!(stamp_of_rev(&before), stamp_of_rev(&after));
    }
}
