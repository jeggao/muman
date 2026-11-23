//! The arithmetic [`crate::coherence`] and [`crate::gate`] compare a
//! song's copies with: copies decoded to two channels at [`RATE`], real
//! FFTs of any length ([`fast_len`]), periodic Hann windows, Welch's
//! cross- and auto-spectra, linear interpolation and correlation, and
//! medians. Its filters, Butterworth sections run once or forward and
//! back, and polyphase resampling, are in [`filter`].
//!
//! Each routine computes what the `numpy` or `scipy` routine it is named
//! for computes, to rounding: the gate's thresholds were set on audio
//! measured with those, so what they were set on is what muman computes,
//! and the tests hold each routine to values those libraries give.
//!
//! Every routine works in `f64`: two copies of one master differ by a
//! noise 40 dB and more under their music, and their coherence is read
//! to a part in ten thousand.

pub mod filter;

use std::cell::RefCell;

use realfft::RealFftPlanner;
pub use rustfft::num_complex::Complex64;

/// The rate every copy is compared at: one timeline for copies at any
/// rate, and the highest rate most lossy copies carry.
pub const RATE: u32 = 48_000;

/// [`RATE`] as a float, for the arithmetic of times and frequencies.
pub const FS: f64 = 48_000.0;

/// Two channels' samples, frame by frame.
pub type Frames = Vec<[f64; 2]>;

/// The frames of 32-bit little-endian float samples in two channels, as
/// ffmpeg writes `f32le`; a trailing partial frame is dropped.
#[must_use]
pub fn frames_of_f32le(bytes: &[u8]) -> Frames {
    bytes
        .chunks_exact(8)
        .map(|b| {
            let l = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let r = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            [f64::from(l), f64::from(r)]
        })
        .collect()
}

/// The mean of the two channels, frame by frame.
#[must_use]
pub fn mid(x: &[[f64; 2]]) -> Vec<f64> {
    x.iter().map(|[l, r]| (l + r) / 2.0).collect()
}

/// A count as a float. Sample and bin counts stay far below 2^52.
#[must_use]
pub fn real(n: usize) -> f64 {
    n as f64
}

/// A length or an index as a place among samples, which a lag may put
/// before the first.
#[must_use]
pub fn signed(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The smallest length at or past `n` whose only prime factors are 2, 3
/// and 5, the lengths `scipy` pads a real transform to.
#[must_use]
pub fn fast_len(n: usize) -> usize {
    let n = n.max(1);
    let mut best = usize::MAX;
    let mut p5 = 1_usize;
    while p5 < best {
        let mut p35 = p5;
        while p35 < best {
            let mut m = p35;
            while m < n {
                m *= 2;
            }
            best = best.min(m);
            if p35 >= n {
                break;
            }
            p35 *= 3;
        }
        if p5 >= n {
            break;
        }
        p5 *= 5;
    }
    best
}

thread_local! {
    static PLANNER: RefCell<RealFftPlanner<f64>> = RefCell::new(RealFftPlanner::new());
}

/// The correlation coefficient of `x` and `y`, as `numpy.corrcoef`
/// computes it, clipped to [-1, 1].
#[must_use]
pub fn corrcoef(x: &[f64], y: &[f64]) -> f64 {
    let n = real(x.len());
    let (mx, my) = (mean(x), mean(y));
    let c = |a: &[f64], ma: f64, b: &[f64], mb: f64| -> f64 {
        a.iter()
            .zip(b)
            .map(|(u, v)| (u - ma) * (v - mb))
            .sum::<f64>()
            / (n - 1.0)
    };
    let (cxy, cxx, cyy) = (c(x, mx, y, my), c(x, mx, x, mx), c(y, my, y, my));
    (cxy / cxx.sqrt() / cyy.sqrt()).clamp(-1.0, 1.0)
}

/// The spectrum of `x`, cut or padded with zeros to `n` samples: its
/// `n / 2 + 1` bins from 0 Hz up, unscaled, as `numpy.fft.rfft` gives it.
#[must_use]
pub fn rfft(x: &[f64], n: usize) -> Vec<Complex64> {
    let plan = PLANNER.with(|p| p.borrow_mut().plan_fft_forward(n));
    let mut input = vec![0.0; n];
    let take = x.len().min(n);
    input[..take].copy_from_slice(&x[..take]);
    let mut out = plan.make_output_vec();
    plan.process(&mut input, &mut out)
        .expect("the buffers are the plan's own");
    out
}

/// The `n` samples whose spectrum is `spectrum`, scaled by `1 / n`, as
/// `numpy.fft.irfft` gives them: bins past `n / 2` are ignored, missing
/// ones are zeros, and the imaginary parts of the bins at 0 Hz and, for
/// an even `n`, at the Nyquist frequency are dropped.
#[must_use]
pub fn irfft(spectrum: &[Complex64], n: usize) -> Vec<f64> {
    let plan = PLANNER.with(|p| p.borrow_mut().plan_fft_inverse(n));
    let mut input = plan.make_input_vec();
    let take = spectrum.len().min(input.len());
    input[..take].copy_from_slice(&spectrum[..take]);
    input[0].im = 0.0;
    if n.is_multiple_of(2) {
        let last = input.len() - 1;
        input[last].im = 0.0;
    }
    let mut out = vec![0.0; n];
    plan.process(&mut input, &mut out)
        .expect("the buffers are the plan's own, the edge bins real");
    let scale = 1.0 / real(n);
    for v in &mut out {
        *v *= scale;
    }
    out
}

/// A periodic Hann window of `n` samples: `0.5 - 0.5 cos(2πk / n)`.
#[must_use]
pub fn hann(n: usize) -> Vec<f64> {
    (0..n)
        .map(|k| 0.5 - 0.5 * (std::f64::consts::TAU * real(k) / real(n)).cos())
        .collect()
}

/// The median, the mean of the two middle values of an even count, as
/// `numpy.median` takes it; NaN of none, or of any NaN.
#[must_use]
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() || values.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len().is_multiple_of(2) {
        f64::midpoint(v[m - 1], v[m])
    } else {
        v[m]
    }
}

