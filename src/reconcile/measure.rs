//! What the offline phase measures of the listed sources: each source's
//! facts, read again or caught up where an earlier version measured it
//! another way, the comparisons each song's resolving asks for, and the
//! audio sizes fitting asks for.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use anyhow::Result;

use crate::align;
use crate::facts::{self, Facts};
use crate::manifest::Song;
use crate::parallel;
use crate::progress;
use crate::resolve;
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{self, Aligned, State, Step};
use crate::store::{Kind, Located, Store};
use crate::tags;

use super::plan::Planned;
use super::{CHECKPOINT, Sub};

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
    let relyriced = relyric(runner, store, keys, state, scratch, out)?;
    if due.is_empty() {
        if retagged || relyriced {
            (how.checkpoint)(state)?;
        }
        return Ok(());
    }
    crate::ui::info(out, &format!("Measuring {} source(s)", due.len()))?;
    // A lookup record or a picture takes a moment, a song's audio seconds
    // to minutes: counted as alike, they would make the time left a guess,
    // so only the audio is counted.
    let media = due.iter().filter(|l| l.kind == Kind::Media).count();
    let step = progress::step("Measuring", Some(media as u64));
    let mut kept = Instant::now();
    parallel::chunked(
        &due,
        parallel::builds(),
        |l| {
            let _working = (l.kind == Kind::Media).then(|| step.working(&progress::label(&l.path)));
            facts::gather(
                runner,
                l,
                Sub::new(scratch, "facts-")?.path(),
                &how.analyses,
            )
        },
        |chunk, measured| {
            for (located, result) in chunk.iter().zip(measured) {
                match result {
                    Ok(f) => {
                        warn_of(out, &located.key, &f)?;
                        if let Some(old) = state.facts.get(&located.key).cloned() {
                            state.rebase(&located.key, &old, &f);
                        }
                        state.facts.insert(located.key.clone(), f);
                        // Whether a copy fetched again is whole is the
                        // damage noted after measuring's to say.
                        if state
                            .failures
                            .get(&located.key)
                            .is_none_or(|f| f.step != Step::Refetch)
                        {
                            state.clear_failure(&located.key);
                        }
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
pub(super) fn warn_of<W: Write>(
    out: &mut W,
    key: &SourceKey,
    f: &crate::facts::Facts,
) -> Result<()> {
    if let Some(damage) = f.damage() {
        crate::ui::warning(out, &damage.describe(key))?;
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

pub(super) fn unkept_warning(key: &SourceKey, unkept: crate::facts::Unkept) -> String {
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
pub(super) fn retag<R: Runner, W: Write>(
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
    let read = parallel::map(&due, parallel::builds(), |l| {
        facts::retag(runner, l, Sub::new(scratch, "tags-")?.path())
    });
    for (located, result) in due.iter().zip(read) {
        match result {
            Ok(read) => {
                if let Some(f) = state.facts.get_mut(&located.key) {
                    f.tags = read.tags;
                    f.release = read.release;
                    f.chapters = read.chapters;
                    f.tags_method = tags::METHOD.to_string();
                    f.lyrics.retain(|l| l.at != facts::LyricsAt::Tag);
                    f.lyrics.extend(read.lyrics);
                }
            }
            Err(e) => crate::ui::warning(out, &format!("Could not read {}: {e:#}", located.key))?,
        }
    }
    Ok(true)
}

/// Read again whether the lyrics of each source whose measures still hold
/// sing anything, where they were measured before muman asked, decoding
/// no audio. Returns whether any were.
pub(super) fn relyric<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    scratch: &Path,
    out: &mut W,
) -> Result<bool> {
    let due: Vec<(Located, Vec<crate::facts::LyricsAt>)> = keys
        .iter()
        .filter_map(|k| store.locate(k))
        .filter_map(|l| {
            let f = state.facts.get(&l.key).filter(|f| f.holds_for(&l.rev()))?;
            let at: Vec<_> = f
                .lyrics
                .iter()
                .filter(|x| x.sings.is_none())
                .map(|x| x.at.clone())
                .collect();
            (!at.is_empty()).then_some((l, at))
        })
        .collect();
    if due.is_empty() {
        return Ok(false);
    }
    crate::ui::info(
        out,
        &format!("Reading the lyrics of {} source(s)", due.len()),
    )?;
    let read = parallel::map(&due, parallel::builds(), |(l, at)| {
        facts::relyric(runner, l, at, Sub::new(scratch, "lyrics-")?.path())
    });
    for ((located, asked), result) in due.iter().zip(read) {
        match result {
            Ok(found) => {
                if let Some(f) = state.facts.get_mut(&located.key) {
                    // What does not read again stays lyrics, as it was,
                    // rather than be read again on every sync.
                    for l in f.lyrics.iter_mut().filter(|l| asked.contains(&l.at)) {
                        let sings = found.iter().find(|(at, _)| *at == l.at).map(|(_, s)| *s);
                        l.sings = Some(sings.unwrap_or(true));
                    }
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
pub(super) fn hash<R: Runner, W: Write>(
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
    let read = parallel::map(&due, parallel::builds(), |l| {
        let _working = step.working(&progress::label(&l.path));
        facts::hash(runner, l, Sub::new(scratch, "hash-")?.path())
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
    let read = parallel::map(&due, parallel::builds(), |l| {
        let _working = step.working(&progress::label(&l.path));
        facts::look(runner, l, Sub::new(scratch, "look-")?.path())
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
pub(super) fn analyze<R: Runner, W: Write>(
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
    let facts = &state.facts;
    let read = parallel::map(&due, parallel::builds(), |(l, kinds)| {
        let _working = step.working(&progress::label(&l.path));
        let audio = facts.get(&l.key).and_then(|f| f.audio.as_ref())?;
        let scratch = match Sub::new(scratch, "analyze-") {
            Ok(s) => s,
            Err(e) => return Some(Err(e)),
        };
        Some(facts::analyze(runner, l, audio, kinds, scratch.path()))
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
                crate::analysis::Pass::new(kinds.clone())
                    .finish(&crate::integrity::Found::default())
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
pub(super) fn analyze_mixes<R: Runner, W: Write>(
    runner: &R,
    store: &Store,
    keys: &BTreeSet<SourceKey>,
    state: &mut State,
    settings: &crate::settings::Settings,
    out: &mut W,
) -> Result<bool> {
    let mut wanted = crate::analysis::wanted(&settings.loudness);
    wanted.retain(|k| k.mixed());
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
            let layout = crate::resolve::Format::encoded(audio, &settings.audio).mix()?;
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
pub(crate) fn compare<R: Runner, W: Write>(
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
            "Comparing {} pair(s), decoding {} recording(s)",
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
        let outside = align::outside(ea, eb, &found);
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
            a_extra_ms: outside.extra_ms,
            a_lacks_ms: outside.lacks_ms,
            b_lacks_ms: outside.track_lacks_ms,
        })
    });
    for aligned in compared.into_iter().flatten() {
        state.record_alignment(aligned);
    }
    Ok(true)
}

/// The audio stream of a source to measure, and where the source is.
pub(super) struct Sizing(u32, Located);

/// Measure the audio bytes of each planned song's source that a copy
/// would carry and whose facts lack them, for fitting. Returns whether
/// any were.
pub(super) fn sizes<R: Runner, W: Write>(
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
    let due: Vec<(SourceKey, Sizing)> = due.into_iter().collect();
    let measured = parallel::map(&due, parallel::builds(), |(_, Sizing(index, located))| {
        facts::audio_bytes(
            runner,
            located,
            *index,
            Sub::new(scratch, "size-").ok()?.path(),
        )
        .ok()
    });
    for ((key, _), bytes) in due.iter().zip(measured) {
        if let Some(audio) = state.facts.get_mut(key).and_then(|f| f.audio.as_mut()) {
            audio.bytes = bytes;
        }
    }
    Ok(true)
}

/// Whether `e` is a disk full, from muman's own writes or from ffmpeg's,
/// which reports it only in words.
pub(super) fn out_of_space(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull)
            || cause.to_string().contains("No space left on device")
    })
}
