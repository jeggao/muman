//! Which of a song's copies are one master, and which are broken, read
//! from their decoded samples alone on the timeline of
//! [`crate::coherence`].
//!
//! - **Alignment.** Each copy's DC is removed first; alignment finds its
//!   polarity and matches its gain. So DC, polarity and gain never count
//!   against a copy.
//! - **Same master.** Digital copies of one master share its clock, so two
//!   whose clocks differ, drifting apart or one holding a wow the other
//!   lacks, are two transfers however coherent they are. A remaster keeps
//!   the clock but changes the level: two copies are also two masters on
//!   any of four checks, the third-octave gain curve's range (read
//!   under both copies' lowpass edges, as a lowpass is no mastering), the
//!   loudness transfer's slope, the mid-band coherence, and the slope over
//!   the loud passages alone, where limiting acts. A pair coherent enough
//!   to carry next to no codec noise, 0.95 or more from 1 to 8 kHz, is two
//!   masters at a gain curve of 1.5 dB: no codec copy that coherent
//!   reshapes the spectrum by half a decibel, while an EQ'd remaster stays
//!   that coherent. Groups link completely: clusters merge closest pair
//!   first, only when every pair across them passes, so two streams that
//!   agree cannot chain another master into the master's group.
//! - **Integrity.** Truncated: lacking more than 10 s, and a tenth of the
//!   music, that other copies of the master hold at the start or end,
//!   counted only on a side where the copy's own music is cut. Dropouts and
//!   clicks are searched over the whole song: a gap is a fall of 40 dB or
//!   more inside the music that another copy of the master plays through;
//!   a click an impulse, its high-passed energy held within a quarter
//!   millisecond, standing well above another full-band copy of the master
//!   at that instant. The two copies must be verified to line up there:
//!   segment lags can be seconds off between different edits. With no
//!   such copy a copy is never broken by clicks, so a vinyl transfer's
//!   crackle alone in its master group hands the pick to no other master;
//!   and a peak at a copy's clip level is never a click.
//!
//! The thresholds were set on masters transcoded, remastered and
//! transferred for the purpose, where each pair's truth is known, and
//! checked on copies as a library holds them. They lean toward two
//! masters: a pair wrongly parted leaves each copy without a mate to be
//! checked against, as every copy was before, where a pair wrongly joined
//! could call a whole copy of another edit truncated.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::Path;

use crate::coherence::{self, Analysis, freq};
use crate::signal::filter::{Band, butter, sosfilt};
use crate::signal::{
    FS, Frames, corrcoef, frames_of_f32le, irfft, median, real, rfft, signed, std,
};

/// What the gate found of a group of copies, in their order.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Gate {
    /// Equal labels are one master, numbered by first copy.
    pub master: Vec<usize>,
    /// How a copy is broken, for each broken one.
    pub broken: Vec<Option<Broken>>,
}

/// How a copy is broken, against the other copies of its master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Broken {
    /// It lacks `seconds` of the song they hold, its own music cut there.
    Truncated { seconds: u32 },
    /// It falls silent `gaps` times, for `ms` in all, where one plays on.
    Dropouts { gaps: u32, ms: u32 },
    /// It holds `count` impulses that a full-band one does not.
    Clicks { count: u32 },
}

impl std::fmt::Display for Broken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Truncated { seconds } => write!(f, "lacks {seconds} s other copies hold"),
            Self::Dropouts { ms, .. } => write!(f, "{:.2} s of dropouts", f64::from(ms) / 1000.0),
            Self::Clicks { count } => write!(f, "{count} clicks"),
        }
    }
}

impl Gate {
    /// How many masters the copies hold.
    #[must_use]
    pub fn masters(&self) -> usize {
        self.master.iter().collect::<BTreeSet<_>>().len()
    }
}

/// The version of [`analyze`]: raised whenever what it finds of the same
/// copies changes, so a gate kept from an earlier one is made again.
pub const VERSION: u32 = 1;

