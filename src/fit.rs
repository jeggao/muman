//! Songs fitted into a size: each song may be written in one of a few
//! formats, its ladder, from the one it is written in now down to lower
//! bitrates, and songs are lowered, the least audible loss for each byte
//! saved first, until their total fits. `export --max-size` fits a zip
//! this way, and [`crate::limit`] the library under `[library]
//! max_size`.
//!
//! **Ladder.** The first rung is the song as it stands. Below it come
//! `[audio] lossy` at the bitrates of [`ladder`] under the one it would
//! be encoded at, raised in proportion past two channels as `[audio]`
//! raises it. A rung must save a tenth of the first: a lossy source is
//! never encoded at or above its own bitrate, which would cost a
//! generation and save nothing.
//!
//! **Loss.** In impairment points per minute of audio, 0 transparent and
//! 100 unusable, read off `impairment` for the codec at its bitrate,
//! between the listed bitrates by the logarithm of the bitrate. A source
//! whose bandwidth was already cut has less to lose, so its bitrate counts
//! for more, up to half again for one cut at 9 kHz. Encoding lossy audio
//! again compounds both codecs' artifacts: its loss counts half again,
//! and a generation costs half a point however high the bitrate. Only the
//! loss a rung adds to the first matters, and as loss and bytes both grow
//! with length, a song is weighed by the minute. The tables follow the
//! shape of public listening tests, Opus transparent well below AAC and
//! Vorbis, MP3 last; they rank choices and claim no absolute score.
//!
//! **Allocation.** Each song's rungs become points of bytes saved against
//! loss added, plus its churn for leaving the first rung: the cost of
//! encoding, which makes a song that moves at all move further rather
//! than many songs move a little. Only the points on their lower convex
//! hull are worth taking. Every song's next hull segment waits in a heap
//! by its loss per byte saved, and the cheapest is taken until the total
//! fits: a Lagrangian sweep, `O(n log n)` over every rung of `n` songs, with
//! no encoding to estimate. Ties go by key, so the songs' order changes
//! nothing.
//!
//! **Raising.** The step that makes the total fit can free more than was
//! missing, and songs lowered before it, at less loss per byte or at
//! none, would then stay lowered with room left that could hold them. So
//! once the total fits, each song lowered is raised back to the best rung
//! that still fits: the songs whose last step lost the most for each byte
//! saved first, then by key. As the total only grows while songs are
//! raised, no song is left that could be written at any rung above its
//! own within the limit. A rung that saves nothing over the first is
//! never raised to.
//!
//! **Settling.** A rung's size is an estimate until it is made and
//! measured, and estimates miss: a VBR encoder spends more on some music
//! than its bitrate. [`settle`] is the one loop that turns a fit on
//! estimates into a fit on sizes measured, for `export --max-size` and
//! `[library] max_size` alike: fit, have the caller measure each rung
//! chosen that is still an estimate, fit again, until every rung chosen is
//! measured, at most `passes` times. When the passes run out with
//! estimates still chosen, as estimates corrected by what was measured
//! keep moving, the items are fitted on measured sizes alone, a lower rung
//! not measured counting as more than the whole room; and when that does
//! not fit, each item's lowest rung is measured and it is fitted again.
//! Items that cannot fit even at their least have their lowest rungs
//! measured before the fit gives up, so no estimate decides that. A rung
//! that could not be made saves nothing from then on. The fit returned is
//! thereby always one of sizes measured, or of a first rung, which is the
//! song as it stands.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::codec::{Codec, Shape};
use crate::resolve::Format;
use crate::settings::Audio;

/// One format a song may be written in, what it takes and what it loses.
#[derive(Debug, Clone, PartialEq)]
pub struct Rung {
    pub format: Format,
    pub bytes: u64,
    /// Impairment added to the first rung's, in points times minutes.
    pub loss: f64,
    /// How far `bytes` may be off, one standard deviation; none for a
    /// size known.
    pub sigma: f64,
    /// Whether `bytes` was measured rather than estimated.
    pub known: bool,
}

