//! How much treble a source's audio holds, block by block, so that a
//! lossy copy measuring wider than another copy of its recording can be
//! told made from it or not ([`crate::quality::excess_db`]).
//!
//! A lossy encoder keeps the energy of each band it codes. Opus codes the
//! band from 15.6 to 20 kHz by little more than its energy, filling the
//! whole band with what its source held anywhere in it: an Opus copy of
//! an MP3 cut at 16 kHz measures 20.6 kHz, but holds no more energy from
//! 15.6 kHz up than the MP3 does. A sibling made from a wider master
//! holds what the master holds there, which is more. So
//! each source keeps its energy per band and per block of
//! [`BLOCK_MS`], over the whole decode, and two copies are compared over
//! the stretch [`crate::align`] finds they share, each over its own
//! energy from 1 to 8 kHz ([`REFERENCE_HZ`]).
//!
//! - **Frames.** Each frame is a power of two of samples, the first
//!   holding 20 ms at the source's rate ([`FRAME_MS`]), one after
//!   another, Hann-windowed. Its power is summed per channel, never
//!   downmixed: in a mix, Opus's intensity stereo, which makes its
//!   channels alike past a band, showed 2.8 dB more than its master in
//!   that band at 64 and 96 kbit/s. Two channels share one complex FFT,
//!   one the real part and the other the imaginary, and the sum of their
//!   powers at a bin is half that of the transform's at the bin and its
//!   mirror. A frame belongs to the block its middle falls in.
//! - **Clipping.** A frame holding a sample at or past [`CLIPPED`] is
//!   left out: clipping splatters harmonics past the lowpass the audio
//!   was made with, which is no sound of the recording, as
//!   [`crate::quality`] says of bandwidth. Where fewer than
//!   [`LEAST_UNCLIPPED`] of the frames that sound are left, as in a
//!   master clipped through every loud passage, every frame counts
//!   instead, so the copy still has a measure.
//! - **Bands.** [`EDGES_HZ`], 500 Hz apart from 1 to 24 kHz; the last
//!   band of a source runs from the last edge under half its rate to half
//!   its rate. The region a comparison sums starts 1 kHz under a wall,
//!   at the nearest edge, which a wall standing on bands of 172 to
//!   188 Hz moves by 250 Hz at the most; or at the edge at or under an
//!   Opus band's foot ([`edge_under_hz`]), so that 15.6 kHz sums from
//!   15.5. A frame of 20 ms spreads a bin's power over about 100 Hz,
//!   across the foot, and the research behind it summed 250 Hz bands
//!   that started there: of an MP3 cut at 16 kHz and an Opus copy made
//!   from it, Opus at 128 kbit/s streaming at 147 kbit/s, the copy held
//!   0.03 dB more from 15.5 kHz up and 0.6 dB more from 15.6, past what
//!   Opus at that rate may hold.
//! - **Storage.** Each band's mean power in a block, as the frames left
//!   out would add none, in centibels over [`FLOOR_DB`], two bytes each:
//!   46 bands every 2 s at 48 kHz, about 11 KB for a four-minute song,
//!   kept as base64. Rounding to a centibel moves a sum by 0.05 dB at the
//!   most.
//!
//! The bands and blocks are what the comparison needs and no more: the
//! research behind [`crate::quality::excess_db`] summed 250 Hz bands of
//! 20 ms frames over aligned copies, and found sums over the whole shared
//! stretch bounded every transcode where the median of per-frame ratios
//! did not, and comparing whole files, not the stretch both play, left 70
//! of 466 transcodes unbounded.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::analysis::{Analysis, Analyzer, Measured, Stream};

/// Names the meter and its constants; a measure by another is measured
/// again, so changing anything below means changing this.
pub const METHOD: &str = "treble/1";

