//! Digital filters as `scipy.signal` designs and runs them: Butterworth
//! lowpass, highpass and bandpass filters as second-order sections
//! ([`butter`]), run once ([`sosfilt`]) or forward and back from steady
//! state over odd-extended edges ([`sosfiltfilt`]), and rational
//! resampling through a Kaiser-windowed lowpass ([`resample_poly`]).
//!
//! A design is the bilinear transform of the analog prototype's poles,
//! prewarped, its sections paired as `zpk2sos` pairs them, nearest poles
//! to the unit circle last: the filters are the prototypes' to the last
//! few bits, and their sections round alike.

use super::{Complex64, bessel_i0, real, sinc};

/// Second-order sections, each `[b0, b1, b2, a0, a1, a2]` with `a0` one.
#[derive(Clone, Debug, PartialEq)]
pub struct Sos(pub Vec<[f64; 6]>);

/// What a Butterworth filter passes.
#[derive(Clone, Copy, Debug)]
pub enum Band {
    /// Under a frequency.
    Low(f64),
    /// Over a frequency.
    High(f64),
    /// Between two frequencies.
    Pass(f64, f64),
}

/// A digital Butterworth filter of `order` at sample rate `fs`, as
/// `scipy.signal.butter(order, ..., fs=fs, output="sos")` designs it.
#[must_use]
pub fn butter(order: usize, band: Band, fs: f64) -> Sos {
    let nyquist = fs / 2.0;
    // scipy prewarps on a rate of 2, the frequencies over Nyquist.
    let warp = |hz: f64| 4.0 * (std::f64::consts::PI * (hz / nyquist) / 2.0).tan();
    let prototype: Vec<Complex64> = (0..order)
        .map(|m| {
            let k = -(real(order) - 1.0) + 2.0 * real(m);
            -Complex64::new(0.0, std::f64::consts::PI * k / (2.0 * real(order))).exp()
        })
        .collect();
    let (zeros, poles, gain) = match band {
        Band::Low(hz) => {
            let wo = warp(hz);
            let p: Vec<Complex64> = prototype.iter().map(|p| p * wo).collect();
            (Vec::new(), p, raised(wo, order))
        }
        Band::High(hz) => {
            let wo = warp(hz);
            let p: Vec<Complex64> = prototype.iter().map(|p| wo / p).collect();
            let product = prototype
                .iter()
                .fold(Complex64::new(1.0, 0.0), |acc, p| acc * (-p));
            (
                vec![Complex64::new(0.0, 0.0); order],
                p,
                (Complex64::new(1.0, 0.0) / product).re,
            )
        }
        Band::Pass(lo, hi) => {
            let (w1, w2) = (warp(lo), warp(hi));
            let (bw, wo) = (w2 - w1, (w1 * w2).sqrt());
            let half: Vec<Complex64> = prototype.iter().map(|p| p * bw / 2.0).collect();
            let root = |p: &Complex64| (p * p - wo * wo).sqrt();
            let mut p: Vec<Complex64> = half.iter().map(|p| p + root(p)).collect();
            p.extend(half.iter().map(|p| p - root(p)));
            (vec![Complex64::new(0.0, 0.0); order], p, raised(bw, order))
        }
    };
    let (z, p, k) = bilinear(&zeros, &poles, gain);
    zpk2sos(&z, &p, k)
}

/// `base` to the power of a filter's order.
fn raised(base: f64, order: usize) -> f64 {
    base.powi(i32::try_from(order).unwrap_or(i32::MAX))
}