/// The least correlation peak against the anchor of a copy compared at
/// all; under it, its audio is unrelated.
const PEAK_MIN: f64 = 0.1;
/// The most two copies of one master drift apart, in parts per million:
/// a digital copy keeps its master's clock to the sample.
const DRIFT_PPM: f64 = 2.0;
/// The most wow, in microseconds RMS, one copy of a master holds over
/// another.
const WARP_US: f64 = 20.0;
/// The widest range, in dB, of the gain between two copies of one master
/// across third octaves.
const TOC_RANGE_DB: f64 = 6.0;
/// How far from 1 the loudness transfer's slope of two copies of one
/// master stands, as its logarithm, over all their music.
const LOUD_SLOPE_DEV: f64 = 0.09;
/// The same over the loud passages alone, fewer windows and so looser.
const LOUD_PASSAGE_DEV: f64 = 0.15;
/// The least mean coherence, from 1 kHz up, of two copies of one master.
const MID_COH: f64 = 0.3;
/// The mean coherence from which a pair carries next to no codec noise.
const COHERENT: f64 = 0.95;
/// The widest range of gain, in dB, such a pair of one master shows.
const COHERENT_TOC_DB: f64 = 1.5;
/// How far a gap falls under the music on both sides of it, in dB.
const DROP_DB: f64 = 40.0;
/// The shortest gap counted, in seconds: a shorter one is a waveform
/// passing through zero.
const DROP_MIN_S: f64 = 0.02;
/// The longest gap counted, in seconds: a longer one is a pause.
const DROP_MAX_S: f64 = 2.0;
/// How far a click stands over the other copy at its place, in dB.
const CLICK_DB: f64 = 12.0;
/// The least share of the high-passed energy within 2 ms of a click that
/// a quarter millisecond about it holds.
const CLICK_CONC: f64 = 0.6;
/// The fewest clicks a second of a copy broken by them.
const CLICK_RATE: f64 = 0.1;
/// The fewest clicks of a copy broken by them, however short it is.
const CLICK_MIN: u32 = 5;
/// The least a truncated copy lacks, in seconds.
const INCOMPLETE_S: f64 = 10.0;

fn mono(x: &[[f64; 2]]) -> Vec<f64> {
    crate::signal::mid(x)
}

/// The gate's verdict on decoded copies in two channels at 48 kHz, which
/// it takes to free each of its DC in place: a whole song is 16 bytes a
/// frame, 180 MB for four minutes, and a second copy of each would double
/// what comparing a song holds.
#[must_use]
pub fn analyze(mut xs: Vec<Frames>) -> Gate {
    let k = xs.len();
    if k < 2 {
        return Gate {
            master: vec![0; k],
            broken: vec![None; k],
        };
    }
    for x in &mut xs {
        let n = real(x.len().max(1));
        let m = [
            x.iter().map(|f| f[0]).sum::<f64>() / n,
            x.iter().map(|f| f[1]).sum::<f64>() / n,
        ];
        for f in x.iter_mut() {
            f[0] -= m[0];
            f[1] -= m[1];
        }
    }
    let a = coherence::analyse(&xs);
    let rel: Vec<bool> = (0..k)
        .map(|i| a.align[i].anchor || a.align[i].peak >= PEAK_MIN)
        .collect();
    let mut same: BTreeMap<(usize, usize), bool> = BTreeMap::new();
    for i in 0..k {
        for j in i + 1..k {
            same.insert((i, j), same_master(&a, i, j, &rel));
        }
    }
    let masters = complete_linkage(&a, &same);
    let broken = integrity(&xs, &a, &masters, &rel);
    Gate {
        master: labels(&masters, k),
        broken,
    }
}

/// One file's first audio stream as the gate reads it, on stdout: two
/// channels, more mixed down by ffmpeg and one repeated, at 48 kHz as
/// 32-bit float. Resampled through soxr, as the gate was measured with,
/// or, with `soxr` false, for an ffmpeg built without it, through
/// ffmpeg's own resampler, whose residue stands far under any check here.
#[must_use]
pub fn pcm_command(path: &Path, soxr: bool) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin"]
        .into_iter()
        .map(OsString::from)
        .collect();
    cmd.extend(crate::ffmpeg::Input::from(path).args());
    let resample = if soxr {
        "aresample=48000:resampler=soxr:precision=28"
    } else {
        "aresample=48000"
    };
    cmd.extend(
        [
            "-map", "0:a:0", "-af", resample, "-ac", "2", "-f", "f32le", "-",
        ]
        .into_iter()
        .map(OsString::from),
    );
    cmd
}

