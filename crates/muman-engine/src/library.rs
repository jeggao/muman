//! The library's files as a sync changes them: every move, removal and
//! file written over goes through a [`Ledger`], which records it in the
//! run's history before making it, so a run stopped at any point is one
//! the next sync follows and `undo` reverses. Writing a song's new file
//! is [`crate::render`]'s, staged beside its place and put there by
//! [`put`], over a file the ledger has kept, or where no file is.
//!
//! A file is removed, written over or kept before either, only as
//! [`Owned`]: one the state lists as muman's, its bytes as muman wrote
//! them, or a file beside one, so too; or one a forced run takes over,
//! kept for `undo`
//! first like any other. No other value names a file the ledger may
//! delete, so a file of the user's own cannot reach a removal by any path
//! through the code: [`put`] and [`clear`], which `render` places a song's
//! files through, refuse a file at the path that no [`Owned`] names, a
//! `.lrc` of the user's own beside a song among them. `tests/architecture.rs`
//! holds every other module to changing no library file, and `render` to
//! replacing none but through them.
//!
//! A song's files go into place one at a time, and a rename can fail
//! midway, as over a file Windows holds open: a song that fails once one
//! of its files is in place says so ([`HalfWritten`]), and its record is
//! then the one the run made before it changed any, as a run stopped
//! there leaves it, so every file at its paths stays muman's.
//!
//! A move that only changes a name's case goes through a temporary name,
//! which a filesystem blind to case would otherwise take for a rename
//! onto itself; a second step that fails puts the file back. A folder a
//! removal or a move leaves empty goes too, the files a file manager
//! leaves in a folder it showed aside.
//!
//! What a run stopped partway leaves, a `.part` it was writing or a
//! `.moving` it was renaming through, is known by the records it made
//! before it changed anything ([`Leftovers`]): a `.part` beside a file of
//! an output the state records as about to be written, and a `.moving`
//! beside a path the stopped run's record lists a move to. Any other file
//! so named is the user's, left alone, whatever its name says. A `.part`
//! of a stopped run is removed; a `.moving` never is, as it holds the
//! only copy of its file, the user's edits to it among them: its move is
//! finished, and one that cannot be is left for the user.

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
    /// or, when `forced`, whatever became of it; or a file beside one, so
    /// too.
    #[must_use]
    pub fn owned(
        &self,
        outputs: &BTreeMap<PathBuf, Written>,
        rel: &Path,
        forced: bool,
    ) -> Option<Owned> {
        let audio = outputs.contains_key(rel)
            && (forced || !crate::reconcile::changed_since_written(self.root, outputs, rel));
        let beside = outputs.values().flat_map(|w| &w.sidecars).any(|s| {
            s.path == rel && (forced || crate::reconcile::sidecar_as_written(self.root, s))
        });
        (audio || beside).then(|| Owned(rel.to_path_buf()))
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

    /// Note that the run is about to change the outputs at `rels`.
    pub fn changing<'p>(&mut self, rels: impl IntoIterator<Item = &'p PathBuf>) {
        if let Some(run) = self.run.as_deref_mut() {
            for rel in rels {
                run.changing(rel);
            }
        }
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
        Ok(self.delete(files))
    }

    /// Remove each of `files`, of songs no song keeps, before the moves
    /// that land where they are: kept for `undo` first apart from the
    /// files the run writes over, as a file moved there may be written over
    /// after, and put back after the moves are.
    pub fn free(&mut self, files: &[Owned]) -> Result<Removal> {
        if let Some(run) = self.run.as_deref_mut().filter(|_| !files.is_empty()) {
            for file in files {
                run.keep_freed(self.root, file.path())?;
            }
            run.checkpoint(self.home)?;
        }
        Ok(self.delete(files))
    }

    // Removes library files, the ledger's own change.
    #[expect(clippy::disallowed_methods)]
    fn delete(&self, files: &[Owned]) -> Removal {
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
        Removal { gone, held }
    }

    /// Move each of `moves`, a song's file and those beside it, all
    /// recorded ahead; a song whose files beside it cannot follow goes
    /// back whole, and one that cannot move is forgotten from the record.
    /// A folder a move empties is removed before the next, which may
    /// move a file to where it was. Each move is said, a folder's file
    /// with its song's, as its name is no song's.
    /// Returns each one's outcome, in order.
    pub fn relocate<W: Write>(
        &mut self,
        moves: &[Relocation],
        out: &mut W,
    ) -> Result<Vec<Result<()>>> {
        if let Some(run) = self.run.as_deref_mut().filter(|_| !moves.is_empty()) {
            for m in moves {
                run.moving(&m.from, &m.to);
                for (a, b) in &m.beside {
                    run.moving(a, b);
                }
            }
            run.checkpoint(self.home)?;
        }
        let mut done = Vec::new();
        for m in moves {
            let result = move_file(self.root, &m.from, &m.to).and_then(|()| {
                for (n, (a, b)) in m.beside.iter().enumerate() {
                    if let Err(e) = move_file(self.root, a, b) {
                        for (a, b) in m.beside[..n].iter().rev() {
                            let _ = move_file(self.root, b, a);
                        }
                        let _ = move_file(self.root, &m.to, &m.from);
                        return Err(e);
                    }
                }
                Ok(())
            });
            match &result {
                Ok(()) => {
                    remove_empty_folders(self.root, std::slice::from_ref(&m.from));
                    for (a, b) in m.said() {
                        crate::ui::info(
                            out,
                            &format!(
                                "Moved: {} → {}",
                                crate::relpath::show(a),
                                crate::relpath::show(b)
                            ),
                        )?;
                    }
                }
                Err(_) => {
                    if let Some(run) = self.run.as_deref_mut() {
                        run.not_moved(&m.from, &m.to);
                        for (a, b) in &m.beside {
                            run.not_moved(a, b);
                        }
                    }
                }
            }
            done.push(result);
        }
        Ok(done)
    }
}

