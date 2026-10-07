//! The library kept under `[library] max_size`: after every song is
//! resolved on its own, the songs are fitted together by [`crate::fit`],
//! and a song that does not fit at its best is written at a lower bitrate
//! of `[audio] lossy`. The chosen format goes into the song's plan, so a
//! song changes format exactly as any other change of plan does.
//!
//! **One library for one song list.** Which songs are lowered, and how
//! far, is a function of the song list, the sources, the settings and the
//! ffmpeg in use, and of nothing a run leaves behind: not the order songs
//! were added in, not the files the library holds, not what earlier runs
//! measured. Two homes with the same song list and sources fit alike, and
//! a library fitted bit by bit ends as one fitted at once. That costs
//! rendering: adding a song may lower others, raising the limit raises
//! them again, and a song is rendered to learn exactly what it takes.
//!
//! **Sizes.** Every rung starts as an estimate made from the facts alone:
//! a copy as the source's audio packets plus `CONTAINER`; a lossless
//! encode as its source, PCM at `PCM_FLAC` of it; a lossy encode as its
//! bitrate times its length; each with a guess at its cover, tags and
//! lyrics. Fitting on estimates picks a rung for each song; each rung
//! picked is rendered into a scratch folder and its real size replaces
//! its estimate; the songs are fitted again, and so on until every rung
//! picked is one whose real size is known, at most `PASSES` times; when
//! they run out with estimates still in the sum, as encoders missing
//! their bitrates keep them moving, the songs are fitted on sizes known
//! alone, each song's lowest rendered first if that is what it takes. Each
//! file counts whole `[library] block_size` blocks, and two standard
//! deviations of the estimates still in the sum are kept free.
//!
//! Real sizes are kept in `state.json` by [`plan_key`], the plan and the
//! ffmpeg version together, and taken from there instead of rendering
//! again, but only when the passes above ask for that rung: sizes known
//! from before never steer the passes, so a home measured over many runs
//! fits exactly as a new one would, only with less rendering. A song
//! rendered to be measured is moved into the library rather than rendered
//! twice. Sizes are taken before any hook runs.
//!
//! **Too little room.** When the songs cannot fit, each one's lowest rung
//! is rendered to measure it before anything else; only when even the
//! real lowest sizes cannot fit are songs left out, the last listed
//! first, until the rest fit. A song left out is not written, and its
//! file goes as a removed song's does.
//!
//! Songs changed since muman wrote them, and the files a failed song
//! keeps, are counted as they are and never moved: they are yours.

use crate::units;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::codec::Codec;
use crate::facts::Facts;
use crate::fit::{self, Fit, Item, Policy, Rung, Source};
use crate::manifest::Manifest;
use crate::parallel;
use crate::reconcile::{Planned, changed_since_written};
use crate::render::{self, Rendered};
use crate::resolve::{AudioRef, Format, Plan};
use crate::runner::Runner;
use crate::source::SourceKey;
use crate::state::{Measured, State, Written};
use crate::store::{self, Located};

/// What a container adds to an audio stream, as a share of it: Ogg's
/// page headers take about 1%, MP4's index less.
const CONTAINER: f64 = 0.015;
/// Bytes a file holds besides its stream, cover and tags.
const HEADERS: u64 = 4 * 1024;
/// FLAC from PCM, as a share of the PCM: music compresses to 50 to 70%.
const PCM_FLAC: f64 = 0.6;
/// The cover guessed for a song: a 1000 px JPEG.
const COVER_GUESS: u64 = 200 * 1024;
/// Lyrics guessed for a song that has them.
const LYRICS_GUESS: u64 = 3 * 1024;
/// How far estimates of a copy, of an encode at a bitrate, and of audio
/// whose packets were never measured may be off, as a share of the audio.
const SIGMA_COPY: f64 = 0.01;
const SIGMA_ENCODE: f64 = 0.15;
const SIGMA_GUESS: f64 = 0.3;
/// Standard deviations of the estimates kept free.
const SIGMAS: f64 = 2.0;
/// How many times songs are fitted again on real sizes. A pass only
/// learns sizes, so the passes end once no rung picked is unknown; real
/// libraries settle in two or three.
const PASSES: usize = 20;

