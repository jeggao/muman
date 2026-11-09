//! What is wrong with a home, as `check` reports it and `status` counts
//! it: the library and the sources against what earlier runs recorded,
//! read without a lock and without running a tool.
//!
//! Each song is resolved as a sync resolves it, from what was measured
//! and from the sources a sync reads, so a song no sync can build is
//! reported though no run recorded it; a failure recorded is reported
//! while it still applies, and while a run holds the home, a file it may
//! be moving or writing is not.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Result;

use crate::dirs::Dirs;
use crate::manifest::Manifest;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{self, Store};

/// One line of a diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    Info(String),
    Warning(String),
    /// A problem, which `check` counts.
    Problem(String),
}

/// A home diagnosed: what was said of it, in order, and what was read to
/// say it.
#[derive(Debug)]
pub struct Diagnosis {
    pub said: Vec<Said>,
    pub manifest: Manifest,
    pub state: State,
    pub store: Store,
}

impl Diagnosis {
    /// How many problems were found.
    #[must_use]
    pub fn problems(&self) -> usize {
        (self.said.iter())
            .filter(|s| matches!(s, Said::Problem(_)))
            .count()
    }
}

#[derive(Default)]
struct Notes(Vec<Said>);

impl Notes {
    // A `Result`, so each check keeps the `?` it had in `check`.
    #[expect(clippy::unnecessary_wraps)]
    fn info(&mut self, line: String) -> Result<()> {
        self.0.push(Said::Info(line));
        Ok(())
    }

    // A `Result`, so each check keeps the `?` it had in `check`.
    #[expect(clippy::unnecessary_wraps)]
    fn warning(&mut self, line: String) -> Result<()> {
        self.0.push(Said::Warning(line));
        Ok(())
    }

    // A `Result`, so each check keeps the `?` it had in `check`.
    #[expect(clippy::unnecessary_wraps)]
    fn problem(&mut self, line: String) -> Result<()> {
        self.0.push(Said::Problem(line));
        Ok(())
    }
}

