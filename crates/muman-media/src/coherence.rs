//! Copies of one recording compared band by band on one timeline: the
//! analysis [`crate::gate`] reads.
//!
//! [`analyse`] puts every copy on one 48 kHz timeline ([`align`]), then
//! measures, over at most 60 s of it:
//!
//! - **Coherence** of every pair's mids per bin, from the cross-spectra
//!   pooled over every frame, its bias corrected as `(n g² - 1) / (n - 1)`
//!   for `n` effective frames: a Hann window at half overlap leaves one
//!   independent frame per [`NEFF`].
//! - **Distance:** the median 1–8 kHz noise-to-signal ratio `(1 - g²) / g²`
//!   of each pair, which codec noise raises and which no gain, polarity or
//!   delay moves.
//! - **Loudness slope** of each pair over the loud passages alone: the
//!   windows of 0.4 s, hop 0.1 s, within 12 dB of both copies' loudest,
//!   each window freed of its mean, as a DC offset would lift quiet
//!   passages. Limiting and compression act on the loud passages.
//! - **Lowpass edge** of each copy's own long-term mid spectrum
//!   ([`edge_hz`]).
//!
//! A song longer than 61 s of common span is read as 7 segments of 8 s
//! spread over it, each aligned on its own, rather than one window: an
//! edit or a long drift then needs no model of the whole song. Nothing
//! above 20 kHz is read, and no input order, name, codec or rate enters
//! anything.

pub mod align;
#[cfg(any(test, feature = "testing"))]
pub mod fixtures;

use crate::signal::{Complex64, FS, Frames, correlate_same, mean, median, real, rfft, std};

pub use align::Info;

/// Samples a frame of the analysis.
pub const N: usize = 2048;
/// Samples between frames.
pub const HOP: usize = 1024;
/// Frames per effective degree of freedom of a Hann window at half
/// overlap.
pub const NEFF: f64 = 1.056;
/// Seconds analysed in all.
const MAX_SPAN_S: f64 = 60.0;
/// Seconds a segment of a whole song.
const SEG_S: f64 = 8.0;

/// The frequency of bin `k` of a frame.
#[must_use]
pub fn freq(k: usize) -> f64 {
    real(k) * FS / real(N)
}

/// What comparing two copies found.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pair {
    /// The median 1–8 kHz noise-to-signal ratio of the mids.
    pub nsr_mid: f64,
    /// The loudness transfer's slope over the loud passages: 1 where the
    /// two play their loud passages alike.
    pub loud_slope: f64,
}

/// The analysis of a group of copies.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub k: usize,
    /// A key of each copy's samples, for tie-breaks that must not depend
    /// on the order copies come in.
    pub keys: Vec<String>,
    /// The aligned copies, gain matched, segments joined.
    pub x: Vec<Frames>,
    pub align: Vec<Info>,
    /// The bins analysed, from 50 Hz up to 20 kHz.
    pub kb: Vec<usize>,
    /// Each copy's mid spectrum per frame, `N / 2 + 1` bins a frame.
    pub mid: Vec<Vec<Complex64>>,
    pub nframes: usize,
    /// Frames where the loudest copy's 1–8 kHz mid stands within 50 dB of
    /// its loudest.
    pub active: Vec<bool>,
    /// The mids' cross-spectra pooled over every frame: (analysed bin, K, K).
    pub stot: Vec<Complex64>,
    /// The mids' bias-corrected coherence: (analysed bin, K, K).
    pub g2mid: Vec<f64>,
    /// Each pair `(i, j)`, `i < j`, at `i * K + j`.
    pub pairs: Vec<Pair>,
    /// Each copy's own lowpass edge.
    pub edge_hz: Vec<Option<f64>>,
}

impl Analysis {
    /// Pair `(i, j)` of the copies, either way round.
    #[must_use]
    pub fn pair(&self, i: usize, j: usize) -> Pair {
        self.pairs[i.min(j) * self.k + i.max(j)]
    }

    /// The pooled cross-spectrum of copies `i` and `j` at analysed bin `b`.
    #[must_use]
    pub fn cross(&self, b: usize, i: usize, j: usize) -> Complex64 {
        self.stot[(b * self.k + i) * self.k + j]
    }

    /// The coherence of copies `i` and `j` at analysed bin `b`.
    #[must_use]
    pub fn coherence(&self, b: usize, i: usize, j: usize) -> f64 {
        self.g2mid[(b * self.k + i) * self.k + j]
    }
}

