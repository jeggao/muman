//! The sources on disk: what yt-dlp fetched, found by the ID in each
//! file's name, the records LRCLIB, MusicBrainz and Cover Art Archive
//! lookups kept, by theirs, and the manual folder, where a song's file brings the lyrics
//! and pictures beside it.
//!
//! What each kind of kept record is, its folder, its files and what it
//! offers, is said once, in [`KEPT`]: the store, `remove --purge`, the
//! providers and `info` all read it there, so a provider added is one
//! entry. A key of any other scheme is a file yt-dlp fetched.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::dirs::Dirs;
use crate::provider::{COVERART, LRCLIB, MUSICBRAINZ, Provider};
use crate::source::{SourceKey, id_of};

/// A manual file this recent may still be copying in.
pub const SETTLING: Duration = Duration::from_secs(10);

const MEDIA: [&str; 20] = [
    "flac", "wav", "aif", "aiff", "m4a", "mp3", "ogg", "oga", "opus", "wv", "ape", "tta", "aac",
    "mka", "mkv", "mp4", "webm", "mov", "alac", "dsf",
];
const IMAGES: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
/// The names a picture covers every song in its folder by.
const FOLDER_COVERS: [&str; 4] = ["cover", "folder", "front", "album"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Audio, with or without video.
    Media,
    Lyrics,
    Image,
    /// A MusicBrainz record, which offers tags alone.
    Tags,
}

