//! The library brought in line with the song list, offline: every listed
//! source measured, the comparisons each song needs made, each song
//! resolved to a plan and the songs fitted into `[library] max_size`
//! when it is set, a song rendered only when its plan changed, and every
//! file muman wrote that no song makes now deleted.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;

use crate::align;
use crate::atomic::Lock;
use crate::clean::Albums;
use crate::dirs::Dirs;
use crate::facts::{self, Facts};
use crate::held::Drift;
use crate::history::Run;
use crate::hooks;
use crate::library::{Ledger, Owned, Relocation};
use crate::limit;
use crate::manifest::{Edit, Manifest, Song};
use crate::naming::{self, Naming};
use crate::parallel;
use crate::progress;
use crate::query::Query;
use crate::render;
use crate::resolve::{self, Input, Plan, Resolved};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{self, Aligned, State, Step, Written};
use crate::store::{self, Located, Store};
use crate::tags::{self, Field};

// Each is a flag of its own on the command line.
#[allow(clippy::struct_excessive_bools)]
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
}

/// Songs left alone for a file not muman's that a run names one by one;
/// a lost state file makes every song such a one.
const NAMED: usize = 10;

/// How long measuring or writing runs between saves of what it did.
const CHECKPOINT: Duration = Duration::from_secs(30);

/// What measuring does besides measuring.
pub struct Measuring<'a> {
    /// Measure again a source that could not be read at this revision.
    pub retry: bool,
    /// Say how many sources are not read again, once a run, by its last
    /// measuring rather than each.
    pub say_unread: bool,
    /// Keep what has been measured so far, as a crash would lose it.
    pub checkpoint: &'a mut dyn FnMut(&State) -> Result<()>,
    /// The analyses of the whole audio a source measured takes.
    pub analyses: Vec<crate::analysis::Kind>,
}

impl std::fmt::Debug for Measuring<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Measuring")
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

/// Measure every source in `keys` the store holds whose facts are not
/// current, into `state`, kept by the checkpoint as it goes and at the
/// end. A file that cannot be read is said so, and is not read again
/// until it changes.
pub fn measure<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    how: &mut Measuring<'_>,
    out: &mut W,
) -> Result<()> {
    let outdated: Vec<Located> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter(|l| {
            !state
                .facts
                .get(&l.key)
                .is_some_and(|f| f.holds_for(&l.rev()))
        })
        .collect();
    let now = state::now_secs();
    let (due, failed): (Vec<Located>, Vec<Located>) = outdated.into_iter().partition(|l| {
        how.retry
            || state
                .failures
                .get(&l.key)
                .is_none_or(|f| f.step != Step::Measure || f.due(Some(&l.rev()), now))
    });
    if how.say_unread && !failed.is_empty() {
        crate::ui::warning(
            out,
            &format!(
                "Not reading {} source(s) that could not be read before; `sync --retry` tries again",
                failed.len()
            ),
        )?;
    }
    let retagged = retag(runner, store, keys, state, scratch, out)?;
    if due.is_empty() {
        if retagged {
            (how.checkpoint)(state)?;
        }
        return Ok(());
    }
    crate::ui::info(out, &format!("Measuring {} source(s)", due.len()))?;
    let step = progress::step("Measuring", Some(due.len() as u64));
    let numbered: Vec<(usize, &Located)> = due.iter().enumerate().collect();
    let mut kept = Instant::now();
    parallel::chunked(
        &numbered,
        parallel::builds(),
        |(n, l)| {
            let _working = step.working(&progress::label(&l.path));
            facts::gather(
                runner,
                l,
                &scratch.join(format!("facts-{n}")),
                &how.analyses,
            )
        },
        |chunk, measured| {
            for ((_, located), result) in chunk.iter().zip(measured) {
                match result {
                    Ok(f) => {
                        warn_of(out, &located.key, &f)?;
                        if let Some(old) = state.facts.get(&located.key).cloned() {
                            state.rebase(&located.key, &old, &f);
                        }
                        state.facts.insert(located.key.clone(), f);
                        state.clear_failure(&located.key);
                    }
                    // No fault of the source's: recorded, it would wait for `--retry`.
                    Err(e) if out_of_space(&e) => {
                        (how.checkpoint)(state)?;
                        return Err(e.context("the disk is full"));
                    }
                    Err(e) => {
                        crate::ui::warning(out, &format!("Could not read {}: {e:#}", located.key))?;
                        state.record_failure(
                            &located.key,
                            Step::Measure,
                            Some(located.rev()),
                            format!("{e:#}"),
                        );
                    }
                }
            }
            if kept.elapsed() >= CHECKPOINT {
                (how.checkpoint)(state)?;
                kept = Instant::now();
            }
            Ok::<(), anyhow::Error>(())
        },
    )?;
    (how.checkpoint)(state)
}

/// What a source's measures hold that its song is worse for, said once
/// when it is measured.
fn warn_of<W: Write>(out: &mut W, key: &SourceKey, f: &crate::facts::Facts) -> Result<()> {
    if let (Some(said), Some(held)) = (f.cut_from, f.duration) {
        crate::ui::warning(
            out,
            &format!(
                "{key} holds {held:.0} s of audio where its header says \
                 {said:.0} s: cut off, as a copy or a download stopped"
            ),
        )?;
    }
    if f.audio.as_ref().is_some_and(|a| a.quality.is_none()) {
        crate::ui::warning(
            out,
            &format!(
                "{key}: no part of its audio measured holds sound to judge, \
                 as silence or a stream that will not decode; it ranks \
                 after every source whose audio does"
            ),
        )?;
    }
    for unkept in &f.unkept {
        crate::ui::warning(out, &unkept_warning(key, *unkept))?;
    }
    Ok(())
}

fn unkept_warning(key: &SourceKey, unkept: crate::facts::Unkept) -> String {
    match unkept {
        crate::facts::Unkept::PreEmphasis => format!(
            "{key}: its cue sheet marks it pre-emphasized, which its song does not keep; \
             it plays bright on a player that is not told to de-emphasize it"
        ),
        crate::facts::Unkept::CueSheet => format!(
            "{key}: a cue sheet beside it splits it into tracks, which muman does not; \
             it is one song"
        ),
    }
}