/// The mean; NaN of none.
#[must_use]
pub fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / real(values.len())
}

/// The cross-spectral density of `a` and `b` by Welch's method, as
/// `scipy.signal.csd` gives it at [`FS`] with `nperseg` samples a
/// segment: periodic Hann segments overlapping by half, each first freed
/// of its mean, `conj(A) B` averaged over the segments, scaled to a
/// density and doubled at every bin but 0 Hz and Nyquist. Empty when
/// either is shorter than a segment.
#[must_use]
pub fn csd(a: &[f64], b: &[f64], nperseg: usize) -> Vec<Complex64> {
    let n = a.len().min(b.len());
    if n < nperseg {
        return Vec::new();
    }
    let win = hann(nperseg);
    let scale = 1.0 / (FS * win.iter().map(|w| w * w).sum::<f64>());
    let step = nperseg - nperseg / 2;
    let segments = (n - nperseg) / step + 1;
    let mut acc = vec![Complex64::new(0.0, 0.0); nperseg / 2 + 1];
    let detrended = |x: &[f64]| -> Vec<f64> {
        let m = mean(x);
        x.iter().zip(&win).map(|(v, w)| (v - m) * w).collect()
    };
    for s in 0..segments {
        let lo = s * step;
        let fa = rfft(&detrended(&a[lo..lo + nperseg]), nperseg);
        let fb = rfft(&detrended(&b[lo..lo + nperseg]), nperseg);
        for ((acc, x), y) in acc.iter_mut().zip(&fa).zip(&fb) {
            *acc += x.conj() * y * scale;
        }
    }
    let last = acc.len() - 1;
    for (k, v) in acc.iter_mut().enumerate() {
        *v /= real(segments);
        if k != 0 && !(nperseg.is_multiple_of(2) && k == last) {
            *v *= 2.0;
        }
    }
    acc
}

/// The power spectral density of `x` by Welch's method, as
/// `scipy.signal.welch` gives it: [`csd`] of `x` with itself.
#[must_use]
pub fn welch(x: &[f64], nperseg: usize) -> Vec<f64> {
    csd(x, x, nperseg).iter().map(|v| v.re).collect()
}

/// `a` correlated with `v`, centred, as `numpy.correlate(a, v, "same")`
/// gives it: as long as the longer of the two.
#[must_use]
pub fn correlate_same(a: &[f64], v: &[f64]) -> Vec<f64> {
    let (n, m) = (a.len(), v.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    // numpy swaps the two when the second is longer, reversing the
    // result, which for real inputs is the correlation taken the other way.
    let (long, short, swapped) = if m > n { (v, a, true) } else { (a, v, false) };
    let (nl, ns) = (long.len(), short.len());
    let lead = ns / 2;
    let mut out = vec![0.0; nl];
    for (i, o) in out.iter_mut().enumerate() {
        let first = lead.saturating_sub(i);
        let last = (nl + lead - i).min(ns);
        *o = (first..last).map(|j| long[i + j - lead] * short[j]).sum();
    }
    if swapped {
        out.reverse();
    }
    out
}

/// `fp` over `xp`, increasing, read at `x` by straight lines between its
/// points, `left` before the first and `right` past the last, as
/// `scipy.interpolate.interp1d` with those fill values reads it.
#[must_use]
pub fn interp_linear(xp: &[f64], fp: &[f64], x: &[f64], left: f64, right: f64) -> Vec<f64> {
    let last = xp.len().saturating_sub(1);
    x.iter()
        .map(|&t| {
            if xp.is_empty() || t < xp[0] {
                return left;
            }
            if t > xp[last] {
                return right;
            }
            let hi = xp.partition_point(|&p| p < t).clamp(1, last.max(1));
            if last == 0 {
                return fp[0];
            }
            let lo = hi - 1;
            let slope = (fp[hi] - fp[lo]) / (xp[hi] - xp[lo]);
            slope * (t - xp[lo]) + fp[lo]
        })
        .collect()
}

/// `sin(πx) / (πx)`, 1 at 0.
#[must_use]
pub fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let y = std::f64::consts::PI * x;
        y.sin() / y
    }
}