fn kind_of(path: &Path) -> Option<Kind> {
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
        folder: "lrclib",
        main: &["lrc"],
        beside: &["json"],
        kind: Kind::Lyrics,
        valid: |id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
    },
    Kept {
        extractor: MUSICBRAINZ,
        provider: Provider::MusicBrainz,
        folder: "musicbrainz",
        main: &["json"],
        beside: &[],
        kind: Kind::Tags,
        valid: crate::musicbrainz::is_mbid,
    },
    Kept {
        extractor: COVERART,
        provider: Provider::CoverArt,
        folder: "coverart",
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
    /// modification times.
    #[must_use]
    pub fn rev(&self) -> String {
        std::iter::once(&self.path)
            .chain(&self.lyrics)
            .chain(&self.covers)
            .map(|p| stamp(p).map_or_else(|| "-".to_string(), |(size, ns)| format!("{size}:{ns}")))
            .collect::<Vec<_>>()
            .join(";")
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
    let (size, ns) = rev.split(';').next()?.split_once(':')?;
    Some((size.parse().ok()?, ns.parse().ok()?))
}

#[derive(Debug, Default)]
pub struct Store {
    manual_root: PathBuf,
    fetched_root: PathBuf,
    /// yt-dlp's files by ID, a `.mkv` before any other.
    fetched: HashMap<String, PathBuf>,
    /// Kept records by scheme, then ID.
    kept: HashMap<&'static str, HashMap<String, PathBuf>>,
    /// Manual files by their path in the manual folder.
    manual: BTreeMap<PathBuf, Kind>,
    /// The name on disk of each manual file whose key differs from it: a
    /// decomposed name is listed composed, and Linux and NTFS open only
    /// the bytes a name was written with.
    on_disk: HashMap<PathBuf, PathBuf>,
}

impl Store {
    /// Read both folders; a folder not made yet holds nothing.
    pub fn scan(dirs: &Dirs) -> Result<Self> {
        let mut store = Self {
            manual_root: dirs.manual(),
            fetched_root: dirs.ytdlp(),
            ..Self::default()
        };
        for file in walk(&dirs.ytdlp(), 2)? {
            let Some(id) = id_of(&file).map(str::to_string) else {
                continue;
            };
            if kind_of(&file) != Some(Kind::Media) {
                continue;
            }
            let better = store
                .fetched
                .get(&id)
                .is_none_or(|have| have.extension().is_none_or(|e| e != "mkv"));
            if better {
                store.fetched.insert(id, file);
            }
        }
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
        let root = dirs.manual();
        for file in walk(&root, usize::MAX)? {
            if let (Some(kind), Ok(rel)) = (kind_of(&file), file.strip_prefix(&root)) {
                let key = crate::relpath::normalized(rel);
                if key != rel {
                    store.on_disk.insert(key.clone(), rel.to_path_buf());
                }
                store.manual.insert(key, kind);
            }
        }
        Ok(store)
    }

    #[must_use]
    pub fn has(&self, key: &SourceKey) -> bool {
        match key {
            SourceKey::Remote { site, id } => match kept(site) {
                Some(kind) => self.kept_file(kind, id).is_some(),
                None => self.fetched.contains_key(id),
            },
            SourceKey::Manual(rel) => self.manual.contains_key(rel.path()),
        }
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
            SourceKey::Remote { id, .. } => Some(Located {
                key: key.clone(),
                path: self.fetched.get(id)?.clone(),
                kind: Kind::Media,
                lyrics: None,
                covers: Vec::new(),
            }),
            SourceKey::Manual(manual) => {
                let rel = manual.path();
                let kind = *self.manual.get(rel)?;
                let (lyrics, covers) = if kind == Kind::Media {
                    self.sidecars(rel)
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

    /// Every file yt-dlp fetched for `id`, whatever its extension.
    pub fn fetched_files(&self, id: &str) -> Result<Vec<PathBuf>> {
        Ok(walk(&self.fetched_root, 2)?
            .into_iter()
            .filter(|p| id_of(p) == Some(id))
            .collect())
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
            let SourceKey::Remote { site, id } = key else {
                continue;
            };
            if kept(site).is_some() {
                continue;
            }
            for file in self.fetched_files(id)? {
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

    /// The `.lrc` and pictures that come with a manual song file.
    fn sidecars(&self, rel: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
        let stem = rel.with_extension("");
        let named = |kind: Kind| -> Vec<PathBuf> {
            self.manual
                .iter()
                .filter(|(p, k)| **k == kind && p.with_extension("") == stem)
                .map(|(p, _)| p.clone())
                .collect()
        };
        let lyrics = named(Kind::Lyrics).into_iter().next();
        let mut covers = named(Kind::Image);
        let folder = rel.parent().unwrap_or(Path::new(""));
        covers.extend(
            self.manual
                .iter()
                .filter(|(p, k)| {
                    **k == Kind::Image
                        && p.parent().unwrap_or(Path::new("")) == folder
                        && p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| {
                            FOLDER_COVERS.contains(&s.to_ascii_lowercase().as_str())
                        })
                })
                .map(|(p, _)| p.clone()),
        );
        (lyrics, covers)
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
                .is_some_and(|t| now.duration_since(t).unwrap_or_default() < wait);
            if fresh {
                settling.push(rel.clone());
            } else {
                ready.push(key);
            }
        }
        (ready, settling)
    }

    /// Every file in either folder no song lists and nothing listed
    /// brings with it: clutter a status run names, never deletes.
    #[must_use]
    pub fn unused(&self, listed: &BTreeSet<SourceKey>) -> Vec<PathBuf> {
        let mut used: BTreeSet<PathBuf> = BTreeSet::new();
        for key in listed {
            if let Some(l) = self.locate(key) {
                used.insert(l.path);
                used.extend(l.lyrics);
                used.extend(l.covers);
            }
        }
        let mut unused: Vec<PathBuf> = self
            .fetched
            .values()
            .chain(self.kept.values().flat_map(HashMap::values))
            .chain(
                self.manual
                    .keys()
                    .map(|r| self.manual_file(r))
                    .collect::<Vec<_>>()
                    .iter(),
            )
            .filter(|p| !used.contains(*p))
            .cloned()
            .collect();
        unused.sort();
        unused
    }
}

/// Files below `dir`, at most `depth` folders down, without hidden ones
/// and without downloads not yet finished.
pub(crate) fn walk(dir: &Path, depth: usize) -> Result<Vec<PathBuf>> {
    files_below(dir, depth, |name| {
        let unfinished = [".part", ".crdownload", ".tmp", ".ytdl"]
            .iter()
            .any(|s| name.ends_with(s));
        !name.starts_with('.') && !unfinished
    })
}

/// Files below `dir`, at most `depth` folders down, in path order, each
/// named, as is every folder between, as `keep` allows; a folder not
/// made yet holds nothing.
///
/// A linked file or folder is taken as what it links to, except a link
/// to a folder it is inside, which would hold the same files again
/// without end; a link that leads nowhere, to nothing or round to
/// itself, is passed over, as a file deleted during the walk is.
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
        assert!(store.unused(&BTreeSet::from([key])).is_empty());
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