/// The error of a song whose files were put in place in part: what
/// failed came after one of them was changed, so the song is as a run
/// stopped there leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalfWritten;

impl std::fmt::Display for HalfWritten {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("left half written, so written whole by the next sync")
    }
}

/// `e`, said [`HalfWritten`] once `changed` says a file of the song was.
#[must_use]
pub fn half_written(e: anyhow::Error, changed: bool) -> anyhow::Error {
    if changed && e.downcast_ref::<HalfWritten>().is_none() {
        e.context(HalfWritten)
    } else {
        e
    }
}

/// Put the file staged at `staged` in its place at `rel` in the library
/// `root`: over a file there only as `claim` holds it, and never a staged
/// path that is no longer a plain file, as a link put in its place is not.
// Puts a staged file in its library place, the ledger's own change.
#[expect(clippy::disallowed_methods)]
pub fn put(root: &Path, staged: &Path, rel: &Path, claim: &[Owned]) -> Result<()> {
    let plain = std::fs::symlink_metadata(staged).is_ok_and(|m| m.file_type().is_file());
    if !plain {
        return Err(anyhow!(
            "{} is not the file muman staged",
            shown(root, staged)
        ));
    }
    held(root, rel, claim)?;
    crate::atomic::rename(staged, &root.join(rel)).with_context(|| {
        format!(
            "renaming {} → {}",
            shown(root, staged),
            crate::relpath::show(rel)
        )
    })
}

/// `file`, in the library `root`, named as every message names a library
/// file, from the root with `/` ([`crate::relpath::show`]); one outside
/// it, a scratch file, as the platform writes it.
#[must_use]
pub fn shown(root: &Path, file: &Path) -> String {
    let Ok(rel) = file.strip_prefix(root) else {
        let scratch = file;
        return scratch.display().to_string();
    };
    crate::relpath::show(rel)
}