/// Diagnose the home and library `dirs` name.
// Each check in the order the report lists its problems.
#[expect(clippy::too_many_lines)]
pub fn diagnose(dirs: &Dirs) -> Result<Diagnosis> {
    let mut said = Notes::default();
    let manifest = Manifest::load(&dirs.home)?;
    for line in crate::migrate::notices(&manifest.renamed, manifest.respelled_notice()) {
        said.info(line)?;
    }
    for line in crate::settings::stale_warnings(&manifest.stale_defaults)
        .into_iter()
        .chain(manifest.misspelled())
    {
        said.warning(line)?;
    }
    let mut state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let library = &dirs.library;
    crate::reconcile::vouch_for_copies(library, &mut state);
    // As the next sync reads it: a file a stopped run moved before the
    // state could say so is where it went, and no file is missing.
    crate::history::follow_moves(
        &dirs.home,
        library,
        &mut state,
        crate::history::Moves::Foresee,
    );
    // What a run under way is writing is no run's leftover.
    let busy = crate::atomic::Lock::busy(&dirs.home);
    if busy {
        said.info(
            "A muman run is under way: the files it is writing, moving or removing are not \
             reported"
                .to_string(),
        )?;
    }
    for (n, hook) in manifest.hooks.iter().enumerate() {
        if let Some(why) = &hook.refused {
            said.problem(format!("Hook {} is never run: {why}", n + 1))?;
        }
    }
    if !busy && crate::history::undo_stopped(&dirs.home) {
        said.problem(
            "Left by an interrupted run: an undo stopped partway, the library not yet as it \
             puts it back; `undo` finishes it"
                .to_string(),
        )?;
    }

    let mut owned: BTreeSet<PathBuf> = BTreeSet::new();
    for (path, written) in &state.outputs {
        owned.insert(path.clone());
        owned.extend(written.sidecar_paths().cloned());
        let file = library.join(path);
        match std::fs::metadata(&file) {
            // A run under way may be moving it, or about to write it.
            _ if busy && !file.is_file() => {}
            Err(_) => said.problem(format!(
                "Missing, `sync` writes it again: {}",
                crate::relpath::show(path)
            ))?,
            Ok(m) if m.len() == 0 => said.problem(format!(
                "Empty, `sync` writes it again: {}",
                crate::relpath::show(path)
            ))?,
            Ok(_)
                if written.plan.is_some()
                    && written.sidecar_paths().any(|s| !library.join(s).exists()) =>
            {
                for missing in written
                    .sidecar_paths()
                    .filter(|s| !library.join(s).exists())
                {
                    said.problem(format!(
                        "Missing beside its song, `sync` writes it again: {}",
                        crate::relpath::show(missing)
                    ))?;
                }
            }
            Ok(_) if written.plan.is_none() && busy => {}
            Ok(_) if written.plan.is_none() => said.problem(format!(
                "Not finished by an interrupted run, `sync` writes it again: {}",
                crate::relpath::show(path)
            ))?,
            Ok(_) => {
                let changed = written
                    .stamp
                    .as_ref()
                    .is_some_and(|s| store::stamp_text(&file).as_ref() != Some(s));
                let beside = written
                    .sidecars
                    .iter()
                    .filter(|s| !crate::reconcile::sidecar_as_written(library, s))
                    .map(|s| &s.path);
                for changed in std::iter::once(path).filter(|_| changed).chain(beside) {
                    said.problem(
                        format!(
                            "Changed since muman wrote it, so left alone: {} (`sync --force` writes it again)",
                            crate::relpath::show(changed)
                        ),
                    )?;
                }
            }
        }
    }
    for file in store::files_below(library, usize::MAX, |_| true)? {
        let Ok(rel) = file.strip_prefix(library) else {
            continue;
        };
        if rel
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("part"))
        {
            if !busy {
                said.problem(format!("Left by an interrupted run: {}", file.display()))?;
            }
            continue;
        }
        let hidden = rel
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
        if !hidden && !owned.contains(rel) && !busy {
            said.info(format!(
                "Not muman's, left alone: {}",
                crate::relpath::show(rel)
            ))?;
        }
    }

    let listed = manifest.keys();
    let unread: std::collections::BTreeSet<&str> = listed
        .iter()
        .filter_map(|k| match k {
            SourceKey::Torrent(t) if !store.has_metainfo(t.hash()) => Some(t.hash()),
            _ => None,
        })
        .collect();
    for hash in unread {
        said.problem(format!(
            "The metainfo of torrent {hash} is gone from sources/torrent: its files bring \
                 no lyrics or pictures and are not checked piece by piece until its .torrent \
                 or a magnet link of it is given to `add` again"
        ))?;
    }
    for key in &listed {
        if !store.has(key) {
            let page = manifest
                .song_with(key)
                .and_then(|s| s.held.get(key))
                .is_some_and(|h| h.url.is_some());
            let fetchable = manifest.settings.sites.fetch_url(key).is_some()
                || page
                || crate::provider::is_kept(key)
                || matches!(key, SourceKey::Torrent(_));
            let others = manifest
                .song_with(key)
                .is_some_and(|s| s.sources.iter().any(|k| k != key && store.has(k)));
            let how = if fetchable {
                "`sync` fetches it again"
            } else if others {
                "its song is made from its other sources"
            } else {
                "its song cannot be written"
            };
            said.problem(format!("Missing from sources, {how}: {key}"))?;
        }
    }
    for (key, f) in listed.iter().filter_map(|k| {
        let rev = store.locate(k)?.rev();
        Some((k, state.facts.get(k).filter(|f| f.holds_for(&rev))?))
    }) {
        if let Some(damage) = f.damage() {
            said.problem(format!("Damaged audio: {}", damage.describe(key)))?;
        }
    }
    for (key, failure) in &state.failures {
        if !failure.applies(store.locate(key).as_ref(), state.facts.get(key)) {
            continue;
        }
        said.problem(format!(
            "Failed {} time(s) to {}: {key}: {}",
            failure.count,
            match failure.step {
                crate::state::Step::Measure => "read",
                crate::state::Step::Fetch => "fetch",
                crate::state::Step::Refetch => "fetch again",
                crate::state::Step::Mend => "mend",
            },
            failure.error
        ))?;
    }
    // Each song as a sync would resolve it, from the sources it reads: one
    // whose source is missing, which `sync` fetches again, waits, and one
    // whose source reads as none is held.
    let (on_disk, waiting): (BTreeSet<SourceKey>, BTreeSet<SourceKey>) =
        listed.iter().cloned().partition(|k| store.has(k));
    let on_disk = on_disk
        .into_iter()
        .filter(|k| !crate::reconcile::unread(&state, &store, k))
        .collect();
    let resolution = crate::reconcile::resolve_songs(
        &manifest,
        &manifest.settings,
        &state,
        library,
        Some(&on_disk),
        &waiting,
    );
    match resolution {
        Ok(r) => {
            for (n, e) in &r.unresolved {
                let song = manifest.songs[*n]
                    .id()
                    .map_or_else(|| format!("song {}", n + 1), ToString::to_string);
                said.problem(format!("No sync can build it: {song}: {e:#}"))?;
            }
            for (n, resolved) in &r.planned {
                let song = manifest.songs[*n]
                    .id()
                    .map_or_else(|| format!("song {}", n + 1), ToString::to_string);
                for why in &resolved.why.contested {
                    said.warning(format!(
                        "Won by its place in `sources` alone: {song}: {why}; \
                         set the tag to settle it"
                    ))?;
                }
            }
        }
        Err(e) => said.problem(format!("No sync can build the songs: {e:#}"))?,
    }
    for song in &manifest.songs {
        let Some((id, f)) = song
            .id()
            .and_then(|id| Some((id, state.song_failures.get(id)?)))
        else {
            continue;
        };
        said.problem(format!(
            "The last {} sync(s) could not write it: {id}: {}",
            f.count, f.error
        ))?;
    }
    Ok(Diagnosis {
        said: said.0,
        manifest,
        state,
        store,
    })
}
