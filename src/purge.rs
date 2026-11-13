//! Fetched sources and lookup records no song uses, deleted: an upload a
//! release took the place of, a source taken out of its song by hand, a
//! record no song lists any more, a torrent's file no song lists and its
//! metainfo once no song lists any of its files. `status` and `info` name
//! them; `purge` is what deletes them.
//!
//! A removed song's sources stay, since `restore` lists it again with
//! them, and a file in the manual folder is never touched, one no song
//! lists being added by the next sync, but for the lyrics and pictures
//! `add` copied into its `added/`, muman's copies of files the user
//! keeps where they were. A copy is muman's only while it holds the
//! bytes the state recorded as `add` made it, so it is deleted as made
//! or not at all: one the user changed since, or any file the user put
//! in `added/`, is the user's, which nothing here could give back. A
//! copy deleted is forgotten. A folder a purge leaves empty, as a torrent's,
//! goes with it, up to the folder of its kind of source, which stays.
//! Nothing here can be undone, but nothing is lost either: a fetched
//! source a song lists again is fetched again by the next sync.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::atomic::Lock;
use crate::cli::Confirm;
use crate::dirs::{ADDED, Dirs};
use crate::manifest::Manifest;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::Store;
use crate::ui::Prompter;

/// Delete every fetched file and kept record no listed or removed song
/// uses, once confirmed. Returns whether anything went.
// Deletes the fetched files and records no song uses.
#[expect(clippy::disallowed_methods)]
pub fn purge<W: Write>(
    dirs: &Dirs,
    confirm: &Confirm,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let doomed = unused_now(dirs)?;
    if doomed.is_empty() {
        crate::ui::success(out, "Nothing unused to delete")?;
        return Ok(false);
    }
    let size = |p: &PathBuf| std::fs::metadata(p).map_or(0, |m| m.len());
    let total: u64 = doomed.iter().map(size).sum();
    let files = crate::change::counted(doomed.len(), "unused file");
    crate::ui::info(
        out,
        &format!("Deleting {files}, {}:", crate::ui::bytes(total)),
    )?;
    for file in &doomed {
        crate::ui::line(out, &format!("  {}", file.display()))?;
    }
    let ask = crate::change::Ask::YesNo(format!("Delete {files}?"));
    if !crate::change::confirmed(confirm, &ask, prompter, out)? {
        return Ok(false);
    }
    // Taken only once answered, as a run takes it: a run under way, as an
    // add fetching what no song lists yet, is waited for, and what it made
    // used since the question is kept.
    let _runs = Lock::runs(&dirs.home)?;
    let still = unused_now(dirs)?;
    let (doomed, kept): (Vec<PathBuf>, Vec<PathBuf>) =
        doomed.into_iter().partition(|f| still.contains(f));
    for file in &kept {
        crate::ui::info(out, &format!("Kept, used since: {}", file.display()))?;
    }
    let total: u64 = doomed.iter().map(size).sum();
    for file in &doomed {
        match crate::atomic::remove(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("deleting {}", file.display()));
            }
            _ => {}
        }
    }
    remove_emptied(dirs, &doomed);
    let copies: BTreeSet<SourceKey> = (doomed.iter())
        .filter_map(|f| f.strip_prefix(dirs.manual()).ok())
        .map(|rel| SourceKey::Manual(rel.into()))
        .collect();
    State::keep_copies(&dirs.home, &BTreeMap::new(), &copies)?;
    crate::ui::success(out, &format!("Freed {}", crate::ui::bytes(total)))?;
    Ok(true)
}

/// Remove each folder the deleted `files` leave empty, as a torrent's
/// own, up to the folder of their kind of source, which stays.
// Removes the folders a purge empties; one still holding anything stays.
#[expect(clippy::disallowed_methods)]
fn remove_emptied(dirs: &Dirs, files: &[PathBuf]) {
    let roots: Vec<PathBuf> = [dirs.ytdlp(), dirs.torrent(), dirs.manual().join(ADDED)]
        .into_iter()
        .chain(crate::store::KEPT.iter().map(|k| k.dir(dirs)))
        .collect();
    let mut folders: BTreeSet<PathBuf> = BTreeSet::new();
    for file in files {
        let Some(root) = roots.iter().find(|r| file.starts_with(r)) else {
            continue;
        };
        folders.extend(
            (file.ancestors().skip(1))
                .take_while(|d| *d != root.as_path())
                .map(Path::to_path_buf),
        );
    }
    let mut folders: Vec<PathBuf> = folders.into_iter().collect();
    folders.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for folder in folders {
        let _ = std::fs::remove_dir(folder);
    }
}

