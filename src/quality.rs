//! How good a source's audio and pictures are, measured from their
//! decoded samples, never from what a container or codec declares: an
//! upscaled image or a transcoded file claims more than it holds.
//!
//! Audio is measured on short excerpts at a quarter, half and three
//! quarters of its length, decoded at the source's own rate, every channel
//! as it is: a mix into two would add channels into clipping that none of
//! them holds, and cancel channels in opposite phase.
//!
//! - **Bandwidth.** Where a lowpass cuts the sound off: the top of the
//!   highest band standing 20 dB above all bands past a short guard,
//!   in each excerpt's mean spectrum, its channels' powers summed, taken
//!   low across the excerpts. A FLAC transcoded from a 16 kHz source
//!   measures 16 kHz; a recording whose treble fades into its noise floor
//!   shows no wall and measures full.
//! - **Real stereo.** One minus the first two channels' squared
//!   correlation at the lag within 1 ms that best aligns them: mono copied
//!   into two channels, at two levels or a few samples apart, has next to
//!   none and counts as mono, as one channel does.
//! - **Clipping.** The share of samples in runs at full scale or just under
//!   the peak, or piled up just under full scale once a lossy codec smeared
//!   them. Float lossless audio holds samples past full scale unclipped, so
//!   in it only runs just under its own peak count.
//!
//! A picture is measured in gray.
//!
//! - **Content.** In a picture that is not square, uniform rows or columns
//!   that match on opposite sides, within 2% of each other in width, are
//!   bars, and the content is what they enclose: a pillarboxed video frame
//!   yields its square art, which the cover is cropped to. Square art is
//!   never trimmed: bands across it are its design, and a solid edge on
//!   one side only is part of any picture.
//! - **Effective resolution.** Detail at a size is the mean squared
//!   Laplacian over the variance, once the picture is averaged down to
//!   that size. Stepping up from 256 px by half an octave, each size must
//!   keep a set share of the detail half its size holds; the effective
//!   resolution is the last size that does. An upscale fails past the size
//!   it was made from, so a large pixel count proves nothing. Measured on
//!   covers: a native 1400 px album art measured 1400 px; a 3000 px
//!   upscale of a 500 px picture measured 362 px; release covers served at
//!   1400 and 1500 px measured 512 px.
//! - **Block artifacts.** How much more the pixels jump across 8×8 block
//!   boundaries than halfway between them. Halfway shares the boundary's
//!   parity, so the pattern a 2× upscale leaves cancels out.

use std::ffi::OsString;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A cover whose detail holds up to fewer pixels than this looks soft
/// on a large screen.
pub const SOFT_COVER: u32 = 500;

/// Names the measures and their constants; facts measured by another
/// are measured again, so changing anything below means changing this.
pub const METHOD: &str = "quality/6";

/// Seconds of audio measured at each of [`SEGMENTS`].
pub const SEGMENT_SECONDS: f64 = 8.0;
/// Where the audio is measured, as shares of its length.
pub const SEGMENTS: [f64; 3] = [0.25, 0.5, 0.75];

const FFT: usize = 4096;
/// FFT bins summed into one band when looking for a wall.
const BAND_BINS: usize = 16;
/// Bands between a wall's top and the bands it stands above, about 375 Hz
/// at 48 kHz: room for an encoder's lowpass to fall.
const WALL_GUARD: usize = 2;
/// How far a wall stands above every band past its guard. A recording's
/// own treble fades gradually and leaves none, so it measures half its
/// sample rate; each excerpt is judged alone, so an encoder starved of bits
/// in one passage still shows its wall.
///
/// Calibrated on 22 public-domain recordings, orchestral, chamber and
/// piano, 16 lossless at 44.1 and 48 kHz and 6 transfers of 78 rpm discs at
/// 96 kHz, and on copies of 8 of them made for the purpose:
///
/// - The measure walls replaced, each frame's top band within 80 dB of its
///   loudest, put 6 of the lossless recordings at 10.5 to 12.1 kHz, where
///   their treble faded into their noise floor, and ranked two of them
///   below their own 48 kbit/s Opus copies. Walls put every recording at
///   half its sample rate.
/// - Of 80 lossy or resampled copies, walls put 69 at least 1 kHz below
///   their source; the old measure, 57. Off the darkest recordings, MP3 at
///   128 kbit/s measured 16 to 16.5 kHz and AAC at 128 kbit/s 17.2 to
///   17.4 kHz; Opus at 128 kbit/s measured 20 to 20.6 kHz on all. Opus at
///   48 kbit/s on solo piano, starved, measured 8.4 and 12.6 kHz.
/// - A copy whose lost treble stood less than 20 dB above what was left in
///   its place shows no wall. Two 44.1 kHz piano recordings resampled to
///   22.05 kHz and back up to 48 kHz measured 24 kHz, above their sources.
const WALL_DB: f64 = 20.0;
/// Walls are looked for above this frequency. Below it a piano's spectrum
/// can fall nearly as steeply as a lowpass: with a 15 dB wall, soft piano
/// showed walls at 1 to 1.3 kHz.
const WALL_MIN_HZ: f64 = 2000.0;
/// Frames quieter than this RMS, about -60 dBFS, say nothing of bandwidth.
const SILENT_RMS: f64 = 1e-3;
/// Energy no gain and lag between the channels explains, below this share,
/// is a mono source in two channels. On the recordings `WALL_DB` was
/// calibrated on, stereo ones, the 78 rpm transfers among them, left 0.19
/// to 0.89 unexplained. Mono copied into channels 1 dB apart left under
/// 2e-7, and at most 3.5e-3 through Opus; the side-to-mid ratio used before
/// counted it as stereo. Mono with one channel delayed by whole samples up
/// to 1 ms left under 1e-11, and by half a sample at most 2.1e-4, or 2.7e-3
/// through Opus; at lag 0 alone, one sample left up to 2.4e-2.
pub const STEREO_INCOHERENCE: f64 = 1e-2;
/// The longest delay between the channels a mono copy may carry, in
/// seconds: a tape head's azimuth or a resampled channel.
const STEREO_LAG: f64 = 1e-3;
const CLIP_LEVEL: f32 = 0.9999;
/// Samples this close to the audio's own peak are at a lowered clip level.
/// On the recordings [`WALL_DB`] was calibrated on, clipped masters turned
/// down by 1 dB measured as clipped as before; at full scale alone they
/// measured nothing. MP3 smears flat tops: clipped masters measuring 2.6e-3
/// to 7.9e-2 measured 2e-5 to 6.2e-3 as MP3. No lossless recording measured
/// over 1e-6, and no master limited below full scale measured any.
const CLIP_OF_PEAK: f32 = 0.999;
const CLIP_RUN: usize = 3;
/// The most bits a sample is counted to use: a 32-bit float holds 24.
const MOST_BITS: u32 = 24;
/// A lossy codec smears clipped flat tops into a pile-up of samples just
/// under full scale, rising from a valley below it and falling into the
/// codec's overshoot above it. It is looked for from this level up, in bins
/// [`PILE_BIN`] wide, against bins up to [`PILE_REACH`] below it and from
/// [`PILE_OVERSHOOT`] above it.
const PILE_FROM: f64 = 0.9;
const PILE_BIN: f64 = 0.01;
const PILE_REACH: f64 = 0.2;
/// A tone's samples crowd just under its own peak; a codec rings past the
/// flat tops it smears, at least 0.06 past them on the clipped masters.
const PILE_OVERSHOOT: f64 = 0.04;
/// How many times the valley below, and some bin past the overshoot, a
/// pile-up holds.
///
/// On the recordings [`WALL_DB`] was calibrated on, clipped masters as MP3,
/// measuring 2e-5 to 6.2e-3 in runs, measured 3.3e-3 to 1e-1 piled up,
/// against 2.6e-3 to 7.9e-2 before encoding. No unclipped master piled up:
/// not 8 peaking at -0.1 dBFS, nor 8 driven 12 dB into a limiter at
/// -0.2 dBFS, each also as MP3 and Opus, and the limited ones as AAC. A loud
/// pure tone through a codec would pile up the same way; none was measured.
const PILE_RISE: f64 = 2.0;
/// The least share of the samples a pile-up's top bin holds.
const PILE_SHARE: f64 = 1e-4;