/// How long a block of frames summed is.
pub const BLOCK_MS: u32 = 2000;
/// The least a frame holds; its length is the power of two at or above
/// this at the source's rate.
const FRAME_MS: u32 = 20;
/// A sample at or past this share of full scale clips: what
/// [`crate::quality`] counts as clipped at full scale.
const CLIPPED: f32 = 0.9999;
/// The least share of the sounding frames left once clipped ones are,
/// under which every frame counts.
const LEAST_UNCLIPPED: f64 = 0.1;
/// The level centibel 0 stands for, in dB of full-scale power: a band's
/// power at or under it is none.
const FLOOR_DB: f64 = -200.0;

/// The frequencies bands meet at, in Hz: every 500 Hz from 1 to 24 kHz.
pub const EDGES_HZ: [u32; 47] = edges();

const fn edges() -> [u32; 47] {
    let mut edges = [0; 47];
    let mut i = 0;
    while i < edges.len() {
        edges[i] = 1000 + 500 * i as u32;
        i += 1;
    }
    edges
}

/// The band every comparison is held to, as its energy over the energy
/// from 1 to 8 kHz: what any codec at any rate codes in full.
pub const REFERENCE_HZ: (u32, u32) = (1000, 8000);

/// A source's energy per band and per block of [`BLOCK_MS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    bands: usize,
    /// Each block's bands in turn, in centibels over [`FLOOR_DB`]; 0 for
    /// none.
    levels: Vec<u16>,
}

impl Measure {
    /// Each block's power in each of its `bands`, block after block.
    #[must_use]
    pub fn of_powers(bands: usize, powers: &[f64]) -> Self {
        let bands = bands.clamp(1, EDGES_HZ.len());
        let levels = powers[..powers.len() / bands * bands]
            .iter()
            .map(|p| level_of(*p))
            .collect();
        Self { bands, levels }
    }

    #[must_use]
    pub fn blocks(&self) -> usize {
        self.levels.len() / self.bands
    }

    /// The power of `band` in `block`.
    fn power(&self, block: usize, band: usize) -> f64 {
        power_of(self.levels[block * self.bands + band])
    }

    /// The energy of the blocks lying whole within `ms`, from `from_hz`
    /// up, over that of the same blocks in [`REFERENCE_HZ`]; `None` where
    /// no block lies whole within it or they hold no reference.
    #[must_use]
    pub fn share(&self, ms: (f64, f64), from_hz: f64) -> Option<f64> {
        let block = f64::from(BLOCK_MS);
        let first = (ms.0.max(0.0) / block).ceil() as usize;
        let last = ((ms.1 / block).floor().max(0.0) as usize).min(self.blocks());
        if first >= last {
            return None;
        }
        let at = |hz: u32| EDGES_HZ.iter().position(|e| *e == hz).unwrap_or(0);
        let (low, high) = (at(REFERENCE_HZ.0), at(REFERENCE_HZ.1).min(self.bands));
        let from = nearest_edge(from_hz).min(self.bands);
        let (mut reference, mut region) = (0.0, 0.0);
        for b in first..last {
            reference += (low..high).map(|band| self.power(b, band)).sum::<f64>();
            region += (from..self.bands)
                .map(|band| self.power(b, band))
                .sum::<f64>();
        }
        (reference > 0.0).then(|| region / reference)
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1 + 2 * self.levels.len());
        bytes.push(u8::try_from(self.bands).unwrap_or(u8::MAX));
        bytes.extend(self.levels.iter().flat_map(|l| l.to_le_bytes()));
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (&bands, rest) = bytes.split_first()?;
        let bands = usize::from(bands);
        let (pairs, odd) = rest.as_chunks::<2>();
        if !(1..=EDGES_HZ.len()).contains(&bands) || !odd.is_empty() || pairs.len() % bands != 0 {
            return None;
        }
        let levels = pairs.iter().map(|p| u16::from_le_bytes(*p)).collect();
        Some(Self { bands, levels })
    }
}

impl Serialize for Measure {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(self.to_bytes()))
    }
}

impl<'de> Deserialize<'de> for Measure {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD
            .decode(text)
            .ok()
            .and_then(|b| Self::from_bytes(&b))
            .ok_or_else(|| serde::de::Error::custom("not a treble measure"))
    }
}