/// Remove `rel` from the library `root` if a file is there, as `claim`
/// holds it.
// Removes a library file the claim holds, the ledger's own change.
#[expect(clippy::disallowed_methods)]
pub fn clear(root: &Path, rel: &Path, claim: &[Owned]) -> Result<()> {
    held(root, rel, claim)?;
    match crate::atomic::remove(&root.join(rel)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r.with_context(|| format!("removing {}", crate::relpath::show(rel))),
    }
}

/// Whether `rel` in the library `root` may be put or cleared as `claim`
/// holds it: no file is there, or one of `claim`.
#[must_use]
pub fn may_change(root: &Path, rel: &Path, claim: &[Owned]) -> bool {
    held(root, rel, claim).is_ok()
}

/// Refuse, before any is changed, a song's files at `rels` that are not
/// all for [`put`] or [`clear`] to change as `claim` holds them, so a song
/// one of them refuses is left as it was, whole, and as the state records
/// it.
pub fn claimed<'p>(
    root: &Path,
    rels: impl IntoIterator<Item = &'p PathBuf>,
    claim: &[Owned],
) -> Result<()> {
    rels.into_iter().try_for_each(|rel| held(root, rel, claim))
}

/// Refuse a file at `rel` that no file of `claim` is, under its name or
/// one a filesystem blind to case takes for it.
fn held(root: &Path, rel: &Path, claim: &[Owned]) -> Result<()> {
    let at = root.join(rel);
    if std::fs::symlink_metadata(&at).is_err() {
        return Ok(());
    }
    let ours = claim.iter().any(|o| {
        o.path() == rel || same_file::is_same_file(root.join(o.path()), &at).unwrap_or(false)
    });
    if ours {
        Ok(())
    } else {
        Err(anyhow!(
            "{} is not muman's, and is left alone",
            crate::relpath::show(rel)
        ))
    }
}

/// What a removal did: the files that went, and each that could not go,
/// with why.
#[derive(Debug, Default)]
pub struct Removal {
    pub gone: Vec<PathBuf>,
    pub held: Vec<(PathBuf, String)>,
}

/// A song's file moved to a new path, with each file beside it that
/// moves. Made only by [`Relocation::new`], so no file is moved, recorded
/// moving or said moved onto its own path: a folder's file stays where it
/// is as its song moves within the folder, and a move of it onto itself
/// would be one `undo` refuses, its two ends one file in the way of the
/// other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    from: PathBuf,
    to: PathBuf,
    beside: Vec<(PathBuf, PathBuf)>,
}

impl Relocation {
    /// The song's file at `from` moved to `to`, with each of `beside`,
    /// from and to, but those whose two ends are one path, which stay.
    #[must_use]
    pub fn new(from: PathBuf, to: PathBuf, beside: Vec<(PathBuf, PathBuf)>) -> Self {
        let beside = beside.into_iter().filter(|(a, b)| a != b).collect();
        Self { from, to, beside }
    }

    #[must_use]
    pub fn from(&self) -> &PathBuf {
        &self.from
    }

    #[must_use]
    pub fn to(&self) -> &PathBuf {
        &self.to
    }

    /// Each file beside it that moves, from and to.
    #[must_use]
    pub fn beside(&self) -> &[(PathBuf, PathBuf)] {
        &self.beside
    }

    /// Each file it moves, from and to: the song's, then those beside it.
    pub fn files(&self) -> impl Iterator<Item = (&PathBuf, &PathBuf)> {
        std::iter::once((&self.from, &self.to)).chain(self.beside.iter().map(|(a, b)| (a, b)))
    }

    /// The moves said of it, each from and to: the song's file, and each
    /// of its folder's files it carries, which a name of the song's own
    /// does not tell of.
    pub fn said(&self) -> impl Iterator<Item = (&PathBuf, &PathBuf)> {
        std::iter::once((&self.from, &self.to)).chain(
            (self.beside.iter())
                .filter(|(a, _)| crate::outputs::of_folder(&self.from, a))
                .map(|(a, b)| (a, b)),
        )
    }
}

