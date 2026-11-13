//! The library brought in line with the song list, offline: every listed
//! source measured, the comparisons each song needs made, each song
//! resolved to a plan and the songs fitted into `[library] max_size`
//! when it is set, a song rendered only when its plan changed, and every
//! file muman wrote that no song makes now deleted.
//!
//! This module runs the steps in order; each is a module of its own:
//! the grounds gathered, in the one way every command that does or says
//! what a sync does gathers them ([`gather`]): the library taken as found
//! before any file of it is judged ([`as_found`]), the moved sources
//! followed, heard as the sync hears them (`keys`), and what the sync
//! waits on and fetches; then `measure` the sources,
//! `keys` renamed or followed, `damage` noted,
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
use crate::history::Recording;
use crate::hooks;
use crate::library::Ledger;
use crate::limit;
use crate::manifest::{Edit, Manifest};
use crate::query::Query;
use crate::query::QueryMatch;
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::{Located, Store};

pub use self::damage::{as_its_torrent, note_damaged};
use self::damage::{note_damage, say_drifts, split_drifts, whole_again};
use self::decide::Found;
pub use self::decide::{
    Ahead, Doing, Fated, Forced, Foreseen, Foresight, Foretold, Grounds, Intent, Rewrite, Rewrites,
    as_held, files_of, foresee, foresight, left_as_built, left_in_place,
};
pub(crate) use self::keys::arriving;
use self::keys::sites_found;
pub use self::keys::{Following, Moves, Unlisted, deleted, found_moves, hearing, moved, not_back};
pub use self::measure::{Measuring, look, measure};
use self::measure::{analyze, hash, sizes};
pub use self::measure::{analyze_mixes, compare};
pub use self::owned::{
    Adopted, LEFT_KEPT, adopt, as_found, changed_since_written, edited_since_written, found,
    rehome, sidecar_as_written, vouch_for_copies,
};
use self::plan::plan_awaiting;
pub use self::plan::{
    Planned, Resolver, name_of, path_of, plan, plan_with, resolve_as_fitted, resolve_songs,
};
pub use self::prune::Pruned;
pub use self::report::{Fate, verdict};
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

/// How long measuring runs between saves of what it measured.
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
    // Makes a step's folder inside the run's scratch folder.
    #[expect(clippy::disallowed_methods)]
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
/// library holds. A dry run follows a moved file in memory alone, so what
/// it holds of one is under the name the next sync gives it, which the
/// store may still give another file, as when two files trade names: it
/// is not kept, or it would stand for that file at the sync.
fn persist(home: &Path, state: &State, dry_run: bool, lock: &Lock) -> Result<()> {
    if dry_run {
        let renamed = state.respelled.keys().cloned().collect();
        State::keep_caches(home, &state.caches().without(&renamed), lock)
    } else {
        state.save(home)
    }
}

/// What every command that does or says what a sync does reads, gathered
/// once, by [`gather`], in one way for all of them: the song list with each
/// manual file it names followed to where it moved, heard as the sync hears
/// it; the state as the library is found; the sources there; and what the
/// sync waits on and fetches first. [`Ground::grounds`] lends it to the
/// sync's decisions ([`Grounds`]) and to every forecast's ([`foresight`]),
/// so no forecast lacks an input the sync has.
#[derive(Debug)]
pub struct Ground {
    /// The song list, each moved file followed: in memory alone for a dry
    /// run.
    pub manifest: Manifest,
    pub state: State,
    pub store: Store,
    pub ahead: Ahead,
    /// The sources as the song list names them, before any is followed.
    pub listed: BTreeSet<SourceKey>,
    /// What following found, what it cannot tell among it.
    pub moves: Moves,
    /// The files no song names that are none of those followed.
    pub unlisted: Unlisted,
    /// The library folder named anew, as the state takes it.
    pub adopted: Option<Adopted>,
}