/// Bitrates in kbit/s a copy of a codec is guessed at when its packets
/// were never measured.
fn nominal_kbps(codec: Codec) -> f64 {
    match codec {
        Codec::Opus => 130.0,
        Codec::Aac => 128.0,
        Codec::Vorbis => 160.0,
        Codec::Mp3 => 192.0,
        Codec::Flac | Codec::Alac => 900.0,
    }
}

/// `bytes` in whole blocks of `block`.
#[must_use]
pub fn blocks(bytes: u64, block: u64) -> u64 {
    bytes.div_ceil(block.max(1)) * block.max(1)
}

/// The key a plan's real size is kept under: a 64-bit FNV-1a hash of
/// the ffmpeg version and the plan, as hex.
#[must_use]
pub fn plan_key(tools: &str, plan: &Plan) -> String {
    let json = serde_json::to_string(plan).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in tools.bytes().chain([0]).chain(json.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The ffmpeg in use, by the first line it prints of its version.
pub fn tools<R: Runner>(runner: &R) -> String {
    let cmd: Vec<std::ffi::OsString> = ["ffmpeg", "-hide_banner", "-version"]
        .into_iter()
        .map(Into::into)
        .collect();
    runner
        .output(&cmd)
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out)
                .lines()
                .next()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "ffmpeg".to_string())
}

/// What fitting did, for messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    pub max: u64,
    /// What the library is projected to take, the margin for any
    /// estimate left included.
    pub projected: u64,
    /// The songs left out for want of room, by their place in the list.
    pub left_out: Vec<usize>,
    /// What the songs would take even at their lowest bitrates.
    pub floor: Option<u64>,
    /// Songs below their best format.
    pub lowered: usize,
    /// Songs rendered to measure them this run.
    pub rendered: usize,
    /// The ffmpeg version real sizes are kept for.
    pub tools: String,
    /// The key of every rung of every song: the real sizes worth keeping.
    pub keys: BTreeSet<String>,
}

/// Songs rendered to be measured at the format fitting chose, by their
/// place in the list: the folder rendered into and what was written
/// there, to be moved into the library.
pub type Made = HashMap<usize, (PathBuf, Rendered)>;

/// One rung a song may be written at, before its real size is known.
#[derive(Debug, Clone)]
struct Choice {
    format: Format,
    plan_key: String,
    /// The key its plan had before this run named its sources anew, by
    /// [`State::rebase`], where that differs.
    was_key: Option<String>,
    estimate: u64,
    sigma: f64,
    loss: f64,
}

/// One planned song's rungs, from its best down.
#[derive(Debug)]
struct Ladder {
    key: String,
    rungs: Vec<Choice>,
    /// The bytes of a song that cannot move.
    fixed: Option<u64>,
}

/// The library files written from each audio source.
type ByAudio<'a> = HashMap<&'a AudioRef, Vec<(&'a PathBuf, &'a Written)>>;

fn by_audio(state: &State) -> ByAudio<'_> {
    let mut by: ByAudio<'_> = HashMap::new();
    for (path, w) in &state.outputs {
        if let Some(plan) = &w.plan {
            by.entry(&plan.audio).or_default().push((path, w));
        }
    }
    by
}