/// Rename a library file, creating its new folder.
// Renames a library file, the ledger's own change.
#[expect(clippy::disallowed_methods)]
fn move_file(library: &Path, from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(library.join(dir))
            .with_context(|| format!("creating {}", crate::relpath::show(dir)))?;
    }
    let step = |a: &Path, b: &Path| {
        crate::atomic::rename(&library.join(a), &library.join(b)).with_context(|| {
            format!(
                "moving {} to {}",
                crate::relpath::show(a),
                crate::relpath::show(b)
            )
        })
    };
    if crate::relpath::folded(&library.join(from)) == crate::relpath::folded(&library.join(to)) {
        let temp = named(to, MOVING);
        step(from, &temp)?;
        step(&temp, to).inspect_err(|_| {
            let _ = crate::atomic::rename(&library.join(&temp), &library.join(from));
        })
    } else if library.join(to).exists() {
        // Left by a move before it that failed.
        Err(anyhow!("{} is taken", crate::relpath::show(to)))
    } else {
        step(from, to)
    }
}

/// What a file is staged as before it is put in place.
const PART: &str = ".part";

/// What a file a move changes only the case of is renamed through.
const MOVING: &str = ".moving";

/// `rel` with `suffix` after its name.
fn named(rel: &Path, suffix: &str) -> PathBuf {
    let mut name = rel.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// What a run stopped partway may have left beside the library's files,
/// known by the records it made before it changed any, as the module's
/// docs say.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Leftovers {
    parts: BTreeSet<PathBuf>,
    moving: BTreeSet<PathBuf>,
}

impl Leftovers {
    /// What runs may have left that `outputs` records and that the moves
    /// to `moving` a stopped run records: a `.part` beside each file of an
    /// output with no plan, as a sync records a song before it writes it,
    /// and a `.moving` beside each path moved to.
    #[must_use]
    pub fn of<'a>(
        outputs: &BTreeMap<PathBuf, Written>,
        moving: impl IntoIterator<Item = &'a PathBuf>,
    ) -> Self {
        let parts = (outputs.iter())
            .filter(|(_, w)| w.plan.is_none())
            .flat_map(|(path, w)| w.files(path))
            .map(|file| named(file, PART))
            .collect();
        let moving = moving.into_iter().map(|to| named(to, MOVING)).collect();
        Self { parts, moving }
    }

    /// The `.part` files of them that are in `library`, which [`Self::clear`]
    /// removes.
    pub fn parts<'s>(&'s self, library: &'s Path) -> impl Iterator<Item = &'s PathBuf> + 's {
        self.parts.iter().filter(move |p| library.join(p).is_file())
    }

    /// Whether `rel`, a path in the library, is one of them.
    #[must_use]
    pub fn contains(&self, rel: &Path) -> bool {
        self.parts.contains(rel) || self.moving.contains(rel)
    }

    /// Remove each `.part` of them that is a file in `library`. A
    /// `.moving` is never removed: a rename moves a file whole, so it is
    /// the only copy of the file its move was taking, which the next sync
    /// finishes moving ([`finish_move`]) or leaves where it is.
    // Removes what a stopped run left beside the paths it wrote.
    #[expect(clippy::disallowed_methods)]
    pub fn clear(&self, library: &Path) {
        for left in &self.parts {
            let left = library.join(left);
            if left.is_file() {
                let _ = crate::atomic::remove(&left);
            }
        }
    }
}

/// Whether a case-only move of `from` to `to` in `library` was stopped
/// between its two renames: the file at its temporary name, and none at
/// either end.
#[must_use]
pub fn half_moved(library: &Path, from: &Path, to: &Path) -> bool {
    library.join(named(to, MOVING)).is_file()
        && std::fs::symlink_metadata(library.join(from)).is_err()
        && std::fs::symlink_metadata(library.join(to)).is_err()
}