/// Where two copies play one recording: copy B plays copy A's moment `t`,
/// in ms, at `offset_ms + t·stretch`, as [`crate::align`] finds; each is
/// as long as it says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shared {
    pub offset_ms: f64,
    pub stretch: f64,
    pub a_ms: f64,
    pub b_ms: f64,
}

impl Shared {
    /// The stretch both play, from and to, in A's time and in B's.
    #[must_use]
    pub fn spans(&self) -> ((f64, f64), (f64, f64)) {
        let stretch = self.stretch.max(f64::MIN_POSITIVE);
        let from = (-self.offset_ms / stretch).max(0.0);
        let to = ((self.b_ms - self.offset_ms) / stretch).min(self.a_ms);
        let b = |t: f64| self.offset_ms + t * stretch;
        ((from, to), (b(from), b(to)))
    }
}

/// The edge nearest `hz`, where a region of bands summed starts.
#[must_use]
pub fn edge_hz(hz: f64) -> f64 {
    f64::from(EDGES_HZ[nearest_edge(hz)])
}

/// The edge at or under `hz`, or the first.
#[must_use]
pub fn edge_under_hz(hz: f64) -> f64 {
    let under = EDGES_HZ.iter().rev().find(|e| f64::from(**e) <= hz);
    f64::from(*under.unwrap_or(&EDGES_HZ[0]))
}

/// The index of the edge nearest `hz`.
fn nearest_edge(hz: f64) -> usize {
    (0..EDGES_HZ.len())
        .min_by(|a, b| {
            let off = |i: usize| (f64::from(EDGES_HZ[i]) - hz).abs();
            off(*a).total_cmp(&off(*b))
        })
        .unwrap_or(0)
}

fn level_of(power: f64) -> u16 {
    if power <= 0.0 || !power.is_finite() {
        return 0;
    }
    let cb = ((10.0 * power.log10() - FLOOR_DB) * 10.0).round();
    cb.clamp(0.0, f64::from(u16::MAX)) as u16
}

fn power_of(level: u16) -> f64 {
    if level == 0 {
        0.0
    } else {
        10_f64.powf((f64::from(level) / 10.0 + FLOOR_DB) / 10.0)
    }
}

/// The band each FFT bin of a frame of `size` at `rate` falls in, up to
/// the bin at half the rate; `None` below the first edge.
fn bands_of_bins(size: usize, rate: u32) -> (Vec<Option<usize>>, usize) {
    let half = f64::from(rate) / 2.0;
    let bands = EDGES_HZ
        .iter()
        .filter(|e| f64::from(**e) < half)
        .count()
        .max(1);
    let of = (0..=size / 2)
        .map(|k| {
            let hz = k as f64 * f64::from(rate) / size as f64;
            EDGES_HZ[..bands].iter().rposition(|e| f64::from(*e) <= hz)
        })
        .collect();
    (of, bands)
}

/// One stream's band powers as it is decoded.
pub struct Meter {
    channels: usize,
    size: usize,
    window: Vec<f64>,
    twiddles: crate::quality::Twiddles,
    of_bin: Vec<Option<usize>>,
    bands: usize,
    /// Samples to a block.
    block: usize,
    /// What a bin's power is multiplied by to be its share of a sample's
    /// mean square.
    scale: f64,
    /// Samples of the frame being filled, interleaved.
    pending: Vec<f32>,
    /// Frames read whole.
    frames: usize,
    /// Each block's summed powers, of every frame and of those unclipped,
    /// and its frames.
    all: Vec<f64>,
    unclipped: Vec<f64>,
    counted: Vec<usize>,
    /// Frames that sound, and of them those unclipped.
    sounding: (usize, usize),
    re: Vec<f64>,
    im: Vec<f64>,
    power: Vec<f64>,
}

impl std::fmt::Debug for Meter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meter")
            .field("size", &self.size)
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