/// What no listed or removed song uses now, by the song list read whole.
fn unused_now(dirs: &Dirs) -> Result<BTreeSet<PathBuf>> {
    let (manifest, state) = {
        let _lock = Lock::folder(&dirs.home)?;
        (Manifest::load(&dirs.home)?, State::load(&dirs.home)?)
    };
    let store = Store::scan(dirs)?;
    let mut known = manifest.keys();
    known.extend(manifest.removed_keys());
    let named = store.purgeable(dirs, &known, &state.copies);
    Ok(store
        .with_their_files(dirs, &known, &named)
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn touch(dirs: &Dirs, rel: &str) -> PathBuf {
        let path = dirs.home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rel).unwrap();
        path
    }

    /// Answers yes, having listed what `lists` names meanwhile, as a run
    /// under way would.
    struct ListsMeanwhile {
        manifest: PathBuf,
        lists: &'static str,
    }

    impl Prompter for ListsMeanwhile {
        fn ask(&mut self, _: &str) -> std::io::Result<bool> {
            std::fs::write(&self.manifest, self.lists)?;
            Ok(true)
        }

        fn choose(&mut self, _: &str, _: &[String], p: &[usize]) -> std::io::Result<Vec<usize>> {
            Ok(p.to_vec())
        }

        fn select(&mut self, _: &str, _: &[String]) -> std::io::Result<Option<usize>> {
            Ok(None)
        }

        fn text(&mut self, _: &str, _: &str) -> std::io::Result<Option<String>> {
            Ok(None)
        }

        fn text_with(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: crate::ui::Check,
        ) -> std::io::Result<Option<String>> {
            Ok(None)
        }
    }

    #[test]
    fn what_a_song_lists_while_purge_asks_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
            locks: Some(dir.path().to_path_buf()),
        };
        let used = touch(&dirs, "sources/lrclib/7.lrc");
        let gone = touch(&dirs, "sources/lrclib/8.lrc");
        std::fs::write(dirs.manifest(), "version = 4\n").unwrap();
        let mut asked = ListsMeanwhile {
            manifest: dirs.manifest(),
            lists: "version = 4\n[[song]]\nsources = [\"lrclib:7\"]\n",
        };
        let confirm = Confirm {
            yes: false,
            all: false,
            dry_run: false,
        };
        let mut out = Vec::new();
        assert!(purge(&dirs, &confirm, Some(&mut asked), &mut out).unwrap());
        let text = String::from_utf8(out).unwrap();
        assert!(used.exists(), "{text}");
        assert!(!gone.exists(), "{text}");
        assert!(text.contains("Kept, used since"), "{text}");
    }

    #[test]
    fn what_no_song_uses_goes_and_the_rest_stays() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
            locks: Some(dir.path().to_path_buf()),
        };
        let listed = crate::testing::metainfo("Lantern Weather", &["a.flac", "b.flac"]);
        let listed_hash = crate::torrent::Metainfo::read(&listed)
            .unwrap()
            .hash()
            .to_string();
        let other = crate::testing::metainfo("The Glass Orchards", &["c.flac"]);
        let other_hash = crate::torrent::Metainfo::read(&other)
            .unwrap()
            .hash()
            .to_string();
        let torrents = dirs.torrent();
        std::fs::create_dir_all(&torrents).unwrap();
        std::fs::write(store::metainfo_path(&torrents, &listed_hash), &listed).unwrap();
        std::fs::write(store::metainfo_path(&torrents, &other_hash), &other).unwrap();
        let gone = [
            touch(&dirs, "sources/yt-dlp/c/Old [ooooooooooo].mkv"),
            touch(&dirs, "sources/yt-dlp/d/Old [ddddddddddd].mkv"),
            touch(
                &dirs,
                "sources/musicbrainz/00000000-0000-0000-0000-000000000001.json",
            ),
            touch(&dirs, "sources/lrclib/8.lrc"),
            touch(&dirs, "sources/lrclib/8.json"),
            touch(&dirs, "sources/lrclib/9.LRC"),
            touch(&dirs, "sources/lrclib/10.json"),
            touch(&dirs, &format!("sources/torrent/{listed_hash}/b.flac")),
            touch(&dirs, &format!("sources/torrent/{other_hash}/c.flac")),
            store::metainfo_path(&torrents, &other_hash),
        ];
        let kept = [
            touch(&dirs, "sources/yt-dlp/c/Song [aaaaaaaaaaa].mkv"),
            touch(&dirs, "sources/yt-dlp/c/Gone [rrrrrrrrrrr].mkv"),
            touch(&dirs, "sources/lrclib/7.lrc"),
            touch(&dirs, "sources/manual/mine.flac"),
            touch(&dirs, &format!("sources/torrent/{listed_hash}/a.flac")),
            store::metainfo_path(&torrents, &listed_hash),
        ];
        std::fs::write(
            dirs.manifest(),
            format!(
                "version = 4\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"lrclib:7\"]\n\
                 [[song]]\nsources = [\"torrent:{listed_hash}/a.flac\"]\n\
                 [[removed]]\nnote = \"Gone\"\nsources = [\"youtube.com:rrrrrrrrrrr\"]\n"
            ),
        )
        .unwrap();
        let confirm = Confirm {
            yes: false,
            all: false,
            dry_run: true,
        };
        let mut out = Vec::new();
        assert!(!purge(&dirs, &confirm, None, &mut out).unwrap());
        assert!(gone.iter().all(|p| p.exists()), "a dry run deletes nothing");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Deleting 10 unused files"), "{text}");

        let confirm = Confirm {
            yes: true,
            all: false,
            dry_run: false,
        };
        assert!(purge(&dirs, &confirm, None, &mut Vec::new()).unwrap());
        assert!(gone.iter().all(|p| !p.exists()));
        assert!(kept.iter().all(|p| p.exists()));
        assert!(
            !torrents.join(&other_hash).exists(),
            "a folder emptied goes"
        );
        assert!(!dirs.ytdlp().join("d").exists());
        assert!(
            dirs.sources("musicbrainz").is_dir(),
            "a kind's folder stays"
        );
    }

    #[test]
    fn a_stray_at_a_listed_key_s_place_goes_and_the_key_s_file_stays() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
            locks: Some(dir.path().to_path_buf()),
        };
        let listed = touch(&dirs, "sources/yt-dlp/youtube.com/vid00000001.mkv");
        let strays = [
            touch(&dirs, "sources/yt-dlp/youtube.com/vid00000001.jpg"),
            touch(&dirs, "sources/yt-dlp/youtube.com/vid00000001.mka"),
        ];
        std::fs::write(
            dirs.manifest(),
            "version = 4\n[[song]]\nsources = [\"youtube.com:vid00000001\"]\n",
        )
        .unwrap();
        let confirm = Confirm {
            yes: true,
            all: false,
            dry_run: false,
        };
        let mut out = Vec::new();
        assert!(purge(&dirs, &confirm, None, &mut out).unwrap());
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Deleting 2 unused files"), "{text}");
        assert!(listed.exists(), "{text}");
        assert!(strays.iter().all(|p| !p.exists()), "{text}");
    }

    #[test]
    fn a_listed_torrent_whose_metainfo_is_gone_keeps_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
            locks: Some(dir.path().to_path_buf()),
        };
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let kept = [
            touch(&dirs, &format!("sources/torrent/{hash}/CD1/a.flac")),
            touch(&dirs, &format!("sources/torrent/{hash}/CD1/cover.jpg")),
        ];
        std::fs::write(
            dirs.manifest(),
            format!("version = 4\n[[song]]\nsources = [\"torrent:{hash}/CD1/a.flac\"]\n"),
        )
        .unwrap();
        let confirm = Confirm {
            yes: true,
            all: false,
            dry_run: false,
        };
        let mut out = Vec::new();
        assert!(!purge(&dirs, &confirm, None, &mut out).unwrap());
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Nothing unused to delete"), "{text}");
        assert!(kept.iter().all(|p| p.exists()), "{text}");
    }
}
