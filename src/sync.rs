//! `muman sync`, and the network steps `add` shares with it: one
//! fetcher into the store for a run, what a step found recorded, each
//! proposal listed as a song, and every lookup due made.
//!
//! A sync fetches the torrent files it lacks beside yt-dlp's fetches, on
//! a thread of their own: the two share no program and no folder, and a
//! torrent no peer seeds would otherwise hold yt-dlp up for its whole
//! stall. What that thread says waits until both end, as two writers
//! would tear each other's lines, and its progress takes the status line
//! only once yt-dlp's gives it up ([`crate::progress::step_beneath`]).
//! Each side's failures are recorded whatever the other's end.
//!
//! What a sync fetches is decided once, by [`crate::due`].

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::Job;
use crate::acquire::{Acquire, Additions, Fetcher, Proposal};
use crate::cli::Matching;
use crate::dirs::Dirs;
use crate::history::Run;
use crate::http::HttpTransport;
use crate::identify::Mode;
use crate::manifest::{Edit, Manifest, Tags};
use crate::reconcile::{Options, Unlisted};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::State;
use crate::store::Store;
use crate::ui::Prompter;
use crate::{atomic, identify, lookup, manifest, plugins, relpath, state, ui};

/// What `sync` was given: its flags, each on its own.
#[derive(Debug, Clone, Copy)]
// Each is a flag of its own on the command line.
#[expect(clippy::struct_excessive_bools)]
pub(crate) struct Request {
    pub rematch: bool,
    pub until_done: bool,
    pub force: bool,
    pub retry: bool,
    pub accept: bool,
    pub update_defaults: bool,
    pub matching: Matching,
}

/// Run `sync`; whether every step succeeded.
pub(crate) fn run<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
    request: Request,
) -> Result<bool> {
    let dirs = &job.dirs;
    crate::atomic::private_dir(&dirs.home)?;
    manifest::start(&dirs.home)?;
    // Before anything is fetched: a template refused would fail every song.
    Manifest::load(&dirs.home)?
        .settings
        .library
        .naming(&dirs.library)?;
    let mut run = Run::begin(&dirs.home)?;
    let mut due = crate::hooks::Due::default();
    let done = (|| {
        if request.update_defaults {
            update_settings(&dirs.home, out)?;
        }
        let proposals = dropped_in(runner, dirs, job.settling, out)?;
        if !job.offline {
            crate::reconcile::note_damaged(runner, dirs, request.retry, out)?;
        }
        let manifest = Manifest::load(&dirs.home)?;
        let retry = request.retry || request.until_done;
        let mut ok = fetch_missing(job, runner, http, &manifest, retry, out)?;
        let how = Listing {
            mode: crate::mode(request.matching),
            verbose: job.verbose,
            tags: Vec::new(),
            loose: false,
        };
        list(runner, dirs, proposals, &how, prompter, out)?;
        let ask = lookup::Ask {
            rematch: request.rematch,
            persist: request.until_done,
        };
        ok &= look_up(job, runner, http, ask, &BTreeSet::new(), out)?;
        let opts = Options {
            force: request.force,
            retry,
            accept: request.accept,
            ..Options::default()
        };
        ok &= crate::reconcile_now(job, runner, opts, Some(&mut run), &mut due, out)?;
        Ok(ok)
    })();
    crate::finished(job, runner, run, done, &due, out)
}

/// Make every lookup due, as `lookup::run` does.
pub(crate) fn look_up<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    ask: lookup::Ask,
    declined: &BTreeSet<SourceKey>,
    out: &mut W,
) -> Result<bool> {
    if job.offline {
        return Ok(true);
    }
    let (ok, ()) = network(job, runner, out, |acquire| {
        Ok((
            lookup::run(acquire, &job.dirs, http, &job.throttles, ask, declined)?,
            (),
        ))
    })?;
    Ok(ok)
}