/// A row or column at most this far from uniform, in gray levels, is a
/// border when it matches the edge.
const BORDER_STDDEV: f64 = 6.0;
const BORDER_MATCH: f64 = 8.0;
/// Trimming that leaves less than this share of a side is not a border.
const MIN_CONTENT: f64 = 0.25;
/// How far from square, as the log of the sides' ratio, a picture may be
/// and still be square art.
pub const SQUARE_TOLERANCE: f64 = 0.03;
/// Bars on opposite sides are a letterbox or pillarbox when their widths
/// differ by at most this share of the side.
const BAR_SYMMETRY: f64 = 0.02;
/// The smallest size detail is judged at: below it every picture has
/// real detail.
const REFERENCE_SIDE: usize = 128;
/// A size holds real detail when its finest detail is at least this
/// share of what half the size holds at its own finest. Measured on
/// covers, native ones kept 0.28 or more at every size, and upscaled ones
/// fell to 0.22 or less past the size they were made from.
const OCTAVE_FLOOR: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioQuality {
    /// The frequency below which the audio carries sound.
    pub bandwidth_hz: f64,
    /// The share of the channels' energy no gain from one to the other
    /// explains: one minus their squared correlation.
    #[serde(alias = "side_ratio")]
    pub incoherence: f64,
    /// The share of samples in clipped runs.
    pub clipping: f64,
    /// The bits its samples use, at most 24: a 16-bit recording padded to
    /// 24 bits uses 16.
    #[serde(default)]
    pub bits: u32,
}

impl AudioQuality {
    #[must_use]
    pub fn is_stereo(&self, incoherence: f64) -> bool {
        self.incoherence > incoherence
    }

    /// The bandwidth in whole steps of `step_hz`, so noise never decides.
    #[must_use]
    pub fn bandwidth_bucket(&self, step_hz: f64) -> i64 {
        bucket(self.bandwidth_hz / step_hz)
    }

