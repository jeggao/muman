//! Fetched sources and lookup records no song uses, deleted: an upload a
//! release took the place of, a source taken out of its song by hand, a
//! record no song lists any more. `status` and `info` name them; `purge`
//! is what deletes them.
//!
//! A removed song's sources stay, since `restore` lists it again with
//! them, and a file in the manual folder is never touched: one no song
//! lists is added by the next sync. Nothing here can be undone, but
//! nothing is lost either: a fetched source a song lists again is fetched
//! again by the next sync.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::atomic::Lock;
use crate::cli::Confirm;
use crate::dirs::Dirs;
use crate::manifest::Manifest;
use crate::source::id_of;
use crate::store::{self, Store};
use crate::ui::Prompter;

/// Delete every fetched file and kept record no listed or removed song
/// uses, once confirmed. Returns whether anything went.
pub fn purge<W: Write>(
    dirs: &Dirs,
    confirm: &Confirm,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let _lock = Lock::folder(&dirs.home)?;
    let manifest = Manifest::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let mut known = manifest.keys();
    known.extend(manifest.removed_keys());
    let doomed = unused_files(dirs, &store, &known)?;
    if doomed.is_empty() {
        crate::ui::success(out, "Nothing unused to delete")?;
        return Ok(false);
    }
    let size = |p: &PathBuf| std::fs::metadata(p).map_or(0, |m| m.len());
    let total: u64 = doomed.iter().map(size).sum();
    crate::ui::info(
        out,
        &format!(
            "Deleting {} unused file(s), {}:",
            doomed.len(),
            crate::ui::bytes(total)
        ),
    )?;
    for file in &doomed {
        writeln!(out, "  {}", file.display())?;
    }
    if !crate::change::confirmed(confirm, prompter, out)? {
        return Ok(false);
    }
    for file in &doomed {
        match crate::atomic::remove(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("deleting {}", file.display()));
            }
            _ => {}
        }
    }
    crate::ui::success(out, &format!("Freed {}", crate::ui::bytes(total)))?;
    Ok(true)
}

/// Every file of a fetched source or a kept record no key of `known`
/// names, with the files that come with it.
fn unused_files(
    dirs: &Dirs,
    store: &Store,
    known: &BTreeSet<crate::source::SourceKey>,
) -> Result<Vec<PathBuf>> {
    let mut doomed = BTreeSet::new();
    for path in store.unused(known) {
        if path.starts_with(dirs.ytdlp()) {
            if let Some(id) = id_of(&path) {
                doomed.extend(store.fetched_files(id)?);
            }
            continue;
        }
        let kind = store::KEPT.iter().find(|k| path.starts_with(k.dir(dirs)));
        let id = path.file_stem().and_then(|s| s.to_str());
        if let (Some(kind), Some(id)) = (kind, id) {
            doomed.extend(kind.files(dirs, id));
        }
    }
    Ok(doomed.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dirs: &Dirs, rel: &str) -> PathBuf {
        let path = dirs.home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rel).unwrap();
        path
    }

    #[test]
    fn what_no_song_uses_goes_and_the_rest_stays() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: dir.path().join("home"),
            library: dir.path().join("lib"),
        };
        let gone = [
            touch(&dirs, "sources/yt-dlp/c/Old [ooooooooooo].mkv"),
            touch(&dirs, "sources/lrclib/8.lrc"),
            touch(&dirs, "sources/lrclib/8.json"),
        ];
        let kept = [
            touch(&dirs, "sources/yt-dlp/c/Song [aaaaaaaaaaa].mkv"),
            touch(&dirs, "sources/yt-dlp/c/Gone [rrrrrrrrrrr].mkv"),
            touch(&dirs, "sources/lrclib/7.lrc"),
            touch(&dirs, "sources/manual/mine.flac"),
        ];
        std::fs::write(
            dirs.manifest(),
            "version = 1\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"lrclib:7\"]\n\
             [[removed]]\nnote = \"Gone\"\nsources = [\"youtube.com:rrrrrrrrrrr\"]\n",
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
        assert!(text.contains("Deleting 3 unused file(s)"), "{text}");

        let confirm = Confirm {
            yes: true,
            all: false,
            dry_run: false,
        };
        assert!(purge(&dirs, &confirm, None, &mut Vec::new()).unwrap());
        assert!(gone.iter().all(|p| !p.exists()));
        assert!(kept.iter().all(|p| p.exists()));
    }
}
