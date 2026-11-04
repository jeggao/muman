//! How loud a song is, as EBU R128 and ITU-R BS.1770 measure it, and the
//! gain that levels it to `[loudness] target`.
//!
//! **Measuring.** [`Meter`] feeds a source's decoded audio to the
//! `ebur128` crate, a port of libebur128, 100 ms at a time, as
//! libebur128 rounds 100 ms: `(rate + 5) / 10` frames. After each hop
//! from the fourth on, the loudness of the last 400 ms is one gating
//! block; after the thirtieth, and every tenth after it, the loudness of
//! the last 3 s is one short-term block, on the grid libebur128 takes
//! them for the loudness range. Blocks at or above −70 LUFS are counted
//! in 0.1 LU bins, as libebur128's histogram mode bins them, so a source
//! is kept in about a kilobyte, and an album's loudness is that of its
//! songs' bins summed: BS.1770's gating over every block of the album,
//! not a mean of its songs' loudness. A one-song album is its song.
//! Peaks are the highest over the channels. True peaks are oversampled
//! by `TruePeak`, the crate's 48-tap polyphase filter to the bit: four
//! times below 96 kHz, twice below 192 kHz, and at 192 kHz and above the
//! sample peak stands for it, as in the crate. The crate filters every
//! sample, and on a decode it cost more than the loudness itself; the
//! filter's output can be at most the loudest sample in its window times
//! the sum of a phase's absolute coefficients, so a run of frames that
//! loud or quieter than the peak so far is skipped, and music is nearly
//! always that far under its peak.
//!
//! Channels are weighed by the speakers the decoder names: the LFE not
//! at all, side, back and rear-top channels by 1.41 as BS.1770 weighs
//! surrounds, every other by 1. A single channel plays from both
//! speakers, so it counts twice, as libebur128's dual mono.
//!
//! **Gating** ([`integrated`]): blocks under −70 LUFS are already left
//! out; of the rest, those under the relative gate, 10 LU below their
//! mean energy, are too, and the loudness is that of the mean energy
//! left. Energy is a bin's centre's, as libebur128's histogram mode
//! takes it.
//!
//! **Gains.** A song's gain is the target less its loudness, held in
//! hundredths of a dB from the moment it is worked out, so a plan
//! compares exactly and a gain rounds the same on every machine. Peaks
//! are held in millionths of full scale. Tags always describe the file
//! as written: where a gain is applied to it, by the Opus header or to
//! the samples, the tags carry what is left, and its peaks as lifted.
//! A raising gain applied is lowered so the peak it lifts stays under
//! `[loudness] ceiling`; a lowering gain lowers every peak and never
//! is. Tags are never lowered: players keep peaks under full scale from
//! the peak tags themselves.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ebur128::{Channel, EbuR128, Mode};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::analysis::{Analysis, Analyzer, Measured, Stream};
use crate::tags::Field;
use crate::units::serde_as;

/// How loud each song is made, from its loudness measured as EBU R128
/// measures it; the module docs hold the design.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Loudness {
    pub mode: LoudnessMode,
    /// The loudness every song is levelled to, in hundredths of a LUFS.
    #[serde(deserialize_with = "serde_as::lufs")]
    pub target: i32,
    /// Peaks between samples, as a DAC rebuilds them, rather than the
    /// samples' own.
    pub true_peak: bool,
    /// The gain `header` and `audio` apply.
    pub scope: GainScope,
    /// The highest peak a raising gain applied may lift a song to, in
    /// hundredths of a dB against full scale.
    #[serde(deserialize_with = "serde_as::db")]
    pub ceiling: i32,
}

impl Default for Loudness {
    fn default() -> Self {
        Self {
            mode: LoudnessMode::Tags,
            target: -1800,
            true_peak: true,
            scope: GainScope::Album,
            ceiling: -100,
        }
    }
}

impl Loudness {
    /// Whether songs are measured at all.
    #[must_use]
    pub fn measured(&self) -> bool {
        self.mode != LoudnessMode::Off
    }

    pub(crate) fn check(&self) -> anyhow::Result<()> {
        if !(-7000..=0).contains(&self.target) {
            anyhow::bail!("[loudness] target must lie between -70 LUFS and 0 LUFS");
        }
        if self.ceiling > 0 {
            anyhow::bail!("[loudness] ceiling must be 0 dB or less, at most full scale");
        }
        Ok(())
    }
}

/// Where a song's gain is put.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoudnessMode {
    /// Nothing measured: the gains a source's own tags carry are kept.
    Off,
    /// ReplayGain tags players apply, R128 gains in Opus.
    #[default]
    Tags,
    /// Tags, and in Opus the gain in the header every decoder applies.
    Header,
    /// The samples scaled, encoding audio that would be copied.
    Audio,
}