/// The bilinear transform at scipy's rate of 2: each analog root `s` to
/// `(4 + s) / (4 - s)`, the zeros at infinity to -1.
fn bilinear(z: &[Complex64], p: &[Complex64], k: f64) -> (Vec<Complex64>, Vec<Complex64>, f64) {
    let fs2 = Complex64::new(4.0, 0.0);
    let map = |s: &Complex64| (fs2 + s) / (fs2 - s);
    let mut zd: Vec<Complex64> = z.iter().map(map).collect();
    zd.extend(std::iter::repeat_n(
        Complex64::new(-1.0, 0.0),
        p.len() - z.len(),
    ));
    let pd = p.iter().map(map).collect();
    let num = z
        .iter()
        .fold(Complex64::new(1.0, 0.0), |acc, s| acc * (fs2 - s));
    let den = p
        .iter()
        .fold(Complex64::new(1.0, 0.0), |acc, s| acc * (fs2 - s));
    (zd, pd, k * (num / den).re)
}

/// One of each conjugate pair, averaged with its mate, sorted by real
/// part, then the real roots sorted, as `scipy.signal._cplxreal` gives
/// them.
fn cplxreal(z: &[Complex64]) -> (Vec<Complex64>, Vec<f64>) {
    let tol = 100.0 * f64::EPSILON;
    let mut sorted = z.to_vec();
    sorted.sort_by(|a, b| {
        a.re.total_cmp(&b.re)
            .then(a.im.abs().total_cmp(&b.im.abs()))
    });
    let is_real = |v: &Complex64| v.im.abs() <= tol * v.norm();
    let reals: Vec<f64> = sorted.iter().filter(|v| is_real(v)).map(|v| v.re).collect();
    let rest: Vec<Complex64> = sorted.iter().copied().filter(|v| !is_real(v)).collect();
    let pos: Vec<Complex64> = rest.iter().copied().filter(|v| v.im > 0.0).collect();
    let neg: Vec<Complex64> = rest.iter().copied().filter(|v| v.im < 0.0).collect();
    let pairs = pos
        .iter()
        .zip(&neg)
        .map(|(a, b)| (a + b.conj()) / 2.0)
        .collect();
    (pairs, reals)
}

/// The coefficients of `(x - r1)(x - r2)…`, highest power first.
fn poly(roots: &[Complex64]) -> Vec<f64> {
    let mut c = vec![Complex64::new(1.0, 0.0)];
    for r in roots {
        let mut next = vec![Complex64::new(0.0, 0.0); c.len() + 1];
        for (i, v) in c.iter().enumerate() {
            next[i] += v;
            next[i + 1] -= v * r;
        }
        c = next;
    }
    c.iter().map(|v| v.re).collect()
}

/// One section from up to two zeros and two poles, as `_single_zpksos`.
fn section(zeros: &[Complex64], poles: &[Complex64]) -> [f64; 6] {
    let mut row = [0.0; 6];
    let (num, den) = (poly(zeros), poly(poles));
    row[3 - num.len()..3].copy_from_slice(&num);
    row[6 - den.len()..6].copy_from_slice(&den);
    row
}