/// The frames of what [`pcm_command`] wrote.
#[must_use]
pub fn frames(pcm: &[u8]) -> Frames {
    frames_of_f32le(pcm)
}

fn labels(groups: &[Vec<usize>], k: usize) -> Vec<usize> {
    let mut sorted = groups.to_vec();
    sorted.sort_by_key(|g| g.iter().copied().min());
    let mut lab = vec![0; k];
    for (n, g) in sorted.iter().enumerate() {
        for &i in g {
            lab[i] = n;
        }
    }
    lab
}

/// The median over segments of two copies' drift against each other, in
/// ppm, each against the segment's anchor, and their wow difference, in µs
/// RMS; and how many segments both were tracked in.
fn clock(a: &Analysis, i: usize, j: usize) -> (f64, f64, usize) {
    let recs = |x: usize| {
        if a.align[x].segments.is_empty() {
            vec![a.align[x].clone()]
        } else {
            a.align[x].segments.clone()
        }
    };
    let (ri, rj) = (recs(i), recs(j));
    let (mut drifts, mut wows) = (Vec::new(), Vec::new());
    for (x, y) in ri.iter().zip(&rj) {
        if !(x.anchor || x.ok) || !(y.anchor || y.ok) {
            continue;
        }
        drifts.push(x.drift_ppm - y.drift_ppm);
        wows.push(if x.anchor || y.anchor {
            x.warp_rms_us.max(y.warp_rms_us)
        } else {
            (x.warp_rms_us - y.warp_rms_us).abs()
        });
    }
    if drifts.is_empty() {
        return (f64::NAN, f64::NAN, 0);
    }
    (median(&drifts), median(&wows), drifts.len())
}

/// The mean coherence of the two mids from 1 kHz to `f_hi`, or to 2 kHz
/// where the copies' edges stand lower.
fn mid_coh(a: &Analysis, i: usize, j: usize, f_hi: f64) -> f64 {
    let v: Vec<f64> = (0..a.kb.len())
        .filter(|&b| freq(a.kb[b]) >= 1000.0 && freq(a.kb[b]) < f_hi.max(2000.0))
        .map(|b| a.coherence(b, i, j))
        .collect();
    v.iter().sum::<f64>() / real(v.len())
}

/// The range in dB of the mids' level difference per third octave from
/// 100 Hz to `f_hi`, its median removed.
fn toc_range(a: &Analysis, i: usize, j: usize, f_hi: f64) -> f64 {
    let mut out = Vec::new();
    let mut fc = 100.0;
    let step = 2f64.powf(1.0 / 6.0);
    while fc * step <= f_hi {
        let sel: Vec<usize> = (0..a.kb.len())
            .filter(|&b| freq(a.kb[b]) >= fc / step && freq(a.kb[b]) < fc * step)
            .collect();
        if !sel.is_empty() {
            let pa: f64 = sel.iter().map(|&b| a.cross(b, i, i).re).sum();
            let pb: f64 = sel.iter().map(|&b| a.cross(b, j, j).re).sum();
            out.push(10.0 * (pa.max(1e-30) / pb.max(1e-30)).log10());
        }
        fc *= 2f64.powf(1.0 / 3.0);
    }
    if out.len() < 2 {
        return 0.0;
    }
    let m = median(&out);
    let hi = out.iter().map(|x| x - m).fold(f64::NEG_INFINITY, f64::max);
    let lo = out.iter().map(|x| x - m).fold(f64::INFINITY, f64::min);
    hi - lo
}

/// The variance of the mono mix over each 400 ms window, hop 100 ms.
fn win_var(x: &[[f64; 2]]) -> Vec<f64> {
    let m = mono(x);
    let (w, hop) = ((0.4 * FS) as usize, (0.1 * FS) as usize);
    if m.len() < w + hop {
        return Vec::new();
    }
    let n = (m.len() - w) / hop;
    let mut c1 = vec![0.0; m.len() + 1];
    let mut c2 = vec![0.0; m.len() + 1];
    for (i, v) in m.iter().enumerate() {
        c1[i + 1] = c1[i] + v;
        c2[i + 1] = c2[i] + v * v;
    }
    (0..n)
        .map(|s| {
            let st = s * hop;
            let mu = (c1[st + w] - c1[st]) / real(w);
            ((c2[st + w] - c2[st]) / real(w) - mu * mu).max(0.0)
        })
        .collect()
}