/// Fetch again every listed source gone from the store, those that
/// failed lately left for later unless `retry`, and record each that
/// fails. Returns whether every one fetched arrived.
pub(crate) fn fetch_missing<R: Runner, W: Write>(
    job: &Job,
    runner: &R,
    http: &(dyn HttpTransport + Sync),
    manifest: &Manifest,
    retry: bool,
    out: &mut W,
) -> Result<bool> {
    if job.offline {
        return Ok(true);
    }
    let mut state = State::load(&job.dirs.home)?;
    let base = state.caches();
    let root = job.dirs.torrent();
    let partial = job
        .dirs
        .home
        .join(crate::dirs::PARTIAL)
        .join(crate::source::TORRENT);
    let places = crate::aria2::Places {
        store: &root,
        partial: &partial,
        options: &manifest.settings.torrent,
    };
    let due = torrents_due(job, &places, manifest, &state.failures, retry, out)?;
    let (fetched, (torrents, said)) = std::thread::scope(|scope| {
        let beside = scope.spawn(|| {
            let mut said = Vec::new();
            let listed = (&manifest.torrents, &manifest.keys());
            let fetched = crate::aria2::fetch_due(runner, &mut said, &places, listed, &due);
            (fetched, said)
        });
        let fetched = network(job, runner, out, |acquire| {
            acquire.missing(manifest, &state.failures, retry)
        });
        let beside = beside
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        (fetched, beside)
    });
    out.write_all(&said)?;
    let mut ok = true;
    let mut failed = Vec::new();
    let mut same = Vec::new();
    let mut error = None;
    match fetched {
        Ok((fine, f)) => {
            ok &= fine;
            failed.extend(f);
        }
        Err(e) => error = Some(e),
    }
    match torrents {
        Ok(f) => {
            ok &= f.failed.is_empty();
            failed.extend(f.failed);
            same = f.same;
        }
        Err(e) => error = error.or(Some(e)),
    }
    if let Some(e) = error {
        record_fetches(job, &mut state, &base, failed, &same, out)?;
        return Err(e);
    }
    let records = lookup::refetch(
        &job.dirs,
        manifest,
        http,
        &job.throttles,
        &state.failures,
        retry,
    )?;
    if records.waiting > 0 {
        ui::warning(
            out,
            &format!(
                "Not fetching {} missing record(s) that failed lately; `sync --retry` tries now",
                records.waiting
            ),
        )?;
    }
    if records.off > 0 {
        ui::info(
            out,
            &format!(
                "Not fetching {} missing record(s) whose provider is off",
                records.off
            ),
        )?;
    }
    for (why, n) in &records.put_off {
        ui::warning(
            out,
            &format!("{why}: {n} missing record(s) wait for the next run"),
        )?;
    }
    for (key, error) in &records.failed {
        ok = false;
        ui::error(out, &format!("Could not fetch {key} again: {error}"))?;
    }
    failed.extend(records.failed);
    record_fetches(job, &mut state, &base, failed, &same, out)?;
    Ok(ok)
}

/// Record each of `failed` that did not arrive, and each of `same`, a
/// torrent's file fetched back as the store held it, as one no fetch
/// mends, saying so; and clear the failure of each the store now holds.
fn record_fetches<W: Write>(
    job: &Job,
    state: &mut State,
    base: &state::Caches,
    failed: Vec<(SourceKey, String)>,
    same: &[SourceKey],
    out: &mut W,
) -> Result<()> {
    let store = Store::scan(&job.dirs)?;
    let arrived: Vec<SourceKey> = state
        .failures
        .iter()
        .filter(|(k, f)| f.step == state::Step::Fetch && store.has(k))
        .map(|(k, _)| k.clone())
        .collect();
    for key in &arrived {
        state.clear_failure(key);
    }
    if !failed.is_empty() || !arrived.is_empty() || !same.is_empty() {
        for key in same {
            let rev = store.locate(key).map(|l| l.rev());
            crate::reconcile::as_its_torrent(state, key, rev, out)?;
        }
        for (key, error) in failed {
            // A damaged download that did not come again counts against
            // its tries, and keeps the damaged copy's revision.
            let (step, rev) = match state.failures.get(&key) {
                Some(f) if f.step == state::Step::Refetch => (state::Step::Refetch, f.rev.clone()),
                _ => (state::Step::Fetch, None),
            };
            state.record_failure(&key, step, rev, error);
        }
        State::keep_measures(&job.dirs.home, &state.since(base))?;
    }
    Ok(())
}