/// Of the paths a stopped run records moving files to, `moving`, each a
/// move of case alone was stopped short of, its file at its temporary
/// name and none at it: where the next sync finishes it ([`finish_move`]).
#[must_use]
pub fn halfway<'a>(
    library: &Path,
    moving: impl IntoIterator<Item = &'a PathBuf>,
) -> BTreeSet<PathBuf> {
    moving
        .into_iter()
        .filter(|to| {
            library.join(named(to, MOVING)).is_file()
                && std::fs::symlink_metadata(library.join(to)).is_err()
        })
        .cloned()
        .collect()
}

/// The temporary name a move of case alone renames `file` through.
#[must_use]
pub fn moving_name(file: &Path) -> PathBuf {
    named(file, MOVING)
}

/// Make the second rename of a move [`half_moved`], returning whether it
/// was made.
// Finishes a move a stopped run made half of.
#[expect(clippy::disallowed_methods)]
#[must_use]
pub fn finish_move(library: &Path, from: &Path, to: &Path) -> bool {
    half_moved(library, from, to)
        && crate::atomic::rename(&library.join(named(to, MOVING)), &library.join(to)).is_ok()
}

/// Each file and folder below `library` that the records of `outputs`
/// spell one way, renamed to that spelling where a filesystem blind to
/// case keeps it under another, as each renamed, from and to.
///
/// A move or a write into `Chan/Lantern` lands, on such a filesystem, in
/// a `Chan/lantern` already there, and a write of `Lantern.lrc` may end in
/// the `lantern.lrc` a stopped run left, so a song retitled in case alone
/// could keep an old spelling, while a filesystem that tells case apart
/// makes each anew: the records then named a path the disk spelled
/// otherwise, and `check` took the song's own file for another's. A path
/// is renamed only where the spelling the records agree on opens that
/// very file or folder, so a filesystem that tells case apart, where it
/// would open another or none, never has one renamed; a folder the
/// records spell two ways is left as it is. No run records the rename:
/// the sync after an `undo` spells each path back as the records it puts
/// back do.
#[must_use]
pub fn spell_as_recorded(
    library: &Path,
    outputs: &BTreeMap<PathBuf, Written>,
) -> Vec<(PathBuf, PathBuf)> {
    use crate::relpath::folded;
    let mut spellings: BTreeMap<String, BTreeSet<&Path>> = BTreeMap::new();
    for (path, written) in outputs {
        for file in written.files(path) {
            for spelled in file.ancestors() {
                if !spelled.as_os_str().is_empty() {
                    spellings
                        .entry(folded(spelled))
                        .or_default()
                        .insert(spelled);
                }
            }
        }
    }
    let mut paths: Vec<&Path> = (spellings.into_values())
        .filter(|s| s.len() == 1)
        .flatten()
        .collect();
    // A path's folder is spelled first, so the path is found in it.
    paths.sort_by_key(|f| f.components().count());
    let mut renamed = Vec::new();
    for spelled in paths {
        let (Some(parent), Some(name)) = (spelled.parent(), spelled.file_name()) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(library.join(parent)) else {
            continue;
        };
        let names: Vec<std::ffi::OsString> = entries.flatten().map(|e| e.file_name()).collect();
        if names.iter().any(|n| n == name) {
            continue;
        }
        let Some(found) = (names.iter()).find(|n| folded(Path::new(n)) == folded(Path::new(name)))
        else {
            continue;
        };
        let was = parent.join(found);
        if !same_file::is_same_file(library.join(&was), library.join(spelled)).unwrap_or(false) {
            continue;
        }
        // Spells a path anew in place: nothing moves, so no ledger is kept.
        #[expect(clippy::disallowed_methods)]
        let done = crate::atomic::rename(&library.join(&was), &library.join(spelled));
        if done.is_ok() {
            renamed.push((was, spelled.to_path_buf()));
        }
    }
    renamed
}

