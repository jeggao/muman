//! How high a source's peaks stand, as a master squeezed louder lowers
//! them against its loudness and a lossy encoding of one master does not.
//!
//! The peak is the level the loudest 0.1% of the decoded frames reach
//! ([`SHARE`]), each frame by its loudest channel: the 99.9th percentile
//! of the samples' magnitude. Frames under −60 dBFS ([`FLOOR_DB`]) are
//! silence and not counted, so a long lead-in moves it not. Levels are
//! counted in bins of 0.01 dB, the peak is a bin's top, and a source keeps
//! only that.
//!
//! The true peak, which this replaces as what `[audio.pick.dynamics]`
//! weighs, is what a lossy codec raises most: its error rings past every
//! peak, and the highest moves most. Measured on 12 game soundtrack
//! pieces, a minute of each as FLAC and as Opus at 64, 96 and 128 kbit/s,
//! MP3 at 128 and 320 kbit/s, AAC at 128 kbit/s and Vorbis at quality 3,
//! the true peak stood up to 1.3 dB higher in a lossy copy, so a step of
//! 1 dB ranked it over the lossless one as often as not; the sample peak,
//! held at full scale, up to 0.5 dB, where the master peaked under it. Of
//! 6 of them, the 99.99th percentile moved up to 0.58 dB, and this one
//! from −0.22 to +0.13 dB. A master squeezed louder lowers it all the
//! same: run through a compressor it stood 1.2 to 3.4 dB lower against
//! the loudness, and raised 6 dB into a limiter 1.0 to 3.4 dB, but for a
//! quiet piece whose peaks the limiter never reached; raised 3 dB, 0.5 to
//! 1.8 dB, but for one whose peaks it barely touched. The 99.99th
//! percentile told those apart by 1.2 dB or more, which is why it is not
//! the measure: a codec moved it half as far. The 99.9th percentile of
//! 10 ms loudness moved under 0.3 dB with a codec, and missed limiting
//! by as much.

use serde::{Deserialize, Serialize};

use crate::analysis::{Analysis, Analyzer, Measured, Stream};

/// Names the meter and its constants; a measure by another is measured
/// again, so changing anything below means changing this.
pub const METHOD: &str = "dynamics/1";

/// The share of the counted frames at or above the peak.
const SHARE: f64 = 1e-3;
/// Frames quieter than this are not counted.
const FLOOR_DB: f64 = -60.0;
/// The loudest level counted apart, in dBFS: decoded lossy audio may run
/// past full scale.
const CEILING_DB: f64 = 12.0;
/// Bins a dB.
const BINS_PER_DB: f64 = 100.0;

/// A source's peak as measured.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    /// The level the loudest [`SHARE`] of its frames reach, in dBFS.
    pub peak_db: f64,
}

/// The levels of one stream's frames as it is decoded.
#[derive(Debug)]
pub struct Meter {
    channels: usize,
    bins: Vec<u64>,
}

impl Meter {
    #[must_use]
    pub fn new(stream: &Stream) -> Self {
        let bins = ((CEILING_DB - FLOOR_DB) * BINS_PER_DB) as usize + 1;
        Self {
            channels: usize::from(stream.channels).max(1),
            bins: vec![0; bins],
        }
    }

    /// The level the loudest [`SHARE`] of the counted frames reach, in
    /// dBFS; `None` where none was counted.
    fn peak_db(&self) -> Option<f64> {
        let counted: u64 = self.bins.iter().sum();
        if counted == 0 {
            return None;
        }
        let above = (counted as f64 * SHARE).ceil() as u64;
        let mut seen = 0;
        let bin = (0..self.bins.len()).rev().find(|&b| {
            seen += self.bins[b];
            seen >= above
        })?;
        Some(FLOOR_DB + (bin + 1) as f64 / BINS_PER_DB)
    }
}

impl Analyzer for Meter {
    fn take(&mut self, frames: &[f32]) {
        let floor = 10_f32.powf((FLOOR_DB / 20.0) as f32);
        let last = self.bins.len() - 1;
        for frame in frames.chunks_exact(self.channels) {
            let level = frame.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            if level < floor {
                continue;
            }
            let db = 20.0 * f64::from(level).log10();
            let bin = ((db - FLOOR_DB) * BINS_PER_DB) as usize;
            self.bins[bin.min(last)] += 1;
        }
    }

    fn finish(self: Box<Self>, into: &mut Analysis) {
        into.dynamics = Some(Measured {
            method: METHOD.to_string(),
            value: self.peak_db().map(|peak_db| Measure { peak_db }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Kind, Pass};
    use crate::testing::wav;

    fn measure(samples: &[f32]) -> Option<f64> {
        let mut pass = Pass::new(vec![Kind::Dynamics]);
        pass.take(&wav(48_000, 1, None, samples));
        let analysis = pass.finish(&crate::integrity::Found::default());
        analysis.dynamics().map(|m| m.peak_db)
    }

    /// Deterministic noise in [-1, 1).
    fn noise(n: usize, seed: u64) -> Vec<f32> {
        crate::testing::words(n, seed)
            .into_iter()
            .map(|w| (f64::from(w) / f64::from(u32::MAX) * 2.0 - 1.0) as f32)
            .collect()
    }

    #[test]
    fn a_few_overshooting_samples_move_the_peak_not() {
        let quiet: Vec<f32> = noise(480_000, 1).iter().map(|s| s * 0.5).collect();
        let peak = measure(&quiet).unwrap();
        assert!((peak - 20.0 * 0.5_f64.log10()).abs() < 0.05, "{peak}");
        let mut rung = quiet.clone();
        for s in rung.iter_mut().step_by(20_000) {
            *s = 0.9;
        }
        assert!((measure(&rung).unwrap() - peak).abs() < 0.05);
    }

    #[test]
    fn a_master_squeezed_louder_peaks_lower_over_its_level() {
        let rms_db = |samples: &[f32]| {
            let power = samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
            10.0 * (power / samples.len() as f64).log10()
        };
        let open: Vec<f32> = noise(480_000, 2).iter().map(|s| s * 0.5).collect();
        let squeezed: Vec<f32> = open.iter().map(|s| (s * 2.0).clamp(-0.6, 0.6)).collect();
        let over = |s: &[f32]| measure(s).unwrap() - rms_db(s);
        assert!(over(&open) - over(&squeezed) > 2.0);
    }

    /// What the meter makes of peaky noise at three levels, one squeezed, with
    /// [`METHOD`].
    const WRITTEN: (&str, &str) = (
        "dynamics/1",
        "f404d3a71d592b6b7e4c6bbd911c95421b5c492b5d58f686b42175c4a9958893",
    );

    #[test]
    fn the_method_rises_with_what_it_makes() {
        let mut digest = crate::testing::Digest::default();
        for (gain, ceiling) in [(0.1, 1.0), (0.5, 1.0), (2.0, 0.6)] {
            let samples: Vec<f32> = noise(96_000, 3)
                .iter()
                .map(|s| (s.powi(3) * gain).clamp(-ceiling, ceiling))
                .collect();
            digest.debug(&measure(&samples));
        }
        crate::testing::check(WRITTEN, METHOD, &digest.hex());
    }

    #[test]
    fn silence_measures_nothing_but_records_its_method() {
        let mut pass = Pass::new(vec![Kind::Dynamics]);
        pass.take(&wav(48_000, 1, None, &vec![0.0; 48_000]));
        let analysis = pass.finish(&crate::integrity::Found::default());
        let measured = analysis.dynamics.unwrap();
        assert_eq!(measured.method, METHOD);
        assert!(measured.value.is_none());
    }
}