/// Read again the tags of every source whose measures still hold but
/// whose tags were read another way. Returns whether any were.
fn retag<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    out: &mut W,
) -> Result<bool> {
    let due: Vec<Located> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter(|l| {
            state
                .facts
                .get(&l.key)
                .is_some_and(|f| f.holds_for(&l.rev()) && !f.tags_hold())
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(out, &format!("Reading the tags of {} source(s)", due.len()))?;
    let numbered: Vec<(usize, &Located)> = due.iter().enumerate().collect();
    let read = parallel::map(&numbered, parallel::builds(), |(n, l)| {
        facts::retag(runner, l, &scratch.join(format!("tags-{n}")))
    });
    for (located, result) in due.iter().zip(read) {
        match result {
            Ok((tags, release)) => {
                if let Some(f) = state.facts.get_mut(&located.key) {
                    f.tags = tags;
                    f.release = release;
                    f.tags_method = tags::METHOD.to_string();
                }
            }
            Err(e) => crate::ui::warning(out, &format!("Could not read {}: {e:#}", located.key))?,
        }
    }
    Ok(true)
}

/// Hash every source whose measures still hold but were made before
/// muman hashed sources, decoding nothing, and name its parts by their
/// digests in every plan kept. Returns whether any were.
fn hash<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    out: &mut W,
) -> Result<bool> {
    let due: Vec<Located> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter(|l| {
            state
                .facts
                .get(&l.key)
                .is_some_and(|f| f.holds_for(&l.rev()) && !f.hashed())
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(out, &format!("Hashing {} source(s)", due.len()))?;
    let step = progress::step("Hashing", Some(due.len() as u64));
    let numbered: Vec<(usize, &Located)> = due.iter().enumerate().collect();
    let read = parallel::map(&numbered, parallel::builds(), |(n, l)| {
        let _working = step.working(&progress::label(&l.path));
        facts::hash(runner, l, &scratch.join(format!("hash-{n}")))
    });
    for (located, result) in due.iter().zip(read) {
        let Some(old) = state.facts.get(&located.key).cloned() else {
            continue;
        };
        let mut new = old.clone();
        match result {
            Ok((digests, served)) => new.take(&digests, served),
            Err(e) => {
                crate::ui::warning(out, &format!("Could not hash {}: {e:#}", located.key))?;
                new.hash_method = facts::HASH_METHOD.to_string();
            }
        }
        state.rebase(&located.key, &old, &new);
        state.facts.insert(located.key.clone(), new);
    }
    Ok(true)
}

/// Take the looks of the pictures of every source in `keys` whose
/// measures hold but whose looks were taken another way, decoding only
/// its pictures. Returns whether any were.
pub fn look<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    out: &mut W,
) -> Result<bool> {
    let due: Vec<Located> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter(|l| {
            state
                .facts
                .get(&l.key)
                .is_some_and(|f| f.holds_for(&l.rev()) && !f.pictured())
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(
        out,
        &format!("Hashing the covers of {} source(s)", due.len()),
    )?;
    let step = progress::step("Hashing covers", Some(due.len() as u64));
    let numbered: Vec<(usize, &Located)> = due.iter().enumerate().collect();
    let read = parallel::map(&numbered, parallel::builds(), |(n, l)| {
        let _working = step.working(&progress::label(&l.path));
        facts::look(runner, l, &scratch.join(format!("look-{n}")))
    });
    for (located, result) in due.iter().zip(read) {
        let Some(f) = state.facts.get_mut(&located.key) else {
            continue;
        };
        match result {
            Ok(looks) => f.take_looks(&looks),
            Err(e) => {
                crate::ui::warning(
                    out,
                    &format!("Could not read the covers of {}: {e:#}", located.key),
                )?;
                f.take_looks(&[]);
            }
        }
    }
    Ok(true)
}

/// Analyze, for each of `wanted`, every source whose measures hold but
/// lack it, by one decode of its own rather than measuring it all again.
/// Returns whether any was.
fn analyze<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    wanted: &[crate::analysis::Kind],
    out: &mut W,
) -> Result<bool> {
    let due: Vec<(Located, Vec<crate::analysis::Kind>)> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter_map(|l| {
            let f = state.facts.get(&l.key).filter(|f| f.holds_for(&l.rev()))?;
            let kinds = f.unanalyzed(wanted);
            (!kinds.is_empty()).then_some((l, kinds))
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(out, &format!("Analyzing {} source(s)", due.len()))?;
    let step = progress::step("Analyzing", Some(due.len() as u64));
    let numbered: Vec<(usize, &(Located, Vec<crate::analysis::Kind>))> =
        due.iter().enumerate().collect();
    let facts = &state.facts;
    let read = parallel::map(&numbered, parallel::builds(), |(n, (l, kinds))| {
        let _working = step.working(&progress::label(&l.path));
        let audio = facts.get(&l.key).and_then(|f| f.audio.as_ref())?;
        let scratch = scratch.join(format!("analyze-{n}"));
        Some(facts::analyze(runner, l, audio, kinds, &scratch))
    });
    for ((located, kinds), result) in due.iter().zip(read) {
        let Some(audio) = state
            .facts
            .get_mut(&located.key)
            .and_then(|f| f.audio.as_mut())
        else {
            continue;
        };
        let analysis = match result {
            Some(Ok(a)) => a,
            Some(Err(e)) => {
                crate::ui::warning(out, &format!("Could not analyze {}: {e:#}", located.key))?;
                crate::analysis::Pass::new(kinds.clone()).finish()
            }
            None => continue,
        };
        audio.analysis.merge(analysis);
    }
    Ok(true)
}

/// Analyze the audio of every source in `keys` that `audio` mixes into
/// another layout, as mixed, where it is not already. Returns whether any
/// was.
fn analyze_mixes<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    settings: &crate::settings::Settings,
    out: &mut W,
) -> Result<bool> {
    let wanted = crate::analysis::wanted(&settings.loudness);
    if wanted.is_empty() {
        return Ok(false);
    }
    let due: Vec<(
        Located,
        crate::facts::AudioFacts,
        String,
        crate::codec::Layout,
    )> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter_map(|l| {
            let f = state.facts.get(&l.key).filter(|f| f.holds_for(&l.rev()))?;
            let audio = f.audio.as_ref()?;
            let layout = settings.audio.written(&audio.shape()).0?;
            let rev = f.audio_rev();
            let unmixed = state
                .mix(&l.key, &rev, layout)
                .is_none_or(|a| !a.stale(&wanted).is_empty());
            unmixed.then(|| (l, audio.clone(), rev, layout))
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(out, &format!("Analyzing {} mix(es)", due.len()))?;
    let step = progress::step("Analyzing mixes", Some(due.len() as u64));
    let read = parallel::map(&due, parallel::builds(), |(l, audio, _, layout)| {
        let _working = step.working(&progress::label(&l.path));
        facts::analyze_mix(runner, l, audio, crate::codec::mix_filter(*layout), &wanted)
    });
    for ((located, _, rev, layout), analysis) in due.into_iter().zip(read) {
        state.record_mix(state::Mix {
            source: located.key,
            audio: rev,
            layout,
            analysis,
        });
    }
    Ok(true)
}

/// Make every comparison the songs' resolving may ask for that is not
/// made already at the sources' revisions. Returns whether any was.
fn compare<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    songs: &[Song],
    state: &mut State,
    out: &mut W,
) -> Result<bool> {
    let rev = |k: &SourceKey| state.facts.get(k).map(Facts::audio_rev);
    let due: Vec<(SourceKey, SourceKey, (String, String))> = songs
        .iter()
        .flat_map(|s| resolve::wanted_alignments(s, &state.facts))
        .filter(|(a, b)| store.has(a) && store.has(b))
        .filter_map(|(a, b)| {
            let revs = (rev(&a)?, rev(&b)?);
            state
                .alignment(&a, &b, &revs)
                .is_none()
                .then_some((a, b, revs))
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    let keys: Vec<SourceKey> = due
        .iter()
        .flat_map(|(a, b, _)| [a.clone(), b.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    // The progress counts recordings decoded, each once for all its pairs.
    crate::ui::info(
        out,
        &format!(
            "Comparing {} pair(s) of {} recording(s)",
            due.len().div_ceil(2),
            keys.len()
        ),
    )?;
    let step = progress::step("Comparing", Some(keys.len() as u64));
    let envelopes: HashMap<SourceKey, Vec<f64>> = keys
        .iter()
        .zip(parallel::map(&keys, parallel::builds(), |k| {
            let path = store.locate(k)?.path;
            let _working = step.working(&progress::label(&path));
            runner
                .output(&align::pcm_command(&path))
                .ok()
                .map(|pcm| align::envelope(&pcm))
        }))
        .filter_map(|(k, e)| Some((k.clone(), e?)))
        .collect();
    let compared = parallel::map(&due, parallel::builds(), |(a, b, revs)| {
        let (ea, eb) = (envelopes.get(a)?, envelopes.get(b)?);
        let found = align::align(ea, eb).unwrap_or(align::Alignment {
            offset_ms: 0,
            stretch_ppm: 0,
            score: 0.0,
            coverage: 0.0,
        });
        let ms = |e: &[f64]| i64::try_from(e.len()).unwrap_or(i64::MAX) * 10;
        Some(Aligned {
            a: a.clone(),
            b: b.clone(),
            method: align::METHOD.to_string(),
            revs: revs.clone(),
            offset_ms: found.offset_ms,
            stretch_ppm: found.stretch_ppm,
            score: found.score,
            coverage: found.coverage,
            a_ms: ms(ea),
            b_ms: ms(eb),
            a_extra_ms: align::audible_outside(ea, eb.len(), &found),
        })
    });
    for aligned in compared.into_iter().flatten() {
        state.record_alignment(aligned);
    }
    Ok(true)
}

/// How a song is named in messages: its title and artist as resolved,
/// else its first source.
pub(crate) fn name_of(
    song: &Song,
    resolved: Option<&Resolved>,
    facts: &BTreeMap<SourceKey, Facts>,
) -> String {
    let from_tags = |tags: &[(String, Vec<String>)]| {
        let get = |f: Field| {
            tags.iter()
                .find(|(k, _)| k == f.vorbis())
                .and_then(|(_, v)| v.first())
                .cloned()
        };
        get(Field::Title).map(|t| match get(Field::Artist) {
            Some(a) => format!("{t} — {a}"),
            None => t,
        })
    };
    resolved
        .and_then(|r| from_tags(&r.plan.tags))
        .or_else(|| {
            let first = song.sources.first()?;
            let offers = &facts.get(first)?.tags;
            let title = offers.get(&Field::Title)?.values.first()?.clone();
            Some(title)
        })
        .or_else(|| song.sources.first().map(ToString::to_string))
        .unwrap_or_else(|| "a song with no sources".to_string())
}

/// What differs between two plans, in words.
fn changes(old: &Plan, new: &Plan) -> Vec<&'static str> {
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
    if old.loudness != new.loudness
        || old.tags.len() - other_tags(old).len() != new.tags.len() - other_tags(new).len()
    {
        what.push("loudness");
    }
    what
}

pub(crate) fn path_of(r: &Resolved) -> PathBuf {
    let mut name = r.stem.as_os_str().to_os_string();
    name.push(".");
    name.push(r.plan.format.extension());
    PathBuf::from(name)
}

/// Give every song a path of its own: a later song resolving to one an
/// earlier song took is named with its audio's ID too, and numbered
/// when that is taken as well, as two manual files of one name are. The
/// names are the song list's alone, never how the library came to be,
/// so one list builds one library.
fn separate(planned: &mut [(usize, Resolved)], naming: &Naming, max_name: usize) {
    // By stem, not path: a FLAC song and an Opus song of one name would
    // write the same `.lrc`.
    let lower = |r: &Resolved| crate::relpath::folded(&r.stem);
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for (_, r) in planned.iter_mut() {
        if taken.insert(lower(r)) {
            continue;
        }
        let base = r.stem.clone();
        // A manual file's name may hold what a path must not.
        let id = naming.value(&r.plan.audio.key.short());
        for n in 1.. {
            r.stem = naming::suffixed(&base, &id, n, max_name);
            if taken.insert(lower(r)) {
                break;
            }
        }
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
#[allow(clippy::too_many_lines)]
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
    if crate::platform::within(&dirs.library, &home.join("sources"))
        || crate::platform::same_path(&dirs.library, home)
    {
        anyhow::bail!(
            "the library {} is in the home's sources, where muman would read the songs it \
             writes back as new ones; choose a folder outside them",
            dirs.library.display()
        );
    }
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
    }
    let listed = manifest.keys();
    if compare(runner, &store, &manifest.songs, &mut state, out)? {
        persist(home, &state, opts.dry_run, &lock)?;
    }

    let on_disk: BTreeSet<SourceKey> = listed.iter().filter(|k| store.has(k)).cloned().collect();
    let (mut planned, mut failed) = plan(&manifest, &state, &dirs.library, Some(&on_disk), out)?;
    let mut ok = failed.is_empty();
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
    let files = files_of(&planned, &manifest, &state.outputs);
    let drifts = crate::held::drifts(&manifest, &state.facts);
    let (taken, kept): (Vec<&Drift>, Vec<&Drift>) =
        drifts.iter().partition(|d| opts.accept || d.followed());
    let holding: BTreeSet<usize> = kept.iter().map(|d| d.song).collect();
    let decided = decide(
        &planned,
        &files,
        &state.outputs,
        &dirs.library,
        opts.force,
        &holding,
    );
    say_drifts(
        &taken,
        &kept,
        &planned,
        &decided,
        (opts.accept, opts.force),
        out,
    )?;

    if opts.dry_run {
        let shown = Shown {
            manifest: &manifest,
            planned: &planned,
            decided: &decided,
            files: &files,
            old: &state.outputs,
            state: &state,
            failed: &failed,
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
    let mut moves: Vec<(usize, PathBuf, PathBuf)> = planned
        .iter()
        .zip(&decided)
        .filter_map(|((_, r), d)| match d {
            Doing::Move { from, order, .. } => Some((*order, from.clone(), path_of(r))),
            _ => None,
        })
        .collect();
    moves.sort_by_key(|(order, ..)| *order);
    let moves = moves.into_iter().map(|(_, from, to)| (from, to)).collect();
    let mut ledger = Ledger::new(&dirs.library, home, run.as_deref_mut());
    let arrived = relocate(moves, &mut state, &mut ledger, out)?;
    if !arrived.is_empty() {
        // The files are already at their new paths; a failure before the
        // end of the run must not leave the state naming the old ones.
        state.save(home)?;
        ledger.checkpoint()?;
    }
    // A song moved is kept, or has its tags written, at its new path; one
    // whose move failed is written there.
    let doing: Vec<Doing> = planned
        .iter()
        .zip(decided)
        .map(|((_, r), d)| match d {
            Doing::Move { retag, .. } if arrived.contains(&path_of(r)) => {
                if retag {
                    Doing::Retag
                } else {
                    Doing::Keep
                }
            }
            Doing::Move { .. }
                if changed_since_written(&dirs.library, &state.outputs, &path_of(r)) =>
            {
                Doing::Guard { file: path_of(r) }
            }
            Doing::Move { .. } => Doing::Write,
            d => d,
        })
        .collect();
    let old = state.outputs.clone();
    let due: Vec<(&PlannedSong, bool)> = planned
        .iter()
        .zip(&doing)
        .filter_map(|(song, d)| match d {
            Doing::Write => Some((song, false)),
            Doing::Retag => Some((song, true)),
            _ => None,
        })
        .collect();
    // Owned before written, and no longer vouched for: a crash between the
    // two must leave neither a file no run would delete nor a rewritten
    // one that looks changed by someone else.
    let mut pending = (!due.is_empty()).then(|| state.clone());
    if let Some(pending) = pending.as_mut() {
        for ((n, r), _) in &due {
            let path = path_of(r);
            let lyrics = Some(path.with_extension("lrc"));
            pending.outputs.insert(
                path,
                Written {
                    sources: manifest.songs[*n].sources.clone(),
                    lyrics,
                    plan: None,
                    stamp: None,
                    digest: None,
                },
            );
        }
        pending.save(home)?;
    }
    let foreign = |r: &Resolved| foreign(&dirs.library, &old, &path_of(r));
    let mut replaced = Vec::new();
    for ((_, r), _) in &due {
        let path = path_of(r);
        if let Some(written) = old.get(&path) {
            replaced.extend(ledger.owned(&old, &path, true));
            if let Some(lyrics) = &written.lyrics {
                replaced.extend(ledger.owned(&old, lyrics, true));
            }
        }
        // Written over only when forced, and then kept to put back.
        replaced.extend(foreign(r).iter().map(|f| Ledger::taken(f)));
    }
    ledger.replacing(&replaced)?;
    if due.len() > 1 {
        crate::ui::info(out, &format!("Writing {} song(s)", due.len()))?;
    }

    let mut outputs: BTreeMap<PathBuf, Written> = BTreeMap::new();
    let mut unowned = Vec::new();
    for ((n, r), d) in planned.iter().zip(&doing) {
        let kept = match d {
            Doing::Keep => Some(path_of(r)),
            Doing::Guard { file } => {
                crate::ui::warning(
                    out,
                    &format!(
                        "Left alone, changed since muman wrote it: {} (`sync --force` writes it again)",
                        crate::relpath::show(file)
                    ),
                )?;
                Some(file.clone())
            }
            Doing::Unowned => {
                unowned.push(path_of(r));
                None
            }
            Doing::Held { file } => Some(file.clone()),
            Doing::Blocked { file } => {
                crate::ui::warning(
                    out,
                    &format!(
                        "Left at {}: {} is not muman's (`sync --force` writes over it)",
                        crate::relpath::show(file),
                        crate::relpath::show(&path_of(r))
                    ),
                )?;
                Some(file.clone())
            }
            _ => None,
        };
        if let Some(written) = kept.and_then(|p| Some((old.get(&p)?.clone(), p))) {
            outputs.insert(
                written.1,
                Written {
                    sources: manifest.songs[*n].sources.clone(),
                    ..written.0
                },
            );
        }
    }
    for path in unowned.iter().take(NAMED) {
        crate::ui::warning(
            out,
            &format!(
                "Left alone, not muman's: {} (`sync --force` writes over it, and `undo` puts it back)",
                crate::relpath::show(path)
            ),
        )?;
    }
    if unowned.len() > NAMED {
        crate::ui::warning(
            out,
            &format!(
                "… and {} more not muman's, left alone",
                unowned.len() - NAMED
            ),
        )?;
    }
    let mut changed = false;
    let mut written = 0_usize;
    let mut learned = Vec::new();
    let every = opts.checkpoint.unwrap_or(CHECKPOINT);
    let mut kept_at = Instant::now();
    let writing = progress::step("Writing", Some(due.len() as u64));
    let mut failed_at = Vec::new();
    let render_one = |((n, r), retag): &(&PlannedSong, bool)| {
        let _working = writing.working(&progress::label(&r.stem));
        if let Some((folder, made)) = placed.get(n) {
            return render::place(folder, made, &dirs.library);
        }
        if *retag {
            let path = path_of(r);
            let lyrics = old.get(&path).and_then(|w| w.lyrics.as_deref());
            return render::retag(&dirs.library, &path, lyrics, &r.plan);
        }
        render::render(
            runner,
            &render::Job {
                plan: &r.plan,
                stem: &r.stem,
                library: &dirs.library,
                sources: &located,
                scratch: &temp.path().join(format!("render-{n}")),
            },
        )
    };
    parallel::chunked(&due, parallel::builds(), render_one, |chunk, rendered| {
        for (((n, r), _), result) in chunk.iter().zip(rendered) {
            let song = &manifest.songs[*n];
            let name = name_of(song, Some(r), &state.facts);
            match result {
                Ok(done) => {
                    changed = true;
                    // A new format writes the song beside its old file, under
                    // another extension.
                    let stem = done.audio.with_extension("");
                    let before = old.get(&done.audio).or_else(|| {
                        old.iter()
                            .find(|(p, w)| {
                                p.with_extension("") == stem && w.sources == song.sources
                            })
                            .map(|(_, w)| w)
                    });
                    let changed = before
                        .and_then(|w| w.plan.as_ref())
                        .map(|p| changes(p, &r.plan));
                    // A song forced, or one a stopped run had begun, is
                    // neither new nor changed.
                    let verb = match (before, changed) {
                        (_, Some(what)) if !what.is_empty() => {
                            format!("Updated ({})", what.join(", "))
                        }
                        (Some(_), _) => "Written again".to_string(),
                        (None, _) => "Added".to_string(),
                    };
                    crate::ui::success(
                        out,
                        &format!("{verb}: {}", crate::relpath::show(&done.audio)),
                    )?;
                    for problem in &done.problems {
                        crate::ui::warning(out, &format!("  {problem}"))?;
                    }
                    written += 1;
                    // Before the stamp, so a hook that tags the file further
                    // does not make it look changed by something else.
                    let values = hooks::written_values(&dirs.library, &done.audio);
                    let values: Vec<(&str, &str)> =
                        values.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    hooks::run(runner, &manifest.hooks, hooks::Event::Written, &values, out)?;
                    if let Some(f) = &fitted {
                        let measured = state::Measured {
                            source: r.plan.audio.key.clone(),
                            audio: done.audio_bytes,
                            lyrics: done.lyrics_bytes,
                        };
                        learned.push((limit::plan_key(&f.tools, &r.plan), measured));
                    }
                    let vouched = Written {
                        sources: song.sources.clone(),
                        lyrics: done.lyrics,
                        plan: Some(r.plan.clone()),
                        stamp: store::stamp_text(&dirs.library.join(&done.audio)),
                        digest: crate::facts::digest_file(&dirs.library.join(&done.audio)),
                    };
                    if let Some(pending) = pending.as_mut() {
                        pending.outputs.insert(done.audio.clone(), vouched.clone());
                        if kept_at.elapsed() >= every {
                            pending.save(home)?;
                            ledger.checkpoint()?;
                            kept_at = Instant::now();
                        }
                    }
                    outputs.insert(done.audio.clone(), vouched);
                }
                Err(e) => {
                    ok = false;
                    failed.extend(song.sources.iter().cloned());
                    failed_at.push(path_of(r));
                    crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    crate::library::remove_empty_folders(&dirs.library, &failed_at);
    let removed = prune(&mut ledger, &old, &mut outputs, &failed, opts.force, out)?;
    let up_to_date = doing.iter().filter(|d| **d == Doing::Keep).count();
    if up_to_date > 0 {
        crate::ui::info(out, &format!("Up to date: {up_to_date} song(s)"))?;
    }

    state.outputs = outputs;
    state.sizes.extend(learned);
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
    for edit in crate::held::records(&manifest, &state.facts, &taken) {
        manifest.edit(edit);
    }
    manifest.save_locked(&lock)?;
    if let Some(run) = run {
        run.saw(&manifest);
    }
    if changed || !removed.is_empty() {
        if manifest.settings.library.touch_root {
            touch(&dirs.library, out)?;
        }
        let (written, removed) = (written.to_string(), removed.len().to_string());
        let library = dirs.library.to_string_lossy();
        let values = [
            ("library", library.as_ref()),
            ("written", written.as_str()),
            ("removed", removed.as_str()),
        ];
        hooks::run(runner, &manifest.hooks, hooks::Event::Changed, &values, out)?;
    }
    Ok(ok)
}

/// The audio stream of a source to measure, and where the source is.
struct Sizing(u32, Located);

/// Measure the audio bytes of each planned song's source that a copy
/// would carry and whose facts lack them, for fitting. Returns whether
/// any were.
fn sizes<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    planned: &Planned,
    state: &mut State,
    scratch: &Path,
    out: &mut W,
) -> Result<bool> {
    let due: BTreeMap<SourceKey, Sizing> = planned
        .iter()
        .filter(|(_, r)| {
            matches!(
                r.plan.format,
                resolve::Format::Copy { .. } | resolve::Format::Encode { kbps: None, .. }
            )
        })
        .filter(|(_, r)| {
            state
                .facts
                .get(&r.plan.audio.key)
                .and_then(|f| f.audio.as_ref())
                .is_some_and(|a| a.bytes.is_none())
        })
        .filter_map(|(_, r)| {
            let located = store.locate(&r.plan.audio.key)?;
            Some((
                r.plan.audio.key.clone(),
                Sizing(r.plan.audio.index, located),
            ))
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(
        out,
        &format!("Measuring the audio size of {} source(s)", due.len()),
    )?;
    let due: Vec<(usize, (SourceKey, Sizing))> = due.into_iter().enumerate().collect();
    let measured = parallel::map(
        &due,
        parallel::builds(),
        |(n, (_, Sizing(index, located)))| {
            facts::audio_bytes(runner, located, *index, &scratch.join(format!("size-{n}"))).ok()
        },
    );
    for ((_, (key, _)), bytes) in due.iter().zip(measured) {
        if let Some(audio) = state.facts.get_mut(key).and_then(|f| f.audio.as_mut()) {
            audio.bytes = bytes;
        }
    }
    Ok(true)
}

/// Say which songs were left out for want of room, and what the library
/// would need.
fn left_out<W: Write>(
    manifest: &Manifest,
    state: &State,
    fitted: &limit::Fitted,
    out: &mut W,
) -> Result<()> {
    const NAMED: usize = 10;
    let names: Vec<String> = fitted
        .left_out
        .iter()
        .take(NAMED)
        .map(|n| name_of(&manifest.songs[*n], None, &state.facts))
        .collect();
    let more = fitted.left_out.len().saturating_sub(NAMED);
    crate::ui::error(
        out,
        &format!(
            "max_size {} cannot hold the library: at the lowest bitrates it takes {}; left out: {}{} (raise [library] max_size or lower [audio] min_bitrate)",
            crate::ui::bytes(fitted.max),
            crate::ui::bytes(fitted.floor.unwrap_or(0)),
            names.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ),
    )?;
    Ok(())
}

/// Say how full the library is projected to be against its limit.
fn fit_summary<W: Write>(fitted: &limit::Fitted, out: &mut W) -> Result<()> {
    crate::ui::info(
        out,
        &format!(
            "Library: {} of {} (max_size); {} song(s) below their best format",
            crate::ui::bytes(fitted.projected),
            crate::ui::bytes(fitted.max),
            fitted.lowered
        ),
    )?;
    Ok(())
}

/// Whether a file muman wrote has changed since, by its size and
/// time: a tagger or player that rewrote it.
/// What `old` records of the song's file, when that file is the song as
/// it would be written now: its plan, its file there and not empty, its
/// lyrics there.
fn current<'a>(
    library: &Path,
    old: &'a BTreeMap<PathBuf, Written>,
    r: &Resolved,
) -> Option<&'a Written> {
    let path = path_of(r);
    let written = old.get(&path)?;
    let present = library.join(&path).metadata().is_ok_and(|m| m.len() > 0)
        && written
            .lyrics
            .as_ref()
            .is_none_or(|l| library.join(l).exists());
    (present && written.plan.as_ref() == Some(&r.plan)).then_some(written)
}

/// The files at a song's path, its audio or its lyrics, that muman did
/// not write: a file of the user's own a sync must not write over. A file
/// that is one muman wrote under a name differing only in case, as a
/// filesystem blind to case reports, is muman's.
fn foreign(library: &Path, old: &BTreeMap<PathBuf, Written>, path: &Path) -> Vec<PathBuf> {
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

/// Whether `e` is a disk full, from muman's own writes or from ffmpeg's,
/// which reports it only in words.
fn out_of_space(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull)
            || cause.to_string().contains("No space left on device")
    })
}

pub(crate) fn changed_since_written(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    path: &Path,
) -> bool {
    old.get(path)
        .and_then(|w| w.stamp.as_ref())
        .zip(store::stamp_text(&library.join(path)))
        .is_some_and(|(was, now)| *was != now)
}

/// Which file the state records is each song's, one file to one song:
/// the one made from the most of the song's own sources, those no other
/// song lists, as the state recorded them when it was written. So a song
/// whose path, plan or sources changed still has its file, and every
/// decision about it, to keep, move, guard, write or delete, is made of
/// that file; a song no file was made from has none. Ties go to the song
/// listed first, then to the path that sorts first.
pub(crate) fn files_of(
    planned: &[PlannedSong],
    manifest: &Manifest,
    old: &BTreeMap<PathBuf, Written>,
) -> BTreeMap<usize, PathBuf> {
    let own = |keys: &[SourceKey]| -> BTreeSet<SourceKey> {
        keys.iter()
            .filter(|k| !crate::manifest::shareable(k))
            .cloned()
            .collect()
    };
    let mut scored: Vec<(usize, usize, &PathBuf)> = Vec::new();
    for (n, _) in planned {
        let mine = own(&manifest.songs[*n].sources);
        for (path, written) in old {
            let shared = own(&written.sources).intersection(&mine).count();
            if shared > 0 {
                scored.push((shared, *n, path));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
    let mut files = BTreeMap::new();
    let mut taken: BTreeSet<&PathBuf> = BTreeSet::new();
    for (_, n, path) in scored {
        if !files.contains_key(&n) && taken.insert(path) {
            files.insert(n, path.clone());
        }
    }
    files
}

/// Whether two plans make the same audio, cover and lyrics: a file of
/// one is a file of the other with its tags written again.
fn media_equal(a: &Plan, b: &Plan) -> bool {
    a.version == b.version
        && a.format == b.format
        && a.chain() == b.chain()
        // A retag sets an Opus header's gain, but only a write puts back the one it had.
        && (a.opus_header().is_none() || b.opus_header().is_some())
        && a.audio == b.audio
        && a.cover == b.cover
        && a.lyrics == b.lyrics
}

/// What a sync does with one song, decided for every song before any is
/// done, so a sync does what `status` says it would.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Doing {
    /// Its file is as it would be written.
    Keep,
    /// Its file moves to the song's path, its tags written again when
    /// they changed; moves are made by `order`, as a move into a path
    /// another move leaves runs after it.
    Move {
        from: PathBuf,
        retag: bool,
        order: usize,
    },
    /// Only its tags changed: they are written into its file.
    Retag,
    /// Written from its sources.
    Write,
    /// Left alone: its file, changed since muman wrote it.
    Guard { file: PathBuf },
    /// Left alone: a file not muman's is at its path.
    Unowned,
    /// Left where it is: a file not muman's is at the song's new path.
    Blocked { file: PathBuf },
    /// Left as built: a source it was built from was fetched again and
    /// holds otherwise, not yet accepted.
    Held { file: PathBuf },
}

/// Say what each source holding otherwise than its song was built
/// from does to the song: followed or taken, or left as built while its
/// file is there.
fn say_drifts<W: Write>(
    taken: &[&Drift],
    kept: &[&Drift],
    planned: &[PlannedSong],
    decided: &[Doing],
    (accept, force): (bool, bool),
    out: &mut W,
) -> Result<()> {
    for d in taken {
        let how = if accept && !d.followed() {
            "taken, as `--accept` asks"
        } else {
            "its song follows it, a file of your own"
        };
        crate::ui::info(out, &format!("{}; {how}", d.show()))?;
    }
    for d in kept {
        let doing = planned
            .iter()
            .zip(decided)
            .find(|((n, _), _)| *n == d.song)
            .map(|(_, doing)| doing);
        let what = match doing {
            Some(Doing::Held { .. } | Doing::Keep) => {
                "its song is left as built; `sync --accept` takes what it holds now"
            }
            Some(_) if force => {
                "its song is written from it, as `--force` asks; `sync --accept` records it"
            }
            Some(_) => {
                "its song is built from it, no file built before being left; \
                 `sync --accept` records it"
            }
            None => "`sync --accept` takes it",
        };
        crate::ui::warning(out, &format!("{}; {what}", d.show()))?;
    }
    Ok(())
}

/// Each of `planned` kept as it is, decided before any moves: its file
/// as it would be written, or one built before from what a source in
/// `holding` no longer holds.
fn settled(
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    old: &BTreeMap<PathBuf, Written>,
    library: &Path,
    holding: &BTreeSet<usize>,
) -> Vec<Option<Doing>> {
    planned
        .iter()
        .map(|(n, r)| {
            if current(library, old, r).is_some() {
                return Some(Doing::Keep);
            }
            files
                .get(n)
                .filter(|f| holding.contains(n) && library.join(f).exists())
                .map(|f| Doing::Held { file: f.clone() })
        })
        .collect()
}

/// What a sync does with each of `planned`, by its file in `files`, as
/// [`settled`] first keeps or holds it:
///
/// | The song's file | Done |
/// |---|---|
/// | At its path, as it would be written | Kept |
/// | Built from what a source in `holding` no longer holds | Left as built until `--accept` |
/// | Elsewhere, of the same media, its path free | Moved, then tags written if they changed |
/// | Changed since written, and not merely moving | Left alone until `--force` |
/// | Any, where a changed file is at its path | Left alone until `--force` |
/// | Elsewhere, and a file not muman's at its path | Left where it is until `--force` |
/// | None, and a file not muman's at its path | Left alone until `--force` |
/// | At its path, of the same media | Its tags written |
/// | Otherwise, or `force` | Written |
///
/// A path is free when no file of anyone's is there, folded as NTFS and
/// APFS fold names, or only a song's file that moves away first, as when
/// two songs of one name become two of two names and the one told apart
/// takes the plain name; a name changing only its case is free to its
/// own file. Moves are listed in the order they can be made;
/// two songs swapping paths are written instead. A changed file moves
/// only when its tags need no writing, so nothing a tagger wrote is
/// written over.
fn decide(
    planned: &[PlannedSong],
    files: &BTreeMap<usize, PathBuf>,
    old: &BTreeMap<PathBuf, Written>,
    library: &Path,
    force: bool,
    holding: &BTreeSet<usize>,
) -> Vec<Doing> {
    use crate::relpath::folded;
    if force {
        return vec![Doing::Write; planned.len()];
    }
    let mut decided = settled(planned, files, old, library, holding);
    // Each song that could move, by what it would take and leave.
    let mut moving: Vec<(usize, PathBuf, PathBuf, bool)> = Vec::new();
    for (i, (n, r)) in planned.iter().enumerate() {
        if decided[i].is_some() {
            continue;
        }
        let path = path_of(r);
        let Some(from) = files.get(n).filter(|f| **f != path) else {
            continue;
        };
        let was = &old[from];
        let present = library.join(from).metadata().is_ok_and(|m| m.len() > 0)
            && was.lyrics.as_ref().is_none_or(|l| library.join(l).exists());
        let same_media = present && was.plan.as_ref().is_some_and(|p| media_equal(p, &r.plan));
        let retag = was.plan.as_ref() != Some(&r.plan);
        let changed = changed_since_written(library, old, from);
        if same_media && !(changed && retag) {
            moving.push((i, from.clone(), path, retag));
        }
    }
    // Made in rounds: a move lands only where nothing is, or where a move
    // already chosen leaves, so the order chosen is an order they can run.
    let lyrics_of = |p: &Path| p.with_extension("lrc");
    let recorded: BTreeSet<&Path> = old
        .iter()
        .flat_map(|(p, w)| std::iter::once(p.as_path()).chain(w.lyrics.as_deref()))
        .collect();
    let mut held: BTreeSet<String> = recorded.iter().map(|p| folded(p)).collect();
    let free = |held: &BTreeSet<String>, p: &Path| {
        !held.contains(&folded(p)) && (recorded.contains(p) || !library.join(p).exists())
    };
    let mut order = 0;
    loop {
        let before = moving.len();
        moving.retain(|(i, from, to, retag)| {
            let lyrics = old[from].lyrics.as_deref();
            let fits = |from: &Path, to: &Path| own_place(library, from, to) || free(&held, to);
            if !fits(from, to) || lyrics.is_some_and(|l| !fits(l, &lyrics_of(to))) {
                return true;
            }
            held.remove(&folded(from));
            held.insert(folded(to));
            if let Some(l) = lyrics {
                held.remove(&folded(l));
                held.insert(folded(&lyrics_of(to)));
            }
            decided[*i] = Some(Doing::Move {
                from: from.clone(),
                retag: *retag,
                order,
            });
            order += 1;
            false
        });
        if moving.len() == before {
            break;
        }
    }
    planned
        .iter()
        .zip(decided)
        .map(|((n, r), d)| {
            if let Some(d) = d {
                return d;
            }
            let path = path_of(r);
            let file = files.get(n);
            let was = file.and_then(|f| old.get(f));
            let present = file.is_some_and(|f| {
                library.join(f).metadata().is_ok_and(|m| m.len() > 0)
                    && was
                        .and_then(|w| w.lyrics.as_ref())
                        .is_none_or(|l| library.join(l).exists())
            });
            let same_media = present
                && was
                    .and_then(|w| w.plan.as_ref())
                    .is_some_and(|p| media_equal(p, &r.plan));
            if let Some(file) = file.filter(|f| changed_since_written(library, old, f)) {
                return Doing::Guard { file: file.clone() };
            }
            // Whosever it is, a changed file at the path is not written over.
            if changed_since_written(library, old, &path) {
                return Doing::Guard { file: path };
            }
            if !foreign(library, old, &path).is_empty() {
                // Its own file stays where it is rather than go for nothing.
                return match file.filter(|_| present) {
                    Some(file) => Doing::Blocked { file: file.clone() },
                    None => Doing::Unowned,
                };
            }
            if file == Some(&path) && same_media {
                return Doing::Retag;
            }
            Doing::Write
        })
        .collect()
}

/// Whether moving `from` to `to` only changes the case of its name, and
/// no other file is under the new spelling: then it takes its own place.
fn own_place(library: &Path, from: &Path, to: &Path) -> bool {
    use crate::relpath::folded;
    folded(from) == folded(to)
        && (!library.join(to).exists()
            || same_file::is_same_file(library.join(from), library.join(to)).unwrap_or(false))
}

/// Make the `moves` [`decide`] chose, each a song's file to its new path,
/// as a new `[library]` template makes them, rather than render them
/// again. A file changed since it was written moves too, still changed:
/// written again at the new path, it would be left at the old one, a
/// second copy. The ledger records each move before it is made. Returns
/// the paths moved to.
fn relocate<W: Write>(
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

/// Each song's place in the song list, and what it resolved to.
pub(crate) type Planned = Vec<(usize, Resolved)>;

/// One song of [`Planned`].
type PlannedSong = (usize, Resolved);

/// Resolve every song, saying which could not be and returning their
/// sources with the plans, each song with a path of its own.
pub(crate) fn plan<W: Write>(
    manifest: &Manifest,
    state: &State,
    library: &Path,
    on_disk: Option<&BTreeSet<SourceKey>>,
    out: &mut W,
) -> Result<(Planned, BTreeSet<SourceKey>)> {
    let naming = Naming::new(&manifest.settings.library, library)?;
    let mut failed = BTreeSet::new();
    let mut planned = Vec::new();
    for key in &manifest.clean.unknown {
        crate::ui::warning(
            out,
            &format!("No cleaning rule is named `{key}` in the song list; it does nothing"),
        )?;
    }
    for line in crate::migrate::notices(&manifest.renamed, manifest.respelled_notice()) {
        crate::ui::info(out, &line)?;
    }
    for line in crate::settings::stale_warnings(&manifest.stale_defaults) {
        crate::ui::warning(out, &line)?;
    }
    let albums = Albums::of(state.facts.values().map(|f| &f.tags));
    for (n, song) in manifest.songs.iter().enumerate() {
        let input = Input {
            song,
            album: song.album.as_ref().and_then(|a| manifest.album(a)),
            facts: &state.facts,
            on_disk,
            alignments: &state.alignments,
            lyrics: &manifest.lyrics,
            clean: &manifest.clean,
            albums: &albums,
            naming: &naming,
            audio: &manifest.settings.audio,
            quality: &manifest.settings.quality,
            placement: manifest.settings.library.lyrics,
        };
        match resolve::resolve(&input) {
            Ok(r) => planned.push((n, r)),
            Err(e) => {
                failed.extend(song.sources.iter().cloned());
                let name = name_of(song, None, &state.facts);
                crate::ui::error(out, &format!("Failed: {name}: {e:#}"))?;
            }
        }
    }
    if let Some(max) = manifest.settings.library.max_path {
        let over = planned
            .iter()
            .filter(|(_, r)| library.join(path_of(r)).to_string_lossy().chars().count() > max)
            .count();
        if over > 0 {
            crate::ui::warning(
                out,
                &format!(
                    "{over} song(s) take more than [library] max_path = {max} characters even \
                     with their names cut short: the library folder, {}, takes {} itself",
                    library.display(),
                    library.to_string_lossy().chars().count()
                ),
            )?;
        }
    }
    crate::loudness::settle(&mut planned, state, &manifest.settings);
    separate(
        &mut planned,
        &naming,
        manifest.settings.library.max_name_bytes,
    );
    Ok((planned, failed))
}

/// Take each output whose size or time changed since it was written,
/// but whose bytes are the ones written, as muman's still: a backup put
/// back or a copy made without its times. Only those are read.
pub(crate) fn vouch_for_copies(library: &Path, state: &mut State) {
    for (path, written) in &mut state.outputs {
        let file = library.join(path);
        let (Some(was), Some(digest)) = (&written.stamp, &written.digest) else {
            continue;
        };
        let Some(now) = store::stamp_text(&file).filter(|now| now != was) else {
            continue;
        };
        if crate::facts::digest_file(&file).as_ref() == Some(digest) {
            written.stamp = Some(now);
        }
    }
}

/// For a dry run, the manual files the next sync will follow to where
/// they moved, said and followed here in memory, so the run reports on
/// the songs as that sync will find them.
fn foresee_moves<W: Write>(
    manifest: &mut Manifest,
    state: &mut State,
    store: &Store,
    settling: Duration,
    out: &mut W,
) -> Result<()> {
    let listed = manifest.keys();
    let mut known = listed.clone();
    known.extend(manifest.removed_keys());
    let (mut ready, _) = store.unlisted(&known, SystemTime::now(), settling);
    let moved = store::moved_manual(&listed, state, store, &mut ready);
    for (from, to) in &moved {
        crate::ui::info(
            out,
            &format!("Moved, followed by the next sync: {from} is now {to}"),
        )?;
        for song in &mut manifest.songs {
            song.rename(from, to);
        }
    }
    if !moved.is_empty() {
        state.rekey(&moved)?;
    }
    Ok(())
}

/// Take `library` as the folder the outputs are in; the files written
/// into another before are no longer muman's to delete.
fn adopt<W: Write>(state: &mut State, library: &Path, out: &mut W) -> Result<()> {
    if library.exists() && !library.is_dir() {
        anyhow::bail!("the library {} is a file, not a folder", library.display());
    }
    if let Some(before) = state
        .library
        .as_deref()
        .filter(|l| !crate::platform::same_path(l, library))
    {
        crate::ui::warning(
            out,
            &format!(
                "The library moved from {}; the files written there are left alone",
                before.display()
            ),
        )?;
        state.outputs.clear();
    }
    state.library = Some(library.to_path_buf());
    Ok(())
}

/// What becomes of a file `old` lists that the outputs do not.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pruned {
    /// Deleted, with its lyrics unless another output takes them.
    Removed(Vec<PathBuf>),
    /// Kept, as a song it came from failed this run.
    KeptForFailed,
    /// Left in place and no longer muman's: changed since written.
    LeftChanged,
}

/// What pruning `old` against `outputs` does to each file, deciding
/// alone, for a sync to carry out and a dry run to show. A `forced` run
/// wrote every song again, so a changed file of a song now written
/// elsewhere goes, kept for `undo` as any file written over is.
fn prune_plan(
    library: &Path,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    forced: bool,
) -> Vec<(PathBuf, Pruned)> {
    let now: BTreeMap<String, PathBuf> = outputs
        .keys()
        .map(|p| (crate::relpath::folded(p), p.clone()))
        .collect();
    let mut plan = Vec::new();
    for (path, written) in old {
        // On a filesystem blind to case and composition, a song renamed
        // only so is the same file as its new name: deleting the old name
        // would delete the new song. Elsewhere they are two files.
        let renamed = now.get(&crate::relpath::folded(path)).is_some_and(|new| {
            same_file::is_same_file(library.join(path), library.join(new)).unwrap_or(false)
        });
        if outputs.contains_key(path) || renamed {
            continue;
        }
        let fate = if written.sources.iter().any(|k| failed.contains(k)) {
            Pruned::KeptForFailed
        } else if changed_since_written(library, old, path)
            && !(forced
                && outputs
                    .values()
                    .any(|w| w.sources.iter().any(|k| written.sources.contains(k))))
        {
            Pruned::LeftChanged
        } else {
            // Another output's lyrics stay, whether the same name or one
            // a filesystem blind to case takes for it.
            let taken = |file: &PathBuf| {
                outputs.values().filter_map(|w| w.lyrics.as_ref()).any(|l| {
                    l == file
                        || (crate::relpath::folded(l) == crate::relpath::folded(file)
                            && same_file::is_same_file(library.join(l), library.join(file))
                                .unwrap_or(false))
                })
            };
            Pruned::Removed(
                std::iter::once(path)
                    .chain(&written.lyrics)
                    .filter(|file| !taken(file))
                    .cloned()
                    .collect(),
            )
        };
        plan.push((path.clone(), fate));
    }
    plan
}

/// Delete every file `old` lists that `outputs` does not, through the
/// ledger, which keeps each first, unless a song that failed this run
/// made it, which keeps it. A file changed since it was written is left,
/// and no longer muman's. Returns what went.
fn prune<W: Write>(
    ledger: &mut Ledger<'_>,
    old: &BTreeMap<PathBuf, Written>,
    outputs: &mut BTreeMap<PathBuf, Written>,
    failed: &BTreeSet<SourceKey>,
    forced: bool,
    out: &mut W,
) -> Result<Vec<PathBuf>> {
    let plan = prune_plan(ledger.root(), old, outputs, failed, forced);
    let doomed: Vec<Owned> = plan
        .iter()
        .filter_map(|(_, fate)| match fate {
            Pruned::Removed(files) => Some(files.iter()),
            _ => None,
        })
        .flatten()
        .filter_map(|file| ledger.owned(old, file, forced))
        .collect();
    let removal = ledger.remove(&doomed)?;
    for (path, fate) in plan {
        let files = match fate {
            Pruned::KeptForFailed => {
                outputs.insert(path.clone(), old[&path].clone());
                continue;
            }
            Pruned::LeftChanged => {
                crate::ui::warning(
                    out,
                    &format!(
                        "Left in place, changed since muman wrote it: {}; it is yours now",
                        crate::relpath::show(&path)
                    ),
                )?;
                continue;
            }
            Pruned::Removed(files) => files,
        };
        let stuck: Vec<&(PathBuf, String)> = removal
            .held
            .iter()
            .filter(|(f, _)| files.contains(f))
            .collect();
        if stuck.is_empty() {
            crate::ui::info(out, &format!("Removed: {}", crate::relpath::show(&path)))?;
            continue;
        }
        // One file held open must not stop the run before the state is
        // saved; it stays listed and goes next time.
        for (file, e) in stuck {
            crate::ui::warning(
                out,
                &format!(
                    "Could not remove {}: {e}; the next sync tries again",
                    file.display()
                ),
            )?;
        }
        outputs.insert(path.clone(), old[&path].clone());
    }
    Ok(removal.gone)
}

/// Set a folder's time to now, so players that rescan by it notice: a
/// song three levels down changes neither the root nor its children.
/// Best effort: a filesystem that refuses it costs only a rescan.
fn touch<W: Write>(dir: &Path, out: &mut W) -> Result<()> {
    if let Err(e) = filetime::set_file_mtime(dir, filetime::FileTime::now()) {
        crate::ui::warning(
            out,
            &format!("Could not update the time of {}: {e}", dir.display()),
        )?;
    }
    Ok(())
}

/// Where each aspect of a song's plan comes from, and why.
fn show_why<W: Write>(r: &Resolved, out: &mut W) -> Result<()> {
    writeln!(out, "  audio   {}: {}", r.plan.audio.key, r.why.audio)?;
    if let Some(why) = &r.why.fit {
        writeln!(out, "  format  {why}")?;
    }
    if let (Some(c), Some(why)) = (&r.plan.cover, &r.why.cover) {
        writeln!(out, "  cover   {}: {why}", c.key)?;
    }
    if let (Some(l), Some(why)) = (&r.plan.lyrics, &r.why.lyrics) {
        writeln!(out, "  lyrics  {}: {why}", l.key)?;
    }
    let width = r.plan.tags.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for ((key, values), why) in r.plan.tags.iter().zip(&r.why.tags) {
        let cleaned = if why.cleaned.is_empty() {
            String::new()
        } else {
            format!(", cleaned: {}", why.cleaned.join(", "))
        };
        writeln!(
            out,
            "  {key:<width$} {} ({}{cleaned})",
            values.join("; "),
            why.from
        )?;
    }
    Ok(())
}

/// What a dry run shows: the decisions a sync would act on.
struct Shown<'a> {
    manifest: &'a Manifest,
    planned: &'a [(usize, Resolved)],
    /// What a sync would do with each of `planned`.
    decided: &'a [Doing],
    /// Each song's file, as [`files_of`] finds it.
    files: &'a BTreeMap<usize, PathBuf>,
    old: &'a BTreeMap<PathBuf, Written>,
    state: &'a State,
    failed: &'a BTreeSet<SourceKey>,
}

/// The state as `decided`'s moves would leave it, which pruning is judged
/// by, saying each move.
fn would_move<W: Write>(
    planned: &[PlannedSong],
    decided: &[Doing],
    old: &BTreeMap<PathBuf, Written>,
    showing: &Showing,
    out: &mut W,
) -> Result<BTreeMap<PathBuf, Written>> {
    let mut moved = old.clone();
    for ((n, r), d) in planned.iter().zip(decided) {
        if let Doing::Move { from, .. } = d
            && let Some(written) = moved.remove(from)
        {
            if showing.matches(*n) {
                crate::ui::info(
                    out,
                    &format!(
                        "Would move: {} → {}",
                        crate::relpath::show(from),
                        crate::relpath::show(&path_of(r))
                    ),
                )?;
            }
            moved.insert(path_of(r), written);
        }
    }
    Ok(moved)
}

/// Which songs `status` shows.
struct Showing {
    /// The songs a query matched; every song without one.
    matched: Option<BTreeSet<usize>>,
    /// Show the songs a sync leaves as they are, as a query does.
    all: bool,
}

impl Showing {
    fn matches(&self, n: usize) -> bool {
        self.matched.as_ref().is_none_or(|m| m.contains(&n))
    }

    fn shows(&self, n: usize, fate: Fate) -> bool {
        self.matched
            .as_ref()
            .map_or(self.all || fate != Fate::Kept, |m| m.contains(&n))
    }
}

/// What a sync does with a song, as `status` counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fate {
    New,
    Changed,
    Again,
    Moved,
    Alone,
    Kept,
}

impl Fate {
    const ALL: [Self; 6] = [
        Self::New,
        Self::Changed,
        Self::Again,
        Self::Moved,
        Self::Alone,
        Self::Kept,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Changed => "changed",
            Self::Again => "written again",
            Self::Moved => "moved",
            Self::Alone => "left alone",
            Self::Kept => "up to date",
        }
    }
}

/// Each listed source an extractor's name keys, as keys were once
/// named, by the key the site of the page it was fetched from names it:
/// one no other song lists nor another source takes, as the page its
/// facts or its song's record recorded. A source of yt-dlp's generic extractor keeps its name: what
/// it recorded is the mirror a redirect ended at, not the address it was
/// fetched by.
fn sites_found(manifest: &Manifest, state: &State) -> BTreeMap<SourceKey, SourceKey> {
    let listed = manifest.keys();
    let mut renames = BTreeMap::new();
    for song in &manifest.songs {
        for key in song
            .sources
            .iter()
            .filter(|k| k.is_old() && k.site() != Some("generic"))
        {
            let page = state
                .facts
                .get(key)
                .and_then(|f| f.served.as_ref()?.url.clone())
                .or_else(|| song.held.get(key)?.url.clone());
            let to = page
                .as_deref()
                .and_then(crate::source::domain_of)
                .and_then(|site| SourceKey::parse(&format!("{site}:{}", key.id()?)).ok());
            if let Some(to) =
                to.filter(|t| !listed.contains(t) && !renames.values().any(|v| v == t))
            {
                renames.insert(key.clone(), to);
            }
        }
    }
    renames
}

/// What `status` says a sync does with a song, its file made by `before`.
fn verdict(d: &Doing, before: Option<&Plan>, plan: &Plan) -> (Fate, String) {
    match d {
        Doing::Keep => (Fate::Kept, "up to date".to_string()),
        Doing::Move { retag: false, .. } => (Fate::Moved, "moved".to_string()),
        Doing::Move { retag: true, .. } => (Fate::Moved, "moved, changes: tags".to_string()),
        Doing::Guard { .. } => (
            Fate::Alone,
            "left alone, changed since muman wrote it".to_string(),
        ),
        Doing::Unowned => (Fate::Alone, "left alone, not muman's".to_string()),
        Doing::Blocked { .. } => (
            Fate::Alone,
            "left where it is, its new path holding a file not muman's".to_string(),
        ),
        Doing::Held { .. } => (
            Fate::Alone,
            "left as built, a source holding otherwise".to_string(),
        ),
        Doing::Retag | Doing::Write => match before {
            Some(p) if p == plan => (Fate::Again, "written again".to_string()),
            Some(p) => (
                Fate::Changed,
                format!("changes: {}", changes(p, plan).join(", ")),
            ),
            None => (Fate::New, "new".to_string()),
        },
    }
}

/// How many of the songs `showing` matches a sync does each thing with,
/// and how many it cannot plan; those up to date it hides it says how
/// to show.
fn tally<W: Write>(
    fates: &BTreeMap<Fate, usize>,
    failed: usize,
    showing: &Showing,
    out: &mut W,
) -> Result<()> {
    let mut parts: Vec<String> = Fate::ALL
        .iter()
        .filter_map(|f| Some(format!("{} {}", fates.get(f)?, f.name())))
        .collect();
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if parts.is_empty() {
        if showing.matched.is_some() {
            crate::ui::info(out, "No song matches the query")?;
        }
        return Ok(());
    }
    let hidden = showing.matched.is_none() && !showing.all && fates.contains_key(&Fate::Kept);
    let hint = if hidden {
        "; `--all` or a query shows those up to date"
    } else {
        ""
    };
    crate::ui::info(out, &format!("Songs: {}{hint}", parts.join(", ")))?;
    Ok(())
}

/// Say what a sync would do, by the decisions it would make: each song
/// `showing` shows, its file and where each aspect comes from, then how
/// many songs it does each thing with. Without a query, then also what
/// it would remove and the files no song lists.
#[allow(clippy::too_many_lines)]
fn status<W: Write>(
    shown: &Shown<'_>,
    showing: &Showing,
    store: &Store,
    listed: &BTreeSet<SourceKey>,
    dirs: &Dirs,
    settling: Duration,
    out: &mut W,
) -> Result<()> {
    let Shown {
        manifest,
        planned,
        decided,
        files,
        old,
        state,
        failed,
    } = shown;
    let library = &dirs.library;
    let moved = would_move(planned, decided, old, showing, out)?;
    let mut after: BTreeMap<PathBuf, Written> = BTreeMap::new();
    let mut fates: BTreeMap<Fate, usize> = BTreeMap::new();
    for ((n, r), d) in planned.iter().zip(*decided) {
        let song = &manifest.songs[*n];
        let mut path = path_of(r);
        let before = moved
            .get(&path)
            .or_else(|| files.get(n).and_then(|f| old.get(f)))
            .and_then(|w| w.plan.as_ref());
        if let Doing::Guard { file } | Doing::Held { file } | Doing::Blocked { file } = d {
            path.clone_from(file);
        }
        let (fate, verdict) = verdict(d, before, &r.plan);
        if showing.matches(*n) {
            *fates.entry(fate).or_default() += 1;
        }
        if showing.shows(*n, fate) {
            writeln!(
                out,
                "{}",
                crate::ui::Style::Accent.paint(&name_of(song, Some(r), &state.facts))
            )?;
            writeln!(
                out,
                "  → {} ({verdict})",
                crate::ui::Style::Path.paint(&crate::relpath::show(&path))
            )?;
            show_why(r, out)?;
        }
        let written = match d {
            Doing::Keep | Doing::Guard { .. } | Doing::Held { .. } | Doing::Blocked { .. } => {
                moved.get(&path).cloned()
            }
            Doing::Unowned => None,
            _ => Some(Written {
                sources: song.sources.clone(),
                lyrics: r.plan.lyrics.as_ref().map(|_| path.with_extension("lrc")),
                plan: Some(r.plan.clone()),
                stamp: None,
                digest: None,
            }),
        };
        if let Some(written) = written {
            after.insert(path, written);
        }
    }
    let planned_songs: BTreeSet<usize> = planned.iter().map(|(n, _)| *n).collect();
    let unplanned = (0..manifest.songs.len())
        .filter(|n| showing.matches(*n) && !planned_songs.contains(n))
        .count();
    tally(&fates, unplanned, showing, out)?;
    // What a query matches is songs; these are files.
    if showing.matched.is_some() {
        return Ok(());
    }
    for (path, fate) in prune_plan(library, &moved, &after, failed, false) {
        match fate {
            Pruned::Removed(_) => crate::ui::warning(
                out,
                &format!("Would remove: {}", crate::relpath::show(&path)),
            )?,
            Pruned::LeftChanged => crate::ui::warning(
                out,
                &format!(
                    "Would leave in place, changed since muman wrote it: {}",
                    crate::relpath::show(&path)
                ),
            )?,
            Pruned::KeptForFailed => {}
        }
    }
    for twin in store.twins() {
        crate::ui::warning(
            out,
            &format!(
                "Not read, as another file in the manual folder has its name once \
                 normalized: {}; rename one of them",
                crate::relpath::show(twin)
            ),
        )?;
    }
    let mut known = listed.clone();
    known.extend(manifest.removed_keys());
    let (mut unlisted, mut settling) = store.unlisted(&known, SystemTime::now(), settling);
    settling.retain(|path| {
        let key = SourceKey::Manual(path.into());
        let was_listed = state.listed.contains(&key);
        if was_listed {
            unlisted.push(key);
        }
        !was_listed
    });
    for key in unlisted {
        let line = if state.listed.contains(&key) {
            format!("Taken out of the song list by hand, kept out by the next sync: {key}")
        } else {
            format!("Not listed yet, added by the next sync: {key}")
        };
        crate::ui::info(out, &line)?;
    }
    for path in settling {
        crate::ui::info(
            out,
            &format!(
                "Still being copied in, left for a later run: {}",
                SourceKey::Manual(path.into())
            ),
        )?;
    }
    // A manual file not listed is added by the next sync; only fetched
    // and kept files can lie unused.
    for path in store
        .unused(listed)
        .iter()
        .filter(|p| !p.starts_with(dirs.manual()))
    {
        crate::ui::info(out, &format!("Unused: {}", path.display()))?;
    }
    Ok(())
}

/// What every run leaves, whatever came before it: each file the state
/// records is there, no song has two, and nothing is half written.
#[cfg(test)]
pub(crate) fn invariant(dirs: &Dirs) -> std::result::Result<(), String> {
    let state = State::load(&dirs.home).map_err(|e| format!("{e:#}"))?;
    let mut owners: BTreeMap<&SourceKey, &PathBuf> = BTreeMap::new();
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
