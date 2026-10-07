//! The library's files as a sync changes them: every move, removal and
//! file written over goes through a [`Ledger`], which records it in the
//! run's history before making it, so a run stopped at any point is one
//! the next sync follows and `undo` reverses. Writing a song's new file
//! is [`crate::render`]'s, into a path the ledger has kept, or one no
//! file holds.
//!
//! A file is removed, or kept before it is written over, only as
//! [`Owned`]: one the state lists as muman's, its bytes as muman wrote
//! them, or its lyrics; or one a forced run takes over, kept for `undo`
//! first like any other. No other value names a file the ledger may
//! delete, so a file of the user's own cannot reach a removal by any path
//! through the code. `tests/architecture.rs` holds every other module to
//! changing no library file.
//!
//! A move that only changes a name's case goes through a temporary name,
//! which a filesystem blind to case would otherwise take for a rename
//! onto itself. A folder a removal or a move leaves empty goes too, the
//! files a file manager leaves in a folder it showed aside.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::history::Run;
use crate::state::Written;

/// A library file proved muman's to remove or write over. Made only by
/// [`Ledger::owned`] and [`Ledger::taken`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Owned(PathBuf);

impl Owned {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// The library a sync changes, and the run that records each change.
pub struct Ledger<'a> {
    root: &'a Path,
    home: &'a Path,
    run: Option<&'a mut Run>,
}

impl std::fmt::Debug for Ledger<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl<'a> Ledger<'a> {
    #[must_use]
    pub fn new(root: &'a Path, home: &'a Path, run: Option<&'a mut Run>) -> Self {
        Self { root, home, run }
    }

    /// `rel` as muman's: a file `outputs` lists whose stamp is as written
    /// or, when `forced`, whatever became of it; or the lyrics of one.
    #[must_use]
    pub fn owned(
        &self,
        outputs: &BTreeMap<PathBuf, Written>,
        rel: &Path,
        forced: bool,
    ) -> Option<Owned> {
        let audio = outputs.contains_key(rel)
            && (forced || !crate::reconcile::changed_since_written(self.root, outputs, rel));
        let lyrics = outputs.values().any(|w| w.lyrics.as_deref() == Some(rel));
        (audio || lyrics).then(|| Owned(rel.to_path_buf()))
    }

    /// `rel` taken over by a forced run, though it is not muman's.
    #[must_use]
    pub fn taken(rel: &Path) -> Owned {
        Owned(rel.to_path_buf())
    }

    /// The library folder.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.root
    }

    /// Write the run's record as it stands.
    pub fn checkpoint(&mut self) -> Result<()> {
        match self.run.as_deref_mut() {
            Some(run) => run.checkpoint(self.home),
            None => Ok(()),
        }
    }

    /// Keep `files` for `undo`, recorded ahead, before they are written
    /// over or removed.
    pub fn replacing(&mut self, files: &[Owned]) -> Result<()> {
        let Some(run) = self.run.as_deref_mut() else {
            return Ok(());
        };
        if files.is_empty() {
            return Ok(());
        }
        for file in files {
            run.keep(self.root, file.path())?;
        }
        run.checkpoint(self.home)
    }

    /// Remove each of `files`, kept for `undo` first.
    pub fn remove(&mut self, files: &[Owned]) -> Result<Removal> {
        self.replacing(files)?;
        let mut gone = Vec::new();
        let mut held = Vec::new();
        for file in files {
            match crate::atomic::remove(&self.root.join(file.path())) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    held.push((file.path().to_path_buf(), e.to_string()));
                }
                _ => gone.push(file.path().to_path_buf()),
            }
        }
        remove_empty_folders(self.root, &gone);
        Ok(Removal { gone, held })
    }

    /// Move each pair of `moves`, a song's file and its lyrics, all
    /// recorded ahead; a pair whose lyrics cannot follow goes back whole,
    /// and a pair that cannot move is forgotten from the record. Returns
    /// each pair's outcome, in order.
    pub fn relocate<W: Write>(
        &mut self,
        moves: &[Relocation],
        out: &mut W,
    ) -> Result<Vec<Result<()>>> {
        if let Some(run) = self.run.as_deref_mut().filter(|_| !moves.is_empty()) {
            for m in moves {
                run.moving(&m.from, &m.to);
                if let Some((a, b)) = &m.lyrics {
                    run.moving(a, b);
                }
            }
            run.checkpoint(self.home)?;
        }
        let mut left = Vec::new();
        let mut done = Vec::new();
        for m in moves {
            let result = move_file(self.root, &m.from, &m.to).and_then(|()| match &m.lyrics {
                Some((a, b)) => move_file(self.root, a, b).inspect_err(|_| {
                    let _ = move_file(self.root, &m.to, &m.from);
                }),
                None => Ok(()),
            });
            match &result {
                Ok(()) => {
                    crate::ui::info(
                        out,
                        &format!(
                            "Moved: {} → {}",
                            crate::relpath::show(&m.from),
                            crate::relpath::show(&m.to)
                        ),
                    )?;
                    left.push(m.from.clone());
                }
                Err(_) => {
                    if let Some(run) = self.run.as_deref_mut() {
                        run.not_moved(&m.from, &m.to);
                        if let Some((a, b)) = &m.lyrics {
                            run.not_moved(a, b);
                        }
                    }
                }
            }
            done.push(result);
        }
        remove_empty_folders(self.root, &left);
        Ok(done)
    }
}

/// What a removal did: the files that went, and each that could not go,
/// with why.
#[derive(Debug, Default)]
pub struct Removal {
    pub gone: Vec<PathBuf>,
    pub held: Vec<(PathBuf, String)>,
}