/// A rung's estimated audio bytes and their standard deviation, from
/// the facts alone.
#[allow(clippy::cast_precision_loss)]
fn estimate(format: Format, facts: &Facts, seconds: f64) -> (f64, f64) {
    let audio = facts.audio.as_ref();
    let measured = audio.and_then(|a| a.bytes).map(|b| b as f64);
    let guess = |codec: Codec| {
        (
            units::stream_bytes(nominal_kbps(codec), seconds),
            SIGMA_GUESS,
        )
    };
    let (bytes, sigma) = match format {
        Format::Copy { codec } => {
            measured.map_or_else(|| guess(codec), |b| (b * (1.0 + CONTAINER), SIGMA_COPY))
        }
        Format::Encode { codec, kbps: None } => {
            let lossless = audio.is_some_and(crate::facts::AudioFacts::is_lossless);
            let pcm = audio.is_some_and(|a| a.codec.starts_with("pcm_"));
            match measured.filter(|_| lossless) {
                Some(b) if pcm => (b * PCM_FLAC, SIGMA_GUESS),
                Some(b) => (b, SIGMA_GUESS),
                None => guess(codec),
            }
        }
        Format::Encode {
            kbps: Some(kbps), ..
        } => (
            units::stream_bytes(f64::from(kbps), seconds) * (1.0 + CONTAINER),
            SIGMA_ENCODE,
        ),
    };
    (bytes, bytes * sigma)
}