/// One record per copy from its records in each segment: the medians of
/// lag, drift, residual, peak and coverage, the largest warp, and each
/// segment's record kept.
fn merge_info(segs: &[align::Segment], k: usize) -> Vec<Info> {
    if segs.len() == 1 {
        return segs[0].info.clone();
    }
    (0..k)
        .map(|i| {
            let recs: Vec<Info> = segs.iter().map(|s| s.info[i].clone()).collect();
            let mut d = recs[0].clone();
            let med = |f: &dyn Fn(&Info) -> Option<f64>| {
                let v: Vec<f64> = recs.iter().filter_map(f).collect();
                (!v.is_empty()).then(|| median(&v))
            };
            d.lag_s = med(&|r| Some(r.lag_s)).unwrap_or(d.lag_s);
            d.drift_ppm = med(&|r| Some(r.drift_ppm)).unwrap_or(d.drift_ppm);
            d.resid_us = med(&|r| r.resid_us).or(d.resid_us);
            d.peak = med(&|r| Some(r.peak)).unwrap_or(d.peak);
            d.coverage = med(&|r| r.coverage).or(d.coverage);
            d.warp_rms_us = recs.iter().map(|r| r.warp_rms_us).fold(0.0, f64::max);
            d.ok = recs.iter().all(|r| r.ok);
            d.anchor = recs.iter().all(|r| r.anchor);
            d.segments = recs;
            d
        })
        .collect()
}

/// The unscaled spectrum of each periodic-Hann frame of a copy's mid.
fn stft_mid(x: &[[f64; 2]]) -> Vec<Complex64> {
    let win = crate::signal::hann(N);
    let mut padded = x.to_vec();
    if padded.len() < N {
        padded.resize(N, [0.0; 2]);
    }
    let frames = 1 + (padded.len() - N) / HOP;
    let mut out = Vec::with_capacity(frames * (N / 2 + 1));
    for t in 0..frames {
        let s = t * HOP;
        let m: Vec<f64> = (0..N)
            .map(|n| f64::midpoint(padded[s + n][0] * win[n], padded[s + n][1] * win[n]))
            .collect();
        out.extend(rfft(&m, N));
    }
    out
}

/// A lowpass edge in a long-term spectrum `p` over frequencies `f`: the
/// lowest frequency from 4.4 kHz where the level, smoothed over 9 bins,
/// falls 25 dB or more across 500 Hz and never comes back within 20 dB
/// of the level below it. A filter's cut falls tens of dB within a few
/// hundred Hz; a dark master's natural roll-off does not.
#[must_use]
pub fn edge_hz(p: &[f64], f: &[f64]) -> Option<f64> {
    let lv: Vec<f64> = correlate_same(p, &[1.0 / 9.0; 9])
        .iter()
        .map(|v| 10.0 * (v + 1e-30).log10())
        .collect();
    let w = ((500.0 / (f[1] - f[0])).round_ties_even() as usize).max(1);
    let mut top = lv.clone();
    for k in (0..top.len().saturating_sub(1)).rev() {
        top[k] = top[k].max(top[k + 1]);
    }
    for k in (0..f.len()).filter(|&k| f[k] >= 4400.0) {
        if k + w >= lv.len() || k < w {
            break;
        }
        let below = lv[k - w];
        if below - lv[k + w] >= 25.0 && below - top[k + w] >= 20.0 {
            return Some(f[k]);
        }
    }
    None
}

/// The level of each 0.4 s window of a copy's mid, hop 0.1 s, each window
/// freed of its mean, in dB.
fn window_levels(y: &[[f64; 2]]) -> Vec<f64> {
    let (w4, h1) = ((0.4 * FS) as usize, (0.1 * FS) as usize);
    let m: Vec<f64> = y.iter().map(|f| f64::midpoint(f[0], f[1])).collect();
    let nwin = (m.len().saturating_sub(w4) / h1).max(1);
    (0..nwin)
        .map(|w| {
            let s = &m[(w * h1).min(m.len())..(w * h1 + w4).min(m.len())];
            let mu = mean(s);
            let var = s.iter().map(|v| (v - mu).powi(2)).sum::<f64>() / real(s.len());
            10.0 * (var + 1e-12).log10()
        })
        .collect()
}