    /// How many of `cutoffs` the share of clipped samples reaches.
    #[must_use]
    pub fn clipping_bucket(&self, cutoffs: &[f64]) -> i64 {
        let reached = cutoffs.iter().filter(|c| self.clipping >= **c).count();
        i64::try_from(reached).unwrap_or(i64::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImageQuality {
    pub width: u32,
    pub height: u32,
    /// What is left once uniform borders are trimmed.
    pub content: Rect,
    /// The shorter side of the content at the resolution its detail
    /// supports, in steps of half an octave.
    pub effective: u32,
    /// Discontinuity across 8×8 block boundaries beyond that inside them.
    pub blockiness: f64,
}

impl ImageQuality {
    #[must_use]
    pub fn is_square(&self, tolerance: f64) -> bool {
        let (w, h) = (
            f64::from(self.content.width),
            f64::from(self.content.height),
        );
        h > 0.0 && (w / h).ln().abs() <= tolerance
    }

    #[must_use]
    pub fn is_cropped(&self) -> bool {
        self.content.width != self.width || self.content.height != self.height
    }

    /// The effective resolution in steps each `step` larger than the
    /// last, 0.1 for 10%; none for a picture with no detail.
    #[must_use]
    pub fn resolution_bucket(&self, step: f64) -> i64 {
        if self.effective == 0 {
            return 0;
        }
        bucket(f64::from(self.effective).ln() / step.ln_1p())
    }

    /// Block artifacts in whole steps of `step`, up to a tenth step.
    #[must_use]
    pub fn blockiness_bucket(&self, step: f64) -> i64 {
        bucket(self.blockiness / step).min(10)
    }
}

/// `x` rounded down to a whole step. The measures stay far inside an
/// `i64`.
#[allow(clippy::cast_possible_truncation)]
fn bucket(x: f64) -> i64 {
    if x.is_finite() { x.floor() as i64 } else { 0 }
}

/// A count as a float. Sample and pixel counts stay far below 2^52.
#[allow(clippy::cast_precision_loss)]
fn real(n: usize) -> f64 {
    n as f64
}

/// The ffmpeg input options that read one measured segment, from
/// `start`: seeking on the input decodes only the segment, where seeking
/// on the output decoded all that came before it, and every segment of a
/// long file held in memory at once.
#[must_use]
pub fn segment_input(start: f64) -> Vec<OsString> {
    [
        "-ss".to_string(),
        format!("{start:.3}"),
        "-t".to_string(),
        format!("{SEGMENT_SECONDS}"),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// The ffmpeg output options that write the audio stream `input:index`
/// as 32-bit float, every channel, at its own rate.
#[must_use]
pub fn segment_output(input: usize, index: u32) -> Vec<OsString> {
    [
        "-map".to_string(),
        format!("{input}:{index}"),
        "-c:a".to_string(),
        "pcm_f32le".to_string(),
        "-f".to_string(),
        "f32le".to_string(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// The ffmpeg output options that write one frame of the picture
/// `input:stream` as a binary graymap, which carries its own size.
#[must_use]
pub fn gray_output(input: usize, stream: &str) -> Vec<OsString> {
    [
        "-map".to_string(),
        format!("{input}:{stream}"),
        "-frames:v".to_string(),
        "1".to_string(),
        "-pix_fmt".to_string(),
        "gray".to_string(),
        "-c:v".to_string(),
        "pgm".to_string(),
        "-f".to_string(),
        "image2".to_string(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// One measured segment's samples, each channel's in turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    channels: usize,
    samples: Vec<f32>,
}

impl Default for Segment {
    fn default() -> Self {
        Self::new(1, Vec::new())
    }
}

impl Segment {
    /// `samples` of `channels` interleaved; a frame cut short is dropped.
    #[must_use]
    pub fn new(channels: usize, mut samples: Vec<f32>) -> Self {
        let channels = channels.max(1);
        samples.truncate(samples.len() / channels * channels);
        Self { channels, samples }
    }

    /// Interleaved little-endian f32 samples of `channels`.
    #[must_use]
    pub fn of_bytes(bytes: &[u8], channels: usize) -> Self {
        let samples = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        Self::new(channels, samples)
    }

    fn channel(&self, c: usize) -> impl Iterator<Item = f32> + '_ {
        self.samples.iter().skip(c).step_by(self.channels).copied()
    }

    /// The first two channels side by side; one channel is both.
    fn front(&self) -> Vec<[f32; 2]> {
        let right = usize::from(self.channels > 1);
        self.samples
            .chunks_exact(self.channels)
            .map(|f| [f[0], f[right]])
            .collect()
    }
}

/// Measure decoded segments of one recording; `None` when they hold no
/// sound at all. With `headroom`, as float lossless audio has, samples
/// above full scale were never clipped there, so only runs at its own
/// peak count.
#[must_use]
pub fn audio(segments: &[Segment], sample_rate: u32, headroom: bool) -> Option<AudioQuality> {
    let window: Vec<f64> = (0..FFT)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * real(i) / real(FFT)).cos())
        .collect();
    let peak = segments
        .iter()
        .flat_map(|s| &s.samples)
        .fold(0.0_f32, |m, x| m.max(x.abs()));
    let level = if headroom {
        peak * CLIP_OF_PEAK
    } else {
        CLIP_LEVEL.min(peak * CLIP_OF_PEAK)
    };
    let mut bandwidth: Option<f64> = None;
    let (mut clipped, mut total) = (0_usize, 0_usize);
    for segment in segments {
        for channel in 0..segment.channels {
            clipped += clipped_samples(segment.channel(channel), level);
        }
        total += segment.samples.len();
        if let Some(spectrum) = mean_spectrum(segment, &window) {
            let hz = wall(&spectrum, sample_rate);
            bandwidth = Some(bandwidth.map_or(hz, |b| b.min(hz)));
        }
    }
    let bandwidth_hz = bandwidth?;
    if peak == 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let reach = (f64::from(sample_rate) * STEREO_LAG).round() as usize;
    let incoherence = if segments.iter().all(|s| s.channels == 1) {
        0.0
    } else {
        let fronts: Vec<Vec<[f32; 2]>> = segments.iter().map(Segment::front).collect();
        incoherence(&fronts, reach)
    };
    let clipping = (real(clipped) / real(total.max(1))).max(pile_up(segments, peak, total));
    Some(AudioQuality {
        bandwidth_hz,
        incoherence,
        clipping,
        bits: bits_used(segments),
    })
}

/// The bits the samples use, at most [`MOST_BITS`]: the place of the
/// lowest bit any sample sets, counted from full scale. A float sample
/// past what 24 bits hold uses them all.
fn bits_used(segments: &[Segment]) -> u32 {
    let scale = f64::from(1_u32 << (MOST_BITS - 1));
    let lowest = segments
        .iter()
        .flat_map(|s| &s.samples)
        .map(|x| f64::from(*x) * scale)
        .filter(|x| *x != 0.0)
        .map(|x| {
            if x.fract() == 0.0 && x.abs() < 2.0 * scale {
                #[allow(clippy::cast_possible_truncation)]
                (x as i64).trailing_zeros()
            } else {
                0
            }
        })
        .min();
    lowest.map_or(0, |zeros| MOST_BITS.saturating_sub(zeros).max(1))
}

/// One minus the squared correlation of the channels at the lag within
/// `reach` samples where it is highest, interpolated between whole lags. A
/// silent channel beside a sounding one is no mono copy, and measures 1.
fn incoherence(segments: &[Vec<[f32; 2]>], reach: usize) -> f64 {
    let lags = 2 * reach + 1;
    let (mut cross, mut right, mut left) = (vec![0.0; lags], vec![0.0; lags], 0.0);
    for samples in segments.iter().filter(|s| s.len() > 2 * reach) {
        let n = samples.len();
        let mut energy = vec![0.0; n + 1];
        for (i, s) in samples.iter().enumerate() {
            energy[i + 1] = energy[i] + f64::from(s[1]).powi(2);
        }
        let middle = &samples[reach..n - reach];
        left += middle.iter().map(|s| f64::from(s[0]).powi(2)).sum::<f64>();
        for k in 0..lags {
            right[k] += energy[n - 2 * reach + k] - energy[k];
            cross[k] += middle
                .iter()
                .zip(&samples[k..])
                .map(|(l, r)| f64::from(l[0]) * f64::from(r[1]))
                .sum::<f64>();
        }
    }
    if left == 0.0 || right.contains(&0.0) {
        return 1.0;
    }
    let rho: Vec<f64> = (0..lags)
        .map(|k| (cross[k] / (left * right[k]).sqrt()).abs())
        .collect();
    let k = (0..lags)
        .max_by(|a, b| rho[*a].total_cmp(&rho[*b]))
        .unwrap_or(reach);
    let mut peak = rho[k];
    if k > 0 && k + 1 < lags {
        let curve = rho[k - 1] - 2.0 * rho[k] + rho[k + 1];
        if curve < 0.0 {
            peak -= (rho[k - 1] - rho[k + 1]).powi(2) / (8.0 * curve);
        }
    }
    (1.0 - peak.min(1.0).powi(2)).max(0.0)
}

/// The share of samples from the valley under a pile-up up, as
/// [`PILE_FROM`] describes; zero when there is none.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pile_up(segments: &[Segment], peak: f32, total: usize) -> f64 {
    let lowest = PILE_FROM - PILE_REACH;
    let peak = f64::from(peak);
    if peak <= PILE_FROM || total == 0 {
        return 0.0;
    }
    let bins = ((peak - lowest) / PILE_BIN).ceil() as usize + 1;
    let mut counts = vec![0_usize; bins];
    for x in segments.iter().flat_map(|s| &s.samples) {
        let a = f64::from(x.abs());
        if a >= lowest {
            counts[(((a - lowest) / PILE_BIN) as usize).min(bins - 1)] += 1;
        }
    }
    let reach = (PILE_REACH / PILE_BIN).round() as usize;
    let Some(top) = (reach..bins).max_by_key(|b| counts[*b]) else {
        return 0.0;
    };
    let held = real(counts[top]);
    let overshoot = (PILE_OVERSHOOT / PILE_BIN).round() as usize;
    let falls = counts
        .get(top + overshoot..)
        .is_some_and(|above| above.iter().any(|c| real(*c) * PILE_RISE <= held));
    let valley = (top - reach..top).min_by_key(|b| counts[*b]).unwrap_or(top);
    if held < PILE_SHARE * real(total) || !falls || real(counts[valley]) * PILE_RISE > held {
        return 0.0;
    }
    real(counts[valley..].iter().sum()) / real(total)
}

fn clipped_samples(channel: impl Iterator<Item = f32>, level: f32) -> usize {
    let (mut clipped, mut run) = (0, 0);
    for x in channel {
        if x.abs() >= level {
            run += 1;
        } else {
            if run >= CLIP_RUN {
                clipped += run;
            }
            run = 0;
        }
    }
    if run >= CLIP_RUN {
        clipped + run
    } else {
        clipped
    }
}

/// The mean power spectrum of a segment's sounding frames, by FFT bin,
/// each channel's power summed: channels in opposite phase cancel in a
/// mix, and measured as silence.
fn mean_spectrum(segment: &Segment, window: &[f64]) -> Option<Vec<f64>> {
    let mut sum = vec![0.0; FFT / 2];
    let mut frames = 0_usize;
    let width = FFT * segment.channels;
    for frame in segment.samples.chunks_exact(width) {
        let energy: f64 = frame.iter().map(|x| f64::from(*x).powi(2)).sum();
        if (energy / real(width)).sqrt() < SILENT_RMS {
            continue;
        }
        for channel in 0..segment.channels {
            let mut re: Vec<f64> = frame
                .iter()
                .skip(channel)
                .step_by(segment.channels)
                .zip(window)
                .map(|(x, w)| f64::from(*x) * w)
                .collect();
            let mut im = vec![0.0; FFT];
            fft(&mut re, &mut im);
            for (k, power) in sum.iter_mut().enumerate() {
                *power += re[k] * re[k] + im[k] * im[k];
            }
        }
        frames += 1;
    }
    (frames > 0).then(|| sum.into_iter().map(|p| p / real(frames)).collect())
}

/// The top of the highest band above [`WALL_MIN_HZ`] that stands
/// [`WALL_DB`] above every band past [`WALL_GUARD`]; half the sample rate
/// when none does.
fn wall(spectrum: &[f64], sample_rate: u32) -> f64 {
    let hz = |bands: usize| real(bands * BAND_BINS) * f64::from(sample_rate) / real(FFT);
    let levels: Vec<f64> = spectrum
        .chunks(BAND_BINS)
        .map(|c| 10.0 * (c.iter().sum::<f64>() + f64::MIN_POSITIVE).log10())
        .collect();
    let mut above = vec![f64::NEG_INFINITY; levels.len() + 1];
    for b in (0..levels.len()).rev() {
        above[b] = above[b + 1].max(levels[b]);
    }
    (0..levels.len().saturating_sub(WALL_GUARD + 1))
        .rev()
        .take_while(|b| hz(b + 1) >= WALL_MIN_HZ)
        .find(|b| levels[*b] - above[b + WALL_GUARD + 1] >= WALL_DB)
        .map_or(f64::from(sample_rate) / 2.0, |b| hz(b + 1))
}

/// An in-place radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f64::consts::TAU / real(len);
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

/// A gray image, row by row.
struct Gray<'a> {
    pixels: &'a [u8],
    width: usize,
}

impl Gray<'_> {
    fn at(&self, x: usize, y: usize) -> f64 {
        f64::from(self.pixels[y * self.width + x])
    }

    fn line_stats(&self, points: impl Iterator<Item = (usize, usize)>) -> (f64, f64) {
        let values: Vec<f64> = points.map(|(x, y)| self.at(x, y)).collect();
        let mean = values.iter().sum::<f64>() / real(values.len().max(1));
        let var =
            values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / real(values.len().max(1));
        (mean, var.sqrt())
    }
}

/// Measure a picture from its raw gray pixels; `None` when the buffer
/// does not hold `width × height` of them.
#[must_use]
pub fn image(pixels: &[u8], width: u32, height: u32) -> Option<ImageQuality> {
    let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    if w < 8 || h < 8 || pixels.len() < w * h {
        return None;
    }
    let gray = Gray { pixels, width: w };
    // Bands across square art are part of its design; only a frame of
    // another shape holds art inside borders.
    let square = (real(w) / real(h)).ln().abs() <= SQUARE_TOLERANCE;
    let content = if square {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    } else {
        content_box(&gray, w, h)
    };
    let crop = crop(&gray, content);
    let (cw, ch) = (content.width as usize, content.height as usize);
    let effective = u32::try_from(effective_side(&crop, cw, ch)).unwrap_or(u32::MAX);
    Some(ImageQuality {
        width,
        height,
        content,
        effective,
        blockiness: blockiness(&gray, content),
    })
}

/// The picture inside uniform bars that match on opposite sides, as a
/// pillarboxed video frame holds its square art. A solid edge on one
/// side only is part of the picture.
fn content_box(gray: &Gray<'_>, w: usize, h: usize) -> Rect {
    let col = |x: usize| gray.line_stats((0..h).map(move |y| (x, y)));
    let row = |y: usize| gray.line_stats((0..w).map(move |x| (x, y)));
    let (left, right) = bars(w, col);
    let (top, bottom) = bars(h, row);
    let n = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    Rect {
        x: n(left),
        y: n(top),
        width: n(w - left - right),
        height: n(h - top - bottom),
    }
}

/// The widths of the bars at each end of a side `len` long, from each
/// line's mean and spread; zero both when they are no matched pair.
fn bars(len: usize, line: impl Fn(usize) -> (f64, f64)) -> (usize, usize) {
    let uniform =
        |l: (f64, f64), edge: f64| l.1 < BORDER_STDDEV && (l.0 - edge).abs() < BORDER_MATCH;
    let (first, last) = (line(0), line(len - 1));
    let mut start = 0;
    while start < len / 2 && uniform(line(start), first.0) {
        start += 1;
    }
    let mut end = 0;
    while end < len / 2 && uniform(line(len - 1 - end), last.0) {
        end += 1;
    }
    let matched = start > 0
        && end > 0
        && (first.0 - last.0).abs() < BORDER_MATCH
        && real(start.abs_diff(end)) <= real(len) * BAR_SYMMETRY
        && real(len - start - end) >= real(len) * MIN_CONTENT;
    if matched { (start, end) } else { (0, 0) }
}

fn crop(gray: &Gray<'_>, r: Rect) -> Vec<f64> {
    let (x0, y0) = (r.x as usize, r.y as usize);
    let mut out = Vec::with_capacity(r.width as usize * r.height as usize);
    for y in y0..y0 + r.height as usize {
        for x in x0..x0 + r.width as usize {
            out.push(gray.at(x, y));
        }
    }
    out
}

/// The part `area` of the gray picture `pixels`, `width` wide, averaged
/// down to exactly `nw` × `nh`.
pub(crate) fn downscale(pixels: &[u8], width: u32, area: Rect, nw: usize, nh: usize) -> Vec<f64> {
    let gray = Gray {
        pixels,
        width: width as usize,
    };
    let (w, h) = (area.width as usize, area.height as usize);
    let img = crop(&gray, area);
    columns(&rows(&img, w, &box_weights(w, nw)), nw, &box_weights(h, nh))
}

/// For each of `n` outputs over `len` inputs, the inputs it averages and
/// how much of each it covers.
fn box_weights(len: usize, n: usize) -> Vec<Vec<(usize, f64)>> {
    let step = real(len) / real(n);
    (0..n)
        .map(|i| {
            let (lo, hi) = (real(i) * step, real(i + 1) * step);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (first, last) = (lo.floor() as usize, (hi.ceil() as usize).min(len));
            (first..last)
                .map(|j| (j, ((real(j + 1)).min(hi) - real(j).max(lo)) / step))
                .collect()
        })
        .collect()
}

/// `img`, `w` wide, resampled along its rows by `weights`.
fn rows<T: AsRef<[(usize, f64)]>>(img: &[f64], w: usize, weights: &[T]) -> Vec<f64> {
    img.chunks_exact(w)
        .flat_map(|row| {
            weights
                .iter()
                .map(|ws| ws.as_ref().iter().map(|(j, k)| row[*j] * k).sum::<f64>())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `img`, `w` wide, resampled along its columns by `weights`.
fn columns<T: AsRef<[(usize, f64)]>>(img: &[f64], w: usize, weights: &[T]) -> Vec<f64> {
    let mut out = Vec::with_capacity(weights.len() * w);
    for ws in weights {
        let mut row = vec![0.0; w];
        for (j, k) in ws.as_ref() {
            for (o, v) in row.iter_mut().zip(&img[j * w..(j + 1) * w]) {
                *o += v * k;
            }
        }
        out.extend(row);
    }
    out
}

/// `img` averaged down to `short` pixels across its shorter side.
fn shrink(img: &[f64], w: usize, h: usize, short: usize) -> (Vec<f64>, usize, usize) {
    let scale = real(short) / real(w.min(h));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (tw, th) = (
        ((real(w) * scale).round() as usize).max(3),
        ((real(h) * scale).round() as usize).max(3),
    );
    let small = columns(&rows(img, w, &box_weights(w, tw)), tw, &box_weights(h, th));
    (small, tw, th)
}

/// The detail `img` holds at its own finest scale against its contrast:
/// mean squared Laplacian over variance.
fn detail(img: &[f64], w: usize, h: usize) -> f64 {
    let n = real(img.len());
    let mean = img.iter().sum::<f64>() / n;
    let variance = img.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    if variance < 1e-9 {
        return 0.0;
    }
    let mut sum = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let at = |dx: usize, dy: usize| img[(y + dy - 1) * w + x + dx - 1];
            let l = at(0, 1) + at(2, 1) + at(1, 0) + at(1, 2) - 4.0 * at(1, 1);
            sum += l * l;
        }
    }
    sum / real((w - 2) * (h - 2)) / variance
}

/// The detail `img` holds at `short` pixels across.
fn detail_at(img: &[f64], w: usize, h: usize, short: usize) -> f64 {
    if short >= w.min(h) {
        return detail(img, w, h);
    }
    let (small, tw, th) = shrink(img, w, h, short);
    detail(&small, tw, th)
}

/// The shorter side of the resolution `img`'s detail supports: the
/// largest size, in steps of half an octave up from twice
/// [`REFERENCE_SIDE`], below which every size keeps [`OCTAVE_FLOOR`] of
/// the detail half of it does. Each size is compared with its own half,
/// so both are averaged down alike. Zero for a picture with no detail.
fn effective_side(img: &[f64], w: usize, h: usize) -> usize {
    let short = w.min(h);
    if detail(img, w, h) == 0.0 {
        return 0;
    }
    if short < 2 * REFERENCE_SIDE {
        return short;
    }
    let mut sizes = Vec::new();
    let mut size = real(2 * REFERENCE_SIDE);
    while size < real(short) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        sizes.push(size.round() as usize);
        size *= std::f64::consts::SQRT_2;
    }
    sizes.push(short);
    let mut effective = REFERENCE_SIDE;
    for size in sizes {
        let half = detail_at(img, w, h, size / 2);
        if half <= 0.0 || detail_at(img, w, h, size) / half < OCTAVE_FLOOR {
            return effective;
        }
        effective = size;
    }
    short
}

/// How much more the pixels jump across JPEG's 8×8 block boundaries
/// than halfway between them, measured inside the content in the
/// image's own block grid. Halfway shares the boundary's parity, so the
/// pattern a 2× upscale leaves cancels out.
fn blockiness(gray: &Gray<'_>, r: Rect) -> f64 {
    let (x0, y0) = (r.x as usize, r.y as usize);
    let (x1, y1) = (x0 + r.width as usize, y0 + r.height as usize);
    let (mut edge, mut edges, mut mid, mut mids) = (0.0, 0_usize, 0.0, 0_usize);
    let mut add = |at: usize, jump: f64| match at % 8 {
        0 => {
            edge += jump;
            edges += 1;
        }
        4 => {
            mid += jump;
            mids += 1;
        }
        _ => {}
    };
    for y in y0..y1 {
        for x in x0 + 1..x1 {
            add(x, (gray.at(x, y) - gray.at(x - 1, y)).abs());
        }
    }
    for y in y0 + 1..y1 {
        for x in x0..x1 {
            add(y, (gray.at(x, y) - gray.at(x, y - 1)).abs());
        }
    }
    if edges == 0 || mids == 0 || mid == 0.0 {
        return 0.0;
    }
    ((edge / real(edges)) / (mid / real(mids)) - 1.0).max(0.0)
}

/// Reads a measured segment file of `channels`, empty when it was not
/// written.
#[must_use]
pub fn read_segment(path: &Path, channels: usize) -> Segment {
    std::fs::read(path)
        .map(|b| Segment::of_bytes(&b, channels))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUTOFFS: &[f64] = &[1e-3, 1e-2];

    /// Deterministic white noise in [-1, 1).
    fn noise(n: usize, seed: u64) -> Vec<f64> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..n)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                f64::from(u32::try_from(state >> 33).unwrap()) / f64::from(1_u32 << 31) - 1.0
            })
            .collect()
    }

