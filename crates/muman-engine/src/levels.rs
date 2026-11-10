//! Every planned song levelled as `[loudness]` says, from what
//! [`crate::loudness`] measured. A song's track gain is the target less
//! its own loudness; its album's, the target less the loudness of every
//! block of the album's songs pooled, an album as [`crate::album::of_each`]
//! tells the songs of one from another's, so the songs an album's
//! agreement spells alike are levelled alike. A single, named for its
//! title, is levelled alone. A gain set by hand wins over one measured, and the
//! source's own gain tags stand where nothing was measured. The mode says
//! how the gain the scope picks is applied, capped by the ceiling, and
//! one applied to the samples of a copied song makes it encoded. Only
//! what was measured is read, so every view of the plans levels alike.
//!
//! A song whose file stays as it was made, failing to resolve or left as
//! built while its source holds other audio, is pooled with its album as
//! that file was made: by the plan the state records of it, at the
//! loudness of the audio that plan names. A source fetched again holds
//! other audio, measured in place of what it held, so each sync keeps the
//! loudness of the audio every library file was made from
//! ([`keep_built`]), and that is read where the source's facts no longer
//! hold it. So the songs of an album agree on its gain while one of them
//! waits for `sync --accept`, and none is written again for it.

use std::collections::{BTreeMap, BTreeSet};

use crate::album::Album;
use crate::analysis::Analysis;
use crate::loudness::{
    Apply, GainScope, Gains, Level, LoudnessMode, Measure, capped, hundredths_to_q78, integrated,
    parse_gain, parse_peak,
};
use crate::resolve::{Format, Plan, Resolved, TagWhy};
use crate::settings::Settings;
use crate::state::{Built, State};
use crate::tags::{Field, Scope};

/// Where a hand-set tag's value came from, as [`crate::resolve`] names it.
const BY_HAND: [&str; 2] = ["song.tags", "album.tags"];

/// What its source's facts hold of the audio `plan` takes, as it is
/// written: mixed where it is mixed.
fn analysis_now<'a>(plan: &Plan, state: &'a State) -> Option<&'a Analysis> {
    let facts = state.facts.get(&plan.audio.key)?;
    if facts.audio_rev() != plan.audio.rev {
        return None;
    }
    match plan.format.mix() {
        Some(layout) => state.mix(&plan.audio.rev, layout),
        None => Some(&facts.audio.as_ref()?.analysis),
    }
}

/// The loudness of the audio `plan` takes, as it is written: as its
/// source holds it, else as kept of a file made from it.
fn measure_of<'a>(plan: &Plan, state: &'a State) -> Option<&'a Measure> {
    analysis_now(plan, state)
        .and_then(Analysis::loudness)
        .or_else(|| {
            let layout = plan.format.mix();
            (state.built.iter())
                .find(|b| b.audio == plan.audio.rev && b.layout == layout)
                .and_then(|b| b.analysis.loudness())
        })
}

/// Keep in `state` the loudness of the audio each library file was made
/// from, while its source's facts hold it, and let go of that of audio
/// no file was made from.
pub fn keep_built(state: &mut State) {
    let plans: Vec<&Plan> = state
        .outputs
        .values()
        .filter_map(|w| w.plan.as_ref())
        .collect();
    let named: BTreeSet<(&str, Option<crate::codec::Layout>)> = plans
        .iter()
        .map(|p| (p.audio.rev.as_str(), p.format.mix()))
        .collect();
    let mut now: Vec<Built> = Vec::new();
    for plan in &plans {
        let layout = plan.format.mix();
        let Some(loudness) = analysis_now(plan, state).and_then(|a| a.loudness.clone()) else {
            continue;
        };
        if !now
            .iter()
            .any(|b| b.audio == plan.audio.rev && b.layout == layout)
        {
            now.push(Built {
                audio: plan.audio.rev.clone(),
                layout,
                analysis: Analysis {
                    loudness: Some(loudness),
                    ..Analysis::default()
                },
            });
        }
    }
    let mut kept: Vec<Built> = (state.built.iter())
        .filter(|b| named.contains(&(b.audio.as_str(), b.layout)))
        .filter(|b| {
            !now.iter()
                .any(|n| n.audio == b.audio && n.layout == b.layout)
        })
        .cloned()
        .collect();
    kept.extend(now);
    state.built = kept;
}

/// The value a tag of `field` has in `r`, and whether it was set by hand.
fn tag_of(r: &Resolved, field: Field) -> Option<(&str, bool)> {
    let value = r
        .plan
        .tags
        .iter()
        .find(|(k, _)| k == field.vorbis())
        .and_then(|(_, v)| v.first())?;
    let by_hand = r
        .why
        .tags
        .iter()
        .any(|t| t.key == field.vorbis() && BY_HAND.contains(&t.from.as_str()));
    Some((value, by_hand))
}

/// The level of `r` a hand-set gain gives; else `measured`; else the one
/// its source's own tags give.
fn level_of(
    r: &Resolved,
    gain: Field,
    peak: Field,
    measured: Option<Level>,
) -> Option<(Level, bool)> {
    let tag_gain = tag_of(r, gain);
    let tag_peak = tag_of(r, peak).and_then(|(p, _)| parse_peak(p));
    if let Some((g, true)) = tag_gain
        && let Some(gain) = parse_gain(g)
    {
        let peak = tag_peak.or(measured.and_then(|m| m.peak));
        return Some((Level { gain, peak }, false));
    }
    if let Some(m) = measured {
        return Some((m, true));
    }
    let gain = parse_gain(tag_gain?.0)?;
    Some((
        Level {
            gain,
            peak: tag_peak,
        },
        false,
    ))
}