/// Which of a song's gains is applied to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GainScope {
    /// Its album's, so the album keeps its songs' levels; a song on no
    /// album takes its own.
    #[default]
    Album,
    /// Its own.
    Track,
}

/// Names the meter and its constants; a measure by another is measured
/// again, so changing anything below means changing this.
pub const METHOD: &str = "loudness/1";

/// Blocks quieter than this are silence to BS.1770's gating.
const ABSOLUTE_GATE: f64 = -70.0;

/// How far under the mean of the blocks the relative gate lies, in LU.
const RELATIVE_GATE: f64 = -10.0;

/// Bins of 0.1 LU from the absolute gate up to +30 LUFS, as libebur128's.
const BINS: u16 = 1000;

/// Gating blocks after the first: 400 ms is four hops.
const MOMENTARY_HOPS: u64 = 4;

/// Short-term blocks span 3 s and follow each other every 1 s.
const SHORT_TERM_HOPS: u64 = 30;
const SHORT_TERM_STEP: u64 = 10;

/// Block loudness counted in 0.1 LU bins at and above −70 LUFS.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Histogram(BTreeMap<u16, u32>);

impl Histogram {
    fn add(&mut self, lufs: f64) {
        if lufs.is_nan() || lufs < ABSOLUTE_GATE {
            return;
        }
        let bin = (((lufs - ABSOLUTE_GATE) * 10.0).floor() as u16).min(BINS - 1);
        *self.0.entry(bin).or_default() += 1;
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The blocks counted.
    #[must_use]
    pub fn blocks(&self) -> u64 {
        self.0.values().map(|&c| u64::from(c)).sum()
    }

    /// `other`'s blocks counted in this too.
    pub fn pool(&mut self, other: &Self) {
        for (&bin, &count) in &other.0 {
            *self.0.entry(bin).or_default() += count;
        }
    }

    /// LEB128 varints: each bin as its step from the last, then its count.
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut last = 0;
        for (&bin, &count) in &self.0 {
            varint(&mut bytes, u32::from(bin - last));
            varint(&mut bytes, count);
            last = bin;
        }
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut at = 0;
        let mut bins = BTreeMap::new();
        let mut last: u32 = 0;
        while at < bytes.len() {
            last = last.checked_add(read_varint(bytes, &mut at)?)?;
            let count = read_varint(bytes, &mut at)?;
            let bin = u16::try_from(last).ok().filter(|&b| b < BINS)?;
            bins.insert(bin, count);
        }
        Some(Self(bins))
    }
}

fn varint(out: &mut Vec<u8>, mut n: u32) {
    while n >= 0x80 {
        out.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn read_varint(bytes: &[u8], at: &mut usize) -> Option<u32> {
    let mut n: u32 = 0;
    for shift in (0..35).step_by(7) {
        let b = *bytes.get(*at)?;
        *at += 1;
        n |= u32::from(b & 0x7f).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

impl Serialize for Histogram {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(self.to_bytes()))
    }
}

impl<'de> Deserialize<'de> for Histogram {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD
            .decode(text)
            .ok()
            .and_then(|b| Self::from_bytes(&b))
            .ok_or_else(|| serde::de::Error::custom("not a loudness histogram"))
    }
}

/// The energy at a bin's centre, as BS.1770 sums energies.
fn energy(bin: u16) -> f64 {
    let lufs = ABSOLUTE_GATE + (f64::from(bin) + 0.5) / 10.0;
    10f64.powf((lufs + 0.691) / 10.0)
}

fn lufs_of(energy: f64) -> f64 {
    10.0 * energy.log10() - 0.691
}

/// The integrated loudness of every block of `histograms` together, in
/// LUFS; `None` where no block reaches the absolute gate.
pub fn integrated<'a>(histograms: impl IntoIterator<Item = &'a Histogram>) -> Option<f64> {
    let mut pooled = Histogram::default();
    for h in histograms {
        pooled.pool(h);
    }
    let sum = |from: f64| {
        pooled
            .0
            .iter()
            .map(|(&bin, &count)| (energy(bin), f64::from(count)))
            .filter(|&(e, _)| e >= from)
            .fold((0.0, 0.0), |(e, n), (be, c)| (e + be * c, n + c))
    };
    let (all, blocks) = sum(0.0);
    if blocks == 0.0 {
        return None;
    }
    let gate = all / blocks * 10f64.powf(RELATIVE_GATE / 10.0);
    let (kept, kept_blocks) = sum(gate);
    (kept_blocks > 0.0).then(|| lufs_of(kept / kept_blocks))
}