    /// Noise lowpassed at `cutoff` by zeroing its spectrum, frame by frame.
    fn lowpassed(n: usize, cutoff: f64, rate: f64) -> Vec<f64> {
        shaped(n, rate, |f| if f > cutoff { 0.0 } else { 1.0 })
    }

    /// Noise with each frequency scaled by `gain`, frame by frame.
    fn shaped(n: usize, rate: f64, gain: impl Fn(f64) -> f64) -> Vec<f64> {
        filtered(&noise(n, 3), rate, gain)
            .into_iter()
            .map(|v| v * 0.3)
            .collect()
    }

    /// `signal` with each frequency scaled by `gain`, frame by frame.
    fn filtered(signal: &[f64], rate: f64, gain: impl Fn(f64) -> f64) -> Vec<f64> {
        let mut out = Vec::new();
        for (i, chunk) in signal.chunks(FFT).enumerate() {
            let mut re = chunk.to_vec();
            re.resize(FFT, 0.0);
            let mut im = vec![0.0; FFT];
            fft(&mut re, &mut im);
            for k in 0..FFT {
                let g = gain(real(k.min(FFT - k)) * rate / real(FFT));
                re[k] *= g;
                im[k] *= g;
            }
            // An inverse FFT is the forward one on the conjugate.
            for v in &mut im {
                *v = -*v;
            }
            fft(&mut re, &mut im);
            out.extend(re.iter().map(|v| v / real(FFT)));
            let _ = i;
        }
        out.truncate(signal.len());
        out
    }