/// One song to fit: its rungs, the first the best it can be written as.
#[derive(Debug, Clone)]
pub struct Item {
    /// Breaks ties between equal choices.
    pub key: String,
    pub rungs: Vec<Rung>,
    /// What leaving the first rung costs, as loss.
    pub churn: f64,
}

/// The rung each item is written at and what they take together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fit {
    pub choice: Vec<usize>,
    pub total: u64,
}

/// The bitrates a codec is lowered through, in kbit/s for two channels.
#[must_use]
pub fn ladder(codec: Codec) -> &'static [u32] {
    match codec {
        Codec::Opus => &[256, 192, 160, 128, 112, 96, 80, 64, 56, 48, 40, 32],
        Codec::Aac | Codec::Vorbis => &[256, 192, 160, 128, 112, 96, 80, 64],
        Codec::Mp3 => &[320, 256, 224, 192, 160, 128, 112, 96],
        Codec::Flac | Codec::Alac | Codec::WavPack => &[],
    }
}

/// Impairment per minute of stereo, full-band music encoded by muman's
/// encoder for a codec, by bitrate in kbit/s, highest first.
fn impairment(codec: Codec) -> &'static [(f64, f64)] {
    match codec {
        Codec::Opus => &[
            (320.0, 0.0),
            (192.0, 0.0),
            (128.0, 0.5),
            (112.0, 1.0),
            (96.0, 2.5),
            (80.0, 5.0),
            (64.0, 9.0),
            (48.0, 16.0),
            (40.0, 22.0),
            (32.0, 30.0),
        ],
        Codec::Aac => &[
            (320.0, 0.0),
            (256.0, 0.5),
            (192.0, 2.0),
            (160.0, 3.5),
            (128.0, 6.0),
            (112.0, 8.0),
            (96.0, 12.0),
            (80.0, 17.0),
            (64.0, 24.0),
        ],
        Codec::Vorbis => &[
            (320.0, 0.0),
            (256.0, 0.0),
            (192.0, 1.0),
            (160.0, 2.0),
            (128.0, 3.5),
            (112.0, 5.0),
            (96.0, 7.0),
            (80.0, 11.0),
            (64.0, 17.0),
        ],
        Codec::Mp3 => &[
            (320.0, 0.5),
            (256.0, 1.0),
            (192.0, 3.0),
            (160.0, 5.0),
            (128.0, 9.0),
            (112.0, 13.0),
            (96.0, 19.0),
            (80.0, 26.0),
            (64.0, 35.0),
        ],
        Codec::Flac | Codec::Alac | Codec::WavPack => &[(1.0, 0.0)],
    }
}

/// [`impairment`] at `kbps`, between rows by the bitrate's logarithm and
/// past the last row along its slope.
fn impairment_at(codec: Codec, kbps: f64) -> f64 {
    let table = impairment(codec);
    let (top, bottom) = (table[0], table[table.len() - 1]);
    if kbps >= top.0 || table.len() == 1 {
        return top.1;
    }
    let pair = table
        .windows(2)
        .find(|w| kbps >= w[1].0)
        .map_or((table[table.len() - 2], bottom), |w| (w[0], w[1]));
    let ((hi, at_hi), (lo, at_lo)) = pair;
    let t = (hi.ln() - kbps.ln()) / (hi.ln() - lo.ln());
    (at_hi + t * (at_lo - at_hi)).max(0.0)
}

/// What a song's audio is, as fitting weighs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Source {
    pub lossless: bool,
    /// Where a lowpass cuts it off, if measured.
    pub bandwidth_hz: Option<f64>,
    pub seconds: f64,
}

/// How much less a source cut at `bandwidth_hz` loses at a bitrate.
fn bandwidth_factor(bandwidth_hz: Option<f64>) -> f64 {
    bandwidth_hz.map_or(1.0, |hz| (20_000.0 / hz.max(1.0)).sqrt().clamp(1.0, 1.5))
}