/// The slope of the loudness transfer from `b` to `a` over the windows
/// within 12 dB of both copies' loudest, its sign the sign of their
/// covariance; 1 with fewer than 5 such windows or a flat level.
fn loud_slope(la: &[f64], lb: &[f64]) -> f64 {
    let top = |l: &[f64]| l.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (amax, bmax) = (top(la), top(lb));
    let act: Vec<usize> = (0..la.len().min(lb.len()))
        .filter(|&w| la[w] > amax - 12.0 && lb[w] > bmax - 12.0)
        .collect();
    let av: Vec<f64> = act.iter().map(|&w| la[w]).collect();
    let bv: Vec<f64> = act.iter().map(|&w| lb[w]).collect();
    if act.len() < 5 || std(&av) <= 1e-6 || std(&bv) <= 1e-6 {
        return 1.0;
    }
    let (ma, mb) = (mean(&av), mean(&bv));
    let cov: f64 = av.iter().zip(&bv).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let sign = if cov == 0.0 { 0.0 } else { cov.signum() };
    sign * std(&av) / std(&bv)
}

/// The copies in segments on the anchor's timeline; one copy alone is
/// its own first [`MAX_SPAN_S`].
fn segments(xs: &[Frames]) -> Vec<align::Segment> {
    if xs.len() > 1 {
        return align::align_segments(xs, MAX_SPAN_S, SEG_S).0;
    }
    let solo = Info {
        anchor: true,
        ok: true,
        peak: 1.0,
        polarity: 1,
        ..Info::default()
    };
    let end = ((MAX_SPAN_S * FS) as usize).min(xs[0].len());
    vec![align::Segment {
        ys: vec![xs[0][..end].to_vec()],
        info: vec![solo],
        anchor: 0,
        start: 0,
    }]
}

/// Whether each of `nframes` frames is active: the loudest copy's
/// 1–8 kHz level in it within 50 dB of the loudest frame's.
fn active(mid: &[Vec<Complex64>], nframes: usize) -> Vec<bool> {
    let bins = N / 2 + 1;
    let sel18: Vec<usize> = (0..bins)
        .filter(|&b| (1000.0..8000.0).contains(&freq(b)))
        .collect();
    let level = |x: &Vec<Complex64>, t: usize| {
        let power: f64 = sel18.iter().map(|&b| x[t * bins + b].norm_sqr()).sum();
        10.0 * (power + 1e-30).log10()
    };
    let lev: Vec<f64> = (0..nframes)
        .map(|t| (mid.iter().map(|x| level(x, t))).fold(f64::NEG_INFINITY, f64::max))
        .collect();
    let loudest = lev.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    lev.iter().map(|&l| l > loudest - 50.0).collect()
}

impl Analysis {
    /// Pool every pair's cross-spectrum in each analysed bin, and take
    /// its coherence, the bias of few frames removed.
    fn fill_spectra(&mut self) {
        let (k, bins, nb) = (self.k, N / 2 + 1, self.kb.len());
        let effective = real(self.nframes) / NEFF;
        self.stot = vec![Complex64::new(0.0, 0.0); nb * k * k];
        self.g2mid = vec![0.0; nb * k * k];
        for (b, &bin) in self.kb.iter().enumerate() {
            for i in 0..k {
                for j in 0..k {
                    let pooled: Complex64 = (0..self.nframes)
                        .map(|t| self.mid[i][t * bins + bin] * self.mid[j][t * bins + bin].conj())
                        .sum::<Complex64>()
                        / real(self.nframes);
                    self.stot[(b * k + i) * k + j] = pooled;
                }
            }
            for i in 0..k {
                for j in 0..k {
                    let (di, dj) = (self.cross(b, i, i).re, self.cross(b, j, j).re);
                    let raw = self.cross(b, i, j).norm_sqr() / (di * dj + 1e-300);
                    self.g2mid[(b * k + i) * k + j] =
                        ((effective * raw - 1.0) / (effective - 1.0)).clamp(0.0, 1.0);
                }
            }
        }
    }

    /// Compare every pair of the copies `ys`, aligned and gain matched.
    fn fill_pairs(&mut self, ys: &[Frames]) {
        let k = self.k;
        let levels: Vec<Vec<f64>> = ys.iter().map(|y| window_levels(y)).collect();
        self.pairs = vec![Pair::default(); k * k];
        let s18: Vec<usize> = (0..self.kb.len())
            .filter(|&b| (1000.0..8000.0).contains(&freq(self.kb[b])))
            .collect();
        for i in 0..k {
            for j in i + 1..k {
                let ratios: Vec<f64> = s18
                    .iter()
                    .map(|&b| {
                        let g = self.coherence(b, i, j).clamp(1e-6, 1.0 - 1e-9);
                        (1.0 - g) / g
                    })
                    .collect();
                self.pairs[i * k + j] = Pair {
                    nsr_mid: median(&ratios),
                    loud_slope: loud_slope(&levels[i], &levels[j]),
                };
            }
        }
    }