impl Ground {
    /// What a sync into `library`, `asked` to force and to accept, decides
    /// from.
    #[must_use]
    pub fn grounds<'a>(&'a self, library: &'a Path, asked: (bool, bool)) -> Grounds<'a> {
        Grounds {
            manifest: &self.manifest,
            state: &self.state,
            store: &self.store,
            library,
            ahead: &self.ahead,
            asked,
        }
    }
}

/// How [`gather`] gathers the grounds: following moves as `following`
/// says, saving them under its lock or, for a dry run, in memory alone;
/// and whether the sync it is for `fetches` before it plans, counting what
/// it fetches as there, as a dry run does (`Some`, with whether
/// `--retry` was given).
#[derive(Debug)]
pub struct Gathering<'a> {
    pub following: Following<'a>,
    pub fetches: Option<bool>,
}

impl Gathering<'_> {
    /// As a command that changes nothing reads what the next sync does:
    /// a dry run of it, holding no lock, a file arrived within `settling`
    /// still being copied in.
    #[must_use]
    pub fn forecast(home: &Path, settling: Duration) -> Gathering<'_> {
        Gathering {
            following: Following {
                home,
                lock: None,
                rec: crate::history::Recording::DryRun,
                dry_run: true,
                settling,
                listing: false,
            },
            fetches: Some(false),
        }
    }
}

/// The grounds a run in `dirs` reads, `manifest` the song list it was
/// given ([`Ground`]): the state taken as the library is found
/// ([`as_found`]), what it adopted said to `out`; for a run that changes
/// the library, what a stopped run left there cleared; the sources
/// scanned; each manual file followed, heard by `hear` where nothing
/// measured tells it ([`Following::follow`]); and what the sync waits on
/// and fetches ([`Ahead::of`]).
pub fn gather<W: Write>(
    dirs: &Dirs,
    mut manifest: Manifest,
    how: Gathering<'_>,
    hear: impl FnMut(&Located) -> Option<String>,
    out: &mut W,
) -> Result<Ground> {
    let Gathering {
        mut following,
        fetches,
    } = how;
    let mut state = State::load(&dirs.home)?;
    let same =
        (state.library.as_deref()).is_some_and(|l| crate::platform::same_path(l, &dirs.library));
    let moves = if following.dry_run {
        crate::history::Moves::Foresee
    } else {
        crate::history::Moves::Follow
    };
    let adopted = as_found(dirs, &mut state, moves)?;
    if let Some(adopted) = &adopted {
        adopted.say(out)?;
    }
    if !following.dry_run {
        if let Some(run) = following.rec.run().filter(|_| same) {
            run.forget_unmade(&dirs.library);
        }
        let moving = crate::history::moving(&dirs.home);
        crate::library::Leftovers::of(&state.outputs, &moving).clear(&dirs.library);
    }
    let store = Store::scan(dirs)?;
    let listed = manifest.keys();
    let settling = following.settling;
    let (moves, unlisted) = following.follow(&mut manifest, &mut state, &store, hear, out)?;
    let ahead = Ahead::of(&manifest, &state, &store, &moves, settling, fetches);
    Ok(Ground {
        manifest,
        state,
        store,
        ahead,
        listed,
        moves,
        unlisted,
        adopted,
    })
}

/// A forecast's [`gather`]: the grounds a command that changes nothing
/// reads in `dirs`, a moved file heard by `runner` as the sync hears it,
/// its files written below `scratch`.
pub fn foreground<R: Runner>(
    runner: &R,
    dirs: &Dirs,
    settling: Duration,
    scratch: &Path,
) -> Result<Ground> {
    let manifest = Manifest::load(&dirs.home)?;
    gather(
        dirs,
        manifest,
        Gathering::forecast(&dirs.home, settling),
        hearing(runner, scratch),
        &mut std::io::sink(),
    )
}