/// A source's loudness as measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    /// 400 ms gating blocks.
    pub momentary: Histogram,
    /// 3 s blocks, for the loudness range and the loudest stretches.
    pub short_term: Histogram,
    /// The highest peak over every channel, oversampled, of full scale 1.
    pub true_peak: f64,
    pub sample_peak: f64,
    /// The gain the source's Opus header holds, which its decoding
    /// applied, in 1/256 dB.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub opus_gain: i16,
}

// serde's skip_serializing_if hands a reference.
#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &i16) -> bool {
    *n == 0
}

impl Measure {
    /// The integrated loudness, in LUFS.
    #[must_use]
    pub fn lufs(&self) -> Option<f64> {
        integrated([&self.momentary])
    }

    /// The peak `[loudness] true_peak` names.
    #[must_use]
    pub fn peak(&self, true_peak: bool) -> f64 {
        if true_peak {
            self.true_peak
        } else {
            self.sample_peak
        }
    }
}

/// The BS.1770 weighting of each channel, by the speakers in `mask`.
fn channel_map(stream: &Stream) -> Vec<Channel> {
    const LFE: u32 = 0x8 | 0x0080_0000;
    const SURROUND: u32 = 0x10 | 0x20 | 0x100 | 0x200 | 0x400 | 0x8000 | 0x1_0000 | 0x2_0000;
    if stream.channels == 1 {
        return vec![Channel::DualMono];
    }
    let Some(mask) = stream.mask else {
        return (0..stream.channels)
            .map(|i| match i {
                0 => Channel::Left,
                1 => Channel::Right,
                _ => Channel::Center,
            })
            .collect();
    };
    let mut speakers = (0..32).map(|b| 1u32 << b).filter(|bit| mask & bit != 0);
    (0..stream.channels)
        .map(|_| match speakers.next() {
            Some(bit) if bit & LFE != 0 => Channel::Unused,
            Some(bit) if bit & SURROUND != 0 => Channel::LeftSurround,
            Some(0x2) => Channel::Right,
            _ => Channel::Left,
        })
        .collect()
}

/// The taps of libebur128's true-peak filter, over every phase.
const TAPS: usize = 48;

/// Frames judged together for whether their samples could lift the peak.
const PEAK_RUN: usize = 64;

/// How much an f32 sum of a window's products may overshoot its bound.
const PEAK_MARGIN: f64 = 1.0 + 1e-4;

/// The true peak of each channel, as `ebur128` measures it in f32.
///
/// A phase's output for a frame is its coefficients times the window
/// ending at that frame, newest sample first, summed in that order from
/// zero; the coefficients are the crate's windowed sinc, cast to f32. Any
/// frame whose window could not lift the peak is skipped; the rest are
/// filtered exactly as the crate filters them, so the peak is the same.
struct TruePeak {
    /// Each phase's coefficients, by tap from the newest sample.
    phases: Vec<Vec<f32>>,
    /// The most any phase lifts the loudest sample of a window, in f64.
    bound: f64,
    /// Each channel's last `TAPS / factor - 1` samples, oldest first.
    history: Vec<Vec<f32>>,
    peaks: Vec<f32>,
    scratch: Vec<f32>,
}

impl TruePeak {
    fn new(rate: u32, channels: usize) -> Option<Self> {
        let factor = match rate {
            ..96_000 => 4,
            96_000..192_000 => 2,
            _ => return None,
        };
        let active = TAPS / factor;
        let mut phases = vec![vec![0f32; active]; factor];
        let window = TAPS as f64;
        for (j, phase) in (0..TAPS).zip((0..factor).cycle()) {
            let x = j as f64;
            let w = 0.5 * (1.0 - f64::cos(2.0 * std::f64::consts::PI * x / window));
            let m = x - window / 2.0;
            let sinc = m * std::f64::consts::PI / factor as f64;
            phases[phase][j / factor] = if m.abs() > 0.000_001 {
                w * sinc.sin() / sinc
            } else {
                w
            } as f32;
        }
        let bound = phases
            .iter()
            .map(|p| p.iter().map(|c| f64::from(c.abs())).sum::<f64>())
            .fold(0.0, f64::max)
            * PEAK_MARGIN;
        Some(Self {
            phases,
            bound,
            history: vec![vec![0.0; active - 1]; channels],
            peaks: vec![0.0; channels],
            scratch: Vec::new(),
        })
    }

