//! Sources holding otherwise than their songs were built from: damaged
//! copies noted to be fetched again, and the songs held as built until
//! `--accept`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use anyhow::Result;

use crate::atomic::Lock;
use crate::dirs::Dirs;
use crate::facts::Facts;
use crate::held::Drift;
use crate::manifest::Manifest;
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{State, Step};
use crate::store::Store;

use super::decide::Doing;
use super::measure::{Measuring, measure};
use super::persist;
use super::plan::PlannedSong;

/// `drifts` of songs not `damaged`: those taken, followed, accepted or
/// whole in place of a damaged copy, and those its song is held as built
/// for.
pub(super) fn split_drifts<'a>(
    drifts: &'a [Drift],
    damaged: &BTreeSet<usize>,
    accept: bool,
    state: &State,
) -> (Vec<&'a Drift>, Vec<&'a Drift>) {
    drifts
        .iter()
        .filter(|d| !damaged.contains(&d.song))
        .partition(|d| accept || d.followed() || whole_again(d, state))
}

/// Whether `d` is a whole copy of a download fetched again because the
/// copy its song was built from was damaged: taken as fetched, as no
/// one chose the damage.
pub(super) fn whole_again(d: &Drift, state: &State) -> bool {
    let undamaged = state
        .facts
        .get(&d.key)
        .is_some_and(|f| f.damage().is_none());
    undamaged && d.was.audio.is_some() && state.damaged.get(&d.key) == d.was.audio.as_ref()
}

/// Note each fetched source found damaged, a torrent's file among them,
/// to be fetched again: once for each damaged copy, counted by its
/// revision; and let go of the note, and of the audio recorded for it,
/// once a whole copy is measured and its song's record names it.
pub(super) fn note_damage(manifest: &Manifest, store: &Store, state: &mut State) {
    let sites = &manifest.settings.sites;
    for key in manifest.keys() {
        let fetched = matches!(key, SourceKey::Remote { .. } | SourceKey::Torrent(_));
        if !fetched || crate::provider::is_kept(&key) {
            continue;
        }
        let Some(located) = store.locate(&key) else {
            continue;
        };
        let rev = located.rev();
        let Some(facts) = state.facts.get(&key).filter(|f| f.holds_for(&rev)) else {
            continue;
        };
        let held = manifest.song_with(&key).and_then(|s| s.held.get(&key));
        let fetchable = matches!(key, SourceKey::Torrent(_))
            || sites.fetch_url(&key).is_some()
            || held.and_then(|h| h.url.as_ref()).is_some();
        let refetching = state
            .failures
            .get(&key)
            .filter(|f| f.step == Step::Refetch)
            .map(|f| f.rev.clone());
        match facts.damage() {
            Some(damage) if fetchable => {
                if refetching.is_some_and(|r| r.as_deref() == Some(rev.as_str())) {
                    continue;
                }
                let audio = crate::held::Held::now(&key, facts, sites).and_then(|h| h.audio);
                let said = damage.describe(&key);
                if let Some(audio) = audio {
                    state.damaged.entry(key.clone()).or_insert(audio);
                }
                state.record_failure(&key, Step::Refetch, Some(rev), said);
            }
            Some(_) => {}
            None => {
                let recorded = held.and_then(|h| h.audio.as_ref());
                // A torrent's hash names every byte of its files: one that
                // holds other audio than its song was built from is damaged.
                let now = crate::held::Held::now(&key, facts, sites).and_then(|h| h.audio);
                if matches!(key, SourceKey::Torrent(_))
                    && let (Some(was), Some(now)) = (recorded, now)
                    && *was != now
                {
                    if refetching.is_none_or(|r| r.as_deref() != Some(rev.as_str())) {
                        state.damaged.entry(key.clone()).or_insert(now);
                        let said = format!("{key} holds other bytes than its torrent's");
                        state.record_failure(&key, Step::Refetch, Some(rev), said);
                    }
                    continue;
                }
                if refetching.is_some() {
                    state.clear_failure(&key);
                }
                if state.damaged.get(&key).is_some_and(|d| Some(d) != recorded) {
                    state.damaged.remove(&key);
                }
            }
        }
    }
}

/// Say what each source holding otherwise than its song was built
/// from does to the song: followed or taken, or left as built while its
/// file is there.
pub(super) fn say_drifts<W: Write>(
    (taken, kept): (&[&Drift], &[&Drift]),
    planned: &[PlannedSong],
    decided: &[Doing],
    force: bool,
    whole: &dyn Fn(&Drift) -> bool,
    out: &mut W,
) -> Result<()> {
    for d in taken {
        let how = if d.followed() {
            "its song follows it, a file of your own"
        } else if whole(d) {
            "taken, a whole copy in place of a damaged one"
        } else {
            "taken, as `--accept` asks"
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

/// The songs of `planned` whose audio would come from a source whose
/// audio changed since the song was built and is damaged now, as a copy
/// stopped short or a download that lost pieces leaves a file: written
/// from it, the song would lose what its file holds, so the file is held
/// as built.
pub(super) fn damaged_since_built(
    planned: &[PlannedSong],
    drifts: &[Drift],
    facts: &BTreeMap<SourceKey, Facts>,
) -> BTreeSet<usize> {
    planned
        .iter()
        .filter(|(n, r)| {
            let key = &r.plan.audio.key;
            facts.get(key).is_some_and(|f| f.damage().is_some())
                && drifts.iter().any(|d| {
                    d.song == *n && d.key == *key && d.changed.contains(&crate::held::Aspect::Audio)
                })
        })
        .map(|(n, _)| *n)
        .collect()
}

/// Measure each fetched source whose facts are not current and note
/// which are damaged, so a sync's network step fetches them again in the
/// run that finds them rather than the next: a file damaged on disk
/// since it was measured is found by measuring, which the offline phase
/// does after the fetches.
pub(crate) fn note_damaged<R: Runner, W: Write>(
    runner: &R,
    dirs: &Dirs,
    retry: bool,
    out: &mut W,
) -> Result<()> {
    let lock = Lock::folder(&dirs.home)?;
    let manifest = Manifest::load(&dirs.home)?;
    let mut state = State::load(&dirs.home)?;
    let store = Store::scan(dirs)?;
    let fetched: BTreeSet<SourceKey> = manifest
        .keys()
        .into_iter()
        .filter(|k| {
            matches!(k, SourceKey::Remote { .. } | SourceKey::Torrent(_))
                && !crate::provider::is_kept(k)
        })
        .collect();
    let temp = crate::atomic::Scratch::new()?;
    let mut how = Measuring {
        retry,
        say_unread: false,
        checkpoint: &mut |s: &State| persist(&dirs.home, s, false, &lock),
        analyses: crate::analysis::wanted(&manifest.settings.loudness),
    };
    measure(
        runner,
        &store,
        &fetched,
        &mut state,
        temp.path(),
        &mut how,
        out,
    )?;
    note_damage(&manifest, &store, &mut state);
    persist(&dirs.home, &state, false, &lock)
}
