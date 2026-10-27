//! The library brought in line with the song list, offline: every listed
//! source measured, the comparisons each song needs made, each song
//! resolved to a plan and the songs fitted into `[library] max_size`
//! when it is set, a song rendered only when its plan changed, and every
//! file muman wrote that no song makes now deleted.
//!
//! This module runs the steps in order; each is a module of its own:
//! `measure` the sources, `keys` renamed or followed, `damage` noted,
//! each song resolved to its `plan`, what to do with its file decided
//! (`decide`) by what is muman's (`owned`), the files moved and written
//! (`write`), the rest pruned (`prune`), and for a dry run the `report`
//! of all it would do.

mod damage;
mod decide;
mod keys;
mod measure;
mod owned;
mod plan;
mod prune;
mod report;
mod write;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};

use crate::atomic::Lock;
use crate::dirs::Dirs;
use crate::history::Run;
use crate::hooks;
use crate::library::Ledger;
use crate::limit;
use crate::manifest::{Edit, Manifest};
use crate::query::Query;
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{Located, Store};

pub(crate) use self::damage::note_damaged;
use self::damage::{note_damage, say_drifts, split_drifts, whole_again};
use self::decide::intend;
pub(crate) use self::decide::{Doing, Foreseen, files_of, foresee};
use self::keys::{foresee_moves, sites_found};
pub(crate) use self::measure::compare;
pub use self::measure::{Measuring, look, measure};
use self::measure::{analyze, analyze_mixes, hash, sizes};
pub(crate) use self::owned::{
    adopt, changed_since_written, edited_since_written, rehome, vouch_for_copies,
};
use self::plan::plan_awaiting;
pub(crate) use self::plan::{Planned, name_of, path_of, plan, plan_with, resolve_songs};
pub(crate) use self::report::{Fate, verdict};
use self::report::{Showing, Shown, fit_summary, left_out, status};
use self::write::{Writing, apply, touch};

// Each is a flag of its own on the command line.
#[expect(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Render every song, changed or not.
    pub force: bool,
    /// Only say what a sync would do.
    pub dry_run: bool,
    /// Measure again sources that could not be read before.
    pub retry: bool,
    /// How long a dropped-in file waits after arriving.
    pub settling: Duration,
    /// How long writing runs between saves of what it wrote; unset, as
    /// long as measuring runs between its saves.
    pub checkpoint: Option<Duration>,
    /// Take what each fetched source holds now where it differs from
    /// what its song was built from.
    pub accept: bool,
    /// Library files changed before the sync, as an undo puts files back,
    /// which its `changed` hook counts with its own.
    pub before: crate::history::Changed,
}

/// Songs left alone for a file not muman's that a run names one by one;
/// a lost state file makes every song such a one.
const NAMED: usize = 10;

/// How long measuring or writing runs between saves of what it did.
const CHECKPOINT: Duration = Duration::from_secs(30);

/// One source's folder under the run's scratch, gone when its step is:
/// the excerpts and prints ffmpeg writes for a source come to 15 MB, and
/// kept until the run ended, thousands of sources filled the temporary
/// folder's disk before they were measured. Its parent is made first,
/// as the sources of a step are measured at once and the first to run
/// has not made it yet; a folder that cannot be made fails its source,
/// never shares another's, whose files it would overwrite.
struct Sub(tempfile::TempDir);

impl Sub {
    fn new(scratch: &Path, name: &str) -> Result<Self> {
        std::fs::create_dir_all(scratch)
            .and_then(|()| tempfile::Builder::new().prefix(name).tempdir_in(scratch))
            .map(Self)
            .with_context(|| format!("creating a scratch folder in {}", scratch.display()))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

/// Keep `state` before the run's end: whole on a sync, and on a dry run
/// only what it measured, which a dry run may cache but never what the
/// library holds.
fn persist(home: &Path, state: &State, dry_run: bool, lock: &Lock) -> Result<()> {
    if dry_run {
        State::keep_caches(home, state, lock)
    } else {
        state.save(home)
    }
}

/// Bring the library in line with the song list, recording what it
/// replaces and removes in `run`. Returns whether every song was
/// written.
pub fn reconcile<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    opts: Options,
    run: Option<&mut Run>,
    out: &mut W,
) -> Result<bool> {
    reconcile_into(runner, dirs, opts, run, out, None)
}

/// Where a dry run writes what a sync would do, and of which songs.
pub struct Report<'a> {
    pub out: &'a mut dyn Write,
    /// The terms of a query; a song it matches is shown even when a sync
    /// leaves it as it is.
    pub query: &'a [String],
    /// Show every song, those up to date too.
    pub all: bool,
}