/// The listed torrent files due to be fetched again, by torrent, as
/// [`crate::due::torrents`] says and says them; what waits in `places`'
/// partial folder too long is cleared first.
fn torrents_due<W: Write>(
    job: &Job,
    places: &crate::aria2::Places<'_>,
    manifest: &Manifest,
    failures: &BTreeMap<SourceKey, state::Failure>,
    retry: bool,
    out: &mut W,
) -> Result<BTreeMap<String, Vec<SourceKey>>> {
    let store = Store::scan(&job.dirs)?;
    let due = crate::due::due(manifest, &store, failures, retry, state::now_secs());
    let torrent = |k: &SourceKey| matches!(k, SourceKey::Torrent(_));
    if !due.now.keys().chain(&due.waiting).any(torrent) {
        return Ok(BTreeMap::new());
    }
    clear_stale(places.partial, places.options.keep_partial, None);
    crate::due::torrents(out, &due)
}

/// Move the song list's settings still at an earlier edition's default
/// to the current one, saying which moved.
pub(crate) fn update_settings<W: Write>(home: &Path, out: &mut W) -> Result<()> {
    let mut manifest = Manifest::load(home)?;
    let moved = manifest.stale_defaults.clone();
    manifest.edit(Edit::UpdateDefaults);
    manifest.save()?;
    if moved.is_empty() {
        ui::info(
            out,
            "No setting is at an older edition's default: each is at this muman's, or set by you",
        )?;
    }
    for s in &moved {
        ui::info(out, &format!("Updated: `{s}` to {}", s.now))?;
    }
    Ok(())
}