/// The loudness transfer's slope over windows where both copies play:
/// the geometric mean of the two regressions.
fn loud_slope(a: &[[f64; 2]], b: &[[f64; 2]]) -> f64 {
    let (pa, pb) = (win_var(a), win_var(b));
    let keep: Vec<usize> = (0..pa.len().min(pb.len()))
        .filter(|&w| pa[w] > 1e-7 && pb[w] > 1e-7)
        .collect();
    if keep.len() < 5 {
        return 1.0;
    }
    let la: Vec<f64> = keep.iter().map(|&w| 10.0 * pa[w].log10()).collect();
    let lb: Vec<f64> = keep.iter().map(|&w| 10.0 * pb[w].log10()).collect();
    if std(&la) < 1e-6 || std(&lb) < 1e-6 {
        return 1.0;
    }
    if corrcoef(&la, &lb) <= 0.0 {
        return f64::INFINITY;
    }
    std(&lb) / std(&la)
}

/// Whether two copies are one master, by every check.
fn same_master(a: &Analysis, i: usize, j: usize, rel: &[bool]) -> bool {
    let (drift, warp, nseg) = clock(a, i, j);
    if !(rel[i] && rel[j]) || nseg == 0 {
        return false;
    }
    let e = 0.75
        * a.edge_hz[i]
            .unwrap_or(20000.0)
            .min(a.edge_hz[j].unwrap_or(20000.0));
    let toc = toc_range(a, i, j, 12000.0f64.min(e));
    let ls = loud_slope(&a.x[i], &a.x[j]);
    let mc = mid_coh(a, i, j, 8000.0f64.min(e));
    let lp = a.pair(i, j).loud_slope;
    let clocks =
        drift.is_finite() && drift.abs() <= DRIFT_PPM && !(warp.is_finite() && warp > WARP_US);
    let levels = ls.is_finite()
        && ls.max(1e-9).ln().abs() <= LOUD_SLOPE_DEV
        && lp.is_finite()
        && lp > 0.0
        && lp.abs().max(1e-9).ln().abs() <= LOUD_PASSAGE_DEV;
    let spectra =
        toc <= TOC_RANGE_DB && mc >= MID_COH && !(mc >= COHERENT && toc > COHERENT_TOC_DB);
    clocks && levels && spectra
}

/// Two copies that may merge: their distance, their content keys in
/// order, and which they are.
type Candidate = (f64, (String, String), (usize, usize));

/// Clusters merged closest pair first, only when every pair across them
/// is one master; ties broken by content keys, so the partition does not
/// depend on the order copies come in.
fn complete_linkage(a: &Analysis, same: &BTreeMap<(usize, usize), bool>) -> Vec<Vec<usize>> {
    let keys = &a.keys;
    let mut lab: Vec<BTreeSet<usize>> = (0..a.k).map(|i| BTreeSet::from([i])).collect();
    let mut cand: Vec<Candidate> = same
        .iter()
        .filter(|(_, s)| **s)
        .map(|(&(i, j), _)| {
            let (ka, kb) = if keys[i] <= keys[j] {
                (keys[i].clone(), keys[j].clone())
            } else {
                (keys[j].clone(), keys[i].clone())
            };
            let d = a.pair(i, j).nsr_mid;
            (
                format!("{d:.9}").parse::<f64>().unwrap_or(d),
                (ka, kb),
                (i, j),
            )
        })
        .collect();
    cand.sort_by(|x, y| {
        x.0.total_cmp(&y.0)
            .then_with(|| x.1.cmp(&y.1))
            .then_with(|| x.2.cmp(&y.2))
    });
    for (_, _, (i, j)) in cand {
        if lab[i] == lab[j] {
            continue;
        }
        let (ci, cj) = (lab[i].clone(), lab[j].clone());
        if ci
            .iter()
            .all(|&x| cj.iter().all(|&y| same[&(x.min(y), x.max(y))]))
        {
            let m: BTreeSet<usize> = ci.union(&cj).copied().collect();
            for &x in &m {
                lab[x].clone_from(&m);
            }
        }
    }
    let set: BTreeSet<Vec<usize>> = lab.into_iter().map(|g| g.into_iter().collect()).collect();
    set.into_iter().collect()
}

