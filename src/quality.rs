//! How good a source's audio and pictures are, measured from their
//! decoded samples, never from what a container or codec declares: an
//! upscaled image or a transcoded file claims more than it holds.
//!
//! Audio is measured on short excerpts at a quarter, half and three
//! quarters of its length, decoded at the source's own rate.
//!
//! - **Bandwidth.** Where a lowpass cuts the sound off: the top of the
//!   highest band that stands 20 dB above every band more than two bands
//!   past it, in each excerpt's mean spectrum, taken low across the
//!   excerpts. A lossy encoder leaves a wall at its lowpass, so a FLAC
//!   transcoded from a 16 kHz source measures 16 kHz whatever its bitrate.
//!   A recording's own treble fades gradually into its noise floor and
//!   leaves no wall, so it measures the whole band up to half its sample
//!   rate. Each excerpt is judged alone, so an encoder
//!   starved of bits in one passage still shows its wall.
//! - **Real stereo.** The share of the channels' energy that no single gain
//!   from one to the other explains, one minus their squared correlation.
//!   A mono recording copied into two channels, at the same level or not,
//!   has next to none, and counts as mono.
//! - **Clipping.** The share of samples in runs at the clipping level: full
//!   scale, or just under the audio's own peak when that is lower, so a
//!   master clipped and then turned down still counts.
//!
//! Calibrated on 22 public-domain recordings, orchestral, chamber and
//! piano, 16 lossless at 44.1 and 48 kHz and 6 transfers of 78 rpm discs
//! at 96 kHz, and on copies of 8 of them made for the purpose:
//!
//! - The measure walls replaced, each frame's top band within 80 dB of its
//!   loudest, put 6 of the lossless recordings at 10.5 to 12.1 kHz, where
//!   their treble faded into their noise floor, and ranked two of them
//!   below their own 48 kbit/s Opus copies. Walls put every recording at
//!   half its sample rate.
//! - Of 80 lossy or resampled copies, walls put 69 at least 1 kHz below
//!   their source; the old measure, 57. Off the darkest recordings, MP3 at
//!   128 kbit/s measured 16 to 16.5 kHz and AAC at 128 kbit/s 17.2 to
//!   17.4 kHz; Opus at 128 kbit/s measured 20 to 20.6 kHz on all. Opus at
//!   48 kbit/s on solo piano, starved, measured 8.4 and 12.6 kHz.
//! - A copy whose lost treble stood less than 20 dB above what was left in
//!   its place shows no wall. Two 44.1 kHz piano recordings resampled to
//!   22.05 kHz and back up to 48 kHz measured 24 kHz, above their sources.
//! - Stereo recordings, the 78 rpm transfers among them, left 0.21 to 0.89
//!   of their energy unexplained. Mono copied into channels 1 dB apart
//!   left under 2e-7, and at most 3.4e-3 through Opus; the side-to-mid
//!   ratio used before counted it as stereo. A channel delayed by one
//!   sample left up to 2.4e-2, and can count as stereo.
//! - Clipped masters turned down by 1 dB measured as clipped as before;
//!   at full scale alone they measured nothing. MP3 smears flat tops:
//!   clipped masters measuring 2.6e-3 to 7.9e-2 measured 2e-5 to 6.2e-3
//!   as MP3. No lossless recording measured over 1e-6, and no master
//!   limited below full scale measured any.
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

/// Names the measures and their constants; facts measured by another
/// are measured again, so changing anything below means changing this.
pub const METHOD: &str = "quality/4";

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
const WALL_DB: f64 = 20.0;
/// Walls are looked for above this frequency. Below it a piano's spectrum
/// can fall nearly as steeply as a lowpass: with a 15 dB wall, soft piano
/// showed walls at 1 to 1.3 kHz.
const WALL_MIN_HZ: f64 = 2000.0;
/// Frames quieter than this RMS, about -60 dBFS, say nothing of bandwidth.
const SILENT_RMS: f64 = 1e-3;
/// Energy no gain between the channels explains, below this share, is a
/// mono source in two channels.
const STEREO_INCOHERENCE: f64 = 1e-2;
const CLIP_LEVEL: f32 = 0.9999;
/// Samples this close to the audio's own peak are at a lowered clip level.
const CLIP_OF_PEAK: f32 = 0.999;
const CLIP_RUN: usize = 3;

/// A row or column at most this far from uniform, in gray levels, is a
/// border when it matches the edge.
const BORDER_STDDEV: f64 = 6.0;
const BORDER_MATCH: f64 = 8.0;
/// Trimming that leaves less than this share of a side is not a border.
const MIN_CONTENT: f64 = 0.25;
const SQUARE_TOLERANCE: f64 = 0.03;
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
}

impl AudioQuality {
    #[must_use]
    pub fn is_stereo(&self) -> bool {
        self.incoherence > STEREO_INCOHERENCE
    }

