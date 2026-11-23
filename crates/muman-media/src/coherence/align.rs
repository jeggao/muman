//! Every copy of a song on one timeline, the first stage of
//! [`crate::coherence`].
//!
//! The copies are placed on the timeline of an anchor chosen from their
//! content: the copy whose summed correlation peak with the others is
//! highest, then the longest, then by a key of its samples
//! ([`content_key`]). The order the copies come in never matters.
//!
//! Each copy, against the anchor:
//!
//! 1. **A global lag:** a partially whitened cross-correlation
//!    ([`phat_lag`]) of the mid signals at 12 kHz, refined at 48 kHz on a
//!    central 8 s.
//! 1. **A drift:** lags of 4 s blocks within 0.3 s of the global lag, and
//!    a weighted line through them robust to outliers.
//! 1. **A fine delay track:** one sub-sample delay per 43 ms hop, the
//!    whitened cross-correlation maximised in closed form around the
//!    drift line, by Newton steps.
//! 1. **Warp or delay.** Where the track's smoothed residual stands above
//!    its own noise, the copy is warped, by wow or flutter or an analog
//!    transfer, and is resampled along it with a windowed sinc. Otherwise
//!    a delay, and a drift where it moves the copy by a twentieth of a
//!    sample over the span, is applied: a delay exactly, by a phase shift,
//!    its fraction fitted from the slope of the cross-spectrum's phase.
//! 1. **Gain:** each copy scaled so that its 1–8 kHz mid energy, the
//!    median of its ratio over 4 s blocks, matches the copies' geometric
//!    mean ([`gain_match`]).
//!
//! A copy sharing under 2 s with the anchor, or correlating under 0.1
//! with it, is unrelated: placed by its lag alone, not tracked.
//!
//! **Whole songs.** Past 61 s of common span, 7 segments of 8 s spread
//! evenly over it are aligned each on its own ([`align_segments`]): the
//! lag of each found around the song's global lag, taken on 2 kHz
//! decimations of the whole songs, or anywhere in the copy where that
//! finds nothing, as a different edit moves a passage further. So neither
//! a long drift nor an edit needs one model of the whole song.

use sha1::{Digest as _, Sha1};

use crate::signal::filter::{Band, butter, resample_poly, sosfilt, sosfiltfilt};
use crate::signal::{
    Complex64, FS, Frames, bessel_i0, csd, fast_len, interp_linear, irfft, median, mid, real, rfft,
    signed, sinc, welch,
};

/// A copy's lag against another, its polarity and its correlation peak.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Global {
    /// Samples: the copy at `n + lag` holds what the anchor holds at `n`.
    pub lag: i64,
    /// 1, or -1 for a copy in opposite polarity.
    pub pol: f64,
    /// The normalised correlation at the lag.
    pub peak: f64,
}

/// What aligning found of one copy against the anchor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// Its lag in seconds, at the middle of the span.
    pub lag_s: f64,
    /// Its clock against the anchor's, parts per million.
    pub drift_ppm: f64,
    /// The RMS of its smoothed delay track, in microseconds, where warped.
    pub warp_rms_us: f64,
    /// Whether it was resampled along its warp.
    pub warp_applied: bool,
    /// The standard error of the residual delay, in microseconds.
    pub resid_us: Option<f64>,
    /// 1, or -1 in opposite polarity.
    pub polarity: i32,
    /// The correlation peak of its global lag.
    pub peak: f64,
    /// Whether it is the anchor.
    pub anchor: bool,
    /// Whether it was tracked and correlates over 0.2.
    pub ok: bool,
    /// For an unrelated copy, the share of the span it covers.
    pub coverage: Option<f64>,
    /// Where its segment starts, in the anchor's seconds.
    pub segment_start_s: Option<f64>,
    /// Its lag within its segment.
    pub lag_s_local: Option<f64>,
    /// The gain that matched it, in dB.
    pub gain_db: f64,
    /// Its record in each segment, of a whole song.
    pub segments: Vec<Info>,
}

/// One aligned stretch: each copy's samples on it, each one's record, its
/// anchor and where it starts in the main anchor's samples.
#[derive(Clone, Debug)]
pub struct Segment {
    pub ys: Vec<Frames>,
    pub info: Vec<Info>,
    pub anchor: usize,
    pub start: i64,
}