    #[allow(clippy::cast_possible_truncation)]
    fn stereo(left: &[f64], right: &[f64]) -> Segment {
        let samples = left
            .iter()
            .zip(right)
            .flat_map(|(l, r)| [*l as f32, *r as f32])
            .collect();
        Segment::new(2, samples)
    }

    #[test]
    fn the_bandwidth_is_the_lowpass_whatever_the_rate() {
        let rate = 48_000.0;
        let low = lowpassed(FFT * 40, 16_000.0, rate);
        let other = lowpassed(FFT * 40, 16_000.0, rate);
        let q = audio(&[stereo(&low, &other)], 48_000, false).unwrap();
        assert!((q.bandwidth_hz - 16_000.0).abs() <= 500.0, "{q:?}");
        let full = noise(FFT * 40, 9);
        let q = audio(&[stereo(&full, &noise(FFT * 40, 10))], 48_000, false).unwrap();
        assert!(q.bandwidth_hz > 23_000.0, "{q:?}");
    }

    #[test]
    fn treble_fading_into_the_noise_floor_is_no_lowpass() {
        let rate = 48_000.0;
        // 6 dB per kHz above 2 kHz, down to a floor 80 dB below.
        let fade = |f: f64| 10_f64.powf(-((f - 2000.0).max(0.0) * 0.0003).min(4.0));
        let natural = shaped(FFT * 40, rate, fade);
        let q = audio(&[stereo(&natural, &natural)], 48_000, false).unwrap();
        assert!(q.bandwidth_hz >= 24_000.0, "{q:?}");
        let encoded = shaped(FFT * 40, rate, |f| if f > 16_000.0 { 0.0 } else { fade(f) });
        let q = audio(&[stereo(&encoded, &encoded)], 48_000, false).unwrap();
        assert!((q.bandwidth_hz - 16_000.0).abs() <= 500.0, "{q:?}");
    }