/// Where the file or folder `rel`, as muman's records spell it, is below
/// `library` now: at `rel` itself, or at the spelling a filesystem blind
/// to case keeps a name of it in, as [`spell_as_recorded`] finds it; none
/// where nothing is. `listed` keeps each folder's names, read once.
pub fn spelled_on_disk(
    library: &Path,
    rel: &Path,
    listed: &mut BTreeMap<PathBuf, Vec<std::ffi::OsString>>,
) -> Option<PathBuf> {
    use crate::relpath::folded;
    let mut at = PathBuf::new();
    for part in rel.components() {
        let name = part.as_os_str();
        if !listed.contains_key(&at) {
            let entries = std::fs::read_dir(library.join(&at)).ok()?;
            let names = entries.flatten().map(|e| e.file_name()).collect();
            listed.insert(at.clone(), names);
        }
        let names = &listed[&at];
        if names.iter().any(|n| n == name) {
            at.push(name);
            continue;
        }
        let wanted = library.join(&at).join(name);
        let found = names.iter().find(|n| {
            folded(Path::new(n)) == folded(Path::new(name))
                && same_file::is_same_file(library.join(&at).join(n), &wanted).unwrap_or(false)
        })?;
        at.push(found);
    }
    Some(at)
}

/// Remove each folder a removed file leaves empty, up to the library.
// Removes the folders a removal emptied.
#[expect(clippy::disallowed_methods)]
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
pub(crate) fn is_desktop_clutter(name: &str) -> bool {
    matches!(name, "desktop.ini" | "Thumbs.db" | ".DS_Store") || name.starts_with("._")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(stamp: Option<String>, lyrics: Option<&str>) -> Written {
        Written {
            sources: Vec::new(),
            sidecars: lyrics
                .map(|l| crate::state::Sidecar::at(l.into()))
                .into_iter()
                .collect(),
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
    fn a_leftover_is_muman_s_only_beside_what_a_record_says_was_being_written() {
        let about_to_be = Written {
            plan: None,
            stamp: None,
            ..written(None, Some("A/x.lrc"))
        };
        let mut done = written(Some("1:1".into()), Some("B/y.lrc"));
        done.plan = Some(crate::resolve::testing::every_part(
            crate::resolve::Format::Copy {
                codec: crate::codec::Codec::Opus,
            },
        ));
        let outputs = BTreeMap::from([
            (PathBuf::from("A/x.opus"), about_to_be),
            (PathBuf::from("B/y.opus"), done),
        ]);
        let moving = [PathBuf::from("C/Z.opus")];
        let left = Leftovers::of(&outputs, &moving);
        for ours in ["A/x.opus.part", "A/x.lrc.part", "C/Z.opus.moving"] {
            assert!(left.contains(Path::new(ours)), "{ours}");
        }
        for theirs in [
            "B/y.opus.part",
            "B/y.lrc.part",
            "A/x.opus.moving",
            "C/Z.opus.part",
            "notes.part",
        ] {
            assert!(!left.contains(Path::new(theirs)), "{theirs}");
        }
    }

    #[test]
    fn a_moving_file_a_stopped_run_left_is_never_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("A")).unwrap();
        let outputs = BTreeMap::from([(PathBuf::from("A/x.opus"), written(None, None))]);
        std::fs::write(root.join("A/x.opus.part"), "half").unwrap();
        std::fs::write(root.join("A/Z.opus.moving"), "the only copy").unwrap();
        let left = Leftovers::of(&outputs, &[PathBuf::from("A/Z.opus")]);
        assert!(left.contains(Path::new("A/Z.opus.moving")));
        left.clear(root);
        assert!(!root.join("A/x.opus.part").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("A/Z.opus.moving")).unwrap(),
            "the only copy"
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
        assert_eq!(removal.held, []);
        assert!(!root.join("A").exists());
    }

    #[test]
    fn a_file_is_put_over_or_cleared_only_as_claimed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("A")).unwrap();
        std::fs::write(root.join("A/x.lrc"), "mine").unwrap();
        std::fs::write(root.join("A/x.lrc.part"), "new").unwrap();
        let rel = Path::new("A/x.lrc");
        assert!(put(root, &root.join("A/x.lrc.part"), rel, &[]).is_err());
        assert!(clear(root, rel, &[]).is_err());
        assert_eq!(std::fs::read_to_string(root.join(rel)).unwrap(), "mine");
        let claim = [Ledger::taken(rel)];
        put(root, &root.join("A/x.lrc.part"), rel, &claim).unwrap();
        assert_eq!(std::fs::read_to_string(root.join(rel)).unwrap(), "new");
        clear(root, rel, &claim).unwrap();
        assert!(!root.join(rel).exists());
        clear(root, rel, &[]).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_a_staged_file_is_not_put() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("elsewhere"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("elsewhere"), root.join("x.opus.part")).unwrap();
        let staged = root.join("x.opus.part");
        assert!(put(root, &staged, Path::new("x.opus"), &[]).is_err());
        assert!(!root.join("x.opus").exists());
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
        let moves = [Relocation::new(
            "A/x.opus".into(),
            "B/x.opus".into(),
            vec![("A/x.lrc".into(), "B/x.lrc".into())],
        )];
        let done = ledger.relocate(&moves, &mut Vec::new()).unwrap();
        let Err(e) = &done[0] else {
            panic!("moved onto a file taken");
        };
        assert_eq!(format!("{e:#}"), "B/x.lrc is taken");
        assert!(root.join("A/x.opus").exists() && !root.join("B/x.opus").exists());
    }

    #[test]
    fn a_library_file_is_named_in_an_error_from_the_root_with_slashes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let staged = root.join("A").join("x.opus.part");
        let e = put(root, &staged, Path::new("A/x.opus"), &[]).unwrap_err();
        assert_eq!(
            format!("{e:#}"),
            "A/x.opus.part is not the file muman staged"
        );
        let e = move_file(root, Path::new("A/y.opus"), Path::new("B/y.opus")).unwrap_err();
        assert!(
            format!("{e:#}").starts_with("moving A/y.opus to B/y.opus: "),
            "{e:#}"
        );
    }

    #[test]
    fn a_path_is_spelled_as_its_records_agree_where_case_is_one_name_to_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for file in [
            "Chan/lantern/Lantern.opus",
            "Chan/lantern/lantern.lrc",
            "Two/a/x.opus",
        ] {
            std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            std::fs::write(root.join(file), "x").unwrap();
        }
        let outputs = BTreeMap::from([
            (
                PathBuf::from("Chan/Lantern/Lantern.opus"),
                written(None, Some("Chan/Lantern/Lantern.lrc")),
            ),
            // Two records spell this folder two ways: it is left as it is.
            (PathBuf::from("Two/A/x.opus"), written(None, None)),
            (PathBuf::from("Two/a/y.opus"), written(None, None)),
        ]);
        let blind = root.join("Chan/Lantern").exists();
        let renamed = spell_as_recorded(root, &outputs);
        let names = |dir: &str| -> Vec<String> {
            let entries = std::fs::read_dir(root.join(dir)).unwrap();
            let mut names: Vec<String> = (entries.flatten())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        if blind {
            let folder = (PathBuf::from("Chan/lantern"), PathBuf::from("Chan/Lantern"));
            let lyrics = (
                PathBuf::from("Chan/Lantern/lantern.lrc"),
                PathBuf::from("Chan/Lantern/Lantern.lrc"),
            );
            assert_eq!(renamed, [folder, lyrics]);
            assert_eq!(names("Chan"), ["Lantern"]);
            assert_eq!(names("Chan/Lantern"), ["Lantern.lrc", "Lantern.opus"]);
        } else {
            assert_eq!(renamed, []);
            assert_eq!(names("Chan"), ["lantern"]);
            assert_eq!(names("Chan/lantern"), ["Lantern.opus", "lantern.lrc"]);
        }
        assert_eq!(names("Two"), ["a"]);
    }
}
