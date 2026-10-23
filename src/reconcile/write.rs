//! Songs' files moved to their new paths, what changed in each said, and
//! the library folder's time set after a change.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::library::{Ledger, Relocation};
use crate::resolve::Plan;
use crate::state::State;

/// What differs between two plans, in words.
pub(super) fn changes(old: &Plan, new: &Plan) -> Vec<&'static str> {
    let mut what = Vec::new();
    // Each codec has its renderer's version: a new codec says it.
    if old.version != new.version && old.format.codec() == new.format.codec() {
        what.push("muman's rendering");
    }
    if old.audio != new.audio {
        what.push("audio");
    } else if old.format != new.format {
        what.push("format");
    }
    if old.cover != new.cover {
        what.push("cover");
    }
    if old.lyrics != new.lyrics {
        what.push("lyrics");
    }
    let other_tags = |p: &Plan| {
        p.tags
            .iter()
            .filter(|(k, _)| {
                !crate::tags::Field::named(k).is_some_and(crate::tags::Field::is_loudness)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    if other_tags(old) != other_tags(new) {
        what.push("tags");
    }
    if old.held_loudness() != new.held_loudness()
        || old.tags.len() - other_tags(old).len() != new.tags.len() - other_tags(new).len()
    {
        what.push("loudness");
    }
    what
}

/// Make the `moves` [`decide`](super::decide::decide) chose, each a song's file to its new path,
/// as a new `[library]` template makes them, rather than render them
/// again. A file changed since it was written moves too, still changed:
/// written again at the new path, it would be left at the old one, a
/// second copy. The ledger records each move before it is made. Returns
/// the paths moved to.
pub(super) fn relocate<W: Write>(
    moves: Vec<(PathBuf, PathBuf)>,
    state: &mut State,
    ledger: &mut Ledger<'_>,
    out: &mut W,
) -> Result<BTreeSet<PathBuf>> {
    let pairs: Vec<Relocation> = moves
        .into_iter()
        .map(|(from, to)| {
            let lyrics = state.outputs[&from]
                .lyrics
                .clone()
                .map(|l| (l, to.with_extension("lrc")));
            Relocation { from, to, lyrics }
        })
        .collect();
    let results = ledger.relocate(&pairs, out)?;
    let mut done = BTreeSet::new();
    for (m, result) in pairs.into_iter().zip(results) {
        if let Err(e) = result {
            crate::ui::warning(
                out,
                &format!(
                    "Could not move {}: {e:#}; it is written again",
                    crate::relpath::show(&m.from)
                ),
            )?;
            continue;
        }
        let Some(mut written) = state.outputs.remove(&m.from) else {
            continue;
        };
        written.lyrics = m.lyrics.map(|(_, to)| to);
        done.insert(m.to.clone());
        state.outputs.insert(m.to, written);
    }
    Ok(done)
}

/// Set a folder's time to now, so players that rescan by it notice: a
/// song three levels down changes neither the root nor its children.
/// Best effort: a filesystem that refuses it costs only a rescan.
pub(super) fn touch<W: Write>(dir: &Path, out: &mut W) -> Result<()> {
    if let Err(e) = filetime::set_file_mtime(dir, filetime::FileTime::now()) {
        crate::ui::warning(
            out,
            &format!("Could not update the time of {}: {e}", dir.display()),
        )?;
    }
    Ok(())
}