/// Run one network step with a fetcher into the store, and every key a
/// song lists or a release replaced known.
pub(crate) fn network<R: Runner, W: Write, T>(
    job: &Job,
    runner: &R,
    out: &mut W,
    step: impl FnOnce(&mut Acquire<'_, R, W>) -> Result<(bool, T)>,
) -> Result<(bool, T)> {
    let manifest = Manifest::load(&job.dirs.home)?;
    let state = State::load(&job.dirs.home)?;
    let store = Store::scan(&job.dirs)?;
    let temp = atomic::Scratch::new()?;
    let ytdlp = job.dirs.ytdlp();
    let partial = job.dirs.home.join(crate::dirs::PARTIAL);
    clear_stale(
        &partial,
        manifest.settings.ytdlp.keep_partial,
        Some(&partial.join(crate::source::TORRENT)),
    );
    let plugins = match &job.cache {
        Some(cache) if manifest.settings.ytdlp.plugins => Some(plugins::folder(cache)?),
        _ => None,
    };
    let passed = crate::download::Passed::write(&manifest.settings.ytdlp, temp.path())?;
    let fetcher = Fetcher {
        store: &ytdlp,
        temp: temp.path(),
        partial: &partial,
        plugins: plugins.as_deref(),
        options: &manifest.settings.ytdlp,
        passed: &passed,
        sites: &manifest.settings.sites,
        live: job.live,
        runs: Cell::new(0),
    };
    let removed = manifest.removed_keys();
    let mut known: BTreeSet<SourceKey> = manifest.keys();
    known.extend(removed.iter().cloned());
    known.extend(state.replaced.keys().cloned());
    let mut acquire = Acquire {
        runner,
        fetcher: &fetcher,
        out,
        known,
        removed,
        store: &store,
    };
    step(&mut acquire)
}

/// Remove what a fetch left unfinished longer than `limit` ago, in any
/// folder below `partial` but `apart`, which keeps its own time, and the
/// folders that leaves empty; younger, a later run resumes it. `apart`
/// itself stays even empty: a torrent fetched beside this run makes its
/// folder in it, and would find it gone.
// Removes downloads left unfinished too long.
#[expect(clippy::disallowed_methods)]
pub(crate) fn clear_stale(partial: &Path, limit: std::time::Duration, apart: Option<&Path>) {
    let mut folders = Vec::new();
    let mut left = vec![partial.to_path_buf()];
    while let Some(dir) = left.pop() {
        if Some(dir.as_path()) == apart {
            continue;
        }
        for path in std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| Some(e.ok()?.path()))
        {
            if path.is_dir() {
                if Some(path.as_path()) != apart {
                    folders.push(path.clone());
                }
                left.push(path);
                continue;
            }
            let old = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > limit));
            if old {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    // Deepest first; only a folder left empty goes.
    folders.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in folders {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Record what a network step found beyond its proposals: albums and
/// renames in the song list, replaced uploads in the state.
pub(crate) fn record(dirs: &Dirs, additions: Additions) -> Result<Vec<Proposal>> {
    let mut manifest = Manifest::load(&dirs.home)?;
    for (source, tracks) in additions.albums {
        manifest.edit(Edit::AddAlbum {
            source,
            tracks: Some(tracks),
        });
    }
    for (from, to) in additions.renames {
        manifest.edit(Edit::Rename { from, to });
    }
    manifest.save()?;
    State::keep_lookups(&dirs.home, &additions.looked_up, &additions.replaced)?;
    Ok(additions.proposals)
}

/// How proposals are listed.
pub(crate) struct Listing {
    pub mode: Mode,
    pub verbose: bool,
    /// Set on every song a proposal is listed as or added to.
    pub tags: Tags,
    /// Whether loose files may yet be given to songs, and so the tags.
    pub loose: bool,
}

/// List each proposal as a song, or as a source of the song it is the
/// same recording as, and set the tags given on each: whether any song
/// was given them.
pub(crate) fn list<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    proposals: Vec<Proposal>,
    how: &Listing,
    prompter: Option<&mut dyn Prompter>,
    out: &mut W,
) -> Result<bool> {
    let edits = identify::identify(
        runner,
        dirs,
        proposals,
        how.mode,
        how.verbose,
        prompter,
        out,
    )?;
    if edits.is_empty() && !how.tags.is_empty() && !how.loose {
        ui::warning(out, "No song was added, so no tag was set")?;
    }
    let mut manifest = Manifest::load(&dirs.home)?;
    let mut any = false;
    for edit in edits {
        let tagged = match &edit {
            Edit::Add { sources, .. } if !how.tags.is_empty() => manifest::id_of(sources).cloned(),
            _ => None,
        };
        manifest.edit(edit);
        if let Some(key) = tagged {
            any = true;
            manifest.edit(Edit::Tag {
                key,
                tags: how.tags.clone(),
            });
        }
    }
    manifest.save()?;
    Ok(any)
}

/// Song files dropped into the manual folder that no song lists, each a
/// proposal; a listed file moved within the folder is followed instead.
pub(crate) fn dropped_in<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    wait: Duration,
    out: &mut W,
) -> Result<Vec<Proposal>> {
    let mut manifest = Manifest::load(&dirs.home)?;
    let state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    for twin in store.twins() {
        ui::warning(
            out,
            &format!(
                "Not read, as another file in the manual folder has its name once \
                 normalized: {}; rename one of them",
                relpath::show(twin)
            ),
        )?;
    }
    let mut unlisted = Unlisted::of(&manifest, &state, &store, wait);
    let scratch = atomic::Scratch::new()?;
    let hear = crate::reconcile::hearing(runner, scratch.path());
    let moves = crate::reconcile::moved(&manifest, &state, &store, &mut unlisted, hear);
    let Unlisted {
        mut ready,
        settling,
    } = unlisted;
    let unlisted: Vec<SourceKey> = ready
        .iter()
        .filter(|k| state.listed.contains(k))
        .cloned()
        .collect();
    ready.retain(|k| !unlisted.contains(k));
    for key in unlisted {
        ui::info(
            out,
            &format!(
                "Taken out of the song list by hand, so kept out: {key}; \
                 `muman restore` lists it again"
            ),
        )?;
        manifest.edit(Edit::Tombstone {
            note: format!("{key}, taken out of the song list by hand"),
            key,
        });
    }
    for path in settling {
        ui::info(
            out,
            &format!(
                "Still being copied in, left for a later run: {}",
                SourceKey::Manual(path.into())
            ),
        )?;
    }
    for line in moves.moved.keys().filter_map(|k| moves.line(k, false)) {
        ui::info(out, &line)?;
    }
    if !moves.moved.is_empty() {
        manifest.edit(Edit::Moved(moves.moved.clone()));
    }
    manifest.save()?;
    // So the song's plan names its file anew and the song is not written
    // again for a file that only moved.
    if !moves.moved.is_empty() {
        let _lock = atomic::Lock::folder(&dirs.home)?;
        let mut state = State::load(&dirs.home)?;
        state.rekey(&moves.moved)?;
        state.save(&dirs.home)?;
    }
    Ok(ready
        .into_iter()
        .map(|key| Proposal {
            label: key.short(),
            sources: vec![key],
            album: None,
        })
        .collect())
}