/// What fitting the songs into `[library] max_size` reads: the song list,
/// the sources and the library, the sources on disk, fetched first and
/// failing, and whether the sync was asked to force and to accept.
pub(crate) struct Fitting<'a> {
    pub manifest: &'a Manifest,
    pub store: &'a Store,
    pub library: &'a Path,
    pub found: (
        &'a BTreeSet<SourceKey>,
        &'a BTreeSet<SourceKey>,
        &'a BTreeSet<SourceKey>,
    ),
    pub asked: (bool, bool),
}

/// Where fitting learns the sizes it fits by: by measuring and rendering
/// from `located`, below `scratch` and in the `workshop`, as a sync does;
/// with none, from what the state records alone, as a forecast does.
pub(crate) struct Learning<'a> {
    pub located: &'a BTreeMap<SourceKey, Located>,
    pub scratch: &'a Path,
    pub workshop: Option<&'a Path>,
}

/// What [`Fitting::fit`] did: the fit, the songs rendered to measure them,
/// and whether it measured anything the state keeps.
pub(crate) struct Fit {
    pub fitted: limit::Fitted,
    pub made: limit::Made,
    pub learnt: bool,
}

impl Fitting<'_> {
    /// The songs of `planned` fitted as the module says, the one way the
    /// sync and every forecast fit them: each copy's audio bytes measured
    /// where its facts lack them, the songs fitted ([`limit::fit_library`])
    /// and those lowered levelled again at the format fitting gave them, a
    /// song rendered at the gains it had before rendered again. Learning
    /// from the state alone, a size it records none of is an estimate, and
    /// its song is in [`limit::Fitted::guessed`]. None when no limit is set.
    pub fn fit<R: Runner, W: Write>(
        &self,
        runner: &R,
        state: &mut State,
        planned: &mut Planned,
        learning: &Learning<'_>,
        out: &mut W,
    ) -> Result<Option<Fit>> {
        if self.manifest.settings.library.max_size.is_none() {
            return Ok(None);
        }
        let (on_disk, to_fetch, failed) = self.found;
        let mut learnt = false;
        let mut unmeasured = BTreeSet::new();
        if learning.workshop.is_some() {
            learnt |= sizes(runner, self.store, planned, state, learning.scratch, out)?;
        } else {
            unmeasured = measure::unmeasured(planned, state);
        }
        let block = self.manifest.settings.library.block_size.0;
        let held = left_as_built(self.manifest, state, planned, self.library, self.asked);
        let at = limit::Workshop {
            sources: learning.located,
            scratch: learning.workshop.unwrap_or(learning.scratch),
            library: self.library,
            kept: limit::kept(state, self.library, failed, block),
            force: self.asked.0,
            held: &held,
            renders: learning.workshop.is_some(),
        };
        let Some((mut fitted, mut made)) =
            limit::fit_library(runner, self.manifest, state, planned, &at, out)?
        else {
            return Ok(None);
        };
        learnt |= fitted.rendered > 0;
        fitted.guessed.extend(unmeasured);
        if fitted.lowered > 0 {
            let as_fitted: BTreeMap<usize, crate::resolve::Plan> =
                planned.iter().map(|(n, r)| (*n, r.plan.clone())).collect();
            let found = (Some(on_disk), to_fetch);
            plan::level_as_fitted(
                self.manifest,
                state,
                self.library,
                found,
                self.asked,
                planned,
            )?;
            // A song rendered at the gains it had before is rendered again.
            for (n, r) in planned.iter() {
                if as_fitted.get(n) != Some(&r.plan) {
                    made.remove(n);
                }
            }
        }
        Ok(Some(Fit {
            fitted,
            made,
            learnt,
        }))
    }
}

/// What a sync did: whether every song was written, and the song list's
/// hooks with what the run changed, to run once its command has let go
/// of the home.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub struct Synced {
    pub ok: bool,
    pub due: hooks::Due,
    /// What a dry run foretells the sync does with each library file.
    pub foretold: Option<Foretold>,
}