impl Meter {
    #[must_use]
    pub fn new(stream: &Stream) -> Self {
        let rate = stream.rate.max(1);
        let size = (rate * FRAME_MS / 1000).max(2).next_power_of_two() as usize;
        let window: Vec<f64> = (0..size)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / size as f64).cos())
            .collect();
        let (of_bin, bands) = bands_of_bins(size, rate);
        let block = (u64::from(rate) * u64::from(BLOCK_MS) / 1000) as usize;
        // Both halves of the spectrum, over the window's energy.
        let energy: f64 = window.iter().map(|w| w * w).sum();
        let scale = 2.0 / (size as f64 * energy);
        Self {
            channels: usize::from(stream.channels).max(1),
            size,
            twiddles: crate::quality::Twiddles::new(size),
            window,
            of_bin,
            bands,
            block,
            scale,
            pending: Vec::with_capacity(size * usize::from(stream.channels).max(1)),
            frames: 0,
            all: Vec::new(),
            unclipped: Vec::new(),
            counted: Vec::new(),
            sounding: (0, 0),
            re: vec![0.0; size],
            im: vec![0.0; size],
            power: vec![0.0; bands],
        }
    }

    /// The frame `pending` holds, into its block.
    fn frame(&mut self) {
        let (n, channels) = (self.size, self.channels);
        let samples = &self.pending;
        let clipped = samples.iter().any(|s| s.abs() >= CLIPPED);
        let square: f64 = samples.iter().map(|s| f64::from(*s).powi(2)).sum();
        let sounds = (square / samples.len() as f64).sqrt() >= crate::quality::SILENT_RMS;
        self.power.fill(0.0);
        for pair in (0..channels).step_by(2) {
            for i in 0..n {
                let w = self.window[i];
                self.re[i] = f64::from(samples[i * channels + pair]) * w;
                self.im[i] = if pair + 1 < channels {
                    f64::from(samples[i * channels + pair + 1]) * w
                } else {
                    0.0
                };
            }
            crate::quality::fft(&mut self.re, &mut self.im, &self.twiddles);
            for (k, band) in self.of_bin.iter().enumerate() {
                let Some(band) = *band else { continue };
                let m = (n - k) % n;
                let p = self.re[k].powi(2)
                    + self.im[k].powi(2)
                    + self.re[m].powi(2)
                    + self.im[m].powi(2);
                self.power[band] += p / 2.0;
            }
        }
        let block = (self.frames * n + n / 2) / self.block;
        if self.counted.len() <= block {
            self.counted.resize(block + 1, 0);
            self.all.resize((block + 1) * self.bands, 0.0);
            self.unclipped.resize((block + 1) * self.bands, 0.0);
        }
        self.counted[block] += 1;
        let at = block * self.bands;
        for (band, p) in self.power.iter().enumerate() {
            self.all[at + band] += p * self.scale;
            if !clipped {
                self.unclipped[at + band] += p * self.scale;
            }
        }
        if sounds {
            self.sounding.0 += 1;
            self.sounding.1 += usize::from(!clipped);
        }
        self.frames += 1;
    }

    fn measure(&self) -> Option<Measure> {
        let (sounding, unclipped) = self.sounding;
        if sounding == 0 {
            return None;
        }
        let sums = if unclipped as f64 >= LEAST_UNCLIPPED * sounding as f64 {
            &self.unclipped
        } else {
            &self.all
        };
        let powers: Vec<f64> = sums
            .chunks(self.bands)
            .zip(&self.counted)
            .flat_map(|(block, n)| block.iter().map(move |p| p / *n as f64))
            .collect();
        Some(Measure::of_powers(self.bands, &powers))
    }
}

impl Analyzer for Meter {
    fn take(&mut self, frames: &[f32]) {
        let width = self.size * self.channels;
        let mut rest = frames;
        while !rest.is_empty() {
            let wanted = width - self.pending.len();
            let (now, later) = rest.split_at(wanted.min(rest.len()));
            self.pending.extend_from_slice(now);
            rest = later;
            if self.pending.len() == width {
                self.frame();
                self.pending.clear();
            }
        }
    }

