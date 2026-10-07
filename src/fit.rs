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

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::codec::Codec;
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
        Codec::Flac | Codec::Alac => &[],
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
        Codec::Flac | Codec::Alac => &[(1.0, 0.0)],
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
/// at or above `[audio] min_kbps`.
#[must_use]
pub fn lower(first: Format, channels: u32, audio: &Audio) -> Vec<Format> {
    let codec = audio.lossy;
    let scale = |kbps: u32| match (audio.kbps(codec, channels), audio.kbps(codec, 2)) {
        (Some(mine), Some(two)) if two > 0 => kbps * mine / two,
        _ => kbps,
    };
    let below = match first {
        Format::Encode {
            codec: c,
            kbps: Some(k),
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
    while projected(sum) > policy.max {
        let Some(next) = heap.pop() else {
            break;
        };
        let rungs = &items[next.item].rungs;
        let now = hulls[next.item][next.to].rung;
        sum = sum.moved(&rungs[choice[next.item]], &rungs[now]);
        choice[next.item] = now;
        heap.extend(step(items, &hulls, next.item, next.to));
    }
    if projected(sum) > policy.max {
        return Err(floor);
    }
    Ok(Fit {
        choice,
        total: projected(sum),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus(kbps: u32) -> Format {
        Format::Encode {
            codec: Codec::Opus,
            kbps: Some(kbps),
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
        }];
        for format in lower(first, 2, &Audio::default()) {
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
        assert_eq!(lower(opus(96), 2, &floor), [opus(80), opus(64)]);
        assert!(
            lower(opus(96), 2, &Audio::default())
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
}