    /// Whole frames, interleaved over `self.peaks.len()` channels.
    fn scan(&mut self, frames: &[f32]) {
        let channels = self.peaks.len();
        let held = self.history[0].len();
        for channel in 0..channels {
            self.scratch.clear();
            self.scratch.extend_from_slice(&self.history[channel]);
            self.scratch
                .extend(frames.chunks_exact(channels).map(|f| f[channel]));
            let mut peak = self.peaks[channel];
            let mut at = held;
            while at < self.scratch.len() {
                let end = (at + PEAK_RUN).min(self.scratch.len());
                let loudest = self.scratch[at - held..end]
                    .iter()
                    .fold(0f32, |m, x| m.max(x.abs()));
                if f64::from(loudest) * self.bound > f64::from(peak) {
                    for n in at..end {
                        let window = &self.scratch[n - held..=n];
                        for phase in &self.phases {
                            let mut out = 0f32;
                            for (tap, c) in phase.iter().enumerate() {
                                out += window[window.len() - 1 - tap] * c;
                            }
                            peak = peak.max(out.abs());
                        }
                    }
                }
                at = end;
            }
            self.peaks[channel] = peak;
            let kept = self.scratch.len() - held;
            self.history[channel].clear();
            self.history[channel].extend_from_slice(&self.scratch[kept..]);
        }
    }

    fn peak(&self, channel: usize) -> f64 {
        f64::from(self.peaks[channel])
    }
}

/// The loudness of one stream as it is decoded.
pub struct Meter {
    ebu: EbuR128,
    true_peak: Option<TruePeak>,
    channels: usize,
    hop: usize,
    hops: u64,
    pending: Vec<f32>,
    momentary: Histogram,
    short_term: Histogram,
}

impl std::fmt::Debug for Meter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meter")
            .field("hops", &self.hops)
            .finish_non_exhaustive()
    }
}

impl Meter {
    /// A meter for `stream`; none for a stream the crate cannot measure.
    #[must_use]
    pub fn new(stream: &Stream) -> Option<Self> {
        let mode = Mode::M | Mode::S | Mode::SAMPLE_PEAK;
        let mut ebu = EbuR128::new(u32::from(stream.channels), stream.rate, mode).ok()?;
        ebu.set_channel_map(&channel_map(stream)).ok()?;
        Some(Self {
            ebu,
            true_peak: TruePeak::new(stream.rate, usize::from(stream.channels)),
            channels: usize::from(stream.channels),
            hop: (stream.rate as usize + 5) / 10,
            hops: 0,
            pending: Vec::new(),
            momentary: Histogram::default(),
            short_term: Histogram::default(),
        })
    }
}

impl Analyzer for Meter {
    fn take(&mut self, frames: &[f32]) {
        if let Some(tp) = &mut self.true_peak {
            tp.scan(frames);
        }
        self.pending.extend_from_slice(frames);
        let hop = self.hop * self.channels;
        let mut at = 0;
        while self.pending.len() - at >= hop {
            if self
                .ebu
                .add_frames_f32(&self.pending[at..at + hop])
                .is_err()
            {
                return;
            }
            at += hop;
            self.hops += 1;
            if self.hops >= MOMENTARY_HOPS
                && let Ok(lufs) = self.ebu.loudness_momentary()
            {
                self.momentary.add(lufs);
            }
            if self.hops >= SHORT_TERM_HOPS
                && (self.hops - SHORT_TERM_HOPS).is_multiple_of(SHORT_TERM_STEP)
                && let Ok(lufs) = self.ebu.loudness_shortterm()
            {
                self.short_term.add(lufs);
            }
        }
        self.pending.drain(..at);
    }

    fn finish(mut self: Box<Self>, into: &mut Analysis) {
        // The last part of a hop makes no block, but its peaks count.
        let rest = std::mem::take(&mut self.pending);
        let _ = self.ebu.add_frames_f32(&rest);
        let highest = |peak: &dyn Fn(u32) -> Option<f64>| {
            (0..u32::try_from(self.channels).unwrap_or(0))
                .filter_map(peak)
                .fold(0.0, f64::max)
        };
        let sample_peak = highest(&|c| self.ebu.sample_peak(c).ok());
        // As the crate's `true_peak`: a channel's is its sample peak when that is higher.
        let true_peak = highest(&|c| {
            let sample = self.ebu.sample_peak(c).ok()?;
            let tp = self
                .true_peak
                .as_ref()
                .map_or(0.0, |tp| tp.peak(c as usize));
            Some(sample.max(tp))
        });
        let value = (!self.momentary.is_empty()).then(|| Measure {
            momentary: std::mem::take(&mut self.momentary),
            short_term: std::mem::take(&mut self.short_term),
            true_peak,
            sample_peak,
            opus_gain: 0,
        });
        into.loudness = Some(Measured {
            method: METHOD.to_string(),
            value,
        });
    }
}

// ---- Gains ----

/// A gain to the target, and the peak it would lift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level {
    /// Hundredths of a dB.
    pub gain: i32,
    /// Millionths of full scale, before the gain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak: Option<u32>,
}

