//! Which library files are muman's: a file at a song's path it did not
//! write, one changed since it wrote it, and the outputs that stay its
//! when the library folder moves or a copy gives them new times.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::state::{State, Written};
use crate::store;

/// The files at a song's path, its audio or its lyrics, that muman did
/// not write: a file of the user's own a sync must not write over. A file
/// that is one muman wrote under a name differing only in case, as a
/// filesystem blind to case reports, is muman's.
pub(super) fn foreign(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
) -> Vec<PathBuf> {
    if old.contains_key(path) {
        return Vec::new();
    }
    let lyrics = path.with_extension("lrc");
    let owned = |rel: &Path| {
        old.iter().any(|(p, w)| {
            std::iter::once(p).chain(&w.lyrics).any(|o| {
                same_file::is_same_file(library.join(o), library.join(rel)).unwrap_or(false)
            })
        })
    };
    [path.to_path_buf(), lyrics]
        .into_iter()
        .filter(|rel| library.join(rel).exists() && !owned(rel))
        .collect()
}

/// Whether `file` is empty, as a disk that filled up leaves one: it holds
/// no one's work, so it counts as unchanged since written and a sync
/// writes it again, as `check` says.
pub(super) fn emptied(file: &Path) -> bool {
    std::fs::metadata(file).is_ok_and(|m| m.len() == 0)
}

/// Whether a file muman wrote has changed since, by its size and
/// time: a tagger or player that rewrote it.
pub(crate) fn changed_since_written(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
) -> bool {
    if emptied(&library.join(path)) {
        return false;
    }
    old.get(path)
        .and_then(|w| w.stamp.as_ref())
        .zip(store::stamp_text(&library.join(path)))
        .is_some_and(|(was, now)| *was != now)
}

/// [`changed_since_written`], or, its size and time as written, its
/// bytes not the ones written: a tagger set to keep files' times, its
/// edit fitting the tag's padding. The bytes are read only here, for a
/// file about to be written over, moved with new tags or removed, so a
/// sync reads no file it leaves as it is.
pub(crate) fn edited_since_written(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
) -> bool {
    if changed_since_written(library, old, path) {
        return true;
    }
    let file = library.join(path);
    let Some(written) = old.get(path).filter(|w| w.stamp.is_some()) else {
        return false;
    };
    written.digest.as_ref().is_some_and(|was| {
        !emptied(&file) && crate::facts::digest_file(&file).is_some_and(|now| now != *was)
    })
}

/// Take each output whose size or time changed since it was written,
/// but whose bytes are the ones written, as muman's still: a backup put
/// back or a copy made without its times. Only those are read.
pub(crate) fn vouch_for_copies(library: &Path, state: &mut State) {
    for (path, written) in &mut state.outputs {
        if let AsWritten::Copied(now) = as_written(&library.join(path), written) {
            written.stamp = Some(now);
        }
    }
}

/// Whether a file is as muman wrote it.
pub(super) enum AsWritten {
    /// By its size and time.
    Same,
    /// By its bytes, a copy having given it this stamp, new times.
    Copied(String),
    /// Changed, not there, or written before muman recorded how.
    Not,
}

/// Whether `file` is as `written` records muman wrote it.
pub(super) fn as_written(file: &Path, written: &Written) -> AsWritten {
    let (Some(was), Some(now)) = (&written.stamp, store::stamp_text(file)) else {
        return AsWritten::Not;
    };
    if now == *was {
        return AsWritten::Same;
    }
    match &written.digest {
        Some(digest) if crate::facts::digest_file(file).as_ref() == Some(digest) => {
            AsWritten::Copied(now)
        }
        _ => AsWritten::Not,
    }
}

/// The outputs of a library folder named anew, as `--library` or the
/// song list names another: each whose file is at its place in `library`
/// as muman wrote it, moved there or copied, stays muman's, re-stamped
/// where a copy gave it new times; the rest are let go of, to be written
/// anew, or left alone where another file is in the way. How many were
/// kept and let go of.
pub(crate) fn rehome(outputs: &mut BTreeMap<PathBuf, Written>, library: &Path) -> (usize, usize) {
    let before = outputs.len();
    outputs.retain(
        |path, written| match as_written(&library.join(path), written) {
            AsWritten::Copied(now) => {
                written.stamp = Some(now);
                true
            }
            AsWritten::Same => true,
            AsWritten::Not => false,
        },
    );
    (outputs.len(), before - outputs.len())
}

/// Take `library` as the folder the outputs are in: the files there as
/// muman wrote them stay its ([`rehome`]), and those written into another
/// before are no longer muman's to delete.
pub(crate) fn adopt<W: Write>(state: &mut State, library: &Path, out: &mut W) -> Result<()> {
    if library.exists() && !library.is_dir() {
        anyhow::bail!("the library {} is a file, not a folder", library.display());
    }
    if let Some(before) = state
        .library
        .clone()
        .filter(|l| !crate::platform::same_path(l, library))
    {
        let (kept, gone) = rehome(&mut state.outputs, library);
        crate::ui::warning(
            out,
            &format!(
                "The library moved from {}: {kept} file(s) here as muman wrote them stay its, \
                 {gone} are written here anew; the files left there are left alone",
                before.display()
            ),
        )?;
    }
    state.library = Some(library.to_path_buf());
    Ok(())
}