/// Level every song planned as `[loudness]` says: each its gains, its
/// album's pooled over the album's songs, applied as the mode says and
/// its tags derived from them. A song of `held`, at the plan its file was
/// made from, is pooled so in place of any plan of it in `planned`.
/// Reads what was measured, measuring nothing, so every view of the
/// plans levels alike.
pub fn settle(
    planned: &mut [(usize, Resolved)],
    held: &[(usize, Resolved)],
    state: &State,
    settings: &Settings,
) {
    let loudness = &settings.loudness;
    if !loudness.measured() {
        return;
    }
    let left: BTreeSet<usize> = held.iter().map(|(n, _)| *n).collect();
    let members: Vec<&(usize, Resolved)> = (planned.iter())
        .filter(|(n, _)| !left.contains(n))
        .chain(held)
        .collect();
    let measures: Vec<Option<&Measure>> = (members.iter())
        .map(|(_, r)| measure_of(&r.plan, state))
        .collect();
    let album_of = crate::album::of_each(members.iter().map(|(_, r)| r));
    let album_by_song: BTreeMap<usize, Option<Album>> = (members.iter().zip(&album_of))
        .map(|((n, _), album)| (*n, album.clone()))
        .collect();
    let mut albums: BTreeMap<&Album, Vec<usize>> = BTreeMap::new();
    for (i, album) in album_of.iter().enumerate() {
        if let Some(album) = album {
            albums.entry(album).or_default().push(i);
        }
    }
    let pooled: BTreeMap<Album, (Option<Level>, usize)> = albums
        .into_iter()
        .map(|(key, songs)| {
            let of_album: Vec<&Measure> = songs.iter().filter_map(|&i| measures[i]).collect();
            let peak = of_album
                .iter()
                .map(|m| m.peak(loudness.true_peak))
                .fold(0.0, f64::max);
            let level = integrated(of_album.iter().map(|m| &m.momentary))
                .and_then(|lufs| Level::to(loudness.target, lufs, peak));
            (key.clone(), (level, of_album.len()))
        })
        .collect();
    for (n, r) in planned.iter_mut() {
        let measure = measure_of(&r.plan, state);
        let own =
            measure.and_then(|m| Level::to(loudness.target, m.lufs()?, m.peak(loudness.true_peak)));
        let Some((track, track_measured)) = level_of(r, Field::TrackGain, Field::TrackPeak, own)
        else {
            continue;
        };
        let key = album_by_song.get(n).and_then(Option::as_ref);
        let (album_measured, songs) = key
            .and_then(|k| pooled.get(k).copied())
            .unwrap_or((track_measured.then_some(track), 1));
        let album = level_of(r, Field::AlbumGain, Field::AlbumPeak, album_measured);
        let wanted = match (loudness.scope, album) {
            (GainScope::Album, Some((a, _))) => a,
            _ => track,
        };
        let gain = capped(wanted.gain, wanted.peak, loudness.ceiling);
        let audio = state
            .facts
            .get(&r.plan.audio.key)
            .and_then(|f| f.audio.as_ref());
        let gains = Gains {
            track,
            album: album.map(|(a, _)| a),
            apply: match loudness.mode {
                LoudnessMode::Header => Apply::Header(hundredths_to_q78(gain)),
                LoudnessMode::Audio => Apply::Volume(gain),
                LoudnessMode::Tags | LoudnessMode::Off => Apply::Tags,
            },
            opus_gain: measure.map_or(0, |m| m.opus_gain),
            bits: audio.filter(|a| !a.float).map_or(0, |a| a.bits),
        };
        let mut plan = r.plan.clone();
        if let (Apply::Volume(g), Format::Copy { codec }, Some(audio)) =
            (gains.apply, plan.format, audio)
            && g != 0
        {
            plan = plan.with_format(Format::encoded_as(codec, audio, &settings.audio));
        }
        r.plan = plan.with_loudness(gains);
        let why = |field: Field, by_measure: bool| {
            let from = match (by_measure, field.scope()) {
                (false, _) => None,
                (true, Scope::AlbumLoudness) if key.is_some() => Some(format!(
                    "{} over its album's {songs} song(s)",
                    crate::resolve::MEASURED
                )),
                (true, _) => Some(crate::resolve::MEASURED.to_string()),
            };
            (field, from)
        };
        let froms = [
            why(Field::TrackGain, track_measured),
            why(Field::TrackPeak, track_measured),
            why(Field::AlbumGain, album.is_some_and(|(_, m)| m)),
            why(Field::AlbumPeak, album.is_some_and(|(_, m)| m)),
        ];
        reorder_why(r, &froms);
    }
}

/// `r`'s reasons in its tags' order, each loudness tag's from `froms`
/// where a measure set it.
fn reorder_why(r: &mut Resolved, froms: &[(Field, Option<String>)]) {
    let old = std::mem::take(&mut r.why.tags);
    for (key, _) in &r.plan.tags {
        let measured = froms
            .iter()
            .find(|(f, _)| f.vorbis() == key)
            .and_then(|(_, from)| from.clone());
        let kept = old.iter().find(|t| &t.key == key).cloned();
        let why = match (measured, kept) {
            (Some(from), _) => TagWhy {
                key: key.clone(),
                from,
                cleaned: Vec::new(),
            },
            (None, Some(t)) => t,
            (None, None) => TagWhy {
                key: key.clone(),
                from: "its source".to_string(),
                cleaned: Vec::new(),
            },
        };
        r.why.tags.push(why);
    }
}