/// The loss, in points per minute, of `format` over the source as it is.
fn per_minute(source: &Source, format: Format) -> f64 {
    let Format::Encode {
        codec,
        kbps: Some(kbps),
        ..
    } = format
    else {
        return 0.0;
    };
    let base = impairment_at(
        codec,
        f64::from(kbps) * bandwidth_factor(source.bandwidth_hz),
    );
    if source.lossless {
        base
    } else {
        1.5 * base + 0.5
    }
}

/// The loss of writing `source` as `format` rather than as `first`, in
/// points times minutes.
#[must_use]
pub fn loss(source: &Source, first: Format, format: Format) -> f64 {
    let minutes = crate::units::minutes_of_seconds(source.seconds);
    ((per_minute(source, format) - per_minute(source, first)) * minutes).max(0.0)
}

/// The formats below `first` a song may be lowered to: `[audio] lossy`
/// at each bitrate of its ladder under the one `first` encodes at, and
/// at or above `[audio] min_bitrate`.
#[must_use]
pub fn lower(first: Format, shape: &Shape<'_>, audio: &Audio) -> Vec<Format> {
    let codec = audio.lossy;
    let (mix, shape) = audio.written(shape);
    let channels = shape.channels;
    let adapt = codec.adapt(&shape).flatten();
    let scale = |kbps: u32| match (audio.kbps(codec, channels), audio.kbps(codec, 2)) {
        (Some(mine), Some(two)) if two > 0 => kbps * mine / two,
        _ => kbps,
    };
    let below = match first {
        Format::Encode {
            codec: c,
            kbps: Some(k),
            ..
        } if c == codec => k,
        _ => u32::MAX,
    };
    ladder(codec)
        .iter()
        .filter(|k| audio.min_kbps.is_none_or(|min| **k >= min))
        .map(|k| scale(*k))
        .filter(|k| *k < below)
        .map(|kbps| Format::Encode {
            codec,
            kbps: Some(kbps),
            adapt,
            mix,
        })
        .collect()
}

/// How far fitting goes: the most the items may take, and the margin
/// kept for estimates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    pub max: u64,
    /// How many standard deviations of the summed estimates are kept free.
    pub sigmas: f64,
}

impl Policy {
    /// Fit into `max` exactly, on sizes taken as they are.
    #[must_use]
    pub fn exact(max: u64) -> Self {
        Self { max, sigmas: 0.0 }
    }
}

/// The items' bytes and their estimates' variance, as rungs are chosen.
#[derive(Debug, Clone, Copy, Default)]
struct Sum {
    bytes: u64,
    variance: f64,
}

impl Sum {
    fn of<'a>(rungs: impl Iterator<Item = &'a Rung>) -> Self {
        rungs.fold(Self::default(), Self::with)
    }

    fn with(self, r: &Rung) -> Self {
        Self {
            bytes: self.bytes + r.bytes,
            variance: self.variance + r.sigma * r.sigma,
        }
    }

    fn moved(self, from: &Rung, to: &Rung) -> Self {
        Self {
            bytes: self.bytes - from.bytes + to.bytes,
            variance: (self.variance - from.sigma * from.sigma + to.sigma * to.sigma).max(0.0),
        }
    }

    /// The bytes, with the margin `sigmas` standard deviations make.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn projected(self, sigmas: f64) -> u64 {
        self.bytes + (sigmas * self.variance.sqrt()).ceil() as u64
    }
}

/// A point of a song's hull: the rung, bytes saved and loss added.
#[derive(Debug, Clone, Copy)]
struct Point {
    rung: usize,
    saved: f64,
    cost: f64,
}