impl Level {
    /// The level that takes a song measured `lufs`, peaking at `peak`, to
    /// `target`, in hundredths of a LUFS.
    #[must_use]
    pub fn to(target: i32, lufs: f64, peak: f64) -> Option<Self> {
        Some(Self {
            gain: hundredths(f64::from(target) / 100.0 - lufs)?,
            peak: millionths(peak),
        })
    }

    /// As written once `applied` hundredths of a dB are in the audio.
    fn written(self, applied: i32) -> (String, Option<String>) {
        let gain = gain_text(self.gain - applied);
        let peak = self.peak.map(|p| {
            let lifted = f64::from(p) / 1e6 * 10f64.powf(f64::from(applied) / 2000.0);
            peak_text(millionths(lifted).unwrap_or(u32::MAX))
        });
        (gain, peak)
    }
}

/// How the gain reaches the song, and how much of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Apply {
    /// By its tags alone.
    Tags,
    /// By the Opus header, in 1/256 dB.
    Header(i16),
    /// To its samples, in hundredths of a dB.
    Volume(i32),
}

/// What a song's loudness asks of how it is written, before any of it
/// is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gains {
    pub track: Level,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Level>,
    pub apply: Apply,
    /// The gain the Opus header of the audio copied holds already.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub opus_gain: i16,
    /// The bits an integer source's samples hold, which a lossless codec
    /// keeps through a gain applied to them; 0 for float or unknown.
    #[serde(default, skip_serializing_if = "is_nought")]
    pub bits: u32,
}

// serde's skip_serializing_if hands a reference.
#[expect(clippy::trivially_copy_pass_by_ref)]
fn is_nought(n: &u32) -> bool {
    *n == 0
}

impl Gains {
    /// The hundredths of a dB the file written in `opus` (whether it is
    /// Opus) and `encoded` holds of the gain.
    #[must_use]
    pub fn applied(&self, opus: bool, encoded: bool) -> i32 {
        match self.apply {
            Apply::Header(q) if opus => q78_to_hundredths(q),
            Apply::Volume(g) if encoded => g,
            _ => 0,
        }
    }

    /// The ReplayGain tags the file carries: what is left of each gain
    /// once `applied` is in it, and each peak as lifted.
    #[must_use]
    pub fn tags(&self, applied: i32) -> Vec<(Field, String)> {
        let mut out = Vec::new();
        for (level, gain, peak) in [
            (Some(self.track), Field::TrackGain, Field::TrackPeak),
            (self.album, Field::AlbumGain, Field::AlbumPeak),
        ] {
            if let Some(level) = level {
                let (g, p) = level.written(applied);
                out.push((gain, g));
                if let Some(p) = p {
                    out.push((peak, p));
                }
            }
        }
        out
    }

    /// These gains as a file in `opus` (whether it is Opus), written
    /// `encoded`, holds them: a header gain outside Opus, or a gain to the
    /// samples of audio copied, applies nothing, as tags alone do, and so
    /// does a gain of nothing, which leaves a copy's header as it was and
    /// the samples as decoded.
    #[must_use]
    pub fn as_held(self, opus: bool, encoded: bool) -> Self {
        let applies = match self.apply {
            Apply::Tags => true,
            Apply::Header(q) => opus && q != 0,
            Apply::Volume(g) => encoded && g != 0,
        };
        if applies {
            self
        } else {
            Self {
                apply: Apply::Tags,
                ..self
            }
        }
    }

    /// The gain the Opus header of a file in `opus`, written `copied`,
    /// holds where the gain is applied there: its own, a copy's being the
    /// source's, and the gain on top. None where the header is as muxed.
    #[must_use]
    pub fn header(&self, opus: bool, copied: bool) -> Option<i16> {
        match self.apply {
            Apply::Header(q) if opus => {
                let base = if copied { self.opus_gain } else { 0 };
                Some(base.saturating_add(q))
            }
            _ => None,
        }
    }
}

/// The part of `wanted` hundredths of a dB that keeps a peak of `peak`
/// millionths under `ceiling` hundredths of a dB: all of a lowering gain.
#[must_use]
pub fn capped(wanted: i32, peak: Option<u32>, ceiling: i32) -> i32 {
    let Some(peak) = peak.filter(|&p| p > 0 && wanted > 0) else {
        return wanted;
    };
    let room = f64::from(ceiling) / 100.0 - 20.0 * (f64::from(peak) / 1e6).log10();
    wanted.min(hundredths(room).unwrap_or(0).max(0))
}