/// Sections pairing each pole nearest the unit circle with its nearest
/// zeros, built from the last section back, as `scipy.signal.zpk2sos`
/// pairs them by default for a digital filter of real zeros.
fn zpk2sos(all_zeros: &[Complex64], all_poles: &[Complex64], gain: f64) -> Sos {
    let (mut pc, mut pr) = cplxreal(all_poles);
    let (zc, mut zr) = cplxreal(all_zeros);
    debug_assert!(zc.is_empty(), "Butterworth zeros are real");
    let n_sections = all_poles.len().div_ceil(2);
    if all_poles.len() % 2 == 1 {
        pr.push(0.0);
        zr.push(0.0);
    }
    let worst = |v: Complex64| (1.0 - v.norm()).abs();
    let nearest = |zs: &[f64], to: Complex64| {
        (0..zs.len())
            .min_by(|&a, &b| {
                (Complex64::new(zs[a], 0.0) - to)
                    .norm()
                    .total_cmp(&(Complex64::new(zs[b], 0.0) - to).norm())
            })
            .expect("a zero is left for every pole")
    };
    let mut sos = vec![[0.0; 6]; n_sections];
    for si in (0..n_sections).rev() {
        let worst_c = (0..pc.len()).min_by(|&a, &b| worst(pc[a]).total_cmp(&worst(pc[b])));
        let worst_r = (0..pr.len()).min_by(|&a, &b| {
            worst(Complex64::new(pr[a], 0.0)).total_cmp(&worst(Complex64::new(pr[b], 0.0)))
        });
        let take_complex = match (worst_c, worst_r) {
            (Some(of_pair), Some(alone)) => {
                worst(pc[of_pair]) <= worst(Complex64::new(pr[alone], 0.0))
            }
            (Some(_), None) => true,
            _ => false,
        };
        let poles = if take_complex {
            let p1 = pc.remove(worst_c.expect("chosen above"));
            vec![p1, p1.conj()]
        } else {
            let p1 = pr.remove(worst_r.expect("chosen above"));
            let next = (0..pr.len()).min_by(|&a, &b| {
                worst(Complex64::new(pr[a], 0.0)).total_cmp(&worst(Complex64::new(pr[b], 0.0)))
            });
            match next {
                Some(i) => vec![Complex64::new(p1, 0.0), Complex64::new(pr.remove(i), 0.0)],
                None => vec![Complex64::new(p1, 0.0), Complex64::new(0.0, 0.0)],
            }
        };
        let mut zeros = Vec::new();
        for _ in 0..2 {
            if !zr.is_empty() {
                zeros.push(Complex64::new(zr.remove(nearest(&zr, poles[0])), 0.0));
            }
        }
        sos[si] = section(&zeros, &poles);
    }
    for c in &mut sos[0][..3] {
        *c *= gain;
    }
    Sos(sos)
}

/// `x` through `sos` in direct form II transposed, from the states `zi`
/// (two a section) or rest; the output and the final states.
#[must_use]
pub fn sosfilt_from(sos: &Sos, x: &[f64], zi: Option<&[[f64; 2]]>) -> (Vec<f64>, Vec<[f64; 2]>) {
    let mut z: Vec<[f64; 2]> = zi.map_or_else(|| vec![[0.0; 2]; sos.0.len()], <[_]>::to_vec);
    let y = x
        .iter()
        .map(|&v| {
            let mut x = v;
            for (s, z) in sos.0.iter().zip(z.iter_mut()) {
                let y = s[0] * x + z[0];
                z[0] = s[1] * x - s[4] * y + z[1];
                z[1] = s[2] * x - s[5] * y;
                x = y;
            }
            x
        })
        .collect();
    (y, z)
}

/// `x` through `sos` from rest, as `scipy.signal.sosfilt`.
#[must_use]
pub fn sosfilt(sos: &Sos, x: &[f64]) -> Vec<f64> {
    sosfilt_from(sos, x, None).0
}

/// The states that hold `sos` steady at a constant input of one, as
/// `scipy.signal.sosfilt_zi`.
#[must_use]
pub fn sosfilt_zi(sos: &Sos) -> Vec<[f64; 2]> {
    let mut scale = 1.0;
    sos.0
        .iter()
        .map(|s| {
            let (b, a) = (&s[..3], &s[3..]);
            // (I - companion(a)^T) zi = b[1:] - a[1:] b[0], solved for two.
            let (m00, m01, m10, m11) = (1.0 + a[1], -1.0, a[2], 1.0);
            let (r0, r1) = (b[1] - a[1] * b[0], b[2] - a[2] * b[0]);
            let det = m00 * m11 - m01 * m10;
            let zi = [(r0 * m11 - m01 * r1) / det, (m00 * r1 - m10 * r0) / det];
            let out = [scale * zi[0], scale * zi[1]];
            scale *= b.iter().sum::<f64>() / a.iter().sum::<f64>();
            out
        })
        .collect()
}