/// The level of the mid in dB per 10 ms window.
fn env(x: &[[f64; 2]]) -> Vec<f64> {
    let m = mono(x);
    let n = m.len() / 480;
    (0..n)
        .map(|w| {
            10.0 * (m[w * 480..(w + 1) * 480].iter().map(|v| v * v).sum::<f64>() / 480.0 + 1e-14)
                .log10()
        })
        .collect()
}

/// Runs of windows 50 dB under the loudest that fall `DROP_DB` under the
/// medians of the 100 ms before and after them: candidate gaps.
fn gaps(levels: &[f64], min_w: usize, max_w: usize) -> Vec<(usize, usize)> {
    let count = levels.len();
    let loud = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let loud = if count == 0 { 0.0 } else { loud };
    let mut out = Vec::new();
    let mut i = 0;
    while i < count {
        if levels[i] > loud - 50.0 {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < count && levels[j] <= loud - 50.0 {
            j += 1;
        }
        let len = j - i;
        if (min_w..=max_w).contains(&len) && i >= 10 && j + 10 <= count {
            let (before, after) = (median(&levels[i - 10..i]), median(&levels[j..j + 10]));
            let inside = levels[i..j]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            if before.min(after) - inside >= DROP_DB {
                out.push((i, len));
            }
        }
        i = j;
    }
    out
}

/// The copy high-passed at 6 kHz, in single precision: a click is read
/// off it to a ratio of 4, and a whole song of every copy is held at once.
fn hp(x: &[[f64; 2]]) -> Vec<[f32; 2]> {
    let sos = butter(4, Band::High(6000.0), FS);
    let l = sosfilt(&sos, &x.iter().map(|f| f[0]).collect::<Vec<_>>());
    let r = sosfilt(&sos, &x.iter().map(|f| f[1]).collect::<Vec<_>>());
    l.into_iter()
        .zip(r)
        .map(|(a, b)| [a as f32, b as f32])
        .collect()
}

fn at_clip(x: &[[f64; 2]], t: usize, peak: f64) -> bool {
    let lo = t.saturating_sub(24);
    x[lo..(t + 24).min(x.len())]
        .iter()
        .flat_map(|f| f.iter())
        .fold(0.0_f64, |m, v| m.max(v.abs()))
        >= 0.97 * peak
}

/// Positions where the high-passed peak stands `ratio` times over its
/// local scale, the RMS of the surrounding 20 ms: one a 2 ms block, local
/// maxima only, the strongest 4000.
fn click_candidates(h: &[[f32; 2]], ratio: f64) -> Vec<usize> {
    let blk = 96;
    let peaks: Vec<f64> = h
        .iter()
        .map(|f| f64::from(f[0].abs().max(f[1].abs())))
        .collect();
    let nb = peaks.len() / blk;
    if nb < 3 {
        return Vec::new();
    }
    let (mut bmax, mut barg) = (vec![0.0; nb], vec![0; nb]);
    let mut energy = vec![0.0; nb + 1];
    for b in 0..nb {
        let block = &peaks[b * blk..(b + 1) * blk];
        let (mut top, mut am) = (f64::NEG_INFINITY, 0);
        for (i, v) in block.iter().enumerate() {
            if *v > top {
                top = *v;
                am = i;
            }
        }
        bmax[b] = top;
        barg[b] = am;
        energy[b + 1] = energy[b] + block.iter().map(|v| v * v).sum::<f64>();
    }
    let score: Vec<f64> = (0..nb)
        .map(|k| {
            let (lo, hi) = (k.saturating_sub(5), (k + 5).min(nb));
            bmax[k] / (((energy[hi] - energy[lo]) / real((hi - lo) * blk)).sqrt() + 1e-9)
        })
        .collect();
    let mut keep: Vec<bool> = score.iter().map(|&s| s > ratio).collect();
    for k in 1..nb {
        keep[k] = keep[k] && score[k] >= score[k - 1];
    }
    for k in 0..nb - 1 {
        keep[k] = keep[k] && score[k] > score[k + 1];
    }
    let mut idx: Vec<usize> = (0..nb).filter(|&k| keep[k]).collect();
    if idx.len() > 4000 {
        idx.sort_by(|x, y| score[*y].total_cmp(&score[*x]));
        idx.truncate(4000);
    }
    let mut out: Vec<usize> = idx.iter().map(|&k| k * blk + barg[k]).collect();
    out.sort_unstable();
    out
}

/// The share of the high-passed energy within 2 ms of `t` held within a
/// quarter millisecond of it.
fn concentration(h: &[[f32; 2]], t: usize) -> f64 {
    let lo = t.saturating_sub(96);
    let e: Vec<f64> = h[lo..(t + 96).min(h.len())]
        .iter()
        .map(|f| f64::from(f[0] * f[0] + f[1] * f[1]))
        .collect();
    let tot: f64 = e.iter().sum();
    if tot <= 0.0 {
        return 0.0;
    }
    let c = t - lo;
    e[c.saturating_sub(12)..(c + 12).min(e.len())]
        .iter()
        .sum::<f64>()
        / tot
}

/// A copy's sounding span, and whether its music starts or stops at full
/// level there, within 12 dB of the span's median: cut, not faded.
fn edges_abrupt(x: &[[f64; 2]]) -> (i64, i64, bool, bool) {
    let sum: Vec<f64> = x.iter().step_by(4).map(|f| f[0] + f[1]).collect();
    let win = 2400 / 4;
    let nw = sum.len() / win;
    if nw < 3 {
        return (0, signed(x.len()), false, false);
    }
    let lv: Vec<f64> = (0..nw)
        .map(|i| {
            10.0 * (sum[i * win..(i + 1) * win]
                .iter()
                .map(|v| v * v)
                .sum::<f64>()
                / real(win)
                + 1e-20)
                .log10()
        })
        .collect();
    let top = lv.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let on: Vec<usize> = (0..nw).filter(|&i| lv[i] > top - 50.0).collect();
    let (Some(&first), Some(&last)) = (on.first(), on.last()) else {
        return (0, signed(x.len()), false, false);
    };
    let med = median(&lv[first..=last]);
    (
        signed(first * win * 4),
        signed((last + 1) * win * 4),
        lv[first] >= med - 12.0,
        lv[last] >= med - 12.0,
    )
}

/// Copy `i`'s sample `t` mapped into copy `j` through the segment nearest
/// it in copy `i`'s own timeline.
fn map_lag(lags: &[(Vec<f64>, Vec<f64>)], i: usize, j: usize, t: f64) -> f64 {
    let (ti, li) = &lags[i];
    let (_, lj) = &lags[j];
    let n = ti.len().min(lj.len());
    let mut best = 0;
    for k in 0..n {
        if (ti[k] + li[k] - t).abs() < (ti[best] + li[best] - t).abs() {
            best = k;
        }
    }
    t - li[best] + lj[best]
}

/// The sample of copy `j` that lines up with copy `i`'s sample `t_i`, by
/// the normalised cross-correlation of the mids over 50 ms either side,
/// searched 10 ms around `t_j`; none where the copies do not line up.
fn verify(xi: &[[f64; 2]], xj: &[[f64; 2]], t_i: i64, t_j: f64) -> Option<i64> {
    let (half, reach) = (2400_i64, 480_i64);
    let t_j = t_j.round_ties_even() as i64;
    let (a0, a1, b0, b1) = (
        t_i - half,
        t_i + half,
        t_j - half - reach,
        t_j + half + reach,
    );
    if a0 < 0 || b0 < 0 || a1 > signed(xi.len()) || b1 > signed(xj.len()) {
        return None;
    }
    let a = mono(&xi[a0 as usize..a1 as usize]);
    let b = mono(&xj[b0 as usize..b1 as usize]);
    let ea: f64 = a.iter().map(|v| v * v).sum();
    if ea <= 0.0 {
        return None;
    }
    let nfft = 1usize << (usize::BITS - (a.len() + b.len() - 1).leading_zeros());
    let fa = rfft(&a, nfft);
    let fb = rfft(&b, nfft);
    let prod: Vec<_> = fb.iter().zip(&fa).map(|(x, y)| x * y.conj()).collect();
    let c = irfft(&prod, nfft);
    let n = b.len() - a.len() + 1;
    let mut cb = vec![0.0; b.len() + 1];
    for (k, v) in b.iter().enumerate() {
        cb[k + 1] = cb[k] + v * v;
    }
    let (mut best, mut bk) = (f64::NEG_INFINITY, 0);
    for k in 0..n {
        let eb = cb[k + a.len()] - cb[k];
        let rho = c[k].abs() / (ea * eb.max(1e-30)).sqrt();
        if rho > best {
            best = rho;
            bk = k;
        }
    }
    (best >= 0.6).then_some(b0 + signed(bk) + half)
}

fn local_rms(x: &[[f64; 2]], t: usize) -> f64 {
    let lo = t.saturating_sub(4800);
    let m = mono(&x[lo..(t + 4800).min(x.len())]);
    if m.is_empty() {
        1e-12
    } else {
        (m.iter().map(|v| v * v).sum::<f64>() / real(m.len())).sqrt() + 1e-12
    }
}

/// Whether copy `i` is cut short of its `mates`: lacking more of the span
/// they sound over than [`INCOMPLETE_S`] and a tenth of it, counted on
/// the sides where its own music is cut.
fn truncated(spans: &[(i64, i64, bool, bool)], i: usize, mates: &[usize]) -> Option<Broken> {
    let (start, end, cut_start, cut_end) = spans[i];
    let first = mates.iter().map(|&j| spans[j].0).min()?;
    let last = mates.iter().map(|&j| spans[j].1).max()?;
    let whole = last.max(end) - first.min(start);
    let (lacks_start, lacks_end) = ((start - first).max(0), (last - end).max(0));
    let cut = lacks_start * i64::from(cut_start) + lacks_end * i64::from(cut_end);
    (cut as f64 > (INCOMPLETE_S * FS).max(0.1 * whole as f64)).then(|| Broken::Truncated {
        seconds: ((lacks_start + lacks_end) as f64 / FS).round() as u32,
    })
}

/// What the searches for dropouts and clicks read of every copy.
struct Copies<'a> {
    xs: &'a [Frames],
    /// Each copy's level per 10 ms ([`env`]).
    envs: Vec<Vec<f64>>,
    /// Each copy high-passed ([`hp`]).
    highs: Vec<Vec<[f32; 2]>>,
    /// Each copy's segment starts and lags, in samples.
    lags: Vec<(Vec<f64>, Vec<f64>)>,
}