/// The rungs worth lowering an item through from its first, in order:
/// the lower convex hull of what each saves against the loss and churn it
/// adds, the first rung included.
#[allow(clippy::cast_precision_loss)]
fn hull(item: &Item) -> Vec<Point> {
    let first = item.rungs[0].bytes;
    let mut points: Vec<Point> = item
        .rungs
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, r)| r.bytes < first)
        .map(|(n, r)| Point {
            rung: n,
            saved: (first - r.bytes) as f64,
            cost: r.loss + item.churn,
        })
        .collect();
    points.sort_by(|a, b| a.saved.total_cmp(&b.saved).then(a.cost.total_cmp(&b.cost)));
    let mut hull = vec![Point {
        rung: 0,
        saved: 0.0,
        cost: 0.0,
    }];
    for p in points {
        while hull.len() >= 2 {
            let (o, a) = (hull[hull.len() - 2], hull[hull.len() - 1]);
            let turn =
                (a.saved - o.saved) * (p.cost - o.cost) - (a.cost - o.cost) * (p.saved - o.saved);
            if turn > 0.0 {
                break;
            }
            hull.pop();
        }
        if p.saved > hull[hull.len() - 1].saved {
            hull.push(p);
        }
    }
    hull
}

/// One song's next hull segment, by its loss per byte saved.
#[derive(Debug)]
struct Step<'a> {
    slope: f64,
    key: &'a str,
    item: usize,
    to: usize,
}

impl PartialEq for Step<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Step<'_> {}

impl PartialOrd for Step<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Reversed, so the heap yields the cheapest step first.
impl Ord for Step<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .slope
            .total_cmp(&self.slope)
            .then_with(|| other.key.cmp(self.key))
    }
}

fn step<'a>(items: &'a [Item], hulls: &[Vec<Point>], item: usize, at: usize) -> Option<Step<'a>> {
    let (from, to) = (hulls[item].get(at)?, hulls[item].get(at + 1)?);
    Some(Step {
        slope: (to.cost - from.cost) / (to.saved - from.saved),
        key: &items[item].key,
        item,
        to: at + 1,
    })
}

/// The rung an item takes least at.
fn least(item: &Item) -> &Rung {
    item.rungs
        .iter()
        .min_by_key(|r| r.bytes)
        .unwrap_or(&item.rungs[0])
}

/// The rung of each item that fits them all under `policy` for the least
/// loss, every item starting at its first: a function of the items
/// alone, whatever order they come in. `Err` with what they take at their
/// least when even that is over `max`.
pub fn allocate(items: &[Item], policy: &Policy) -> Result<Fit, u64> {
    let mut choice = vec![0; items.len()];
    let mut sum = Sum::of(items.iter().map(|i| &i.rungs[0]));
    let projected = |s: Sum| s.projected(policy.sigmas);
    if projected(sum) <= policy.max {
        return Ok(Fit {
            choice,
            total: projected(sum),
        });
    }
    let floor = projected(Sum::of(items.iter().map(least)));
    if floor > policy.max {
        return Err(floor);
    }
    let hulls: Vec<Vec<Point>> = items.iter().map(hull).collect();
    let mut heap: BinaryHeap<Step<'_>> = (0..items.len())
        .filter_map(|i| step(items, &hulls, i, 0))
        .collect();
    let mut last = vec![0.0; items.len()];
    while projected(sum) > policy.max {
        let Some(next) = heap.pop() else {
            break;
        };
        let rungs = &items[next.item].rungs;
        let now = hulls[next.item][next.to].rung;
        sum = sum.moved(&rungs[choice[next.item]], &rungs[now]);
        choice[next.item] = now;
        last[next.item] = next.slope;
        heap.extend(step(items, &hulls, next.item, next.to));
    }
    if projected(sum) > policy.max {
        return Err(floor);
    }
    raise(items, policy, &last, &mut choice, &mut sum);
    Ok(Fit {
        choice,
        total: projected(sum),
    })
}