    /// Wider bandwidth first, in 500 Hz steps so noise never decides.
    #[must_use]
    pub fn bandwidth_bucket(&self) -> i64 {
        bucket(self.bandwidth_hz / 500.0)
    }

    /// Fewer clipped samples first, by order of magnitude from the
    /// inaudible.
    #[must_use]
    pub fn clipping_bucket(&self) -> i64 {
        match self.clipping {
            c if c < 1e-3 => 0,
            c if c < 1e-2 => 1,
            _ => 2,
        }
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
    pub fn is_square(&self) -> bool {
        let (w, h) = (
            f64::from(self.content.width),
            f64::from(self.content.height),
        );
        h > 0.0 && (w / h).ln().abs() <= SQUARE_TOLERANCE
    }

    #[must_use]
    pub fn is_cropped(&self) -> bool {
        self.content.width != self.width || self.content.height != self.height
    }

    /// Higher effective resolution first, in steps of about 10%.
    #[must_use]
    pub fn resolution_bucket(&self) -> i64 {
        if self.effective == 0 {
            return i64::MIN;
        }
        bucket(f64::from(self.effective).ln() / 1.1_f64.ln())
    }

    /// Fewer block artifacts first, in tenths.
    #[must_use]
    pub fn blockiness_bucket(&self) -> i64 {
        bucket(self.blockiness / 0.1).min(10)
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

/// The ffmpeg output options that write one measured segment of the
/// audio stream `input:index` as 32-bit float stereo at its own rate.
#[must_use]
pub fn segment_output(input: usize, index: u32, start: f64) -> Vec<OsString> {
    [
        "-map".to_string(),
        format!("{input}:{index}"),
        "-ss".to_string(),
        format!("{start:.3}"),
        "-t".to_string(),
        format!("{SEGMENT_SECONDS}"),
        "-ac".to_string(),
        "2".to_string(),
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

/// Interleaved little-endian f32 stereo samples.
#[must_use]
pub fn stereo_samples(bytes: &[u8]) -> Vec<[f32; 2]> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| {
            [
                f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
            ]
        })
        .collect()
}

/// Measure decoded segments of one recording; `None` when they hold no
/// sound at all.
#[must_use]
pub fn audio(segments: &[Vec<[f32; 2]>], sample_rate: u32) -> Option<AudioQuality> {
    let window: Vec<f64> = (0..FFT)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * real(i) / real(FFT)).cos())
        .collect();
    let peak = segments
        .iter()
        .flatten()
        .fold(0.0_f32, |m, s| m.max(s[0].abs()).max(s[1].abs()));
    let level = CLIP_LEVEL.min(peak * CLIP_OF_PEAK);
    let mut bandwidth: Option<f64> = None;
    let (mut ll, mut rr, mut lr) = (0.0, 0.0, 0.0);
    let (mut clipped, mut total) = (0_usize, 0_usize);
    for samples in segments {
        for s in samples {
            let (l, r) = (f64::from(s[0]), f64::from(s[1]));
            ll += l * l;
            rr += r * r;
            lr += l * r;
        }
        for channel in 0..2 {
            clipped += clipped_samples(samples.iter().map(|s| s[channel]), level);
        }
        total += samples.len() * 2;
        if let Some(spectrum) = mean_spectrum(samples, &window) {
            let hz = wall(&spectrum, sample_rate);
            bandwidth = Some(bandwidth.map_or(hz, |b| b.min(hz)));
        }
    }
    let bandwidth_hz = bandwidth?;
    if ll + rr == 0.0 {
        return None;
    }
    // A silent channel beside a sounding one is no mono copy.
    let incoherence = if ll == 0.0 || rr == 0.0 {
        1.0
    } else {
        (1.0 - lr * lr / (ll * rr)).max(0.0)
    };
    Some(AudioQuality {
        bandwidth_hz,
        incoherence,
        clipping: real(clipped) / real(total.max(1)),
    })
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

/// The mean power spectrum of a segment's sounding frames, by FFT bin;
/// `None` when every frame is silent.
fn mean_spectrum(samples: &[[f32; 2]], window: &[f64]) -> Option<Vec<f64>> {
    let mut sum = vec![0.0; FFT / 2];
    let mut frames = 0_usize;
    for frame in samples.as_chunks::<FFT>().0 {
        let mono: Vec<f64> = frame
            .iter()
            .map(|s| f64::midpoint(f64::from(s[0]), f64::from(s[1])))
            .collect();
        let rms = (mono.iter().map(|x| x * x).sum::<f64>() / real(FFT)).sqrt();
        if rms < SILENT_RMS {
            continue;
        }
        let mut re: Vec<f64> = mono.iter().zip(window).map(|(x, w)| x * w).collect();
        let mut im = vec![0.0; FFT];
        fft(&mut re, &mut im);
        for (k, power) in sum.iter_mut().enumerate() {
            *power += re[k] * re[k] + im[k] * im[k];
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

/// Reads a measured segment file, empty when it was not written.
#[must_use]
pub fn read_segment(path: &Path) -> Vec<[f32; 2]> {
    std::fs::read(path)
        .map(|b| stereo_samples(&b))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut out = Vec::new();
        for (i, chunk) in noise(n, 3).chunks(FFT).enumerate() {
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
            out.extend(re.iter().map(|v| v / real(FFT) * 0.3));
            let _ = i;
        }
        out.truncate(n);
        out
    }

    #[allow(clippy::cast_possible_truncation)]
    fn stereo(left: &[f64], right: &[f64]) -> Vec<[f32; 2]> {
        left.iter()
            .zip(right)
            .map(|(l, r)| [*l as f32, *r as f32])
            .collect()
    }

    #[test]
    fn the_bandwidth_is_the_lowpass_whatever_the_rate() {
        let rate = 48_000.0;
        let low = lowpassed(FFT * 40, 16_000.0, rate);
        let other = lowpassed(FFT * 40, 16_000.0, rate);
        let q = audio(&[stereo(&low, &other)], 48_000).unwrap();
        assert!((q.bandwidth_hz - 16_000.0).abs() <= 500.0, "{q:?}");
        let full = noise(FFT * 40, 9);
        let q = audio(&[stereo(&full, &noise(FFT * 40, 10))], 48_000).unwrap();
        assert!(q.bandwidth_hz > 23_000.0, "{q:?}");
    }

    #[test]
    fn treble_fading_into_the_noise_floor_is_no_lowpass() {
        let rate = 48_000.0;
        // 6 dB per kHz above 2 kHz, down to a floor 80 dB below.
        let fade = |f: f64| 10_f64.powf(-((f - 2000.0).max(0.0) * 0.0003).min(4.0));
        let natural = shaped(FFT * 40, rate, fade);
        let q = audio(&[stereo(&natural, &natural)], 48_000).unwrap();
        assert!(q.bandwidth_hz >= 24_000.0, "{q:?}");
        let encoded = shaped(FFT * 40, rate, |f| if f > 16_000.0 { 0.0 } else { fade(f) });
        let q = audio(&[stereo(&encoded, &encoded)], 48_000).unwrap();
        assert!((q.bandwidth_hz - 16_000.0).abs() <= 500.0, "{q:?}");
    }

    #[test]
    fn mono_in_two_channels_is_not_stereo() {
        let n = noise(FFT * 4, 1);
        assert!(!audio(&[stereo(&n, &n)], 48_000).unwrap().is_stereo());
        let quieter: Vec<f64> = n.iter().map(|v| v * 0.89).collect();
        assert!(!audio(&[stereo(&n, &quieter)], 48_000).unwrap().is_stereo());
        assert!(
            audio(&[stereo(&n, &noise(FFT * 4, 2))], 48_000)
                .unwrap()
                .is_stereo()
        );
    }

    #[test]
    fn a_clipped_signal_is_flagged() {
        let sine: Vec<f64> = (0..FFT * 4)
            .map(|i| (1.6 * (real(i) * 0.05).sin()).clamp(-1.0, 1.0))
            .collect();
        let q = audio(&[stereo(&sine, &sine)], 48_000).unwrap();
        assert!(q.clipping > 0.1, "{q:?}");
        assert_eq!(q.clipping_bucket(), 2);
        let turned_down: Vec<f64> = sine.iter().map(|v| v * 0.89).collect();
        let q = audio(&[stereo(&turned_down, &turned_down)], 48_000).unwrap();
        assert_eq!(q.clipping_bucket(), 2, "{q:?}");
        let clean: Vec<f64> = (0..FFT * 4).map(|i| 0.8 * (real(i) * 0.05).sin()).collect();
        assert_eq!(
            audio(&[stereo(&clean, &clean)], 48_000)
                .unwrap()
                .clipping_bucket(),
            0
        );
    }

    #[test]
    fn silence_measures_nothing() {
        assert!(audio(&[vec![[0.0, 0.0]; FFT * 2]], 48_000).is_none());
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
        assert!(up.resolution_bucket() < native.resolution_bucket());
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
        assert!(q.is_square() && q.is_cropped());
        let wide = gray(&picture(512));
        let full: Vec<u8> = (0..h)
            .flat_map(|y| wide[y * 512..y * 512 + w].to_vec())
            .collect();
        let full = image(&full, 320, 180).unwrap();
        assert!(!full.is_square() && !full.is_cropped());
    }

    #[test]
    fn square_art_keeps_its_bands() {
        let side = 256;
        let art = gray(&picture(side));
        let mut banded = vec![255_u8; side * side];
        banded[40 * side..216 * side].copy_from_slice(&art[40 * side..216 * side]);
        let q = image(&banded, 256, 256).unwrap();
        assert!(!q.is_cropped() && q.is_square(), "{q:?}");
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
}