impl std::fmt::Debug for Report<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Report")
            .field("query", &self.query)
            .field("all", &self.all)
            .finish_non_exhaustive()
    }
}

/// [`reconcile`], a dry run's report written to `report` when given, apart
/// from what it does on the way; without one, every song to `out`.
// The offline phase's steps, in the order the guide numbers them.
#[expect(clippy::too_many_lines)]
pub fn reconcile_into<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    opts: Options,
    mut run: Option<&mut Run>,
    out: &mut W,
    report: Option<Report<'_>>,
) -> Result<bool> {
    let lock = Lock::folder(&dirs.home)?;
    let home = &dirs.home;
    crate::manifest::present(home)?;
    dirs.apart()?;
    let _library = if opts.dry_run {
        None
    } else {
        Some(Lock::library(&dirs.library)?)
    };
    let mut manifest = Manifest::load(home)?;
    if let Some(run) = run.as_deref_mut() {
        run.saw(&manifest);
    }
    let query = report
        .as_ref()
        .map(|r| Query::parse(r.query, &crate::query::extractors(&manifest)))
        .transpose()?
        .filter(|q| !q.is_empty());
    let mut state = State::load(home)?;
    if state
        .library
        .as_deref()
        .is_some_and(|l| crate::platform::same_path(l, &dirs.library))
    {
        crate::history::follow_moves(home, &dirs.library, &mut state);
    }
    let store = Store::scan(dirs)?;
    if opts.dry_run {
        foresee_moves(&mut manifest, &mut state, &store, opts.settling, out)?;
    }
    let temp = crate::atomic::Scratch::new()?;
    let listed = manifest.keys();
    let mut how = Measuring {
        retry: opts.retry,
        say_unread: true,
        checkpoint: &mut |s: &State| persist(home, s, opts.dry_run, &lock),
        analyses: crate::analysis::wanted(&manifest.settings.loudness),
    };
    measure(
        runner,
        &store,
        &listed,
        &mut state,
        temp.path(),
        &mut how,
        out,
    )?;
    // Here rather than in every measuring, so the plans' real sizes are
    // named anew by the fitting that reads them.
    if hash(runner, &store, &listed, &mut state, temp.path(), out)? {
        persist(home, &state, opts.dry_run, &lock)?;
    }
    let analyses = crate::analysis::wanted(&manifest.settings.loudness);
    let analyzed = analyze(
        runner,
        &store,
        &listed,
        &mut state,
        temp.path(),
        &analyses,
        out,
    )?;
    let mixed = analyze_mixes(runner, &store, &listed, &mut state, &manifest.settings, out)?;
    if analyzed || mixed {
        persist(home, &state, opts.dry_run, &lock)?;
    }
    if !opts.dry_run {
        note_damage(&manifest, &store, &mut state);
    }
    let renames = sites_found(&manifest, &state);
    for (from, to) in &renames {
        let line = if opts.dry_run {
            format!("Renamed by the next sync, by the site it came from: {from} → {to}")
        } else {
            format!("Renamed by the site it came from: {from} → {to}")
        };
        crate::ui::info(out, &line)?;
    }
    if !opts.dry_run && !renames.is_empty() {
        for (from, to) in &renames {
            manifest.edit(Edit::Respell {
                from: from.clone(),
                to: to.clone(),
            });
        }
        manifest.save_locked(&lock)?;
        if let Some(run) = run.as_deref_mut() {
            run.saw(&manifest);
        }
        state.rekey(&renames)?;
        state.save(home)?;
        // Its file goes to its new key's place, so it is not fetched again.
        for (from, to) in &renames {
            let at = store.locate(from).map(|l| l.path);
            if let Some(at) = at.filter(|p| p.starts_with(dirs.ytdlp())) {
                crate::store::place(&dirs.ytdlp(), to, &at)?;
            }
        }
    }
    let store = if renames.is_empty() || opts.dry_run {
        store
    } else {
        Store::scan(dirs)?
    };
    let listed = manifest.keys();
    if compare(runner, &store, &manifest.songs, &mut state, out)? {
        persist(home, &state, opts.dry_run, &lock)?;
    }

    // A dry run fetches nothing: what a sync would fetch counts as there,
    // and a song waiting on a source never measured is fetched, not failed.
    let to_fetch: BTreeSet<SourceKey> = if opts.dry_run {
        let due = crate::sync::due(
            &manifest,
            &store,
            &state.failures,
            opts.retry,
            crate::state::now_secs(),
        );
        (due.now.into_iter())
            .filter(|(_, f)| *f == crate::sync::Fetch::Missing)
            .map(|(k, _)| k)
            .collect()
    } else {
        BTreeSet::new()
    };
    let on_disk: BTreeSet<SourceKey> = listed
        .iter()
        .filter(|k| store.has(k) || to_fetch.contains(k))
        .cloned()
        .collect();
    let (mut planned, mut failed) = plan_awaiting(
        &manifest,
        &state,
        &dirs.library,
        Some(&on_disk),
        &to_fetch,
        out,
    )?;
    let mut ok = failed.is_empty();
    failed.extend(to_fetch.iter().cloned());
    adopt(&mut state, &dirs.library, out)?;
    vouch_for_copies(&dirs.library, &mut state);
    if !opts.dry_run {
        let paths = planned
            .iter()
            .map(|(_, r)| path_of(r))
            .chain(state.outputs.keys().cloned());
        crate::library::clear_leftovers(&dirs.library, paths);
    }
    let located: BTreeMap<SourceKey, Located> = listed
        .iter()
        .filter_map(|k| Some((k.clone(), store.locate(k)?)))
        .collect();
    // Beside the home rather than in the system's temporary folder, which
    // may be memory: fitting may render much of the library into it.
    let workshop = crate::atomic::Scratch::within(home)?;
    let (fitted, placed) = if manifest.settings.library.max_size.is_some() {
        if sizes(runner, &store, &planned, &mut state, temp.path(), out)? {
            persist(home, &state, opts.dry_run, &lock)?;
        }
        let block = manifest.settings.library.block_size.0;
        let at = limit::Workshop {
            sources: &located,
            scratch: workshop.path(),
            library: &dirs.library,
            kept: limit::kept(&state, &dirs.library, &failed, block),
            force: opts.force,
        };
        let fitted = limit::fit_library(runner, &manifest, &mut state, &mut planned, &at, out)?;
        if fitted.as_ref().is_some_and(|(f, _)| f.rendered > 0) {
            persist(home, &state, opts.dry_run, &lock)?;
        }
        if let Some((f, _)) = fitted.as_ref().filter(|(f, _)| !f.left_out.is_empty()) {
            ok = false;
            left_out(&manifest, &state, f, out)?;
        }
        fitted.map_or((None, HashMap::new()), |(f, made)| (Some(f), made))
    } else {
        (None, HashMap::new())
    };
    if let Some(run) = run.as_deref_mut() {
        run.outputs_before(&state.outputs);
    }
    let Foreseen {
        files,
        drifts,
        damaged,
        decided,
    } = foresee(
        &manifest,
        &state,
        &planned,
        &dirs.library,
        (opts.force, opts.accept),
    );
    let (taken, kept) = split_drifts(&drifts, &damaged, opts.accept, &state);
    say_drifts(
        (&taken, &kept),
        &planned,
        &decided,
        opts.force,
        &|d| whole_again(d, &state),
        out,
    )?;
    for ((n, r), d) in planned.iter().zip(&decided) {
        let Some(f) = state
            .facts
            .get(&r.plan.audio.key)
            .filter(|_| damaged.contains(n))
        else {
            continue;
        };
        if let (Doing::Held { .. }, Some(damage)) = (d, f.damage()) {
            ok = false;
            crate::ui::error(
                out,
                &format!(
                    "Failed: {}: {}, since its song was built; its file is left as built \
                     (`sync --force` writes it from what the source holds now)",
                    name_of(&manifest.songs[*n], Some(r), &state.facts),
                    damage.describe(&r.plan.audio.key)
                ),
            )?;
        }
    }

    let edits = crate::held::records(&manifest, &state.facts, &taken);
    let mut intent = intend(
        &planned,
        decided,
        &manifest,
        &state.outputs,
        &dirs.library,
        (&failed, opts.force),
        edits,
    );
    if opts.dry_run {
        let shown = Shown {
            manifest: &manifest,
            planned: &planned,
            intent: &intent,
            files: &files,
            old: &state.outputs,
            state: &state,
            to_fetch: &to_fetch,
        };
        // Matched here, once measuring has given each song its tags.
        let matched = match &query {
            Some(q) => {
                let views = crate::query::views(&manifest, &state, &dirs.library)?;
                q.check_fields(&views)?;
                let n = views.iter().enumerate().filter(|(_, v)| q.matches(v));
                Some(n.map(|(n, _)| n).collect())
            }
            None => None,
        };
        let showing = Showing {
            matched,
            all: report.as_ref().is_none_or(|r| r.all),
        };
        let fit = fitted.as_ref().filter(|_| showing.matched.is_none());
        if let Some(Report {
            out: mut report, ..
        }) = report
        {
            out.flush()?;
            status(
                &shown,
                &showing,
                &store,
                &listed,
                dirs,
                opts.settling,
                &mut report,
            )?;
            if let Some(f) = fit {
                fit_summary(f, &mut report)?;
            }
        } else {
            status(&shown, &showing, &store, &listed, dirs, opts.settling, out)?;
            if let Some(f) = fit {
                fit_summary(f, out)?;
            }
        }
        drop(lock);
        return Ok(ok);
    }
    let edits = std::mem::take(&mut intent.edits);
    let mut ledger = Ledger::new(&dirs.library, home, run.as_deref_mut());
    let job = Writing {
        runner,
        dirs,
        manifest: &manifest,
        located: &located,
        placed: &placed,
        fitted: fitted.as_ref(),
        scratch: temp.path(),
        every: opts.checkpoint.unwrap_or(CHECKPOINT),
    };
    let applied = apply(
        &job,
        &planned,
        intent,
        (&mut state, &mut ledger),
        (&mut failed, opts.force),
        out,
    )?;
    ok &= applied.ok;
    state.sizes.extend(applied.learned);
    if let Some(f) = &fitted {
        let block = manifest.settings.library.block_size.0;
        let size = limit::library_size(&state, &dirs.library, block);
        if size > f.max {
            crate::ui::warning(
                out,
                &format!(
                    "The library takes {}, over its max_size of {}; the next sync fits it again",
                    crate::ui::bytes(size),
                    crate::ui::bytes(f.max)
                ),
            )?;
        } else {
            fit_summary(
                &limit::Fitted {
                    projected: size,
                    ..f.clone()
                },
                out,
            )?;
        }
    }
    // A removed song's measures stay while its sources do, so restoring
    // it measures nothing again.
    let mut kept = listed.clone();
    kept.extend(manifest.removed_keys().into_iter().filter(|k| store.has(k)));
    state.facts.retain(|k, _| kept.contains(k));
    match &fitted {
        Some(f) => state.sizes.retain(|key, _| f.keys.contains(key)),
        None => state.sizes.clear(),
    }
    state.failures.retain(|k, _| listed.contains(k));
    state.listed = listed
        .iter()
        .filter(|k| matches!(k, SourceKey::Manual(_)))
        .cloned()
        .collect();
    state
        .alignments
        .retain(|a| kept.contains(&a.a) && kept.contains(&a.b));
    state.mixes.retain(|m| kept.contains(&m.source));
    state.save(home)?;
    for edit in edits {
        manifest.edit(edit);
    }
    manifest.save_locked(&lock)?;
    if let Some(run) = run {
        run.saw(&manifest);
        run.synced();
    }
    let counts = [
        ("written", applied.written + opts.before.written),
        ("moved", applied.moved + opts.before.moved),
        ("removed", applied.removed + opts.before.removed),
    ];
    if counts.iter().any(|(_, n)| *n > 0) {
        if manifest.settings.library.touch_root {
            touch(&dirs.library, out)?;
        }
        let counts = counts.map(|(name, n)| (name, n.to_string()));
        let library = dirs.library.to_string_lossy();
        let values: Vec<(&str, &str)> = std::iter::once(("library", library.as_ref()))
            .chain(counts.iter().map(|(name, n)| (*name, n.as_str())))
            .collect();
        hooks::run(runner, &manifest.hooks, hooks::Event::Changed, &values, out)?;
    }
    Ok(ok)
}

/// What every run leaves, whatever came before it: each file the state
/// records is there, no song has two, and nothing is half written.
#[cfg(test)]
pub(crate) fn invariant(dirs: &Dirs) -> std::result::Result<(), String> {
    let state = State::load(&dirs.home).map_err(|e| format!("{e:#}"))?;
    let mut owners: BTreeMap<&SourceKey, &std::path::PathBuf> = BTreeMap::new();
    for (path, written) in &state.outputs {
        for file in std::iter::once(path).chain(&written.lyrics) {
            if !dirs.library.join(file).exists() {
                return Err(format!("{file:?} is recorded, not there"));
            }
        }
        for key in written
            .sources
            .iter()
            .filter(|k| !crate::manifest::shareable(k))
        {
            if let Some(other) = owners.insert(key, path) {
                return Err(format!("{key} made both {other:?} and {path:?}"));
            }
        }
    }
    let files = crate::store::files_below(&dirs.library, usize::MAX, |_| true)
        .map_err(|e| format!("{e:#}"))?;
    match files
        .iter()
        .find(|f| f.to_string_lossy().ends_with(".part"))
    {
        Some(part) => Err(format!("{} is left half written", part.display())),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests;