/// What a song's file is guessed to hold besides its audio, and its
/// lyrics file to take, from its plan.
fn besides_audio(plan: &Plan) -> (u64, u64) {
    let tags: usize = plan
        .tags
        .iter()
        .flat_map(|(k, values)| values.iter().map(move |v| k.len() + v.len() + 8))
        .sum();
    let cover = plan.cover.as_ref().map_or(0, |_| COVER_GUESS);
    let placement = plan.lyrics.as_ref().map(|l| l.placement);
    let embedded = placement
        .filter(|p| p.embedded())
        .map_or(0, |_| LYRICS_GUESS);
    let sidecar = placement
        .filter(|p| p.sidecar())
        .map_or(0, |_| LYRICS_GUESS);
    (tags as u64 + cover + embedded + HEADERS, sidecar)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ladder(
    manifest: &Manifest,
    state: &State,
    written: &ByAudio<'_>,
    song: usize,
    plan: &Plan,
    library: &Path,
    tools: &str,
) -> Ladder {
    let block = manifest.settings.library.block_size.0;
    let key = manifest.songs[song]
        .sources
        .first()
        .map_or_else(|| plan.audio.key.to_string(), ToString::to_string);
    let facts = state.facts.get(&plan.audio.key);
    let seconds = facts.and_then(|f| f.duration).filter(|d| *d > 0.0);
    let (extras, lyrics) = besides_audio(plan);
    let choice = |format: Format, loss: f64| -> Choice {
        let (audio, sigma) = match (facts, seconds) {
            (Some(f), Some(s)) => estimate(format, f, s),
            _ => (0.0, 0.0),
        };
        let plan = Plan {
            format,
            ..plan.clone()
        };
        let renamed = !state.rebased.is_empty() || !state.respelled.is_empty();
        let was = renamed.then(|| plan_key(tools, &state.as_before(&plan)));
        let plan_key = plan_key(tools, &plan);
        Choice {
            format,
            was_key: was.filter(|w| *w != plan_key),
            plan_key,
            estimate: blocks(audio as u64 + extras, block) + blocks(lyrics, block),
            sigma,
            loss,
        }
    };
    let fixed = written
        .get(&plan.audio)
        .into_iter()
        .flatten()
        .find(|(path, _)| changed_since_written(library, &state.outputs, path))
        .map(|(path, w)| on_disk(library, path, w, block));
    let mut ladder = Ladder {
        key,
        rungs: vec![choice(plan.format, 0.0)],
        fixed,
    };
    let (Some(seconds), Some(audio), None) =
        (seconds, facts.and_then(|f| f.audio.as_ref()), ladder.fixed)
    else {
        return ladder;
    };
    let source = Source {
        lossless: audio.is_lossless(),
        bandwidth_hz: audio.quality.map(|q| q.bandwidth_hz),
        seconds,
    };
    for format in fit::lower(plan.format, audio.channels, &manifest.settings.audio) {
        let lower = choice(format, fit::loss(&source, plan.format, format));
        if lower.estimate.saturating_mul(10) <= ladder.rungs[0].estimate.saturating_mul(9) {
            ladder.rungs.push(lower);
        }
    }
    ladder
}

/// What a file of the library takes on disk, its lyrics included.
fn on_disk(library: &Path, path: &Path, w: &Written, block: u64) -> u64 {
    let size = |p: &Path| store::stamp(&library.join(p)).map_or(0, |(size, _)| size);
    blocks(size(path), block) + w.lyrics.as_ref().map_or(0, |l| blocks(size(l), block))
}

/// What fitting has learnt in this run: the real sizes the passes asked
/// for, the rungs that could not be rendered, and the songs rendered to
/// be measured, by their place in the list and rung.
#[derive(Debug, Default)]
struct Learnt {
    known: HashMap<String, u64>,
    broken: BTreeSet<String>,
    made: HashMap<(usize, usize), (PathBuf, Rendered)>,
    rendered: usize,
}

impl Learnt {
    /// Learn the real sizes of the `unknown` rungs, by ladder and rung:
    /// from `state.sizes` where kept, else by rendering them.
    #[allow(clippy::too_many_arguments)]
    fn measure<R: Runner, W: Write>(
        &mut self,
        runner: &R,
        at: &Workshop<'_>,
        state: &mut State,
        planned: &Planned,
        ladders: &[Ladder],
        unknown: Vec<(usize, usize)>,
        block: u64,
        out: &mut W,
    ) -> Result<()> {
        let mut due = Vec::new();
        for (i, c) in unknown {
            let rung = &ladders[i].rungs[c];
            match state.sizes.get(&rung.plan_key) {
                Some(m) => {
                    let bytes = blocks(m.audio, block) + blocks(m.lyrics, block);
                    self.known.insert(rung.plan_key.clone(), bytes);
                }
                None => due.push((i, c)),
            }
        }
        if due.is_empty() {
            return Ok(());
        }
        crate::ui::info(
            out,
            &format!(
                "Rendering {} song(s) to measure them for max_size",
                due.len()
            ),
        )?;
        let results = parallel::map(&due, parallel::builds(), |(i, c)| {
            let (n, r) = &planned[*i];
            let plan = Plan {
                format: ladders[*i].rungs[*c].format,
                ..r.plan.clone()
            };
            let folder = at.scratch.join(format!("{n}-{c}"));
            let done = render::render(
                runner,
                &render::Job {
                    plan: &plan,
                    stem: &r.stem,
                    library: &folder.join("library"),
                    sources: at.sources,
                    scratch: &folder.join("work"),
                },
            );
            (folder.join("library"), done)
        });
        for ((i, c), (folder, result)) in due.into_iter().zip(results) {
            let key = ladders[i].rungs[c].plan_key.clone();
            let Ok(done) = result else {
                self.broken.insert(key);
                continue;
            };
            self.rendered += 1;
            let bytes = blocks(done.audio_bytes, block) + blocks(done.lyrics_bytes, block);
            self.known.insert(key.clone(), bytes);
            let measured = Measured {
                source: planned[i].1.plan.audio.key.clone(),
                audio: done.audio_bytes,
                lyrics: done.lyrics_bytes,
            };
            state.sizes.insert(key, measured);
            self.made.insert((planned[i].0, c), (folder, done));
        }
        Ok(())
    }
}

/// The items to fit: each ladder's rungs at their real sizes where they
/// are learnt, else at their estimates; a lower rung that could not be
/// rendered saves nothing.
fn items(ladders: &[Ladder], learnt: &Learnt) -> Vec<Item> {
    ladders
        .iter()
        .map(|l| {
            let rungs = match l.fixed {
                Some(bytes) => vec![Rung {
                    format: l.rungs[0].format,
                    bytes,
                    loss: 0.0,
                    sigma: 0.0,
                }],
                None => l
                    .rungs
                    .iter()
                    .enumerate()
                    .map(|(n, c)| {
                        let (bytes, sigma) = match learnt.known.get(&c.plan_key) {
                            Some(b) => (*b, 0.0),
                            None if n > 0 && learnt.broken.contains(&c.plan_key) => {
                                (l.rungs[0].estimate.saturating_mul(2), 0.0)
                            }
                            None => (c.estimate, c.sigma),
                        };
                        Rung {
                            format: c.format,
                            bytes,
                            loss: c.loss,
                            sigma,
                        }
                    })
                    .collect(),
            };
            Item {
                key: l.key.clone(),
                rungs,
                churn: 0.0,
            }
        })
        .collect()
}

/// What a pass of fitting leads to.
enum Pass {
    /// Every song has a rung.
    Fit(Fit),
    /// The songs do not fit, and these lowest rungs, by ladder and rung,
    /// must be measured before any song is left out.
    Measure(Vec<(usize, usize)>),
}

/// Fit `ladders`; when they cannot fit, measure each one's lowest rung
/// first, and only once the lowest are all real sizes leave out songs,
/// the last listed first, until the rest fit.
fn fit_or_leave_out(
    ladders: &mut Vec<Ladder>,
    planned: &mut Planned,
    learnt: &Learnt,
    policy: &Policy,
    left_out: &mut Vec<usize>,
    floor: &mut Option<u64>,
) -> Pass {
    loop {
        let now = items(ladders, learnt);
        let at_least = match fit::allocate(&now, policy) {
            Ok(fit) => return Pass::Fit(fit),
            Err(at_least) => at_least,
        };
        let lowest = |i: &Item| {
            i.rungs
                .iter()
                .enumerate()
                .min_by_key(|(_, r)| r.bytes)
                .map_or(0, |(n, _)| n)
        };
        let unknown: Vec<(usize, usize)> = now
            .iter()
            .enumerate()
            .filter(|(i, _)| ladders[*i].fixed.is_none())
            .map(|(i, item)| (i, lowest(item)))
            .filter(|(i, c)| {
                let key = &ladders[*i].rungs[*c].plan_key;
                !learnt.known.contains_key(key) && !learnt.broken.contains(key)
            })
            .collect();
        if !unknown.is_empty() {
            return Pass::Measure(unknown);
        }
        *floor = Some(at_least);
        let least: Vec<u64> = now
            .iter()
            .map(|i| i.rungs.iter().map(|r| r.bytes).min().unwrap_or(0))
            .collect();
        let keep = kept_of(&least, policy.max);
        left_out.extend(planned[keep..].iter().map(|(n, _)| *n));
        ladders.truncate(keep);
        planned.truncate(keep);
    }
}

/// For passes that ran out with the last fit made on estimates, as
/// encoders missing their bitrates keep them moving: a fit on sizes
/// measured, each song's lowest rendered first if that is what it takes.
#[allow(clippy::too_many_arguments)]
fn settle_measured<R: Runner, W: Write>(
    runner: &R,
    at: &Workshop<'_>,
    state: &mut State,
    planned: &Planned,
    ladders: &[Ladder],
    learnt: &mut Learnt,
    policy: &Policy,
    block: u64,
    out: &mut W,
) -> Result<Option<fit::Fit>> {
    if let Some(known) = settle(ladders, learnt, policy) {
        return Ok(Some(known));
    }
    let lowest: Vec<(usize, usize)> = ladders
        .iter()
        .enumerate()
        .filter(|(_, l)| l.fixed.is_none())
        .filter_map(|(i, l)| {
            let (c, rung) = l.rungs.iter().enumerate().min_by_key(|(_, r)| r.estimate)?;
            let known =
                learnt.known.contains_key(&rung.plan_key) || learnt.broken.contains(&rung.plan_key);
            (!known).then_some((i, c))
        })
        .collect();
    learnt.measure(runner, at, state, planned, ladders, lowest, block, out)?;
    Ok(settle(ladders, learnt, policy))
}

/// The rungs of `fit` whose size is still an estimate.
fn unknown_in(fit: &fit::Fit, ladders: &[Ladder], learnt: &Learnt) -> Vec<(usize, usize)> {
    fit.choice
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            let rung = &ladders[*i].rungs[**c];
            ladders[*i].fixed.is_none()
                && !learnt.known.contains_key(&rung.plan_key)
                && !learnt.broken.contains(&rung.plan_key)
        })
        .map(|(i, c)| (i, *c))
        .collect()
}

