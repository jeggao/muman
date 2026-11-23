//! Generated audio for the tests of [`crate::coherence`], of
//! [`crate::gate`] and of what runs them: noise whose level rises and falls
//! as music's does, and the copies of it a test compares.

use crate::signal::filter::{Band, butter, sosfilt};
use crate::signal::{FS, Frames};

/// Uniform noise in -1 to 1 from `seed`, by xorshift.
#[must_use]
pub fn noise(n: usize, seed: u64) -> Vec<f64> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 52) as f64 - 1.0
        })
        .collect()
}

/// `seconds` of stereo noise shaped as music is: most of its power low
/// but none under 40 Hz, a little to the top of the band, its level
/// swinging over 8 dB. White noise alone will not do: it decorrelates
/// within a sample, where music's lows hold two drifting copies together;
/// nor will noise down to 0 Hz, whose slow wander no recording holds and
/// which leaves a silenced stretch off zero once a copy's mean is removed.
#[must_use]
pub fn music(seconds: f64, seed: u64) -> Frames {
    let n = (seconds * FS) as usize;
    let low = butter(2, Band::Low(500.0), FS);
    let high = butter(2, Band::High(40.0), FS);
    let shaped = |s: u64| -> Vec<f64> {
        let w = noise(n, s);
        let mixed: Vec<f64> = sosfilt(&low, &w)
            .iter()
            .zip(&w)
            .map(|(l, w)| 6.0 * l + 0.05 * w)
            .collect();
        sosfilt(&high, &mixed)
    };
    let (a, b) = (shaped(seed), shaped(seed + 1));
    (0..n)
        .map(|i| {
            let t = i as f64 / FS;
            let env = 0.1 + 0.08 * (1.0 + (std::f64::consts::TAU * 0.7 * t).sin());
            [env * (a[i] + 0.4 * b[i]), env * (a[i] - 0.4 * b[i])]
        })
        .collect()
}

/// `x` with noise of its own at `level` of full scale, as a lossy copy's.
#[must_use]
pub fn noisy(x: &[[f64; 2]], level: f64, seed: u64) -> Frames {
    let (a, b) = (noise(x.len(), seed), noise(x.len(), seed + 1));
    x.iter()
        .enumerate()
        .map(|(i, f)| [f[0] + level * a[i], f[1] + level * b[i]])
        .collect()
}

/// `x` played `ppm` parts per million fast, by straight lines between
/// its samples.
#[must_use]
pub fn sped(x: &[[f64; 2]], ppm: f64) -> Frames {
    let rate = 1.0 + ppm * 1e-6;
    let frames = ((x.len() - 1) as f64 / rate) as usize;
    (0..frames)
        .map(|i| {
            let at = i as f64 * rate;
            let (whole, part) = (at as usize, at.fract());
            [0, 1].map(|c| x[whole][c] * (1.0 - part) + x[whole + 1][c] * part)
        })
        .collect()
}

/// `x` cut off above `hz` as a codec cuts it: an 8th-order lowpass four
/// times over, falling 25 dB within 500 Hz.
#[must_use]
pub fn lowpassed(x: &[[f64; 2]], hz: f64) -> Frames {
    let sos = butter(8, Band::Low(hz), FS);
    let run = |c: usize| {
        (0..4).fold(x.iter().map(|f| f[c]).collect::<Vec<_>>(), |v, _| {
            sosfilt(&sos, &v)
        })
    };
    run(0)
        .into_iter()
        .zip(run(1))
        .map(|(a, b)| [a, b])
        .collect()
}