/// Bring the library in line with the song list, recording what it
/// replaces and removes in the run `rec` names.
pub fn reconcile<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    opts: Options,
    rec: Recording<'_>,
    out: &mut W,
) -> Result<Synced> {
    reconcile_into(runner, dirs, opts, rec, out, None)
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
    mut rec: Recording<'_>,
    out: &mut W,
    report: Option<Report<'_>>,
) -> Result<Synced> {
    let lock = Lock::folder(&dirs.home)?;
    let home = &dirs.home;
    crate::state::present(home)?;
    dirs.apart()?;
    let _library = if opts.dry_run {
        None
    } else {
        Some(Lock::library(&dirs.library, dirs.lock_folder()?)?)
    };
    let manifest = Manifest::load(home)?;
    if let Some(run) = rec.run() {
        run.saw(&manifest);
    }
    let query = report
        .as_ref()
        .map(|r| Query::parse(r.query, &crate::query::extractors(&manifest)))
        .transpose()?
        .filter(|q| !q.is_empty());
    let temp = crate::atomic::Scratch::new()?;
    let how = Gathering {
        following: Following {
            home,
            lock: Some(&lock),
            rec: rec.reborrow(),
            dry_run: opts.dry_run,
            settling: opts.settling,
            listing: false,
        },
        fetches: opts.dry_run.then_some(opts.retry),
    };
    let Ground {
        mut manifest,
        mut state,
        store,
        ahead,
        moves,
        unlisted,
        ..
    } = gather(dirs, manifest, how, hearing(runner, temp.path()), out)?;
    let unsure = &moves.unsure;
    let listed = manifest.keys();
    let mut how = Measuring {
        retry: opts.retry,
        say_unread: true,
        checkpoint: &mut |s: &State| persist(home, s, opts.dry_run, &lock),
        analyses: crate::analysis::wanted(
            &manifest.settings.loudness,
            manifest.settings.audio.pick.dynamics.enabled,
            manifest.settings.audio.pick.bandwidth.weight() != 0,
        ),
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
    let analyses = crate::analysis::wanted(
        &manifest.settings.loudness,
        manifest.settings.audio.pick.dynamics.enabled,
        manifest.settings.audio.pick.bandwidth.weight() != 0,
    );
    let analyzed = analyze(
        runner,
        &store,
        &listed,
        &mut state,
        temp.path(),
        &analyses,
        out,
    )?;
    let fitted = manifest.settings.library.max_size.is_some();
    let mixed = analyze_mixes(
        runner,
        &store,
        &listed,
        &mut state,
        (&manifest.settings, fitted),
        out,
    )?;
    if analyzed || mixed {
        persist(home, &state, opts.dry_run, &lock)?;
    }
    if !opts.dry_run {
        note_damage(&manifest, &store, &mut state, out)?;
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
        // In the order the keys module's docs give, so a run stopped
        // anywhere keeps the file.
        let ytdlp = dirs.ytdlp();
        let mut linked = Vec::new();
        let mut unlinked = Vec::new();
        for (from, to) in &renames {
            let at = store.locate(from).map(|l| l.path);
            if let Some(at) = at.filter(|p| p.starts_with(&ytdlp)) {
                match crate::store::link(&ytdlp, to, &at)? {
                    Some(_) => linked.push(at),
                    None => unlinked.push((to, at)),
                }
            }
        }
        state.rekey(&renames)?;
        state.save(home)?;
        for (to, at) in unlinked {
            crate::store::place(&ytdlp, to, &at)?;
        }
        for (from, to) in &renames {
            manifest.edit(Edit::Respell {
                from: from.clone(),
                to: to.clone(),
            });
        }
        manifest.save_locked(&lock, &mut rec)?;
        for at in linked {
            crate::store::unplace(&ytdlp, &at)?;
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

    let asked = (opts.force, opts.accept);
    let on_disk = Grounds {
        manifest: &manifest,
        state: &state,
        store: &store,
        library: &dirs.library,
        ahead: &ahead,
        asked,
    }
    .on_disk();
    let (mut planned, failed) = plan_awaiting(
        &manifest,
        &state,
        &dirs.library,
        (Some(&on_disk), &ahead.waiting),
        asked,
        out,
    )?;
    let mut ok = failed.is_empty() && unsure.is_empty();
    let mut failed = ahead.failing(failed);
    // Every song this sync fails, by its place in the list, with why, so
    // `check` names each until a sync writes it.
    let mut song_errors: Vec<(usize, String)> = Vec::new();
    let located: BTreeMap<SourceKey, Located> = listed
        .iter()
        .filter_map(|k| Some((k.clone(), store.locate(k)?)))
        .collect();
    // Beside the home rather than in the system's temporary folder, which
    // may be memory: fitting may render much of the library into it. A
    // dry run on a home it may not write, as a read-only mount, uses that.
    let workshop = match crate::atomic::Scratch::within(home) {
        Err(_) if opts.dry_run => crate::atomic::Scratch::new()?,
        workshop => workshop?,
    };
    let fitting = Fitting {
        manifest: &manifest,
        store: &store,
        library: &dirs.library,
        found: (&on_disk, &ahead.to_fetch, &failed),
        asked,
    };
    let learning = Learning {
        located: &located,
        scratch: temp.path(),
        workshop: Some(workshop.path()),
    };
    let fit = fitting.fit(runner, &mut state, &mut planned, &learning, out)?;
    if fit.as_ref().is_some_and(|f| f.learnt) {
        persist(home, &state, opts.dry_run, &lock)?;
    }
    let (fitted, placed) = fit.map_or((None, HashMap::new()), |f| (Some(f.fitted), f.made));
    if let Some(f) = fitted.as_ref().filter(|f| !f.left_out.is_empty()) {
        ok = false;
        left_out(&manifest, &state, f, out)?;
        let why = format!(
            "max_size {} cannot hold it, at the lowest bitrates",
            crate::ui::bytes(f.max)
        );
        song_errors.extend(f.left_out.iter().map(|n| (*n, why.clone())));
    }
    if let Some(run) = rec.run() {
        // Saved as the run records it, before any file is changed, so a
        // stopped run's moves are followed from the outputs as found.
        persist(home, &state, opts.dry_run, &lock)?;
        run.outputs_before(&state.outputs);
    }
    // A dry run's renders are thrown away and their sizes kept, so the sync
    // after renders none of them, and writes each as one it did not render.
    let placed_songs: BTreeSet<usize> =
        (placed.keys().copied()).filter(|_| !opts.dry_run).collect();
    let (
        Foreseen {
            files,
            drifts,
            damaged,
            decided,
            ..
        },
        mut intent,
    ) = Grounds {
        manifest: &manifest,
        state: &state,
        store: &store,
        library: &dirs.library,
        ahead: &ahead,
        asked,
    }
    .intend(&planned, &failed, &placed_songs, Vec::new());
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
        if let (Doing::Held { file }, Some(damage)) = (d, f.damage()) {
            ok = false;
            song_errors.push((*n, damage.describe(&r.plan.audio.key)));
            let left = if file.is_some() {
                "its file is left as built"
            } else {
                "no file built before is left, and it is not written from it"
            };
            crate::ui::error(
                out,
                &format!(
                    "Failed: {}: {}, since its song was built; {left} \
                     (`sync --force` writes it from what the source holds now)",
                    name_of(&manifest.songs[*n], Some(r), &state.facts),
                    damage.describe(&r.plan.audio.key)
                ),
            )?;
        }
    }

    if !opts.dry_run {
        for (n, r) in &planned {
            for said in r
                .why
                .doubts
                .iter()
                .filter_map(crate::resolve::Doubt::pinned_to_nothing)
            {
                let name = name_of(&manifest.songs[*n], Some(r), &state.facts);
                crate::ui::warning(out, &format!("{name}: {said}"))?;
            }
        }
    }

    intent.edits = crate::held::records(&manifest, &state.facts, &taken);
    if opts.dry_run {
        let shown_planned = as_held(&planned, &decided, &state.outputs);
        let shown = Shown {
            manifest: &manifest,
            planned: &shown_planned,
            intent: &intent,
            files: &files,
            old: &state.outputs,
            state: &state,
            to_fetch: &ahead.to_fetch,
            moves: &moves,
            unlisted: &unlisted,
        };
        // Matched here, once measuring has given each song its tags.
        let matched = match &query {
            Some(q) => {
                let views: Vec<crate::query::View> =
                    (crate::query::views_of(&manifest, &state.facts, &shown_planned).into_iter())
                        .map(|(view, _)| view)
                        .collect();
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
            status(&shown, &showing, &store, &listed, dirs, &mut report)?;
            if let Some(f) = fit {
                fit_summary(f, &mut report)?;
            }
        } else {
            status(&shown, &showing, &store, &listed, dirs, out)?;
            if let Some(f) = fit {
                fit_summary(f, out)?;
            }
        }
        drop(lock);
        let moving = crate::history::moving(home);
        let leftovers = crate::library::Leftovers::of(&state.outputs, &moving);
        return Ok(Synced {
            ok,
            due: hooks::Due::default(),
            foretold: Some(intent.foretold(&Found::of(&dirs.library, &state), &leftovers)),
        });
    }
    let edits = std::mem::take(&mut intent.edits);
    let mut ledger = Ledger::new(&dirs.library, home, rec.run());
    let job = Writing {
        runner,
        dirs,
        manifest: &manifest,
        located: &located,
        placed: &placed,
        fitted: fitted.as_ref(),
        scratch: temp.path(),
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
    state.songs_failed(
        (song_errors.into_iter().chain(applied.songs_failed))
            .filter_map(|(n, e)| Some((manifest.songs[n].id()?.clone(), e))),
    );
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
    // it measures nothing again, and so do those of files not followed,
    // which tell them for the next run.
    let mut kept = listed.clone();
    kept.extend(manifest.removed_keys().into_iter().filter(|k| store.has(k)));
    kept.extend(unsure.iter().flat_map(|(_, files)| files).cloned());
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
    // Comparisons and mixes are of audio: one goes once no source kept
    // holds its audio.
    let audio: BTreeSet<String> = state
        .facts
        .values()
        .map(crate::facts::Facts::audio_rev)
        .collect();
    state
        .alignments
        .retain(|a| audio.contains(&a.revs.0) && audio.contains(&a.revs.1));
    state.mixes.retain(|m| audio.contains(&m.audio));
    crate::levels::keep_built(&mut state);
    if let Some(run) = rec.run() {
        run.outputs_after(home, &state.outputs)?;
    }
    state.save(home)?;
    for edit in edits {
        manifest.edit(edit);
    }
    manifest.save_locked(&lock, &mut rec)?;
    if let Some(run) = rec.run() {
        run.synced();
    }
    let counts = (
        applied.written + opts.before.written,
        applied.moved + opts.before.moved,
        applied.removed + opts.before.removed,
    );
    let mut due = hooks::Due::default();
    due.add(&manifest.hooks, &dirs.library, counts);
    if due.changed() && manifest.settings.library.touch_root {
        touch(&dirs.library, out)?;
    }
    Ok(Synced {
        ok,
        due,
        foretold: None,
    })
}

/// What every run leaves, whatever came before it: each file the state
/// records is there, no song has two, and nothing is half written.
#[cfg(any(test, feature = "testing"))]
pub fn invariant(dirs: &Dirs) -> std::result::Result<(), String> {
    let state = State::load(&dirs.home).map_err(|e| format!("{e:#}"))?;
    let mut owners: BTreeMap<&SourceKey, &std::path::PathBuf> = BTreeMap::new();
    for (path, written) in &state.outputs {
        for file in written.files(path) {
            if !dirs.library.join(file).exists() {
                return Err(format!(
                    "{} is recorded, not there",
                    crate::relpath::show(file)
                ));
            }
        }
        for key in written
            .sources
            .iter()
            .filter(|k| !crate::manifest::shareable(k))
        {
            if let Some(other) = owners.insert(key, path) {
                return Err(format!(
                    "{key} made both {} and {}",
                    crate::relpath::show(other),
                    crate::relpath::show(path)
                ));
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

/// What a command that changes nothing, as `check` and `info`, foretells
/// the next sync does with each library file, by the grounds it gathers
/// ([`foreground`]) and what it foresees of them ([`foresight`]), a file
/// dropped in within `settling` still being copied in: none where the
/// sync measures or renders first what could make it decide otherwise
/// ([`Foresight::whole`]).
#[cfg(any(test, feature = "testing"))]
pub fn forecast<R: Runner>(
    runner: &R,
    dirs: &Dirs,
    settling: Duration,
) -> Result<Option<Foretold>> {
    let scratch = crate::atomic::Scratch::new()?;
    let ground = foreground(runner, dirs, settling, scratch.path())?;
    let foresight = foresight(runner, &ground.grounds(&dirs.library, (false, false)))?;
    if !foresight.whole() {
        return Ok(None);
    }
    let moving = crate::history::moving(&dirs.home);
    let leftovers = crate::library::Leftovers::of(&ground.state.outputs, &moving);
    let found = Found::of(&dirs.library, &ground.state);
    Ok(Some(foresight.intent.foretold(&found, &leftovers)))
}

/// Each file below a library folder, by its path, with what it holds and
/// its stamp, which a move keeps and a write does not.
#[cfg(any(test, feature = "testing"))]
pub type Stamped = BTreeMap<std::path::PathBuf, (Vec<u8>, Option<String>)>;

/// Every file below `library`, as [`Stamped`] holds it.
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub fn stamped(library: &Path) -> Stamped {
    (crate::store::files_below(library, usize::MAX, |_| true).unwrap_or_default())
        .into_iter()
        .filter_map(|file| {
            let bytes = std::fs::read(&file).ok()?;
            let stamp = crate::store::stamp_text(&file);
            Some((
                file.strip_prefix(library).ok()?.to_path_buf(),
                (bytes, stamp),
            ))
        })
        .collect()
}

/// Whether a sync, from the library `before` to the one `after`, did with
/// each file what a dry run before it `foretold`: each left as it was,
/// moved as it was from where it was, written, or gone.
#[cfg(any(test, feature = "testing"))]
pub fn as_foretold(
    foretold: &Foretold,
    before: &Stamped,
    after: &Stamped,
) -> std::result::Result<(), String> {
    let paths: BTreeSet<&std::path::PathBuf> = (before.keys())
        .chain(after.keys())
        .chain(foretold.0.keys())
        .collect();
    for path in paths {
        let meant = match foretold.0.get(path) {
            Some(fate) => fate.clone(),
            None if before.contains_key(path) => Fated::From(path.clone()),
            None => Fated::Gone,
        };
        let done = match after.get(path) {
            None => Fated::Gone,
            Some(now) => match &meant {
                Fated::From(was) if before.get(was) == Some(now) => Fated::From(was.clone()),
                _ if before.get(path) == Some(now) => Fated::From(path.clone()),
                _ => Fated::Written,
            },
        };
        if done != meant {
            return Err(format!(
                "{}: foretold {meant:?}, done {done:?}",
                crate::relpath::show(path)
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

/// Whether `key`'s file reads as none now, its Measure failure applying:
/// no song is made from it, and its song is held as it was rather than
/// written from bytes that do not read.
#[must_use]
pub fn unread(state: &State, store: &Store, key: &SourceKey) -> bool {
    (state.failures.get(key)).is_some_and(|f| {
        f.step == crate::state::Step::Measure
            && f.applies(store.locate(key).as_ref(), state.facts.get(key))
    })
}