/// A fit of sizes measured: each song lowered only to a rung whose real
/// size is known, or kept at its best, estimated as a copy is closely.
fn settle(ladders: &[Ladder], learnt: &Learnt, policy: &Policy) -> Option<fit::Fit> {
    let mut items = items(ladders, learnt);
    for (item, ladder) in items.iter_mut().zip(ladders) {
        if ladder.fixed.is_some() {
            continue;
        }
        for (rung, choice) in item.rungs.iter_mut().zip(&ladder.rungs).skip(1) {
            if !learnt.known.contains_key(&choice.plan_key) {
                rung.bytes = policy.max.saturating_add(1);
                rung.sigma = 0.0;
            }
        }
    }
    fit::allocate(&items, policy).ok()
}

/// How many of songs taking at least `least` bytes each, the first
/// kept longest, may stay within `max`; fewer than all, as it is asked
/// only once they do not fit: the margin allocating adds can refuse
/// songs whose plain sizes fit, and leaving none out would ask again for
/// ever.
fn kept_of(least: &[u64], max: u64) -> usize {
    let mut total: u64 = least.iter().sum();
    let mut keep = least.len();
    while keep > 0 && (total > max || keep == least.len()) {
        keep -= 1;
        total -= least[keep];
    }
    keep
}

/// Where fitting renders, and from what.
#[derive(Debug, Clone, Copy)]
pub struct Workshop<'a> {
    pub sources: &'a BTreeMap<SourceKey, Located>,
    /// An empty folder of fitting's own.
    pub scratch: &'a Path,
    pub library: &'a Path,
    /// What files no plan makes take and keep: a failed song's.
    pub kept: u64,
}