/// Raise each item the sweep lowered back to the best rung that still
/// fits, as the module docs say: those whose `last` step lost the most
/// for each byte saved first, ties by key.
fn raise(items: &[Item], policy: &Policy, last: &[f64], choice: &mut [usize], sum: &mut Sum) {
    let mut lowered: Vec<usize> = (0..items.len()).filter(|i| choice[*i] > 0).collect();
    lowered.sort_by(|a, b| {
        last[*b]
            .total_cmp(&last[*a])
            .then_with(|| items[*a].key.cmp(&items[*b].key))
    });
    for i in lowered {
        let rungs = &items[i].rungs;
        let from = &rungs[choice[i]];
        let better = (0..choice[i]).find(|c| {
            (*c == 0 || rungs[*c].bytes < rungs[0].bytes)
                && sum.moved(from, &rungs[*c]).projected(policy.sigmas) <= policy.max
        });
        if let Some(c) = better {
            *sum = sum.moved(from, &rungs[c]);
            choice[i] = c;
        }
    }
}

/// How [`settle`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// Every item has a rung, each lowered one measured.
    Fit(Fit),
    /// Even every item's lowest rung, measured, takes more than the room:
    /// what they take at their least.
    Over(u64),
}

/// Rungs to measure, by item and rung.
pub type Wanted = [(usize, usize)];

/// Fit `items` under `policy` on sizes measured, as the module docs say:
/// `measure` is asked for the rungs whose sizes must be known, and sets
/// each it can in `items`, `known` with its real bytes.
pub fn settle(
    items: &mut [Item],
    policy: &Policy,
    passes: usize,
    measure: &mut dyn FnMut(&mut [Item], &Wanted) -> anyhow::Result<()>,
) -> anyhow::Result<Settled> {
    for _ in 0..passes {
        let wanted = match allocate(items, policy) {
            Ok(fit) => {
                let unknown: Vec<(usize, usize)> = fit
                    .choice
                    .iter()
                    .enumerate()
                    .filter(|(i, c)| !items[*i].rungs[**c].known)
                    .map(|(i, c)| (i, *c))
                    .collect();
                if unknown.is_empty() {
                    return Ok(Settled::Fit(fit));
                }
                unknown
            }
            Err(floor) => {
                let lowest = lowest_unknown(items);
                if lowest.is_empty() {
                    return Ok(Settled::Over(floor));
                }
                lowest
            }
        };
        learn(items, &wanted, measure)?;
    }
    if let Some(fit) = on_measured(items, policy) {
        return Ok(Settled::Fit(fit));
    }
    let lowest = lowest_unknown(items);
    learn(items, &lowest, measure)?;
    match on_measured(items, policy) {
        Some(fit) => Ok(Settled::Fit(fit)),
        None => Ok(Settled::Over(
            Sum::of(items.iter().map(least)).projected(policy.sigmas),
        )),
    }
}

/// Have `measure` learn `wanted`; a rung it could not make saves nothing
/// from then on.
fn learn(
    items: &mut [Item],
    wanted: &Wanted,
    measure: &mut dyn FnMut(&mut [Item], &Wanted) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    measure(items, wanted)?;
    for &(i, c) in wanted {
        let first = items[i].rungs[0].bytes;
        let rung = &mut items[i].rungs[c];
        if !rung.known {
            if c > 0 {
                rung.bytes = first;
            }
            rung.sigma = 0.0;
            rung.known = true;
        }
    }
    Ok(())
}

/// Each item's lowest rung, where it is still an estimate.
fn lowest_unknown(items: &[Item]) -> Vec<(usize, usize)> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let (c, rung) = item.rungs.iter().enumerate().min_by_key(|(_, r)| r.bytes)?;
            (!rung.known).then_some((i, c))
        })
        .collect()
}