/// `x` through `sos` forward and back, as `scipy.signal.sosfiltfilt`
/// with its default odd extension of three times the filter's taps at
/// each end, each pass started from steady state; `x` itself when no
/// longer than that extension.
#[must_use]
pub fn sosfiltfilt(sos: &Sos, x: &[f64]) -> Vec<f64> {
    let no_zero = sos.0.iter().filter(|s| s[2] == 0.0).count();
    let no_pole = sos.0.iter().filter(|s| s[5] == 0.0).count();
    let edge = 3 * (2 * sos.0.len() + 1 - no_zero.min(no_pole));
    if x.len() <= edge {
        return x.to_vec();
    }
    let n = x.len();
    let mut ext = Vec::with_capacity(n + 2 * edge);
    ext.extend((1..=edge).rev().map(|i| 2.0 * x[0] - x[i]));
    ext.extend_from_slice(x);
    ext.extend((1..=edge).map(|i| 2.0 * x[n - 1] - x[n - 1 - i]));
    let zi = sosfilt_zi(sos);
    let scaled = |v: f64| -> Vec<[f64; 2]> { zi.iter().map(|z| [z[0] * v, z[1] * v]).collect() };
    let (mut y, _) = sosfilt_from(sos, &ext, Some(&scaled(ext[0])));
    y.reverse();
    let (mut y, _) = sosfilt_from(sos, &y, Some(&scaled(y[0])));
    y.reverse();
    y[edge..edge + n].to_vec()
}

/// A symmetric Kaiser window of `m` samples.
#[must_use]
pub fn kaiser(m: usize, beta: f64) -> Vec<f64> {
    if m == 1 {
        return vec![1.0];
    }
    let denom = bessel_i0(beta);
    let alpha = real(m - 1) / 2.0;
    (0..m)
        .map(|n| {
            let r = (real(n) - alpha) / alpha;
            bessel_i0(beta * (1.0 - r * r).sqrt()) / denom
        })
        .collect()
}