    #[test]
    fn mono_in_two_channels_is_not_stereo() {
        let n = noise(FFT * 4, 1);
        assert!(
            !audio(&[stereo(&n, &n)], 48_000, false)
                .unwrap()
                .is_stereo(STEREO_INCOHERENCE)
        );
        let quieter: Vec<f64> = n.iter().map(|v| v * 0.89).collect();
        assert!(
            !audio(&[stereo(&n, &quieter)], 48_000, false)
                .unwrap()
                .is_stereo(STEREO_INCOHERENCE)
        );
        let late: Vec<f64> = std::iter::repeat_n(0.0, 3)
            .chain(n.iter().copied())
            .collect();
        assert!(
            !audio(&[stereo(&n, &late)], 48_000, false)
                .unwrap()
                .is_stereo(STEREO_INCOHERENCE)
        );
        let far: Vec<f64> = std::iter::repeat_n(0.0, 200)
            .chain(n.iter().copied())
            .collect();
        assert!(
            audio(&[stereo(&n, &far)], 48_000, false)
                .unwrap()
                .is_stereo(STEREO_INCOHERENCE)
        );
        assert!(
            audio(&[stereo(&n, &noise(FFT * 4, 2))], 48_000, false)
                .unwrap()
                .is_stereo(STEREO_INCOHERENCE)
        );
    }