/// A fit of sizes measured: each item lowered only to a rung measured, or
/// kept at its first.
fn on_measured(items: &[Item], policy: &Policy) -> Option<Fit> {
    let known: Vec<Item> = items
        .iter()
        .map(|item| {
            let mut item = item.clone();
            for rung in item.rungs.iter_mut().skip(1).filter(|r| !r.known) {
                rung.bytes = policy.max.saturating_add(1);
                rung.sigma = 0.0;
            }
            item
        })
        .collect();
    allocate(&known, policy).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEREO: Shape<'static> = Shape {
        channels: 2,
        layout: Some("stereo"),
        sample_rate: 48_000,
        bits: 16,
        float: false,
    };

    fn opus(kbps: u32) -> Format {
        Format::Encode {
            codec: Codec::Opus,
            kbps: Some(kbps),
            adapt: None,
            mix: None,
        }
    }

    /// A song of `seconds` as `first`, which takes `first_bytes`, with
    /// the Opus ladder below it.
    fn song(key: &str, first: Format, first_bytes: u64, lossless: bool, seconds: f64) -> Item {
        let source = Source {
            lossless,
            bandwidth_hz: Some(20_000.0),
            seconds,
        };
        let mut rungs = vec![Rung {
            format: first,
            bytes: first_bytes,
            loss: 0.0,
            sigma: 0.0,
            known: true,
        }];
        for format in lower(first, &STEREO, &Audio::default()) {
            let Format::Encode { kbps: Some(k), .. } = format else {
                unreachable!()
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let bytes = (f64::from(k) * 125.0 * seconds) as u64;
            if bytes * 10 <= first_bytes * 9 {
                rungs.push(Rung {
                    format,
                    bytes,
                    loss: loss(&source, first, format),
                    sigma: 0.0,
                    known: false,
                });
            }
        }
        Item {
            key: key.to_string(),
            rungs,
            churn: 0.1,
        }
    }

    const MB: u64 = 1_000_000;

    #[test]
    fn what_fits_already_moves_nothing() {
        let items = [song("a", opus(160), 4 * MB, false, 200.0)];
        assert_eq!(
            allocate(&items, &Policy::exact(4 * MB)),
            Ok(Fit {
                choice: vec![0],
                total: 4 * MB
            })
        );
    }

    #[test]
    fn lossless_songs_are_lowered_before_lossy_ones() {
        let flac = Format::Copy { codec: Codec::Flac };
        let copy = Format::Copy { codec: Codec::Opus };
        let items = [
            song("a", copy, 3_200_000, false, 200.0),
            song("b", flac, 25 * MB, true, 200.0),
            song("c", copy, 3_200_000, false, 200.0),
        ];
        let fit = allocate(&items, &Policy::exact(12 * MB)).unwrap();
        assert_eq!(fit.choice[0], 0);
        assert_eq!(fit.choice[2], 0);
        assert_eq!(
            items[1].rungs[fit.choice[1]].format,
            opus(192),
            "the lowest bitrate still transparent"
        );
        assert!(fit.total <= 12 * MB);
    }

    #[test]
    fn room_left_once_fitted_raises_songs_back() {
        let flac = Format::Copy { codec: Codec::Flac };
        let items: Vec<Item> = [25.0, 30.0, 35.0, 40.0, 45.0, 60.0]
            .into_iter()
            .enumerate()
            .map(|(n, seconds)| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let bytes = (seconds * 125_000.0) as u64;
                song(&format!("s{n}"), flac, bytes, true, seconds)
            })
            .collect();
        let max = 15_700_000;
        let fit = allocate(&items, &Policy::exact(max)).unwrap();
        assert!(fit.total <= max, "{}", fit.total);
        assert_eq!(fit.choice[0], 0, "the first fits at its best: {fit:?}");
        for (i, c) in fit.choice.iter().enumerate() {
            let rungs = &items[i].rungs;
            for higher in 0..*c {
                let raised = fit.total - rungs[*c].bytes + rungs[higher].bytes;
                assert!(raised > max, "song {i} fits at rung {higher}: {fit:?}");
            }
        }
    }

    #[test]
    fn a_tight_budget_lowers_every_song_and_still_fits() {
        let items: Vec<Item> = (0..40)
            .map(|n| song(&format!("s{n:02}"), opus(160), 4 * MB, false, 200.0))
            .collect();
        let fit = allocate(&items, &Policy::exact(40 * 2 * MB)).unwrap();
        assert!(fit.total <= 80 * MB, "{}", fit.total);
        let lowest = fit.choice.iter().min().unwrap();
        let highest = fit.choice.iter().max().unwrap();
        assert!(highest - lowest <= 2, "spread evenly: {:?}", fit.choice);
    }

    #[test]
    fn the_order_of_the_songs_changes_nothing() {
        let mut items: Vec<Item> = (0..12)
            .map(|n| {
                let seconds = 120.0 + 15.0 * f64::from(n);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let bytes = (seconds * 20_000.0) as u64;
                song(&format!("s{n:02}"), opus(160), bytes, n % 3 == 0, seconds)
            })
            .collect();
        let budget = items.iter().map(|i| i.rungs[0].bytes).sum::<u64>() * 2 / 3;
        let before = allocate(&items, &Policy::exact(budget)).unwrap();
        let picked = |items: &[Item], fit: &Fit| {
            let mut picked: Vec<(String, usize)> = items
                .iter()
                .zip(&fit.choice)
                .map(|(i, c)| (i.key.clone(), *c))
                .collect();
            picked.sort();
            picked
        };
        let first = picked(&items, &before);
        items.reverse();
        let after = allocate(&items, &Policy::exact(budget)).unwrap();
        assert_eq!(picked(&items, &after), first);
    }

    #[test]
    fn a_budget_below_the_floor_says_the_floor() {
        let items = [song("a", opus(160), 4 * MB, false, 200.0)];
        let floor = items[0].rungs.iter().map(|r| r.bytes).min().unwrap();
        assert_eq!(allocate(&items, &Policy::exact(MB / 2)), Err(floor));
    }

    #[test]
    fn a_lossy_source_is_never_raised_or_kept_at_its_bitrate() {
        let item = song(
            "a",
            Format::Copy { codec: Codec::Opus },
            3_200_000,
            false,
            200.0,
        );
        assert!(
            item.rungs[1..]
                .iter()
                .all(|r| r.bytes * 10 <= 3_200_000 * 9),
            "{:?}",
            item.rungs
        );
        let floor = Audio {
            min_kbps: Some(64),
            ..Audio::default()
        };
        assert_eq!(lower(opus(96), &STEREO, &floor), [opus(80), opus(64)]);
        assert!(
            lower(opus(96), &STEREO, &Audio::default())
                .iter()
                .all(|f| matches!(
                    f,
                    Format::Encode { kbps: Some(k), .. } if *k < 96
                ))
        );
    }

    #[test]
    fn a_narrow_source_loses_less_at_a_bitrate() {
        let wide = Source {
            lossless: true,
            bandwidth_hz: Some(20_000.0),
            seconds: 60.0,
        };
        let narrow = Source {
            bandwidth_hz: Some(11_000.0),
            ..wide
        };
        let flac = Format::Copy { codec: Codec::Flac };
        assert!(loss(&narrow, flac, opus(64)) < loss(&wide, flac, opus(64)));
        assert!(impairment_at(Codec::Opus, 72.0) > impairment_at(Codec::Opus, 80.0));
        assert!(impairment_at(Codec::Opus, 72.0) < impairment_at(Codec::Opus, 64.0));
    }

    /// Thirty four-minute songs at Opus 160.
    fn library() -> Vec<Item> {
        (0..30)
            .map(|n| song(&format!("s{n:02}"), opus(160), 4_800_000, false, 240.0))
            .collect()
    }

    fn total(items: &[Item], choice: &[usize]) -> u64 {
        items
            .iter()
            .zip(choice)
            .map(|(i, c)| i.rungs[*c].bytes)
            .sum()
    }

    #[test]
    fn estimates_keep_a_margin_known_sizes_do_not() {
        let mut items = library();
        for item in &mut items {
            for rung in &mut item.rungs[1..] {
                #[allow(clippy::cast_precision_loss)]
                let sigma = rung.bytes as f64 / 10.0;
                rung.sigma = sigma;
            }
        }
        let now = total(&items, &[0; 30]);
        let policy = Policy {
            max: now / 2,
            sigmas: 2.0,
        };
        let fit = allocate(&items, &policy).unwrap();
        assert!(fit.total > total(&items, &fit.choice), "a margin is kept");
        assert!(fit.total <= policy.max);
    }

    /// Measures each rung asked for as `miss` times its estimate, or fails
    /// it where `broken` says, counting what it was asked.
    fn measurer<'a>(
        miss: f64,
        broken: &'static [(usize, usize)],
        asked: &'a std::cell::RefCell<Vec<(usize, usize)>>,
    ) -> impl FnMut(&mut [Item], &Wanted) -> anyhow::Result<()> + 'a {
        move |items, wanted| {
            for &(i, c) in wanted {
                asked.borrow_mut().push((i, c));
                if broken.contains(&(i, c)) {
                    continue;
                }
                let rung = &mut items[i].rungs[c];
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                #[allow(clippy::cast_precision_loss)]
                let real = (rung.bytes as f64 * miss) as u64;
                rung.bytes = real;
                rung.sigma = 0.0;
                rung.known = true;
            }
            Ok(())
        }
    }

    fn sum_of(items: &[Item], fit: &Fit) -> u64 {
        fit.choice
            .iter()
            .enumerate()
            .map(|(i, c)| items[i].rungs[*c].bytes)
            .sum()
    }

    #[test]
    fn a_fit_settles_on_sizes_measured_however_estimates_miss() {
        for miss in [0.7, 1.0, 1.44] {
            let mut items: Vec<Item> = (0..12)
                .map(|n| {
                    song(
                        &format!("s{n:02}"),
                        Format::Copy { codec: Codec::Flac },
                        30 * MB,
                        true,
                        200.0,
                    )
                })
                .collect();
            let asked = std::cell::RefCell::new(Vec::new());
            let policy = Policy::exact(120 * MB);
            let Settled::Fit(fit) =
                settle(&mut items, &policy, 3, &mut measurer(miss, &[], &asked)).unwrap()
            else {
                panic!("over at {miss}");
            };
            assert!(sum_of(&items, &fit) <= 120 * MB, "{miss}");
            for (i, c) in fit.choice.iter().enumerate() {
                assert!(
                    items[i].rungs[*c].known,
                    "{miss}: song {i} rung {c} an estimate"
                );
            }
        }
    }

    #[test]
    fn songs_that_cannot_fit_have_their_lowest_measured_before_giving_up() {
        let mut items = vec![song("a", opus(160), 4 * MB, false, 200.0)];
        let asked = std::cell::RefCell::new(Vec::new());
        let over = settle(
            &mut items,
            &Policy::exact(MB / 10),
            4,
            &mut measurer(1.0, &[], &asked),
        )
        .unwrap();
        assert!(matches!(over, Settled::Over(_)), "{over:?}");
        let last = items[0].rungs.len() - 1;
        assert!(asked.borrow().contains(&(0, last)), "{:?}", asked.borrow());
    }

    #[test]
    fn a_rung_that_cannot_be_made_saves_nothing() {
        let mut items = vec![
            song(
                "a",
                Format::Copy { codec: Codec::Flac },
                30 * MB,
                true,
                200.0,
            ),
            song(
                "b",
                Format::Copy { codec: Codec::Flac },
                30 * MB,
                true,
                200.0,
            ),
        ];
        let broken: &'static [(usize, usize)] = &[(0, 1), (0, 2), (0, 3)];
        let asked = std::cell::RefCell::new(Vec::new());
        let Settled::Fit(fit) = settle(
            &mut items,
            &Policy::exact(45 * MB),
            10,
            &mut measurer(1.0, broken, &asked),
        )
        .unwrap() else {
            panic!("over");
        };
        assert!(sum_of(&items, &fit) <= 45 * MB);
        assert!(!broken.contains(&(0, fit.choice[0])), "{fit:?}");
    }
}