/// A lowpass of `taps` taps (odd) cutting at `cutoff` of Nyquist through
/// a Kaiser window of `beta`, scaled to unit gain at 0 Hz, as
/// `scipy.signal.firwin` designs it.
#[must_use]
pub fn firwin(taps: usize, cutoff: f64, beta: f64) -> Vec<f64> {
    let alpha = 0.5 * real(taps - 1);
    let win = kaiser(taps, beta);
    let h: Vec<f64> = (0..taps)
        .map(|i| {
            let m = real(i) - alpha;
            cutoff * sinc(cutoff * m) * win[i]
        })
        .collect();
    let sum: f64 = h.iter().sum();
    h.iter().map(|v| v / sum).collect()
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// `x` resampled by `up / down` through a Kaiser-windowed lowpass at the
/// lower Nyquist, as `scipy.signal.resample_poly` with its defaults:
/// `ceil(len · up / down)` samples, each centred on its input, zeros
/// taken past both ends.
#[must_use]
pub fn resample_poly(x: &[f64], up: usize, down: usize) -> Vec<f64> {
    let g = gcd(up, down);
    let (up, down) = (up / g, down / g);
    if up == 1 && down == 1 {
        return x.to_vec();
    }
    let n_in = x.len();
    let n_out = (n_in * up).div_ceil(down);
    let max_rate = up.max(down);
    let half_len = 10 * max_rate;
    let h: Vec<f64> = firwin(2 * half_len + 1, 1.0 / real(max_rate), 5.0)
        .iter()
        .map(|v| v * real(up))
        .collect();
    // Output k sits at upsampled position k·down; tap i of the filter
    // centred there reads upsampled position k·down + half_len - i, which
    // holds an input sample only at multiples of `up`.
    (0..n_out)
        .map(|k| {
            let centre = k * down + half_len;
            (centre % up..h.len().min(centre + 1))
                .step_by(up)
                .filter_map(|i| x.get((centre - i) / up).map(|v| h[i] * v))
                .sum()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f64], b: &[f64], tol: f64) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() <= tol, "{x} against {y}");
        }
    }

    /// A sine of 3 cycles over 64 samples on a slow ramp.
    fn wave() -> Vec<f64> {
        (0..64)
            .map(|n| (std::f64::consts::TAU * real(n) * 3.0 / 64.0).sin() + 0.25 * real(n) / 64.0)
            .collect()
    }

    #[test]
    fn butterworth_sections_are_scipys() {
        let low = butter(2, Band::Low(6000.0), 48000.0);
        close(
            &low.0[0],
            &[
                0.097_631_072_937_817_49,
                0.195_262_145_875_634_98,
                0.097_631_072_937_817_49,
                1.0,
                -0.942_809_041_582_063_2,
                0.333_333_333_333_333_2,
            ],
            1e-14,
        );
        let high = butter(4, Band::High(6000.0), 48000.0);
        close(
            &high.0[0],
            &[
                0.346_821_807_846_938_3,
                -0.693_643_615_693_876_6,
                0.346_821_807_846_938_3,
                1.0,
                -0.855_397_932_775_170_4,
                0.209_715_357_756_554_78,
            ],
            1e-14,
        );
        close(
            &high.0[1],
            &[
                1.0,
                -2.0,
                1.0,
                1.0,
                -1.113_029_854_163_347_9,
                0.574_061_915_083_954_5,
            ],
            1e-14,
        );
        let band = butter(2, Band::Pass(1000.0, 8000.0), 48000.0);
        close(
            &band.0[0],
            &[
                0.125_317_808_613_537_42,
                0.250_635_617_227_074_84,
                0.125_317_808_613_537_42,
                1.0,
                -0.755_449_341_909_742_8,
                0.334_523_070_049_283_9,
            ],
            1e-14,
        );
        close(
            &band.0[1],
            &[
                1.0,
                -2.0,
                1.0,
                1.0,
                -1.821_796_003_286_884_3,
                0.840_724_856_720_391_1,
            ],
            1e-14,
        );
    }

    #[test]
    fn filtering_forward_and_back_is_scipys() {
        let y = sosfiltfilt(&butter(2, Band::Low(6000.0), 48000.0), &wave());
        close(
            &y[..6],
            &[
                -0.001_259_053_807_216_636_8,
                0.288_913_972_932_464_35,
                0.554_266_053_806_981,
                0.772_294_689_537_714_4,
                0.924_666_949_245_188_2,
                0.998_693_710_621_976_4,
            ],
            1e-12,
        );
        close(
            &y[61..],
            &[
                -0.510_298_461_619_106_3,
                -0.292_516_904_827_585_86,
                -0.041_671_045_022_285_455,
            ],
            1e-12,
        );
    }

    #[test]
    fn a_signal_no_longer_than_its_extension_is_left_as_it_is() {
        let x = vec![0.5; 9];
        assert_eq!(sosfiltfilt(&butter(2, Band::Low(6000.0), 48000.0), &x), x);
    }

    #[test]
    fn resampling_by_a_quarter_is_scipys() {
        let y = resample_poly(&wave(), 1, 4);
        close(
            &y,
            &[
                0.121_688_534_467_910_7,
                0.922_234_604_762_299_6,
                0.745_320_112_573_364_2,
                -0.339_805_933_802_760_03,
                -0.935_485_601_531_405_4,
                -0.306_145_199_515_762_4,
                0.802_213_445_375_186_9,
                1.033_709_734_855_469_6,
                0.124_558_629_329_782_04,
                -0.782_345_827_734_934_9,
                -0.553_762_640_601_666_7,
                0.558_832_399_778_646_1,
                1.181_503_198_343_683_2,
                0.596_232_100_146_074_2,
                -0.505_696_784_817_747_4,
                -0.651_611_494_759_862,
            ],
            1e-12,
        );
        assert_eq!(
            resample_poly(&wave(), 3, 3),
            wave(),
            "a ratio of one changes nothing"
        );
    }

    #[test]
    fn the_kaiser_window_is_scipys() {
        close(
            &kaiser(5, 5.0),
            &[
                0.036_710_892_271_286_676,
                0.552_851_769_699_132_4,
                1.0,
                0.552_851_769_699_132_4,
                0.036_710_892_271_286_676,
            ],
            1e-14,
        );
    }
}