/// Fit the planned songs into `[library] max_size`, setting each one's
/// format and dropping from `planned` the songs left out, as described
/// above. Real sizes rendered go into `state.sizes`. None when no limit
/// is set.
pub fn fit_library<R: Runner, W: Write>(
    runner: &R,
    manifest: &Manifest,
    state: &mut State,
    planned: &mut Planned,
    at: &Workshop<'_>,
    out: &mut W,
) -> Result<Option<(Fitted, Made)>> {
    let Some(max) = manifest.settings.library.max_size else {
        return Ok(None);
    };
    let max = max.0.saturating_sub(at.kept);
    let block = manifest.settings.library.block_size.0;
    let tools = tools(runner);
    let written = by_audio(state);
    let mut ladders: Vec<Ladder> = planned
        .iter()
        .map(|(n, r)| ladder(manifest, state, &written, *n, &r.plan, at.library, &tools))
        .collect();
    drop(written);
    for rung in ladders.iter().flat_map(|l| &l.rungs) {
        if let Some(was) = rung.was_key.as_ref().and_then(|k| state.sizes.get(k))
            && !state.sizes.contains_key(&rung.plan_key)
        {
            state.sizes.insert(rung.plan_key.clone(), was.clone());
        }
    }
    let keys: BTreeSet<String> = ladders
        .iter()
        .flat_map(|l| l.rungs.iter().map(|c| c.plan_key.clone()))
        .collect();
    let policy = Policy {
        max,
        sigmas: SIGMAS,
    };
    let mut learnt = Learnt::default();
    let mut left_out = Vec::new();
    let mut floor = None;
    let mut fit = None;
    let mut settled = false;
    for _ in 0..PASSES {
        let unknown = match fit_or_leave_out(
            &mut ladders,
            planned,
            &learnt,
            &policy,
            &mut left_out,
            &mut floor,
        ) {
            Pass::Measure(lowest) => lowest,
            Pass::Fit(chosen) => {
                let unknown = unknown_in(&chosen, &ladders, &learnt);
                fit = Some(chosen);
                if unknown.is_empty() {
                    settled = true;
                    break;
                }
                unknown
            }
        };
        learnt.measure(runner, at, state, planned, &ladders, unknown, block, out)?;
    }
    if !settled
        && let Some(known) = settle_measured(
            runner,
            at,
            state,
            planned,
            &ladders,
            &mut learnt,
            &policy,
            block,
            out,
        )?
    {
        fit = Some(known);
    }
    let mut placed: Made = HashMap::new();
    let mut lowered = 0;
    if let Some(fit) = &fit {
        for ((n, r), (ladder, c)) in planned.iter_mut().zip(ladders.iter().zip(&fit.choice)) {
            r.plan.format = ladder.rungs[*c].format;
            if let Some(file) = learnt.made.remove(&(*n, *c)) {
                placed.insert(*n, file);
            }
            if *c != 0 {
                lowered += 1;
                r.why.fit = Some(format!(
                    "{} to fit max_size; at best {}",
                    ladder.rungs[*c].format.describe(),
                    ladder.rungs[0].format.describe()
                ));
            }
        }
    }
    left_out.sort_unstable();
    let fitted = Fitted {
        max,
        projected: fit.map_or(0, |f| f.total),
        left_out,
        floor,
        lowered,
        rendered: learnt.rendered,
        tools,
        keys,
    };
    Ok(Some((fitted, placed)))
}