    #[test]
    fn a_clipped_signal_is_flagged() {
        let sine: Vec<f64> = (0..FFT * 4)
            .map(|i| (1.6 * (real(i) * 0.05).sin()).clamp(-1.0, 1.0))
            .collect();
        let q = audio(&[stereo(&sine, &sine)], 48_000, false).unwrap();
        assert!(q.clipping > 0.1, "{q:?}");
        assert_eq!(q.clipping_bucket(CUTOFFS), 2);
        let turned_down: Vec<f64> = sine.iter().map(|v| v * 0.89).collect();
        let q = audio(&[stereo(&turned_down, &turned_down)], 48_000, false).unwrap();
        assert_eq!(q.clipping_bucket(CUTOFFS), 2, "{q:?}");
        let clean: Vec<f64> = (0..FFT * 4).map(|i| 0.8 * (real(i) * 0.05).sin()).collect();
        assert_eq!(
            audio(&[stereo(&clean, &clean)], 48_000, false)
                .unwrap()
                .clipping_bucket(CUTOFFS),
            0
        );
    }

    #[test]
    fn clipping_smeared_by_a_codec_is_flagged() {
        let rate = 48_000.0;
        let music = shaped(FFT * 40, rate, |f| if f > 8000.0 { 0.0 } else { 1.0 });
        let peak = music.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        // Clipped at full scale with 4 dB to spare, then lowpassed as an
        // encoder would, which rounds the flat tops and rings past them.
        let clipped: Vec<f64> = music
            .iter()
            .map(|v| (v / peak * 1.6).clamp(-1.0, 1.0))
            .collect();
        let smeared = filtered(&clipped, rate, |f| if f > 16_000.0 { 0.0 } else { 1.0 });
        let q = audio(&[stereo(&smeared, &smeared)], 48_000, false).unwrap();
        assert!(q.clipping_bucket(CUTOFFS) >= 1, "{q:?}");
        let hot: Vec<f64> = music.iter().map(|v| v / peak * 0.99).collect();
        let hot = filtered(&hot, rate, |f| if f > 16_000.0 { 0.0 } else { 1.0 });
        let q = audio(&[stereo(&hot, &hot)], 48_000, false).unwrap();
        assert_eq!(q.clipping_bucket(CUTOFFS), 0, "{q:?}");
    }

    #[test]
    fn a_full_scale_tone_is_no_pile_up() {
        let tone: Vec<f64> = (0..FFT * 8)
            .map(|i| 0.97 * (real(i) * 0.05).sin())
            .collect();
        let q = audio(&[stereo(&tone, &tone)], 48_000, false).unwrap();
        assert_eq!(q.clipping_bucket(CUTOFFS), 0, "{q:?}");
    }

    #[test]
    fn silence_measures_nothing() {
        assert!(audio(&[Segment::new(2, vec![0.0; FFT * 4])], 48_000, false).is_none());
    }

    #[test]
    fn a_channel_in_opposite_phase_still_sounds() {
        let n = lowpassed(FFT * 40, 15_000.0, 48_000.0);
        let inverted: Vec<f64> = n.iter().map(|x| -x).collect();
        let q = audio(&[stereo(&n, &inverted)], 48_000, false).unwrap();
        assert!(
            (q.bandwidth_hz - 15_000.0).abs() < 1000.0,
            "{}",
            q.bandwidth_hz
        );
    }

    #[allow(clippy::cast_possible_truncation)]
    fn channels(signals: &[Vec<f64>]) -> Segment {
        let frames = signals[0].len();
        let samples = (0..frames)
            .flat_map(|i| signals.iter().map(move |s| s[i] as f32))
            .collect();
        Segment::new(signals.len(), samples)
    }

    #[test]
    fn channels_are_measured_apart_not_mixed() {
        // Six channels of noise at half scale, which a mix into two would
        // push past full scale.
        let six: Vec<Vec<f64>> = (0..6)
            .map(|c| noise(FFT * 20, c).iter().map(|x| x * 0.5).collect())
            .collect();
        let q = audio(&[channels(&six)], 48_000, false).unwrap();
        assert!(q.clipping < 1e-5, "{}", q.clipping);
        assert!(q.is_stereo(STEREO_INCOHERENCE));
    }

    #[test]
    fn one_channel_is_mono() {
        let q = audio(&[channels(&[noise(FFT * 8, 3)])], 48_000, false).unwrap();
        assert!(!q.is_stereo(STEREO_INCOHERENCE));
    }