/// Hundredths of a dB as 1/256 dB, the Opus header's unit.
#[must_use]
pub fn hundredths_to_q78(cdb: i32) -> i16 {
    let steps = (f64::from(cdb) * 256.0 / 100.0).round();
    steps.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

/// 1/256 dB as hundredths of a dB, rounded.
#[must_use]
pub fn q78_to_hundredths(q: i16) -> i32 {
    (i32::from(q) * 100 * 2 + 256).div_euclid(512)
}

fn hundredths(db: f64) -> Option<i32> {
    let n = (db * 100.0).round();
    (n.is_finite() && n.abs() < 1e6).then_some(n as i32)
}

fn millionths(fraction: f64) -> Option<u32> {
    let n = (fraction * 1e6).round();
    (n.is_finite() && n >= 0.0 && n < f64::from(u32::MAX)).then_some(n as u32)
}

/// A gain as muman writes it, `-6.12 dB`, from whole hundredths.
#[must_use]
pub fn gain_text(cdb: i32) -> String {
    let sign = if cdb < 0 { "-" } else { "" };
    let n = cdb.unsigned_abs();
    format!("{sign}{}.{:02} dB", n / 100, n % 100)
}

fn peak_text(millionths: u32) -> String {
    format!("{}.{:06}", millionths / 1_000_000, millionths % 1_000_000)
}

/// A gain as tags write it, in hundredths of a dB.
#[must_use]
pub fn parse_gain(text: &str) -> Option<i32> {
    let number = text
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace());
    hundredths(number.parse::<f64>().ok().filter(|db| db.abs() < 100.0)?)
}