/// The modified Bessel function of the first kind of order zero, by its
/// power series, which converges to the last bit for the window shapes
/// used here.
#[must_use]
pub fn bessel_i0(x: f64) -> f64 {
    let q = x * x / 4.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut k = 1.0;
    while term > sum * 1e-17 {
        term *= q / (k * k);
        sum += term;
        k += 1.0;
    }
    sum
}

/// The population standard deviation, as `numpy.std`; NaN of none.
#[must_use]
pub fn std(values: &[f64]) -> f64 {
    let m = mean(values);
    (values.iter().map(|v| (v - m).abs().powi(2)).sum::<f64>() / real(values.len())).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_pad_to_factors_of_two_three_and_five() {
        let got: Vec<usize> = [1, 2, 7, 11, 13, 97, 1001, 5_800_001]
            .map(fast_len)
            .to_vec();
        assert_eq!(got, [1, 2, 8, 12, 15, 100, 1024, 5_832_000]);
    }

    #[test]
    fn a_spectrum_gives_its_samples_back() {
        let x: Vec<f64> = (0..45)
            .map(|n| (real(n) * 0.7).sin() + 0.1 * real(n))
            .collect();
        for n in [45, 64, 60] {
            let back = irfft(&rfft(&x, n), n);
            for (a, b) in back.iter().zip(&x) {
                assert!((a - b).abs() < 1e-12, "length {n}: {a} against {b}");
            }
        }
        assert_eq!(rfft(&x, 64).len(), 33, "bins from 0 Hz to Nyquist");
    }

    #[test]
    fn medians_average_the_two_middle_values() {
        assert!((median(&[3.0, 1.0, 2.0]) - 2.0).abs() < 1e-15);
        assert!((median(&[4.0, 1.0, 2.0, 3.0]) - 2.5).abs() < 1e-15);
        assert!(median(&[]).is_nan());
        assert!(median(&[1.0, f64::NAN]).is_nan());
    }

    #[test]
    fn a_delay_shows_in_the_cross_spectrums_phase() {
        let a: Vec<f64> = (0..10_000).map(|n| (0.3 * real(n)).cos()).collect();
        let b: Vec<f64> = (0..10_000).map(|n| (0.3 * real(n) - 0.6).cos()).collect();
        let c = csd(&a, &b, 4096);
        let peak = (0..c.len())
            .max_by(|x, y| c[*x].norm().total_cmp(&c[*y].norm()))
            .unwrap();
        assert_eq!(peak, 196);
        assert!((c[peak].arg() + 0.6).abs() < 1e-6, "{}", c[peak].arg());
        assert!(
            csd(&a[..100], &b[..100], 4096).is_empty(),
            "shorter than a segment"
        );
        assert!(welch(&a, 4096).iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn correlation_is_centred_on_the_longer() {
        assert_eq!(
            correlate_same(&[1.0, 2.0, 3.0, 4.0], &[1.0, 0.0, -1.0]),
            [-2.0, -2.0, -2.0, 3.0]
        );
        assert_eq!(
            correlate_same(&[1.0, 0.0, -1.0], &[1.0, 2.0, 3.0, 4.0]),
            [3.0, -2.0, -2.0, -2.0]
        );
    }

    #[test]
    fn interpolation_fills_past_its_ends() {
        let got = interp_linear(
            &[0.0, 2.0, 4.0],
            &[0.0, 4.0, 0.0],
            &[-1.0, 1.0, 2.0, 3.5, 9.0],
            7.0,
            8.0,
        );
        assert_eq!(got, [7.0, 2.0, 4.0, 1.0, 8.0]);
    }

    #[test]
    fn samples_read_as_frames_of_two_channels() {
        let mut bytes = Vec::new();
        for v in [0.5_f32, -0.25, 1.0, 0.0, 9.0] {
            bytes.extend(v.to_le_bytes());
        }
        assert_eq!(
            frames_of_f32le(&bytes),
            [[0.5, -0.25], [1.0, 0.0]],
            "a partial frame is dropped"
        );
        assert_eq!(mid(&[[0.5, -0.25]]), [0.125]);
    }
}