/// `v[start..stop]` as Python slices it: negative bounds count from the
/// end, and bounds past either end are clamped.
fn py_slice<T>(v: &[T], start: i64, stop: i64) -> &[T] {
    let n = signed(v.len());
    let fix = |i: i64| if i < 0 { (i + n).max(0) } else { i.min(n) };
    let (a, b) = (fix(start), fix(stop));
    if b <= a {
        &v[0..0]
    } else {
        &v[a as usize..b as usize]
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The lag of `b` against `a` within `reach` samples either way, by their
/// cross-correlation whitened to the power `beta` of its magnitude; with
/// the sign of the peak and its height over the median.
#[must_use]
pub fn phat_lag(a: &[f64], b: &[f64], reach: usize) -> (i64, f64, f64) {
    phat_lag_beta(a, b, reach, 0.8)
}

fn phat_lag_beta(a: &[f64], b: &[f64], reach: usize, beta: f64) -> (i64, f64, f64) {
    let n = fast_len((a.len() + b.len()).max(2));
    let fa = rfft(a, n);
    let fb = rfft(b, n);
    let mut cross: Vec<Complex64> = fb.iter().zip(&fa).map(|(y, x)| y * x.conj()).collect();
    let gmax = cross.iter().map(|bin| bin.norm()).fold(0.0, f64::max);
    for bin in &mut cross {
        *bin /= (bin.norm() + 1e-12 * gmax + 1e-300).powf(beta);
    }
    let corr = irfft(&cross, n);
    let reach = reach.min(n / 2 - 1);
    let mut lags: Vec<i64> = (0..=signed(reach)).collect();
    let mut near: Vec<f64> = corr[..=reach].to_vec();
    if reach == 0 {
        lags.extend(0..signed(n));
        near.extend_from_slice(&corr);
    } else {
        lags.extend(-signed(reach)..0);
        near.extend_from_slice(&corr[n - reach..]);
    }
    let (mut at, mut best) = (0, -1.0);
    for (k, x) in near.iter().enumerate() {
        if x.abs() > best {
            best = x.abs();
            at = k;
        }
    }
    let abs: Vec<f64> = near.iter().map(|x| x.abs()).collect();
    let sharp = near[at].abs() / (median(&abs) + 1e-300);
    (lags[at], if near[at] >= 0.0 { 1.0 } else { -1.0 }, sharp)
}

/// The normalised correlation of `a[n]` with `b[n + lag]` over their
/// overlap.
#[must_use]
pub fn norm_corr(a: &[f64], b: &[f64], lag: i64) -> f64 {
    let (aa, bb) = if lag >= 0 {
        (
            py_slice(a, 0, signed(b.len()) - lag),
            py_slice(b, lag, i64::MAX),
        )
    } else {
        (
            py_slice(a, -lag, i64::MAX),
            py_slice(b, 0, signed(a.len()) + lag),
        )
    };
    let m = aa.len().min(bb.len());
    let (aa, bb) = (&aa[..m], &bb[..m]);
    dot(aa, bb) / (dot(aa, aa) * dot(bb, bb) + 1e-300).sqrt()
}

/// A key of a copy's samples, for tie-breaks that must not depend on the
/// order copies come in: the SHA-1 of every 97th frame at 16-bit scale.
#[must_use]
pub fn content_key(x: &[[f64; 2]]) -> String {
    let mut h = Sha1::new();
    for frame in x.iter().step_by(97) {
        for v in frame {
            let q = (*v as f32 * 32768.0_f32).round_ties_even() as i32;
            h.update(q.to_le_bytes());
        }
    }
    muman_core::hash::hex(&h.finalize())
}

/// `x` rounded to 3 decimals as Python's `round(x, 3)` does: correctly,
/// from its exact binary value.
fn round3(x: f64) -> f64 {
    format!("{x:.3}").parse().unwrap_or(x)
}

/// The anchor: the copy best correlated with the rest, then the longest,
/// as a short excerpt must never set the timeline, then by content key.
fn pick_anchor(score: &[f64], lens: &[usize], keys: &[String]) -> usize {
    let mut best = 0;
    for i in 1..score.len() {
        let a = (round3(score[i]), lens[i], &keys[i]);
        let b = (round3(score[best]), lens[best], &keys[best]);
        if a > b {
            best = i;
        }
    }
    best
}

/// Pairwise lags at 12 kHz, the anchor, and each copy's lag against it
/// refined at 48 kHz, with its polarity and correlation peak.
#[must_use]
pub fn global_lags(xs: &[Frames]) -> (usize, Vec<Global>) {
    let k = xs.len();
    let ds: Vec<Vec<f64>> = xs.iter().map(|x| resample_poly(&mid(x), 1, 4)).collect();
    let reach = (30.0 * FS / 4.0) as usize;
    let mut peak = vec![vec![0.0; k]; k];
    let mut lag = vec![vec![0_i64; k]; k];
    let mut pol = vec![vec![1.0; k]; k];
    for i in 0..k {
        for j in i + 1..k {
            let (l, s, _) = phat_lag(&ds[i], &ds[j], reach);
            let c = norm_corr(&ds[i], &ds[j], l).abs();
            peak[i][j] = c;
            peak[j][i] = c;
            lag[i][j] = l;
            lag[j][i] = -l;
            pol[i][j] = s;
            pol[j][i] = s;
        }
    }
    let score: Vec<f64> = peak.iter().map(|r| r.iter().sum()).collect();
    let keys: Vec<String> = xs.iter().map(|x| content_key(x)).collect();
    let lens: Vec<usize> = xs.iter().map(Vec::len).collect();
    let anchor = pick_anchor(&score, &lens, &keys);
    let a = mid(&xs[anchor]);
    let out = (0..k)
        .map(|j| {
            if j == anchor {
                return Global {
                    lag: 0,
                    pol: 1.0,
                    peak: 1.0,
                };
            }
            let l4 = lag[anchor][j] * 4;
            let b = mid(&xs[j]);
            let c0 = a.len() / 2;
            let width = (8 * FS as usize).min(a.len() / 2);
            let from = signed(c0.saturating_sub(width / 2));
            let sa = py_slice(&a, from, signed(c0 + width / 2));
            let s0 = from + l4;
            let lo = (s0 - 64).max(0);
            let sb = py_slice(&b, lo, s0 + signed(sa.len()) + 64);
            if sb.len() < sa.len() / 2 {
                return Global {
                    lag: l4,
                    pol: pol[anchor][j],
                    peak: peak[anchor][j],
                };
            }
            let (lf, s, _) = phat_lag(sa, sb, (64 + (s0 - lo).abs()) as usize);
            Global {
                lag: lo - from + lf,
                pol: s,
                peak: peak[anchor][j],
            }
        })
        .collect();
    (anchor, out)
}

/// A weighted least-squares line through `(t, y)`, refitted four times
/// on the points within four robust deviations of it; its slope,
/// intercept and the points kept.
fn robust_line(t: &[f64], y: &[f64], w: &[f64]) -> (f64, f64, Vec<bool>) {
    let mut keep: Vec<bool> = w.iter().map(|&v| v > 0.0).collect();
    let (mut slope, mut icpt) = (0.0, 0.0);
    for _ in 0..4 {
        if keep.iter().filter(|&&k| k).count() < 3 {
            let m = if y.is_empty() { 0.0 } else { median(y) };
            return (0.0, m, keep);
        }
        let idx: Vec<usize> = (0..t.len()).filter(|&i| keep[i]).collect();
        let sw: f64 = idx.iter().map(|&i| w[i]).sum();
        let tm = idx.iter().map(|&i| w[i] * t[i]).sum::<f64>() / sw;
        let ym = idx.iter().map(|&i| w[i] * y[i]).sum::<f64>() / sw;
        let stt: f64 = idx.iter().map(|&i| w[i] * (t[i] - tm).powi(2)).sum();
        let sty: f64 = idx.iter().map(|&i| w[i] * (t[i] - tm) * (y[i] - ym)).sum();
        slope = if stt > 0.0 { sty / stt } else { 0.0 };
        icpt = ym - slope * tm;
        let r: Vec<f64> = t
            .iter()
            .zip(y)
            .map(|(t, y)| y - (slope * t + icpt))
            .collect();
        let rk: Vec<f64> = idx.iter().map(|&i| r[i].abs()).collect();
        let mad = median(&rk) * 1.4826 + 0.5;
        let lim = (4.0 * mad).max(2.0);
        keep = (0..t.len())
            .map(|i| w[i] > 0.0 && r[i].abs() < lim)
            .collect();
    }
    (slope, icpt, keep)
}

/// The lag of `b` against each block of `a`, searched around `lag0`:
/// block centres, lags and how sharply each was found.
fn block_lags(
    a: &[f64],
    b: &[f64],
    lag0: i64,
    block: usize,
    reach: usize,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (mut at, mut found, mut weights) = (Vec::new(), Vec::new(), Vec::new());
    if a.len() < block {
        return (at, found, weights);
    }
    let reach_i = signed(reach);
    for s in (0..=a.len() - block).step_by(block) {
        let sa = &a[s..s + block];
        if dot(sa, sa) < 1e-9 * real(block) {
            continue;
        }
        let si = signed(s);
        let lo = (si + lag0 - reach_i).max(0);
        let hi = signed(b.len()).min(si + lag0 + signed(block) + reach_i);
        let sb = py_slice(b, lo, hi);
        if sb.len() < block / 2 {
            continue;
        }
        let (lr, _, sharp) = phat_lag(sa, sb, (reach_i + (si + lag0 - lo).abs()) as usize);
        at.push(real(s) + real(block) / 2.0);
        found.push((lo - si + lr) as f64);
        weights.push(sharp);
    }
    (at, found, weights)
}

/// `numpy.hanning(m)`, written as numpy computes it.
fn hanning(m: usize) -> Vec<f64> {
    if m == 1 {
        return vec![1.0];
    }
    (0..m)
        .map(|k| {
            let n = 1.0 - real(m) + 2.0 * real(k);
            0.5 + 0.5 * (std::f64::consts::PI * n / (real(m) - 1.0)).cos()
        })
        .collect()
}

/// The sub-sample delay of `b` against `a` in each 2048-sample window,
/// hop 1024, around `pred(t)` at the window's centre: centres, delays
/// and their confidences.
fn fine_track(a: &[f64], b: &[f64], pred: impl Fn(f64) -> f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    const WIN: usize = 2048;
    const HOP: usize = 1024;
    let hann: Vec<f64> = hanning(WIN + 1)[..WIN].to_vec();
    let band: Vec<usize> = (0..=WIN / 2)
        .filter(|&k| {
            let f = real(k) * FS / real(WIN);
            (200.0..=10_000.0).contains(&f)
        })
        .collect();
    let mut wins: Vec<(f64, i64)> = Vec::new();
    if a.len() >= WIN {
        for s in (0..=a.len() - WIN).step_by(HOP) {
            let c = real(s) + real(WIN) / 2.0;
            let pl = pred(c).round_ties_even() as i64;
            let start = signed(s) + pl;
            if start >= 0 && start + signed(WIN) <= signed(b.len()) {
                wins.push((real(s), pl));
            }
        }
    }
    if wins.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut gs: Vec<Vec<Complex64>> = Vec::with_capacity(wins.len());
    let mut energy = Vec::with_capacity(wins.len());
    for &(s, pl) in &wins {
        let s = s as usize;
        let fa: Vec<f64> = (0..WIN).map(|k| a[s + k] * hann[k]).collect();
        let bs = (signed(s) + pl) as usize;
        let fb: Vec<f64> = (0..WIN).map(|k| b[bs + k] * hann[k]).collect();
        let sa = rfft(&fa, WIN);
        let sb = rfft(&fb, WIN);
        energy.push(band.iter().map(|&k| sa[k].norm_sqr()).sum::<f64>());
        gs.push(band.iter().map(|&k| sb[k] * sa[k].conj()).collect());
    }
    let gmax = gs.iter().flatten().map(|v| v.norm()).fold(0.0, f64::max);
    for g in &mut gs {
        for v in g.iter_mut() {
            *v /= (v.norm() + 1e-12 * gmax + 1e-300).powf(0.8);
        }
    }
    let omega: Vec<f64> = band
        .iter()
        .map(|&k| std::f64::consts::TAU * (real(k) * FS / real(WIN)) / FS)
        .collect();
    let emax = energy.iter().copied().fold(0.0, f64::max);
    let reach = WIN / 8;
    let mut cen = Vec::with_capacity(wins.len());
    let mut del = Vec::with_capacity(wins.len());
    let mut conf = Vec::with_capacity(wins.len());
    for (n, g) in gs.iter().enumerate() {
        let mut full = vec![Complex64::new(0.0, 0.0); WIN / 2 + 1];
        for (&k, v) in band.iter().zip(g) {
            full[k] = *v;
        }
        let c = irfft(&full, WIN);
        let (mut best, mut delay) = (f64::NEG_INFINITY, 0.0);
        let cand = (0..=reach)
            .map(|k| (k as f64, c[k]))
            .chain((1..=reach).rev().map(|k| (-(k as f64), c[WIN - k])));
        for (lag, v) in cand {
            if v > best {
                best = v;
                delay = lag;
            }
        }
        for _ in 0..4 {
            let (mut r1, mut r2) = (0.0, 0.0);
            for (v, &wk) in g.iter().zip(&omega) {
                let turned = v * Complex64::new(0.0, wk * delay).exp();
                r1 += (turned * Complex64::new(0.0, wk)).re;
                r2 += (turned * -(wk * wk)).re;
            }
            let step = if r2 < 0.0 { r1 / r2.min(-1e-300) } else { 0.0 };
            delay -= step.clamp(-1.0, 1.0);
        }
        let turned: Complex64 = g
            .iter()
            .zip(&omega)
            .map(|(v, &wk)| v * Complex64::new(0.0, wk * delay).exp())
            .sum();
        let mag: f64 = g.iter().map(|v| v.norm()).sum();
        let mut cf = turned.re / (mag + 1e-300);
        if energy[n] <= 1e-10 * emax {
            cf = 0.0;
        }
        let (s, pl) = wins[n];
        cen.push(s + real(WIN) / 2.0);
        del.push(pl as f64 + delay);
        conf.push(cf);
    }
    (cen, del, conf)
}

/// Solves a 3×3 system by Gaussian elimination with partial pivoting.
fn solve3(mut m: [[f64; 3]; 3], mut b: [f64; 3]) -> [f64; 3] {
    for col in 0..3 {
        let piv = (col..3)
            .max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))
            .unwrap_or(col);
        m.swap(col, piv);
        b.swap(col, piv);
        let pivot = m[col];
        for row in col + 1..3 {
            let f = m[row][col] / pivot[col];
            for (cell, above) in m[row].iter_mut().zip(&pivot).skip(col) {
                *cell -= f * above;
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; 3];
    for row in (0..3).rev() {
        let s: f64 = (row + 1..3).map(|k| m[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / m[row][row];
    }
    x
}

/// A local quadratic regression of `y(t)` under a Gaussian kernel of
/// width `sigma` and weights `w`, read at `t`: a slow wow keeps its
/// amplitude, which a moving average flattens. `t` lies on a regular grid
/// with gaps.
fn smooth_track(t: &[f64], y: &[f64], w: &[f64], sigma: f64) -> Vec<f64> {
    use crate::signal::correlate_same;
    if t.len() < 3 {
        return y.to_vec();
    }
    let diffs: Vec<f64> = t.windows(2).map(|p| p[1] - p[0]).collect();
    let hop = median(&diffs);
    let idx: Vec<usize> = t
        .iter()
        .map(|v| ((v - t[0]) / hop).round_ties_even() as usize)
        .collect();
    let cells = idx[idx.len() - 1] + 1;
    let (mut wg, mut yg) = (vec![0.0; cells], vec![0.0; cells]);
    for (k, &i) in idx.iter().enumerate() {
        wg[i] = w[k];
        yg[i] = y[k];
    }
    let half = (4.0 * sigma / hop).ceil() as i64;
    let offsets: Vec<f64> = (-half..=half).map(|k| k as f64 * hop / sigma).collect();
    let kern: Vec<f64> = offsets.iter().map(|u| (-0.5 * u * u).exp()).collect();
    let corr = |v: &[f64], p: i32| {
        let taps: Vec<f64> = (kern.iter().zip(&offsets))
            .map(|(k, u)| k * u.powi(p))
            .collect();
        correlate_same(v, &taps)
    };
    let mm: Vec<Vec<f64>> = (0..5).map(|p| corr(&wg, p)).collect();
    let wy: Vec<f64> = wg.iter().zip(&yg).map(|(w, y)| w * y).collect();
    let bb: Vec<Vec<f64>> = (0..3).map(|p| corr(&wy, p)).collect();
    idx.iter()
        .map(|&i| {
            let reg = 1e-9 * (mm[0][i] + 1e-300);
            let mx = [
                [mm[0][i] + reg, mm[1][i], mm[2][i]],
                [mm[1][i], mm[2][i] + reg, mm[3][i]],
                [mm[2][i], mm[3][i], mm[4][i] + reg],
            ];
            solve3(mx, [bb[0][i], bb[1][i], bb[2][i]])[0]
        })
        .collect()
}

const TAPS: i64 = 24;
const PHASES: usize = 2048;

/// The windowed-sinc kernel at each of [`PHASES`] fractions of a sample.
fn kernel_table() -> Vec<Vec<f64>> {
    let (half, beta, fc) = (TAPS as f64, 9.0, 0.92);
    let denom = bessel_i0(beta);
    (0..=PHASES)
        .map(|p| {
            let fr = real(p) / real(PHASES);
            let h: Vec<f64> = (-TAPS + 1..=TAPS)
                .map(|o| {
                    let n = o as f64 - fr;
                    let r = (1.0 - (n / half).powi(2)).clamp(0.0, 1.0);
                    fc * sinc(fc * n) * bessel_i0(beta * r.sqrt()) / denom
                })
                .collect();
            let s: f64 = h.iter().sum();
            h.iter().map(|v| v / s).collect()
        })
        .collect()
}

/// `x` read at fractional positions `pos` through a windowed sinc.
fn resample_at(x: &[[f64; 2]], pos: &[f64]) -> Frames {
    let table = kernel_table();
    let n = signed(x.len());
    pos.iter()
        .map(|&p| {
            let base = p.floor();
            let ph = ((p - base) * PHASES as f64).round_ties_even() as usize;
            let k = &table[ph];
            let base = base as i64;
            let mut out = [0.0; 2];
            for (j, o) in (-TAPS + 1..=TAPS).enumerate() {
                let bi = base + o;
                if bi >= 0 && bi < n {
                    let f = x[bi as usize];
                    out[0] += f[0] * k[j];
                    out[1] += f[1] * k[j];
                }
            }
            out
        })
        .collect()
}

/// `y[t] = x[start + t + delay]` for `length` samples, a fractional
/// delay applied by a phase shift over a padded stretch.
fn frac_shift(x: &[[f64; 2]], start: i64, length: usize, delay: f64) -> Frames {
    let di = delay.floor();
    let fr = delay - di;
    let pad = 256_usize;
    let s0 = start + di as i64 - signed(pad);
    let len = length + 2 * pad;
    let mut seg = vec![[0.0; 2]; len];
    let lo = s0.max(0);
    let hi = signed(x.len()).min(s0 + signed(len));
    for i in lo..hi.max(lo) {
        seg[(i - s0) as usize] = x[i as usize];
    }
    if fr.abs() > 1e-9 {
        let m = fast_len(len);
        for ch in 0..2 {
            let col: Vec<f64> = seg.iter().map(|f| f[ch]).collect();
            let mut s = rfft(&col, m);
            for (k, v) in s.iter_mut().enumerate() {
                let f = real(k) / real(m);
                *v *= Complex64::new(0.0, std::f64::consts::TAU * f * fr).exp();
            }
            let y = irfft(&s, m);
            for (f, v) in seg.iter_mut().zip(&y) {
                f[ch] = *v;
            }
        }
    }
    seg[pad..pad + length].to_vec()
}

/// The residual fractional delay of `b` against `a` from the
/// coherence-weighted slope of their cross-spectrum's phase over a
/// central 30 s at most: the delay and its standard error.
fn phase_slope(a: &[f64], b: &[f64]) -> (f64, f64) {
    let most = (30.0 * FS) as usize;
    let (a, b) = if a.len() > most {
        let c0 = (a.len() - most) / 2;
        (&a[c0..c0 + most], &b[c0..c0 + most])
    } else {
        (a, b)
    };
    let pab = csd(a, b, 4096);
    if pab.is_empty() {
        return (0.0, f64::INFINITY);
    }
    let paa = welch(a, 4096);
    let pbb = welch(b, 4096);
    let nseg = (2 * a.len() / 4096).saturating_sub(1).max(1);
    let (mut sw, mut swp, mut sel) = (0.0, 0.0, 0);
    for k in 0..pab.len() {
        let hz = real(k) * FS / 4096.0;
        let g2 = pab[k].norm_sqr() / (paa[k] * pbb[k] + 1e-300);
        if !(200.0..=12_000.0).contains(&hz) || g2 <= 0.3 {
            continue;
        }
        sel += 1;
        let w = std::f64::consts::TAU * hz / FS;
        let var = (1.0 - g2) / (2.0 * real(nseg) * g2 + 1e-300);
        let wt = 1.0 / var.max(1e-12);
        swp += wt * w * pab[k].arg();
        sw += wt * w * w;
    }
    if sel < 8 {
        return (0.0, f64::INFINITY);
    }
    (-swp / sw, 1.0 / sw.sqrt())
}

/// Every copy placed on the anchor's timeline over their common span, of
/// at most `max_span_s` around its middle: the aligned copies, each one's
/// record, the anchor and where the span starts in its samples.
// The steps read in the order they run; split up, the order would be in the call sites.
#[expect(clippy::too_many_lines)]
#[must_use]
pub fn align_all(xs: &[Frames], max_span_s: f64) -> Segment {
    const WARP_SIGMA_S: f64 = 0.15;
    const WARP_MIN_US: f64 = 8.0;
    let copies = xs.len();
    let (anchor, mut gl) = global_lags(xs);
    let a_full = mid(&xs[anchor]);
    let la = signed(xs[anchor].len());
    for j in 0..copies {
        let ov = la.min(signed(xs[j].len()) - gl[j].lag) - (-gl[j].lag).max(0);
        if j != anchor && ov < (2.0 * FS) as i64 {
            gl[j].peak = 0.0;
        }
    }
    let mut span_of: Vec<usize> = (0..copies).filter(|&j| gl[j].peak >= 0.1).collect();
    if span_of.is_empty() {
        span_of.push(anchor);
    }
    let lead = |j: &usize| -gl[*j].lag;
    let tail = |j: &usize| signed(xs[*j].len()) - gl[*j].lag;
    let margin = (0.35 * FS) as i64;
    let base0 = span_of.iter().map(lead).fold(0, i64::max);
    let base1 = span_of.iter().map(tail).fold(la, i64::min);
    let (mut t0, mut t1) = (base0 + margin, base1 - margin);
    if t1 - t0 < (4.0 * FS) as i64 {
        (t0, t1) = (base0, base1);
    }
    let span = (max_span_s * FS) as i64;
    if t1 - t0 > span {
        let centre = (t0 + t1).div_euclid(2);
        t0 = centre - span.div_euclid(2);
        t1 = t0 + span;
    }
    if t1 - t0 < 2048 {
        t1 = t0 + 2048;
    }
    let frames = (t1 - t0) as usize;
    let a_span: Vec<f64> = py_slice(&a_full, t0, t1).to_vec();
    let mut a12: Option<Vec<f64>> = None;
    let mut ys = Vec::with_capacity(copies);
    let mut info = Vec::with_capacity(copies);
    for j in 0..copies {
        let turned: Frames = xs[j]
            .iter()
            .map(|f| [f[0] * gl[j].pol, f[1] * gl[j].pol])
            .collect();
        if j == anchor {
            ys.push(frac_shift(&turned, 0, frames, t0 as f64));
            info.push(Info {
                resid_us: Some(0.0),
                polarity: 1,
                peak: 1.0,
                anchor: true,
                ok: true,
                ..Info::default()
            });
            continue;
        }
        if gl[j].peak < 0.1 {
            ys.push(frac_shift(&turned, 0, frames, (gl[j].lag + t0) as f64));
            let lo = t0.max(-gl[j].lag);
            let hi = t1.min(signed(xs[j].len()) - gl[j].lag);
            info.push(Info {
                lag_s: gl[j].lag as f64 / FS,
                polarity: gl[j].pol as i32,
                peak: gl[j].peak,
                coverage: Some((hi - lo).max(0) as f64 / real(frames)),
                ..Info::default()
            });
            continue;
        }
        let b_full = mid(&turned);
        let lag0 = gl[j].lag;
        let a12 = a12.get_or_insert_with(|| resample_poly(&a_full, 1, 4));
        let b12 = resample_poly(&b_full, 1, 4);
        let a12s = py_slice(a12, t0.div_euclid(4), t1.div_euclid(4));
        let (tb, lb, wb) = block_lags(
            a12s,
            &b12,
            (lag0 + t0).div_euclid(4),
            FS as usize,
            (0.3 * FS / 4.0) as usize,
        );
        let tb: Vec<f64> = tb.iter().map(|v| v * 4.0).collect();
        let lb: Vec<f64> = lb.iter().map(|v| v * 4.0).collect();
        let (mut slope, mut icpt) = if tb.len() >= 3 {
            let (rise, at_zero, _) = robust_line(&tb, &lb, &wb);
            (rise, at_zero)
        } else {
            (0.0, (lag0 + t0) as f64)
        };
        let (cen, delays, conf) = {
            let (rise, at_zero) = (slope, icpt);
            fine_track(&a_span, &b_full, move |t| at_zero + rise * t)
        };
        let pos_conf: Vec<f64> = conf.iter().copied().filter(|&c| c > 0.0).collect();
        let good: Vec<bool> = if pos_conf.is_empty() {
            conf.iter().map(|&c| c > 0.0).collect()
        } else {
            let lim = (0.3 * median(&pos_conf)).max(0.05);
            conf.iter().map(|&c| c > lim).collect()
        };
        let pick = |values: &[f64], taken: &[bool]| -> Vec<f64> {
            (values.iter().zip(taken))
                .filter(|(_, taken)| **taken)
                .map(|(value, _)| *value)
                .collect()
        };
        let (mut rms, mut warped) = (0.0, false);
        let mut kept = good.clone();
        let mut sm = Vec::new();
        if good.iter().filter(|&&v| v).count() >= 5 {
            let cg = pick(&cen, &good);
            let dg = pick(&delays, &good);
            let wg: Vec<f64> = pick(&conf, &good).iter().map(|c| c * c).collect();
            let (s2, c2, k2) = robust_line(&cg, &dg, &wg);
            (slope, icpt) = (s2, c2);
            let resid: Vec<f64> = (delays.iter().zip(&cen))
                .map(|(delay, at)| delay - (icpt + slope * at))
                .collect();
            let mut it = k2.iter();
            for v in kept.iter_mut().filter(|v| **v) {
                *v = *it.next().unwrap_or(&false);
            }
            let rg = pick(&resid, &kept);
            let noise = if kept.iter().filter(|&&v| v).count() > 3 {
                let dif: Vec<f64> = rg.windows(2).map(|p| (p[1] - p[0]).abs()).collect();
                median(&dif) / 0.954 + 1e-6
            } else {
                1.0
            };
            let cw: Vec<f64> = pick(&conf, &kept).iter().map(|c| c * c).collect();
            sm = smooth_track(&pick(&cen, &kept), &rg, &cw, WARP_SIGMA_S * FS);
            let neff = (2.5 * WARP_SIGMA_S * FS / 1024.0).max(1.0);
            rms = (sm.iter().map(|v| v * v).sum::<f64>() / real(sm.len())).sqrt();
            warped = rms > 3.0 * noise / neff.sqrt() && rms * 1e6 / FS > WARP_MIN_US;
        }
        let drift_sig = slope.abs() * real(frames) > 0.05;
        let mut se = f64::INFINITY;
        let placed = if warped {
            let cg = pick(&cen, &kept);
            let t_rel: Vec<f64> = (0..frames).map(real).collect();
            let wow = interp_linear(&cg, &sm, &t_rel, sm[0], sm[sm.len() - 1]);
            let pos: Vec<f64> = t_rel
                .iter()
                .zip(&wow)
                .map(|(t, w)| icpt + slope * t + w + t)
                .collect();
            resample_at(&turned, &pos)
        } else if drift_sig {
            let pos: Vec<f64> = (0..frames)
                .map(|t| icpt + slope * real(t) + real(t))
                .collect();
            resample_at(&turned, &pos)
        } else {
            let dl = icpt + slope * real(frames) / 2.0;
            let di = dl.round_ties_even();
            let bf: Frames = b_full.iter().map(|v| [*v, 0.0]).collect();
            let yi: Vec<f64> = frac_shift(&bf, 0, frames, di)
                .iter()
                .map(|f| f[0])
                .collect();
            let (dd, error) = phase_slope(&a_span, &yi);
            se = error;
            let dl = di
                + if se.is_finite() && dd.abs() < 2.0 {
                    dd
                } else {
                    dl - di
                };
            frac_shift(&turned, 0, frames, dl)
        };
        if warped || drift_sig {
            se = phase_slope(&a_span, &mid(&placed)).1;
        }
        ys.push(placed);
        info.push(Info {
            lag_s: (icpt + slope * real(frames) / 2.0 - t0 as f64) / FS,
            drift_ppm: slope * 1e6,
            warp_rms_us: if warped { rms * 1e6 / FS } else { 0.0 },
            warp_applied: warped,
            resid_us: se.is_finite().then(|| se * 1e6 / FS),
            polarity: gl[j].pol as i32,
            peak: gl[j].peak,
            ok: gl[j].peak > 0.2,
            ..Info::default()
        });
    }
    Segment {
        ys,
        info,
        anchor,
        start: t0,
    }
}

/// Scale factors bringing each copy's 1–8 kHz mid energy, the median over
/// 4 s blocks of its log ratio, to the geometric mean of the copies in
/// `use_`, all of them when it is empty; and each one's gain in dB. A
/// copy outside `use_` keeps its level.
#[must_use]
pub fn gain_match(ys: &[Frames], use_: &[usize]) -> (Vec<f64>, Vec<f64>) {
    let k = ys.len();
    let n = ys.iter().map(Vec::len).min().unwrap_or(0);
    let block = (4.0 * FS) as usize;
    let sos = butter(6, Band::Pass(1000.0, 8000.0), FS);
    let e: Vec<Vec<f64>> = ys
        .iter()
        .map(|y| {
            let m = sosfiltfilt(&sos, &mid(&y[..n]));
            let nb = (n / block).max(1);
            (0..nb)
                .map(|i| {
                    let s = &m[(i * block).min(m.len())..((i + 1) * block).min(m.len())];
                    s.iter().map(|v| v * v).sum::<f64>() / real(s.len()) + 1e-30
                })
                .collect()
        })
        .collect();
    let use_: Vec<usize> = if use_.is_empty() {
        (0..k).collect()
    } else {
        use_.to_vec()
    };
    let nb = e[0].len();
    let emax = use_
        .iter()
        .flat_map(|&i| e[i].iter().copied())
        .fold(f64::NEG_INFINITY, f64::max);
    let mut act: Vec<bool> = (0..nb)
        .map(|b| use_.iter().map(|&i| e[i][b]).fold(f64::INFINITY, f64::min) > 1e-9 * emax)
        .collect();
    if !act.iter().any(|&a| a) {
        act.fill(true);
    }
    let cols: Vec<usize> = (0..nb).filter(|&b| act[b]).collect();
    let le: Vec<Vec<f64>> = e
        .iter()
        .map(|r| cols.iter().map(|&b| r[b].ln()).collect())
        .collect();
    let means: Vec<f64> = (0..cols.len())
        .map(|c| use_.iter().map(|&i| le[i][c]).sum::<f64>() / real(use_.len()))
        .collect();
    let rel: Vec<f64> = (0..k)
        .map(|i| {
            if use_.contains(&i) {
                let d: Vec<f64> = le[i].iter().zip(&means).map(|(l, m)| l - m).collect();
                median(&d)
            } else {
                0.0
            }
        })
        .collect();
    let scale = rel.iter().map(|r| (-r / 2.0).exp()).collect();
    let db = rel
        .iter()
        .map(|r| r * 10.0 / std::f64::consts::LN_10)
        .collect();
    (scale, db)
}

/// The lag, at 12 kHz, of the anchor's window `a12[s12..s12 + w12]` in
/// `b12`, searched within `reach12` of `pred12`, or over the whole copy;
/// with the normalised correlation there.
fn local_lag(
    a12: &[f64],
    b12: &[f64],
    s12: i64,
    w12: i64,
    pred12: i64,
    reach12: Option<i64>,
) -> (i64, f64) {
    let sa = py_slice(a12, s12, s12 + w12);
    let (lo, hi) = match reach12 {
        None => (0, signed(b12.len())),
        Some(r) => (
            (s12 + pred12 - r).max(0),
            signed(b12.len()).min(s12 + pred12 + w12 + r),
        ),
    };
    let sb = py_slice(b12, lo, hi);
    if signed(sb.len()) < w12.div_euclid(2) || sa.len() < 64 {
        return (pred12, 0.0);
    }
    let (l, _, _) = phat_lag(sa, sb, sb.len().max(sa.len()));
    let lag = lo + l - s12;
    let p = s12 + lag;
    if p < 0 || p + signed(sa.len()) > signed(b12.len()) {
        return (lag, 0.0);
    }
    (
        lag,
        norm_corr(sa, &b12[p as usize..p as usize + sa.len()], 0).abs(),
    )
}

/// The 12 kHz and 2 kHz decimations of a whole song's mid, each through
/// an 8th-order Butterworth lowpass then subsampled: every copy through
/// the same filter, so its phase delay cancels in a lag between copies.
fn decimate(m: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let m12: Vec<f64> = sosfilt(&butter(8, Band::Low(5000.0), FS), m)
        .into_iter()
        .step_by(4)
        .collect();
    let m2: Vec<f64> = sosfilt(&butter(8, Band::Low(800.0), FS / 4.0), &m12)
        .into_iter()
        .step_by(6)
        .collect();
    (m12, m2)
}

/// [`global_lags`] for whole songs, on 2 kHz decimations; also each
/// copy's 12 kHz decimation, for the search of each segment.
fn global_lags_long(xs: &[Frames]) -> (usize, Vec<Global>, Vec<Vec<f64>>) {
    let k = xs.len();
    let dec: Vec<(Vec<f64>, Vec<f64>)> = xs.iter().map(|x| decimate(&mid(x))).collect();
    let reach = (60.0 * FS / 24.0) as usize;
    let mut peak = vec![vec![0.0; k]; k];
    let mut lag = vec![vec![0_i64; k]; k];
    let mut pol = vec![vec![1.0; k]; k];
    for i in 0..k {
        for j in i + 1..k {
            let (l, s, _) = phat_lag(&dec[i].1, &dec[j].1, reach);
            let c = norm_corr(&dec[i].1, &dec[j].1, l).abs();
            peak[i][j] = c;
            peak[j][i] = c;
            lag[i][j] = l;
            lag[j][i] = -l;
            pol[i][j] = s;
            pol[j][i] = s;
        }
    }
    let score: Vec<f64> = peak.iter().map(|r| r.iter().sum()).collect();
    let keys: Vec<String> = xs.iter().map(|x| content_key(x)).collect();
    let lens: Vec<usize> = xs.iter().map(Vec::len).collect();
    let anchor = pick_anchor(&score, &lens, &keys);
    let out = (0..k)
        .map(|j| {
            if j == anchor {
                Global {
                    lag: 0,
                    pol: 1.0,
                    peak: 1.0,
                }
            } else {
                Global {
                    lag: lag[anchor][j] * 24,
                    pol: pol[anchor][j],
                    peak: peak[anchor][j],
                }
            }
        })
        .collect();
    (anchor, out, dec.into_iter().map(|d| d.0).collect())
}

/// The lag of a copy at 12 kHz, `b12`, against the anchor's `seg`
/// samples from `s`, searched within 0.75 s of its `global` lag, and,
/// found weakly there, anywhere: the lag in samples at 48 kHz and its
/// correlation peak.
fn segment_lag(a12: &[f64], b12: &[f64], global: &Global, s: i64, seg: i64) -> (i64, f64) {
    let (from, len, pred) = (s.div_euclid(4), seg.div_euclid(4), global.lag.div_euclid(4));
    let near = Some((3.0 * FS / 4.0) as i64);
    let (mut lag, mut peak) = local_lag(a12, b12, from, len, pred, near);
    if peak < 0.5 * global.peak.max(0.2) {
        let anywhere = local_lag(a12, b12, from, len, pred, None);
        if anywhere.1 > peak {
            (lag, peak) = anywhere;
        }
    }
    (4 * lag, peak)
}

/// The copies aligned over at most `max_span_s` of their common span: in
/// one window around its middle, or, where the span is longer, in
/// `max_span_s / seg_s` segments of `seg_s` spread evenly over it. The
/// segments, the main anchor and the span in samples.
#[must_use]
pub fn align_segments(xs: &[Frames], max_span_s: f64, seg_s: f64) -> (Vec<Segment>, usize, usize) {
    let k = xs.len();
    let longest = xs.iter().map(Vec::len).max().unwrap_or(0);
    if k == 1 || longest <= ((max_span_s + 1.0) * FS) as usize {
        let s = align_all(xs, max_span_s);
        let (anchor, n) = (s.anchor, s.ys[0].len());
        return (vec![s], anchor, n);
    }
    let (anchor, gl, d12) = global_lags_long(xs);
    let rel: Vec<usize> = (0..k)
        .filter(|&j| j == anchor || gl[j].peak >= 0.1)
        .collect();
    let t0 = rel.iter().map(|&j| -gl[j].lag).fold(0, i64::max);
    let t1 = rel
        .iter()
        .map(|&j| signed(xs[j].len()) - gl[j].lag)
        .fold(signed(xs[anchor].len()), i64::min);
    let span = t1 - t0;
    let span_out = span.max(0) as usize;
    if span <= (max_span_s * FS) as i64 + FS as i64 {
        return (vec![align_all(xs, max_span_s)], anchor, span_out);
    }
    let seg = (seg_s * FS) as i64;
    let nseg = ((max_span_s / seg_s).floor() as usize).max(2);
    let margin = (0.5 * FS) as i64;
    let (lo_t, hi_t) = (t0 + margin, t1 - margin - seg);
    let starts: Vec<i64> = (0..nseg)
        .map(|s| {
            (lo_t as f64 + real(s) * (hi_t - lo_t) as f64 / real(nseg - 1)).round_ties_even() as i64
        })
        .collect();
    let a12 = &d12[anchor];
    let b12s: Vec<Option<Vec<f64>>> = (0..k)
        .map(|j| {
            (j != anchor && rel.contains(&j))
                .then(|| d12[j].iter().map(|v| v * gl[j].pol).collect())
        })
        .collect();
    let mut out = Vec::new();
    for &s in &starts {
        let mut lags: Vec<Option<i64>> = vec![None; k];
        let mut ok = true;
        for &j in rel.iter().filter(|&&j| j != anchor) {
            let b12 = b12s[j].as_deref().expect("made for every related copy");
            let (lag, peak) = segment_lag(a12, b12, &gl[j], s, seg);
            if peak < 0.1 {
                ok = false;
            }
            lags[j] = Some(lag);
        }
        if !ok && starts.len() > 1 {
            continue;
        }
        let sub: Vec<Frames> = (0..k)
            .map(|j| {
                let x = &xs[j];
                if j == anchor {
                    py_slice(x, s, s + seg).to_vec()
                } else if let Some(l) = lags[j] {
                    let b0 = s + l - margin;
                    let piece = py_slice(x, b0.max(0), (b0 + seg + 2 * margin).max(0));
                    let mut v = vec![[0.0; 2]; (-b0).max(0) as usize];
                    v.extend_from_slice(piece);
                    v
                } else {
                    let b0 = s + gl[j].lag;
                    let piece = py_slice(x, b0.max(0), (b0 + seg).max(0));
                    if piece.is_empty() {
                        vec![[0.0; 2]; seg as usize]
                    } else {
                        piece.to_vec()
                    }
                }
            })
            .collect();
        let mut sg = align_all(&sub, seg_s + 2.0);
        for (j, inf) in sg.info.iter_mut().enumerate() {
            inf.segment_start_s = Some(s as f64 / FS);
            inf.lag_s_local = Some(inf.lag_s);
            inf.lag_s = if j == anchor {
                0.0
            } else {
                lags[j].unwrap_or(0) as f64 / FS
            };
        }
        sg.start = if sg.anchor == anchor { s + sg.start } else { s };
        out.push(sg);
    }
    if out.is_empty() {
        return (vec![align_all(xs, max_span_s)], anchor, span_out);
    }
    (out, anchor, span_out)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{music, sped};
    use super::*;

    #[test]
    fn a_later_quieter_inverted_copy_lands_on_the_anchor() {
        let x = music(5.0, 11);
        let y: Frames = x[1000..]
            .iter()
            .map(|f| [-0.5 * f[0], -0.5 * f[1]])
            .collect();
        let s = align_all(&[x, y], 60.0);
        assert_eq!(s.anchor, 0, "the longer copy sets the timeline");
        let i = &s.info[1];
        assert!((i.lag_s * FS + 1000.0).abs() < 0.01, "{}", i.lag_s * FS);
        assert_eq!((i.polarity, i.ok, i.warp_applied), (-1, true, false));
        assert!(i.drift_ppm.abs() < 1.0, "{}", i.drift_ppm);
        let (scale, db) = gain_match(&s.ys, &[0, 1]);
        assert!((db[0] - db[1] - 20.0 * 2f64.log10()).abs() < 0.01, "{db:?}");
        let worst = (s.ys[0].iter().zip(&s.ys[1]))
            .map(|(a, b)| (a[0] * scale[0] - b[0] * scale[1]).abs())
            .fold(0.0, f64::max);
        assert!(worst < 1e-6, "{worst}");
    }

    #[test]
    fn a_fast_copys_drift_is_measured_and_taken_out() {
        let x = music(5.0, 12);
        let s = align_all(&[x.clone(), sped(&x, 300.0)], 60.0);
        let drift = s
            .info
            .iter()
            .map(|i| i.drift_ppm)
            .fold(0.0_f64, |m, v| if v.abs() > m.abs() { v } else { m });
        assert!((drift.abs() - 300.0).abs() < 5.0, "{drift}");
    }

    #[test]
    fn unrelated_audio_is_placed_but_not_tracked() {
        let s = align_all(&[music(6.0, 13), music(6.0, 99)], 60.0);
        let other = 1 - s.anchor;
        assert!(s.info[other].peak < 0.1, "{}", s.info[other].peak);
        assert!(!s.info[other].ok && s.info[other].coverage.is_some());
    }

    #[test]
    fn keys_follow_the_samples_alone() {
        let x = music(1.0, 14);
        assert_eq!(content_key(&x), content_key(&x.clone()));
        assert_ne!(content_key(&x), content_key(&music(1.0, 15)));
    }
}