impl Copies<'_> {
    /// The sample of copy `j` lining up with copy `i`'s sample `t`.
    fn place(&self, i: usize, j: usize, t: i64) -> Option<i64> {
        let guess = map_lag(&self.lags, i, j, t as f64);
        verify(&self.xs[i], &self.xs[j], t, guess)
    }

    /// The gaps of copy `i` that one of its `mates` plays through.
    fn dropouts(&self, i: usize, mates: &[usize]) -> Option<Broken> {
        let (least, most) = ((DROP_MIN_S * 100.0) as usize, (DROP_MAX_S * 100.0) as usize);
        let (mut count, mut ms) = (0_u32, 0_u32);
        for (start, len) in gaps(&self.envs[i], least, most) {
            if mates.iter().any(|&j| self.plays_through(i, j, start, len)) {
                count += 1;
                ms += len as u32 * 10;
            }
        }
        (count > 0).then_some(Broken::Dropouts { gaps: count, ms })
    }

    /// Whether copy `j` plays through copy `i`'s gap of `len` windows from
    /// window `start`: placed by either side of the gap, 150 ms out, its
    /// level there falls nowhere 20 dB under its level about it.
    fn plays_through(&self, i: usize, j: usize, start: usize, len: usize) -> bool {
        let middle = (start as f64 + len as f64 / 2.0) * 480.0;
        let sides = [signed(start) * 480 - 7200, signed(start + len) * 480 + 7200];
        let placed = sides.into_iter().find_map(|side| {
            let there = self.place(i, j, side)?;
            Some(there as f64 + (middle - side as f64))
        });
        let Some(placed) = placed else { return false };
        let window = (placed / 480.0).floor() as i64;
        let env = &self.envs[j];
        if window - 15 < 0 || window + 15 >= signed(env.len()) {
            return false;
        }
        let window = window as usize;
        let low = (env[window - 3..window + 4].iter().copied()).fold(f64::INFINITY, f64::min);
        low >= median(&env[window - 15..window + 15]) - 20.0
    }

    /// The clicks of copy `i` that one of `full`, the full-band copies of
    /// its master, does not hold.
    fn clicks(&self, i: usize, full: &[usize]) -> Option<Broken> {
        if full.is_empty() {
            return None;
        }
        let (high, own) = (&self.highs[i], &self.xs[i]);
        let peak = (own.iter().flat_map(|f| f.iter())).fold(0.0_f64, |m, v| m.max(v.abs())) + 1e-12;
        let mut count = 0_u32;
        for t in click_candidates(high, 5.0) {
            if concentration(high, t) < CLICK_CONC || at_clip(own, t, peak) {
                continue;
            }
            if full.iter().any(|&j| self.lacks_impulse(i, j, t)) {
                count += 1;
            }
        }
        let rate = f64::from(count) / (real(own.len()) / FS).max(1.0);
        (count >= CLICK_MIN && rate >= CLICK_RATE).then_some(Broken::Clicks { count })
    }

    /// Whether copy `i`'s impulse at sample `t` stands [`CLICK_DB`] over
    /// anything copy `j` holds within 2 ms of the same place, their
    /// levels there matched; not where `j` is all but silent.
    fn lacks_impulse(&self, i: usize, j: usize, t: usize) -> bool {
        let Some(there) = self.place(i, j, signed(t)) else {
            return false;
        };
        let (high, other) = (&self.highs[i], &self.highs[j]);
        if there - 96 < 0 || there + 96 >= signed(other.len()) {
            return false;
        }
        let there = there as usize;
        let (level, level_there) = (local_rms(&self.xs[i], t), local_rms(&self.xs[j], there));
        if level_there < 0.03 * level {
            return false;
        }
        let top = (other[there - 96..there + 96].iter())
            .flat_map(|f| f.iter())
            .fold(0.0_f32, |m, v| m.max(v.abs()));
        let height = f64::from(high[t][0].abs().max(high[t][1].abs()));
        height >= 10f64.powf(CLICK_DB / 20.0) * (f64::from(top) * level / level_there + 1e-12)
    }
}