/// Give each planned song the format its file was last written in, when
/// it differs from its best only by fitting: what `list` and `info` show
/// is the library as it stands.
pub fn as_written(manifest: &Manifest, state: &State, planned: &mut Planned) {
    if manifest.settings.library.max_size.is_none() {
        return;
    }
    let written = by_audio(state);
    for (_, r) in planned.iter_mut() {
        let fitted = written
            .get(&r.plan.audio)
            .into_iter()
            .flatten()
            .find_map(|(_, w)| {
                let plan = w.plan.as_ref()?;
                let same = Plan {
                    format: r.plan.format,
                    ..plan.clone()
                };
                (same == r.plan).then_some(plan.format)
            });
        if let Some(format) = fitted {
            r.plan.format = format;
        }
    }
}

/// What the files a failed song keeps take: each file written from a
/// source in `failed`.
#[must_use]
pub fn kept(state: &State, library: &Path, failed: &BTreeSet<SourceKey>, block: u64) -> u64 {
    state
        .outputs
        .iter()
        .filter(|(_, w)| w.sources.iter().any(|k| failed.contains(k)))
        .map(|(path, w)| on_disk(library, path, w, block))
        .sum()
}

/// What the library's files take, each in whole blocks.
#[must_use]
pub fn library_size(state: &State, library: &Path, block: u64) -> u64 {
    state
        .outputs
        .iter()
        .map(|(path, w)| on_disk(library, path, w, block))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaving_songs_out_leaves_out_one_at_least() {
        assert_eq!(kept_of(&[10, 10, 10], 25), 2);
        assert_eq!(kept_of(&[10, 10, 10], 15), 1);
        assert_eq!(
            kept_of(&[10, 10, 10], 30),
            2,
            "refused with the margin, though the sizes alone fit"
        );
        assert_eq!(kept_of(&[], 30), 0);
    }

    #[test]
    fn a_file_takes_whole_blocks() {
        assert_eq!(blocks(1, 4096), 4096);
        assert_eq!(blocks(4096, 4096), 4096);
        assert_eq!(blocks(4097, 4096), 8192);
        assert_eq!(blocks(0, 4096), 0);
    }

    #[test]
    fn a_plan_s_key_changes_with_the_plan_and_the_tools() {
        let plan = |kbps| Plan {
            version: crate::resolve::RENDER_VERSION,
            format: Format::Encode {
                codec: Codec::Opus,
                kbps: Some(kbps),
            },
            audio: AudioRef {
                key: SourceKey::youtube("vid00000001"),
                rev: "r".into(),
                index: 1,
            },
            cover: None,
            lyrics: None,
            tags: Vec::new(),
        };
        let key = plan_key("ffmpeg version 7", &plan(96));
        assert_eq!(key, plan_key("ffmpeg version 7", &plan(96)));
        assert_ne!(key, plan_key("ffmpeg version 7", &plan(80)));
        assert_ne!(key, plan_key("ffmpeg version 8", &plan(96)));
    }
}