    fn finish(self: Box<Self>, into: &mut Analysis) {
        into.treble = Some(Measured {
            method: METHOD.to_string(),
            value: self.measure(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Kind, Pass};
    use crate::testing::wav;

    /// Deterministic noise in [-1, 1).
    fn noise(n: usize, seed: u64) -> Vec<f64> {
        crate::testing::words(n, seed)
            .into_iter()
            .map(|w| f64::from(w) / f64::from(u32::MAX) * 2.0 - 1.0)
            .collect()
    }

    /// Noise at `rate` with each frequency scaled by `gain`, 4096
    /// samples at a time.
    fn shaped(n: usize, rate: u32, seed: u64, gain: impl Fn(f64) -> f64) -> Vec<f32> {
        const SIZE: usize = 4096;
        let twiddles = crate::quality::Twiddles::new(SIZE);
        let mut out = Vec::new();
        for chunk in noise(n, seed).chunks(SIZE) {
            let mut re = chunk.to_vec();
            re.resize(SIZE, 0.0);
            let mut im = vec![0.0; SIZE];
            crate::quality::fft(&mut re, &mut im, &twiddles);
            for k in 0..SIZE {
                let g = gain(k.min(SIZE - k) as f64 * f64::from(rate) / SIZE as f64);
                re[k] *= g;
                im[k] = -im[k] * g;
            }
            crate::quality::fft(&mut re, &mut im, &twiddles);
            out.extend(re.iter().map(|v| (v / SIZE as f64 * 0.3) as f32));
        }
        out.truncate(n);
        out
    }

    fn measure(rate: u32, channels: u16, samples: &[f32]) -> Option<Measure> {
        let mut pass = Pass::new(vec![Kind::Treble]);
        pass.take(&wav(rate, channels, None, samples));
        pass.finish(&crate::integrity::Found::default())
            .treble()
            .cloned()
    }

    fn db(x: f64) -> f64 {
        10.0 * x.log10()
    }

    #[test]
    fn treble_past_a_lowpass_measures_as_nothing_and_under_it_as_much() {
        let rate = 48_000;
        let open = shaped(10 * 48_000, rate, 1, |_| 1.0);
        let cut = shaped(
            10 * 48_000,
            rate,
            1,
            |f| if f > 16_000.0 { 0.0 } else { 1.0 },
        );
        let (open, cut) = (
            measure(rate, 1, &open).unwrap(),
            measure(rate, 1, &cut).unwrap(),
        );
        assert_eq!(open.blocks(), 5);
        assert_eq!(open.bands, 46);
        let whole = (0.0, 10_000.0);
        let above = |m: &Measure| m.share(whole, 16_500.0).unwrap();
        assert!(db(above(&open)) > -2.0, "{}", db(above(&open)));
        assert!(db(above(&cut)) < -60.0, "{}", db(above(&cut)));
        let under = |m: &Measure| m.share(whole, 12_000.0).unwrap();
        let between = open.share(whole, 12_000.0).unwrap() - open.share(whole, 16_000.0).unwrap();
        assert!((db(under(&cut)) - db(between)).abs() < 0.2);
    }

    #[test]
    fn channels_sum_their_power_and_a_frame_clipping_is_left_out() {
        let rate = 44_100;
        let left = shaped(4 * 44_100, rate, 2, |_| 1.0);
        let right = shaped(4 * 44_100, rate, 3, |f| if f > 8000.0 { 0.0 } else { 1.0 });
        let both: Vec<f32> = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [*l, *r])
            .collect();
        let m = measure(rate, 2, &both).unwrap();
        assert_eq!(m.bands, 43, "to 22 kHz, then a band to 22.05");
        let alone = measure(rate, 1, &left).unwrap();
        let (s, a) = (
            m.share((0.0, 4000.0), 12_000.0).unwrap(),
            alone.share((0.0, 4000.0), 12_000.0).unwrap(),
        );
        assert!((db(a) - db(s) - 3.0).abs() < 0.3, "{} {}", db(a), db(s));
        let mut clipped = shaped(4 * 44_100, rate, 4, |f| if f > 8000.0 { 0.0 } else { 1.0 });
        for x in clipped.iter_mut().skip(44_100).step_by(2048) {
            *x = 1.0;
        }
        let m = measure(rate, 1, &clipped).unwrap();
        assert!(db(m.share((0.0, 4000.0), 12_000.0).unwrap()) < -60.0);
        let all: Vec<f32> = clipped.iter().map(|x| (x * 8.0).clamp(-1.0, 1.0)).collect();
        assert!(
            measure(rate, 1, &all).is_some(),
            "clipped in every frame, every frame counts"
        );
    }

    #[test]
    fn a_measure_is_kept_as_base64_and_refused_when_it_is_none() {
        let m = Measure::of_powers(2, &[1.0, 1e-3, 0.0, 2.0]);
        let text = serde_json::to_string(&m).unwrap();
        let back: Measure = serde_json::from_str(&text).unwrap();
        assert_eq!(back, m);
        assert!((back.power(0, 1) - 1e-3).abs() < 1e-5);
        assert!(back.power(1, 0).abs() < f64::MIN_POSITIVE);
        assert!(serde_json::from_str::<Measure>("\"AAAA\"").is_err());
        assert!(serde_json::from_str::<Measure>("\"!!\"").is_err());
    }

    #[test]
    fn a_share_counts_whole_blocks_within_the_stretch() {
        let m = Measure::of_powers(46, &[[1.0; 46], [0.0; 46], [1.0; 46]].concat());
        assert!(
            m.share((2500.0, 5000.0), 15_000.0).is_none(),
            "no block whole"
        );
        assert!(m.share((1000.0, 3000.0), 15_000.0).is_none());
        let first = m.share((0.0, 2000.0), 15_000.0).unwrap();
        assert!((first - 18.0 / 14.0).abs() < 1e-3, "{first}");
        assert_eq!(m.share((0.0, 99_999.0), 15_000.0), Some(first));
        assert_eq!(nearest_edge(15_193.0), 28);
        let khz = |hz: f64| (hz / 100.0).round() / 10.0;
        assert!((khz(edge_hz(15_620.0)) - 15.5).abs() < 1e-9);
        assert!((khz(edge_under_hz(15_600.0)) - 15.5).abs() < 1e-9);
        assert!((khz(edge_under_hz(12_000.0)) - 12.0).abs() < 1e-9);
        assert!((khz(edge_under_hz(400.0)) - 1.0).abs() < 1e-9);
    }

    /// What the meter makes of noise lowpassed at three frequencies, at two
    /// rates, with [`METHOD`].
    const WRITTEN: (&str, &str) = (
        "treble/1",
        "f3e201382fe755f893642bac0db5f94cfd6a2245a722e0435f9b77fec87d8fa6",
    );

    #[test]
    fn the_method_rises_with_what_it_makes() {
        let mut digest = crate::testing::Digest::default();
        for (rate, cut) in [(44_100, 16_000.0), (48_000, 19_000.0), (32_000, 9000.0)] {
            let samples = shaped(
                rate as usize * 3,
                rate,
                5,
                |f| if f > cut { 0.0 } else { 1.0 },
            );
            let stereo: Vec<f32> = samples.iter().flat_map(|x| [*x, x * 0.5]).collect();
            digest.debug(&measure(rate, 2, &stereo).map(|m| m.to_bytes()));
        }
        crate::testing::check(WRITTEN, METHOD, &digest.hex());
    }

    #[test]
    fn silence_measures_nothing_but_records_its_method() {
        let mut pass = Pass::new(vec![Kind::Treble]);
        pass.take(&wav(48_000, 1, None, &vec![0.0; 48_000]));
        let analysis = pass.finish(&crate::integrity::Found::default());
        let measured = analysis.treble.unwrap();
        assert_eq!(measured.method, METHOD);
        assert!(measured.value.is_none());
    }
}