    #[test]
    fn float_past_full_scale_is_no_clipping() {
        // A sine peaking at 2, as a float master holds, and the same cut
        // at full scale.
        let over: Vec<f64> = (0..FFT * 20)
            .map(|i| 2.0 * (std::f64::consts::TAU * 441.0 * real(i) / 48_000.0).sin())
            .collect();
        let cut: Vec<f64> = over.iter().map(|x| x.clamp(-1.0, 1.0)).collect();
        let master = audio(&[stereo(&over, &over)], 48_000, true).unwrap();
        let clipped = audio(&[stereo(&cut, &cut)], 48_000, false).unwrap();
        assert!(master.clipping < 0.01, "{}", master.clipping);
        assert!(clipped.clipping > 0.5, "{}", clipped.clipping);
        let unheld = audio(&[stereo(&over, &over)], 48_000, false).unwrap();
        assert!(unheld.clipping > 0.5, "{}", unheld.clipping);
    }

    /// Doubles a picture's size as image tools do, each new pixel a
    /// mix of the nearest old ones a quarter and three quarters away.
    fn upscale2(img: &[f64], w: usize, h: usize) -> Vec<f64> {
        let at = |x: isize, y: isize| {
            let cx = x.clamp(0, w.cast_signed() - 1).cast_unsigned();
            let cy = y.clamp(0, h.cast_signed() - 1).cast_unsigned();
            img[cy * w + cx]
        };
        let mut out = Vec::with_capacity(4 * w * h);
        for y in 0..2 * h {
            for x in 0..2 * w {
                let (sx, sy) = ((x / 2).cast_signed(), (y / 2).cast_signed());
                let (nx, ny) = (
                    if x % 2 == 0 { sx - 1 } else { sx + 1 },
                    if y % 2 == 0 { sy - 1 } else { sy + 1 },
                );
                out.push(
                    0.5625 * at(sx, sy) + 0.1875 * (at(nx, sy) + at(sx, ny)) + 0.0625 * at(nx, ny),
                );
            }
        }
        out
    }

    /// A picture with detail at every scale, falling off as a photo's
    /// does: noise at each octave, smoothly enlarged and summed.
    fn picture(side: usize) -> Vec<f64> {
        let mut img = vec![0.0; side * side];
        for octave in 0_u32..6 {
            let coarse = side >> octave;
            let mut layer = noise(coarse * coarse, 5 + u64::from(octave));
            let mut width = coarse;
            while width < side {
                layer = upscale2(&layer, width, width);
                width *= 2;
            }
            let weight = f64::from(1_u32 << octave).powf(0.5);
            for (p, v) in img.iter_mut().zip(layer) {
                *p += weight * v;
            }
        }
        img
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn gray(img: &[f64]) -> Vec<u8> {
        let peak = img.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        img.iter()
            .map(|v| (127.5 + 127.0 * v / peak).round() as u8)
            .collect()
    }

    #[test]
    fn an_upscaled_picture_is_known_by_the_detail_it_lacks() {
        let native = image(&gray(&picture(512)), 512, 512).unwrap();
        assert_eq!(native.effective, 512, "{native:?}");
        let small = picture(192);
        let up = upscale2(&upscale2(&small, 192, 192), 384, 384);
        let up = image(&gray(&up), 768, 768).unwrap();
        assert!(up.effective < 362, "{up:?}");
        assert!(up.resolution_bucket(0.1) < native.resolution_bucket(0.1));
    }

    #[test]
    fn a_pillarboxed_frame_yields_its_inner_square() {
        let (w, h) = (320, 180);
        let art = gray(&picture(256));
        let mut frame = vec![0_u8; w * h];
        for y in 0..h {
            frame[y * w + 70..y * w + 250].copy_from_slice(&art[y * 256..y * 256 + 180]);
        }
        let q = image(&frame, 320, 180).unwrap();
        assert_eq!(
            q.content,
            Rect {
                x: 70,
                y: 0,
                width: 180,
                height: 180
            }
        );
        assert!(q.is_square(SQUARE_TOLERANCE) && q.is_cropped());
        let wide = gray(&picture(512));
        let full: Vec<u8> = (0..h)
            .flat_map(|y| wide[y * 512..y * 512 + w].to_vec())
            .collect();
        let full = image(&full, 320, 180).unwrap();
        assert!(!full.is_square(SQUARE_TOLERANCE) && !full.is_cropped());
    }

    #[test]
    fn square_art_keeps_its_bands() {
        let side = 256;
        let art = gray(&picture(side));
        let mut banded = vec![255_u8; side * side];
        banded[40 * side..216 * side].copy_from_slice(&art[40 * side..216 * side]);
        let q = image(&banded, 256, 256).unwrap();
        assert!(!q.is_cropped() && q.is_square(SQUARE_TOLERANCE), "{q:?}");
    }

    #[test]
    fn block_edges_raise_blockiness() {
        let (w, h) = (128, 128);
        let smooth = gray(&picture(128));
        let blocky: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let base = smooth[(y / 8 * 8) * w + x / 8 * 8];
                base.wrapping_add(u8::try_from((x % 8 + y % 8) / 4).unwrap())
            })
            .collect();
        let a = image(&smooth, 128, 128).unwrap();
        let b = image(&blocky, 128, 128).unwrap();
        assert!(b.blockiness > a.blockiness + 0.5, "{a:?} {b:?}");
    }

    #[test]
    fn a_flat_picture_has_no_effective_resolution() {
        let q = image(&vec![90_u8; 64 * 64], 64, 64).unwrap();
        assert_eq!(q.effective, 0);
    }

    #[test]
    fn a_short_buffer_is_no_measure() {
        assert!(image(&[0; 10], 64, 64).is_none());
    }

    #[test]
    fn a_padded_recording_uses_the_bits_it_was_made_with() {
        let at = |bits: u32| -> Vec<f64> {
            let step = f64::from(1_u32 << (bits - 1));
            noise(FFT * 4, 7)
                .iter()
                .map(|x| (x * 0.5 * step).round() / step)
                .collect()
        };
        let bits = |n: &[f64]| audio(&[stereo(n, n)], 48_000, false).unwrap().bits;
        assert_eq!(bits(&at(16)), 16);
        assert_eq!(bits(&at(24)), 24);
        assert_eq!(bits(&noise(FFT * 4, 7)), 24, "float uses them all");
    }
}