/// A song's file moved to a new path, with its lyrics where it has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    pub from: PathBuf,
    pub to: PathBuf,
    pub lyrics: Option<(PathBuf, PathBuf)>,
}

/// Rename a library file, creating its new folder.
fn move_file(library: &Path, from: &Path, to: &Path) -> Result<()> {
    let (from, to) = (library.join(from), library.join(to));
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let step = |a: &Path, b: &Path| {
        crate::atomic::rename(a, b)
            .with_context(|| format!("moving {} to {}", a.display(), b.display()))
    };
    if crate::relpath::folded(&from) == crate::relpath::folded(&to) {
        let mut temp = to.clone().into_os_string();
        temp.push(".moving");
        let temp = PathBuf::from(temp);
        step(&from, &temp)?;
        step(&temp, &to)
    } else if to.exists() {
        // Left by a move before it that failed.
        Err(anyhow!("{} is taken", to.display()))
    } else {
        step(&from, &to)
    }
}

/// Remove what a run stopped partway left beside the paths muman writes:
/// a `.part` it was writing, a `.moving` it was renaming through. Only
/// beside those paths, so a download of the user's own named so is not.
pub fn clear_leftovers(library: &Path, paths: impl Iterator<Item = PathBuf>) {
    for path in paths {
        for file in [path.clone(), path.with_extension("lrc")] {
            for suffix in [".part", ".moving"] {
                let mut left = library.join(&file).into_os_string();
                left.push(suffix);
                let left = PathBuf::from(left);
                if left.is_file() {
                    let _ = crate::atomic::remove(&left);
                }
            }
        }
    }
}

/// Remove each folder a removed file leaves empty, up to the library.
pub fn remove_empty_folders(library: &Path, removed: &[PathBuf]) {
    let mut folders: Vec<PathBuf> = removed
        .iter()
        .flat_map(|p| {
            p.ancestors()
                .skip(1)
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .filter(|p| !p.as_os_str().is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    folders.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for folder in folders {
        let folder = library.join(folder);
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
        if names
            .iter()
            .all(|n| is_desktop_clutter(&n.to_string_lossy()))
        {
            for name in &names {
                let _ = std::fs::remove_file(folder.join(name));
            }
            // A folder that still holds anything stays; the error says so.
            let _ = std::fs::remove_dir(&folder);
        }
    }
}

/// Files a file manager leaves in a folder it showed, which keep an
/// emptied album folder from being removed otherwise.
fn is_desktop_clutter(name: &str) -> bool {
    matches!(name, "desktop.ini" | "Thumbs.db" | ".DS_Store") || name.starts_with("._")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(stamp: Option<String>, lyrics: Option<&str>) -> Written {
        Written {
            sources: Vec::new(),
            lyrics: lyrics.map(PathBuf::from),
            plan: None,
            stamp,
            digest: None,
        }
    }

    #[test]
    fn only_a_file_muman_wrote_as_it_wrote_it_is_owned() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("A")).unwrap();
        std::fs::write(root.join("A/x.opus"), "x").unwrap();
        std::fs::write(root.join("A/y.opus"), "y").unwrap();
        let stamp = crate::store::stamp_text(&root.join("A/x.opus"));
        let outputs = BTreeMap::from([
            (PathBuf::from("A/x.opus"), written(stamp, Some("A/x.lrc"))),
            (PathBuf::from("A/y.opus"), written(Some("1:1".into()), None)),
        ]);
        let ledger = Ledger::new(root, root, None);
        assert!(
            ledger
                .owned(&outputs, Path::new("A/x.opus"), false)
                .is_some()
        );
        assert!(
            ledger
                .owned(&outputs, Path::new("A/x.lrc"), false)
                .is_some()
        );
        assert!(
            ledger
                .owned(&outputs, Path::new("A/y.opus"), false)
                .is_none(),
            "changed since written"
        );
        assert!(
            ledger
                .owned(&outputs, Path::new("A/y.opus"), true)
                .is_some()
        );
        assert!(
            ledger
                .owned(&outputs, Path::new("A/mine.opus"), true)
                .is_none(),
            "never listed: only `taken` may name it"
        );
    }

    #[test]
    fn a_removal_takes_the_folder_it_empties() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("A/B")).unwrap();
        std::fs::write(root.join("A/B/x.opus"), "x").unwrap();
        std::fs::write(root.join("A/B/.DS_Store"), "").unwrap();
        let mut ledger = Ledger::new(root, root, None);
        let removal = ledger
            .remove(&[Ledger::taken(Path::new("A/B/x.opus"))])
            .unwrap();
        assert_eq!(removal.gone, [PathBuf::from("A/B/x.opus")]);
        assert!(removal.held.is_empty());
        assert!(!root.join("A").exists());
    }

    #[test]
    fn a_pair_whose_lyrics_cannot_follow_goes_back_whole() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("A")).unwrap();
        std::fs::create_dir_all(root.join("B")).unwrap();
        std::fs::write(root.join("A/x.opus"), "x").unwrap();
        std::fs::write(root.join("A/x.lrc"), "l").unwrap();
        std::fs::write(root.join("B/x.lrc"), "taken").unwrap();
        let mut ledger = Ledger::new(root, root, None);
        let moves = [Relocation {
            from: "A/x.opus".into(),
            to: "B/x.opus".into(),
            lyrics: Some(("A/x.lrc".into(), "B/x.lrc".into())),
        }];
        let done = ledger.relocate(&moves, &mut Vec::new()).unwrap();
        assert!(done[0].is_err());
        assert!(root.join("A/x.opus").exists() && !root.join("B/x.opus").exists());
    }
}