/// Each copy's integrity: truncated, holding dropouts another copy of its
/// master plays through, or clicks no full-band copy of it holds, the
/// first of them found.
fn integrity(
    xs: &[Frames],
    found: &Analysis,
    masters: &[Vec<usize>],
    rel: &[bool],
) -> Vec<Option<Broken>> {
    let group_of: BTreeMap<usize, &Vec<usize>> = masters
        .iter()
        .flat_map(|g| g.iter().map(move |&i| (i, g)))
        .collect();
    let spans: Vec<(i64, i64, bool, bool)> = (0..found.k)
        .map(|i| {
            let (start, end, cut_start, cut_end) = edges_abrupt(&xs[i]);
            let lag = if found.align[i].anchor {
                0
            } else {
                (found.align[i].lag_s * FS).round_ties_even() as i64
            };
            (start - lag, end - lag, cut_start, cut_end)
        })
        .collect();
    let segments = |j: usize| {
        let whole = &found.align[j];
        if whole.segments.is_empty() {
            vec![whole.clone()]
        } else {
            whole.segments.clone()
        }
    };
    let copies = Copies {
        xs,
        envs: xs.iter().map(|x| env(x)).collect(),
        highs: xs.iter().map(|x| hp(x)).collect(),
        lags: (0..found.k)
            .map(|j| {
                let recs = segments(j);
                let starts = recs.iter().map(|r| r.segment_start_s.unwrap_or(0.0) * FS);
                (
                    starts.collect(),
                    recs.iter().map(|r| r.lag_s * FS).collect(),
                )
            })
            .collect(),
    };
    (0..found.k)
        .map(|i| {
            if !rel[i] {
                return None;
            }
            let mates: Vec<usize> = (group_of[&i].iter().copied())
                .filter(|&j| j != i && rel[j])
                .collect();
            let full: Vec<usize> = (mates.iter().copied())
                .filter(|&j| found.edge_hz[j].unwrap_or(20000.0) >= 12000.0)
                .collect();
            truncated(&spans, i, &mates)
                .or_else(|| copies.dropouts(i, &mates))
                .or_else(|| copies.clicks(i, &full))
        })
        .collect()
}

#[cfg(test)]
mod tests;