/// A peak as tags write it, in millionths of full scale.
#[must_use]
pub fn parse_peak(text: &str) -> Option<u32> {
    millionths(text.trim().parse::<f64>().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Kind, Pass};
    use crate::testing::{sine, wav};

    fn measure(rate: u32, channels: u16, mask: Option<u32>, samples: &[f32]) -> Option<Measure> {
        let mut p = Pass::new(vec![Kind::Loudness]);
        p.take(&wav(rate, channels, mask, samples));
        p.finish(&crate::integrity::Found::default())
            .loudness
            .and_then(|m| m.value)
    }

    /// A 1 kHz sine at `dbfs` peak in every one of `channels`.
    fn tone(rate: u32, channels: u16, dbfs: f64, seconds: f64) -> Vec<f32> {
        sine(
            rate,
            channels,
            1000.0,
            10f64.powf(dbfs / 20.0) as f32,
            seconds,
        )
    }

    fn near(a: f64, b: f64, within: f64) -> bool {
        (a - b).abs() <= within
    }

    /// Seeded noise in `channels`, swelling and falling every half
    /// second, so the peak so far is passed again and again.
    fn swells(rate: u32, channels: u16, seconds: f64) -> Vec<f32> {
        let mut state: u32 = 7;
        (0..(f64::from(rate) * seconds) as usize)
            .flat_map(|n| {
                let swell = (f64::from(n as u32) / f64::from(rate) * 4.0).sin().abs();
                (0..channels).map(move |c| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = f64::from(state >> 8) / f64::from(1u32 << 24) - 0.5;
                    (noise * swell * (1.0 + f64::from(c) / 4.0)) as f32
                })
            })
            .collect()
    }

    #[test]
    fn the_true_peak_is_the_crates_to_the_bit() {
        for (rate, channels) in [
            (44_100, 2),
            (48_000, 1),
            (96_000, 2),
            (44_100, 3),
            (22_050, 6),
        ] {
            let s = swells(rate, channels, 2.5);
            let mut ebu = EbuR128::new(u32::from(channels), rate, Mode::TRUE_PEAK).unwrap();
            for hop in s.chunks(usize::from(channels) * 1234) {
                ebu.add_frames_f32(hop).unwrap();
            }
            let mut tp = TruePeak::new(rate, usize::from(channels)).unwrap();
            for piece in s.chunks(usize::from(channels) * 777) {
                tp.scan(piece);
            }
            for c in 0..u32::from(channels) {
                let ours = tp.peak(c as usize).max(ebu.sample_peak(c).unwrap());
                assert_eq!(
                    ours.to_bits(),
                    ebu.true_peak(c).unwrap().to_bits(),
                    "{rate} Hz, channel {c}/{channels}: {ours}"
                );
                assert!(ours > 0.1, "{ours}");
            }
        }
        assert!(TruePeak::new(192_000, 2).is_none());
    }

    #[test]
    fn a_stereo_sine_at_minus_23_dbfs_measures_minus_23_lufs() {
        let m = measure(48_000, 2, Some(0x3), &tone(48_000, 2, -23.0, 20.0)).unwrap();
        let lufs = m.lufs().unwrap();
        assert!(near(lufs, -23.0, 0.1), "{lufs}");
        assert!(near(20.0 * m.sample_peak.log10(), -23.0, 0.05));
        assert!(!m.short_term.is_empty());
    }

    #[test]
    fn one_channel_plays_from_both_speakers() {
        let m = measure(44_100, 1, None, &tone(44_100, 1, -23.0, 10.0)).unwrap();
        assert!(near(m.lufs().unwrap(), -23.0, 0.1), "{:?}", m.lufs());
    }

    #[test]
    fn quiet_stretches_fall_under_the_relative_gate() {
        // EBU Tech 3341's third case: -36, -23 and -36 dBFS, 10, 60 and 10 s.
        let rate = 48_000;
        let mut s = tone(rate, 2, -36.0, 10.0);
        s.extend(tone(rate, 2, -23.0, 60.0));
        s.extend(tone(rate, 2, -36.0, 10.0));
        let lufs = measure(rate, 2, Some(0x3), &s).unwrap().lufs().unwrap();
        assert!(near(lufs, -23.0, 0.1), "{lufs}");
    }

    #[test]
    fn the_gating_is_libebur128s_over_its_histogram() {
        let rate = 44_100;
        let mut s = tone(rate, 2, -26.0, 20.0);
        s.extend(tone(rate, 2, -20.0, 20.1));
        s.extend(tone(rate, 2, -40.0, 5.0));
        let ours = measure(rate, 2, Some(0x3), &s).unwrap().lufs().unwrap();
        let mut ebu = EbuR128::new(2, rate, Mode::I | Mode::HISTOGRAM).unwrap();
        ebu.add_frames_f32(&s).unwrap();
        let theirs = ebu.loudness_global().unwrap();
        assert!(near(ours, theirs, 0.01), "{ours} against {theirs}");
    }

    #[test]
    fn silence_has_no_loudness_but_its_method_is_recorded() {
        let mut p = Pass::new(vec![Kind::Loudness]);
        p.take(&wav(48_000, 2, None, &vec![0.0; 96_000]));
        let a = p.finish(&crate::integrity::Found::default());
        assert_eq!(a.loudness.as_ref().unwrap().method, METHOD);
        assert!(a.loudness().is_none());
    }

    #[test]
    fn a_hundred_milliseconds_rounds_as_libebur128_rounds_it() {
        let stream = Stream {
            rate: 11_025,
            channels: 1,
            mask: None,
        };
        assert_eq!(Meter::new(&stream).unwrap().hop, 1103);
    }

    #[test]
    fn a_true_peak_lies_between_samples() {
        // A quarter of the rate at 45°: every sample sits at 0.707 of the peak.
        let rate = 48_000;
        let s: Vec<f32> = (0..rate * 2)
            .flat_map(|n| {
                let x = (0.5
                    * (std::f64::consts::FRAC_PI_2 * f64::from(n) + std::f64::consts::FRAC_PI_4)
                        .sin()) as f32;
                [x, x]
            })
            .collect();
        let m = measure(rate, 2, Some(0x3), &s).unwrap();
        assert!(near(m.sample_peak, 0.5 * 0.707, 0.01), "{}", m.sample_peak);
        assert!(near(m.true_peak, 0.5, 0.02), "{}", m.true_peak);
    }

    #[test]
    fn the_lfe_counts_for_nothing() {
        let rate = 48_000;
        let quiet = tone(rate, 1, -30.0, 10.0);
        let loud = tone(rate, 1, -10.0, 10.0);
        let frames = |lfe: &[f32]| -> Vec<f32> {
            quiet
                .iter()
                .zip(lfe)
                .flat_map(|(&q, &l)| [q, q, q, l, q, q])
                .collect()
        };
        let mask = Some(0x3f);
        let with = measure(rate, 6, mask, &frames(&loud))
            .unwrap()
            .lufs()
            .unwrap();
        let without = measure(rate, 6, mask, &frames(&quiet))
            .unwrap()
            .lufs()
            .unwrap();
        assert!(near(with, without, 0.01), "{with} against {without}");
    }

    #[test]
    fn an_album_is_every_block_of_its_songs() {
        let rate = 48_000;
        let a = measure(rate, 2, Some(0x3), &tone(rate, 2, -20.0, 30.0)).unwrap();
        let b = measure(rate, 2, Some(0x3), &tone(rate, 2, -26.0, 10.0)).unwrap();
        let album = integrated([&a.momentary, &b.momentary]).unwrap();
        assert!(album < a.lufs().unwrap() && album > b.lufs().unwrap());
        assert!(
            album > f64::midpoint(a.lufs().unwrap(), b.lufs().unwrap()),
            "longer weighs more"
        );
        assert_eq!(integrated([&a.momentary]), a.lufs());
        assert_eq!(integrated(std::iter::empty()), None);
    }

    #[test]
    fn a_histogram_round_trips_through_its_text() {
        let m = measure(48_000, 2, Some(0x3), &tone(48_000, 2, -18.0, 5.0)).unwrap();
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Measure>(&json).unwrap(), m);
        assert!(serde_json::from_str::<Histogram>("\"!!\"").is_err());
        assert!(!json.contains("opus_gain"));
    }

    #[test]
    fn only_a_raising_gain_is_capped_by_the_ceiling() {
        // A peak at half of full scale, -6.02 dB.
        assert_eq!(capped(300, Some(500_000), -100), 300);
        assert_eq!(capped(800, Some(500_000), -100), 502);
        assert_eq!(capped(800, Some(1_100_000), -100), 0);
        assert_eq!(capped(-800, Some(1_100_000), -100), -800);
        assert_eq!(capped(800, None, -100), 800);
    }

    #[test]
    fn tags_carry_what_is_left_of_a_gain_applied() {
        let gains = Gains {
            track: Level {
                gain: -612,
                peak: Some(900_000),
            },
            album: Some(Level {
                gain: -500,
                peak: Some(950_000),
            }),
            apply: Apply::Volume(-500),
            opus_gain: 0,
            bits: 16,
        };
        let text = |applied| {
            gains
                .tags(applied)
                .into_iter()
                .map(|(_, v)| v)
                .collect::<Vec<_>>()
        };
        assert_eq!(text(0), ["-6.12 dB", "0.900000", "-5.00 dB", "0.950000"]);
        assert_eq!(text(-500), ["-1.12 dB", "0.506107", "0.00 dB", "0.534224"]);
        assert_eq!(gains.applied(false, true), -500);
        assert_eq!(gains.applied(false, false), 0);
        assert_eq!(gains.header(true, true), None);
    }

    #[test]
    fn the_opus_header_adds_to_the_gain_a_copy_held() {
        let gains = Gains {
            track: Level {
                gain: -612,
                peak: None,
            },
            album: None,
            apply: Apply::Header(hundredths_to_q78(-612)),
            opus_gain: 256,
            bits: 0,
        };
        assert_eq!(hundredths_to_q78(-612), -1567);
        assert_eq!(gains.header(true, true), Some(256 - 1567));
        assert_eq!(gains.header(true, false), Some(-1567));
        assert_eq!(gains.header(false, true), None);
        assert_eq!(gains.applied(true, false), -612);
        assert_eq!(q78_to_hundredths(-1567), -612);
        assert_eq!(q78_to_hundredths(256), 100);
    }

    #[test]
    fn gains_and_peaks_read_as_tags_write_them() {
        assert_eq!(parse_gain("-6.12 dB"), Some(-612));
        assert_eq!(parse_gain("+2.5dB"), Some(250));
        assert_eq!(parse_gain("loud"), None);
        assert_eq!(parse_peak("0.980000"), Some(980_000));
        assert_eq!(gain_text(-5), "-0.05 dB");
        assert_eq!(gain_text(1234), "12.34 dB");
        assert_eq!(peak_text(1_023_000), "1.023000");
        assert_eq!(
            Level::to(-1800, -9.5, 1.0),
            Some(Level {
                gain: -850,
                peak: Some(1_000_000)
            })
        );
    }

    /// What the meter makes of tones and swelling noise over every
    /// oversampling, speaker weighting and one channel, and of silence,
    /// with [`METHOD`].
    const WRITTEN: (&str, &str) = (
        "loudness/1",
        "567c8a4145d8f08ed4043e9e493b5c57dc6b5b76f2879e7a6d4ff488c600b2e8",
    );

    #[test]
    fn the_method_rises_with_what_it_makes() {
        let mut quiet = tone(48_000, 2, -40.0, 2.0);
        quiet.extend(tone(48_000, 2, -18.0, 3.0));
        let samples = [
            (48_000, 2, Some(0x3), quiet),
            (44_100, 1, None, tone(44_100, 1, -23.0, 4.0)),
            (48_000, 6, Some(0x3F), swells(48_000, 6, 4.0)),
            (96_000, 2, Some(0x3), swells(96_000, 2, 3.5)),
            (22_050, 2, None, swells(22_050, 2, 3.5)),
            (48_000, 2, Some(0x3), vec![0.0; 96_000]),
        ];
        let mut digest = crate::testing::Digest::default();
        for (rate, channels, mask, s) in samples {
            digest.debug(&measure(rate, channels, mask, &s));
        }
        crate::testing::check(WRITTEN, METHOD, &digest.hex());
    }
}