    /// Each copy's lowpass edge, in its spectrum over the active frames,
    /// or over every frame when none is active.
    fn edges(&self) -> Vec<Option<f64>> {
        let bins = N / 2 + 1;
        let hz: Vec<f64> = self.kb.iter().map(|&b| freq(b)).collect();
        let mut frames: Vec<usize> = (0..self.nframes).filter(|&t| self.active[t]).collect();
        if frames.is_empty() {
            frames = (0..self.nframes).collect();
        }
        (0..self.k)
            .map(|i| {
                let power = |b: usize| -> f64 {
                    let sum: f64 = (frames.iter())
                        .map(|&t| self.mid[i][t * bins + b].norm_sqr())
                        .sum();
                    sum / real(frames.len())
                };
                let spectrum: Vec<f64> = self.kb.iter().map(|&b| power(b)).collect();
                edge_hz(&spectrum, &hz)
            })
            .collect()
    }
}

/// The analysis of decoded copies in two channels at 48 kHz.
///
/// # Panics
///
/// When given no copies.
#[must_use]
pub fn analyse(xs: &[Frames]) -> Analysis {
    let k = xs.len();
    assert!(k > 0, "a group of copies holds one or more");
    let segs = segments(xs);
    let mut ys: Vec<Frames> = (0..k)
        .map(|i| segs.iter().flat_map(|s| s.ys[i].iter().copied()).collect())
        .collect();
    let mut info = merge_info(&segs, k);
    let related: Vec<usize> = (0..k).filter(|&i| info[i].peak >= 0.1).collect();
    let (scale, gdb) = align::gain_match(&ys, &related);
    for (y, s) in ys.iter_mut().zip(&scale) {
        for f in y.iter_mut() {
            f[0] *= s;
            f[1] *= s;
        }
    }
    for (inf, g) in info.iter_mut().zip(&gdb) {
        inf.gain_db = -g;
    }
    let bins = N / 2 + 1;
    let mut found = Analysis {
        k,
        keys: xs.iter().map(|x| align::content_key(x)).collect(),
        kb: (0..bins)
            .filter(|&b| freq(b) >= 50.0 && freq(b) < 20000.0)
            .collect(),
        ..Analysis::default()
    };
    found.mid = crate::parallel::map(&ys, crate::parallel::builds(), |y| {
        let mut out = Vec::new();
        let mut at = 0;
        for s in &segs {
            let len = s.ys[0].len();
            out.extend(stft_mid(&y[at..at + len]));
            at += len;
        }
        out
    });
    found.nframes = found.mid[0].len() / bins;
    found.active = active(&found.mid, found.nframes);
    found.fill_spectra();
    found.fill_pairs(&ys);
    found.edge_hz = found.edges();
    found.x = ys;
    found.align = info;
    found
}

#[cfg(test)]
mod tests {
    use super::fixtures::{lowpassed, music, noisy};
    use super::*;

    #[test]
    fn a_copys_own_noise_is_its_distance() {
        let x = music(5.0, 1);
        let a = analyse(&[x.clone(), noisy(&x, 0.001, 7), noisy(&x, 0.01, 9)]);
        let (quiet, loud) = (a.pair(0, 1).nsr_mid, a.pair(0, 2).nsr_mid);
        assert!(quiet > 0.0 && quiet < 0.05, "{quiet}");
        assert!(
            (50.0..200.0).contains(&(loud / quiet)),
            "ten times the noise, a hundred times the ratio: {quiet} {loud}"
        );
        assert_eq!(a.pair(1, 0), a.pair(0, 1), "either way round");
        assert!(
            (a.pair(0, 1).loud_slope - 1.0).abs() < 0.02,
            "{}",
            a.pair(0, 1).loud_slope
        );
        assert_eq!(
            a.edge_hz,
            [None, None, None],
            "sound to the top shows no edge"
        );
    }

    #[test]
    fn a_lowpass_shows_as_an_edge() {
        let x = music(5.0, 2);
        let a = analyse(&[x.clone(), lowpassed(&x, 15000.0)]);
        let cut = a.edge_hz.iter().flatten().copied().next().expect("an edge");
        assert!((14500.0..16500.0).contains(&cut), "{cut}");
        assert_eq!(
            a.edge_hz.iter().flatten().count(),
            1,
            "the other copy has none"
        );
    }

    #[test]
    fn one_copy_is_read_alone() {
        let a = analyse(&[music(3.0, 3)]);
        assert_eq!((a.k, a.align[0].anchor, a.edge_hz.len()), (1, true, 1));
    }
}
