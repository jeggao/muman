//! A source's audio analyzed whole, in one pass of every analyzer that
//! wants it.
//!
//! ffmpeg decodes the audio stream to 32-bit float WAV on its stdout, at
//! the source's own rate and in its own channels, and [`Pass`] hands the
//! frames to each [`Analyzer`] as they arrive: no file is written, so an
//! hour-long video costs no scratch space. The WAV header, unlike raw
//! PCM, carries the rate and channels the decoder produced, which can
//! differ from what a container declares, and the speakers' mask an
//! analyzer weighs channels by. The pass shares the decode that the
//! fingerprint already takes in [`crate::facts`]' one ffmpeg run.
//!
//! Each kind of analysis is versioned by its own method, so a kind added
//! or changed is caught up alone, by one decode, without measuring prints
//! or quality again: [`Analysis::stale`] names the kinds a source lacks,
//! and a pass runs only those. A kind that measured nothing, as loudness
//! on silence, records its method all the same, so it is not tried again
//! until the file changes.
//!
//! What a source gets, and what each step of a song's writing does with
//! it, sits in four layers, each one place for what joins it later:
//!
//! 1. **Analysis**, here: one decode, many analyzers. Loudness
//!    ([`crate::loudness`]) is the first; a dynamic-range meter, silence
//!    at the ends, DC offset or tempo would each be one more [`Kind`],
//!    one field of [`Analysis`] and a module of their own. Integrity
//!    ([`crate::integrity`]) is measured by the pass itself, which counts
//!    the frames decoded and the samples that were no number before any
//!    analyzer sees them, beside what the packets and the stream say
//!    ([`integrity::Found`]); every source takes it, whatever the
//!    settings, and a mix, which holds no other packets, none.
//! 1. **Album pooling**, in [`crate::reconcile`]'s planning: songs are
//!    grouped by album once, and loudness pools its songs' blocks over
//!    each group, as an album's loudness range or dynamic range would.
//! 1. **Derivation**, in [`crate::resolve::Plan`]: what a song should get
//!    is held as whole numbers, and its tags, lossless edits and filters
//!    are derived from them and the format it is written in.
//! 1. **Output**, in [`crate::render`] and [`crate::codec`]: tags a retag
//!    rewrites; lossless edits after muxing, such as the Opus header's
//!    gain, which a retag makes too; and filters on the samples, which
//!    only an encode can apply.

use serde::{Deserialize, Serialize};

use crate::integrity;
use crate::loudness;

/// What is analyzed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Loudness,
    Integrity,
}

impl Kind {
    /// The method this kind is measured by; a measure by another is
    /// measured again.
    #[must_use]
    pub fn method(self) -> &'static str {
        match self {
            Self::Loudness => loudness::METHOD,
            Self::Integrity => integrity::METHOD,
        }
    }

    /// Whether a source's audio mixed into another layout is measured for
    /// it too, as mixed.
    #[must_use]
    pub fn mixed(self) -> bool {
        match self {
            Self::Loudness => true,
            Self::Integrity => false,
        }
    }
}

/// A measure and the method it was taken by; `value` is `None` where the
/// method found nothing to measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct Measured<T> {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
}

/// What the whole of a source's audio was found to hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<Measured<loudness::Measure>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<Measured<integrity::Measure>>,
}

impl Analysis {
    /// The kinds of `wanted` this analysis lacks, or holds by another
    /// method.
    #[must_use]
    pub fn stale(&self, wanted: &[Kind]) -> Vec<Kind> {
        wanted
            .iter()
            .copied()
            .filter(|&kind| {
                let method = match kind {
                    Kind::Loudness => self.loudness.as_ref().map(|m| m.method.as_str()),
                    Kind::Integrity => self.integrity.as_ref().map(|m| m.method.as_str()),
                };
                method != Some(kind.method())
            })
            .collect()
    }

    /// The loudness measured, if it was by today's method.
    #[must_use]
    pub fn loudness(&self) -> Option<&loudness::Measure> {
        self.loudness
            .as_ref()
            .filter(|m| m.method == loudness::METHOD)
            .and_then(|m| m.value.as_ref())
    }

    /// What the integrity measure found, if it was by today's method.
    #[must_use]
    pub fn integrity(&self) -> Option<&integrity::Measure> {
        self.integrity
            .as_ref()
            .filter(|m| m.method == integrity::METHOD)
            .and_then(|m| m.value.as_ref())
    }

    /// `newer` over this analysis: each kind it took replaces this one's.
    pub fn merge(&mut self, newer: Self) {
        if newer.loudness.is_some() {
            self.loudness = newer.loudness;
        }
        if newer.integrity.is_some() {
            self.integrity = newer.integrity;
        }
    }
}

/// The kinds the settings want measured: integrity always.
#[must_use]
pub fn wanted(loudness: &crate::settings::Loudness) -> Vec<Kind> {
    if loudness.measured() {
        vec![Kind::Integrity, Kind::Loudness]
    } else {
        vec![Kind::Integrity]
    }
}

/// The decoded stream, as its WAV header gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    pub rate: u32,
    pub channels: u16,
    /// Which speaker each channel is, in `WAVE_FORMAT_EXTENSIBLE`'s bits,
    /// one per channel in order; `None` when the header names none.
    pub mask: Option<u32>,
}

/// One kind of analysis under way.
pub trait Analyzer {
    /// Whole frames, interleaved, every sample finite.
    fn take(&mut self, frames: &[f32]);
    /// The measure, into its field of `into`.
    fn finish(self: Box<Self>, into: &mut Analysis);
}

fn start(kind: Kind, stream: &Stream) -> Option<Box<dyn Analyzer>> {
    match kind {
        Kind::Loudness => loudness::Meter::new(stream).map(|m| Box::new(m) as Box<dyn Analyzer>),
        Kind::Integrity => None,
    }
}

/// The WAV container's chunks before its samples.
#[derive(Default)]
struct Header {
    bytes: Vec<u8>,
    stream: Option<Stream>,
}

/// What a WAV header up to its `data` chunk says, `Ok(None)` while more
/// bytes are needed: the stream and where the samples begin.
pub(crate) fn parse_header(bytes: &[u8]) -> Result<Option<(Stream, usize)>, String> {
    if bytes.len() < 12 {
        return Ok(None);
    }
    if !matches!(&bytes[..4], b"RIFF" | b"RF64") || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV stream".into());
    }
    let mut at = 12;
    let mut stream = None;
    loop {
        let Some(chunk) = bytes.get(at..at + 8) else {
            return Ok(None);
        };
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
        let body = at + 8;
        if &chunk[..4] == b"data" {
            let stream = stream.ok_or("WAV samples before their format")?;
            return Ok(Some((stream, body)));
        }
        let end = body + size + size % 2;
        let Some(content) = bytes.get(body..end) else {
            return Ok(None);
        };
        if &chunk[..4] == b"fmt " {
            stream = Some(parse_format(content)?);
        }
        at = end;
    }
}

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

fn parse_format(fmt: &[u8]) -> Result<Stream, String> {
    let u16_at = |i: usize| fmt.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| {
        fmt.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let short = || "a WAV format chunk cut short".to_string();
    let tag = u16_at(0).ok_or_else(short)?;
    let channels = u16_at(2).ok_or_else(short)?;
    let rate = u32_at(4).ok_or_else(short)?;
    let bits = u16_at(14).ok_or_else(short)?;
    let (float, mask) = if tag == WAVE_FORMAT_EXTENSIBLE {
        (
            u16_at(24) == Some(WAVE_FORMAT_IEEE_FLOAT),
            u32_at(20).filter(|&m| m != 0),
        )
    } else {
        (tag == WAVE_FORMAT_IEEE_FLOAT, None)
    };
    if !float || bits != 32 {
        return Err("WAV samples that are not 32-bit float".into());
    }
    if channels == 0 || rate == 0 {
        return Err("a WAV stream of no channels or no rate".into());
    }
    Ok(Stream {
        rate,
        channels,
        mask,
    })
}

/// One decode handed to every analyzer of the kinds asked for.
pub struct Pass {
    kinds: Vec<Kind>,
    header: Header,
    analyzers: Vec<Box<dyn Analyzer>>,
    /// Bytes of a frame not yet whole.
    carry: Vec<u8>,
    frames: Vec<f32>,
    broken: bool,
    /// Frames read, and samples among them that were no number.
    counted: u64,
    unfinite: u64,
}

impl std::fmt::Debug for Pass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pass")
            .field("kinds", &self.kinds)
            .finish_non_exhaustive()
    }
}

impl Pass {
    #[must_use]
    pub fn new(kinds: Vec<Kind>) -> Self {
        Self {
            kinds,
            header: Header::default(),
            analyzers: Vec::new(),
            carry: Vec::new(),
            frames: Vec::new(),
            broken: false,
            counted: 0,
            unfinite: 0,
        }
    }

    /// Whether there is anything to analyze.
    #[must_use]
    pub fn wants(&self) -> bool {
        !self.kinds.is_empty()
    }

    /// Bytes of the stream, in pieces of any size.
    pub fn take(&mut self, bytes: &[u8]) {
        if self.broken {
            return;
        }
        let Some(stream) = self.header.stream else {
            self.header.bytes.extend_from_slice(bytes);
            match parse_header(&self.header.bytes) {
                Ok(None) => {}
                Ok(Some((stream, body))) => {
                    self.header.stream = Some(stream);
                    self.analyzers = self
                        .kinds
                        .iter()
                        .filter_map(|&k| start(k, &stream))
                        .collect();
                    let rest = self.header.bytes.split_off(body);
                    self.take(&rest);
                }
                Err(_) => self.broken = true,
            }
            return;
        };
        let frame = usize::from(stream.channels) * 4;
        self.carry.extend_from_slice(bytes);
        let whole = self.carry.len() / frame * frame;
        self.counted += (whole / frame) as u64;
        self.frames.clear();
        let mut unfinite = 0;
        self.frames
            .extend(self.carry[..whole].as_chunks::<4>().0.iter().map(|b| {
                let s = f32::from_le_bytes(*b);
                if s.is_finite() {
                    s
                } else {
                    unfinite += 1;
                    0.0
                }
            }));
        self.unfinite += unfinite;
        self.carry.drain(..whole);
        for a in &mut self.analyzers {
            a.take(&self.frames);
        }
    }

    /// The stream begins again, as when its run is retried alone.
    pub fn again(&mut self) {
        *self = Self::new(std::mem::take(&mut self.kinds));
    }

    /// What was found, with what `found` says of the stream beside its
    /// samples. Every kind asked for records its method, with no value
    /// where the stream did not read or held nothing to measure.
    #[must_use]
    pub fn finish(self, found: &integrity::Found) -> Analysis {
        let mut analysis = Analysis::default();
        for a in self.analyzers {
            a.finish(&mut analysis);
        }
        for kind in self.kinds {
            let method = kind.method().to_string();
            match kind {
                Kind::Loudness => {
                    analysis.loudness.get_or_insert(Measured {
                        method,
                        value: None,
                    });
                }
                Kind::Integrity => {
                    let rate = self.header.stream.map(|s| s.rate);
                    let value = integrity::measure(rate, self.counted, self.unfinite, found);
                    analysis.integrity = Some(Measured { method, value });
                }
            }
        }
        analysis
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{sine, wav};

    fn none() -> integrity::Found {
        integrity::Found::default()
    }

    #[test]
    fn a_header_is_read_whatever_chunks_come_before_the_samples() {
        let samples = sine(48_000, 1, 1000.0, 0.5, 0.5);
        let mut bytes = wav(48_000, 1, None, &samples);
        let list = b"LIST\x05\x00\x00\x00abcde\x00";
        bytes.splice(36..36, list.iter().copied());
        let (stream, body) = parse_header(&bytes).unwrap().unwrap();
        assert_eq!(
            stream,
            Stream {
                rate: 48_000,
                channels: 1,
                mask: None
            }
        );
        assert_eq!(bytes.len() - body, samples.len() * 4);
        assert_eq!(parse_header(&bytes[..30]), Ok(None));
        assert!(parse_header(b"RIFF\0\0\0\0AVI LIST").is_err());
    }

    #[test]
    fn the_same_stream_analyzes_alike_however_it_is_cut() {
        let samples = sine(44_100, 2, 997.0, 0.25, 3.0);
        let bytes = wav(44_100, 2, Some(0x3), &samples);
        let whole = {
            let mut p = Pass::new(vec![Kind::Loudness]);
            p.take(&bytes);
            p.finish(&none())
        };
        for piece in [1, 7, 4093] {
            let mut p = Pass::new(vec![Kind::Loudness]);
            for chunk in bytes.chunks(piece) {
                p.take(chunk);
            }
            assert_eq!(p.finish(&none()), whole, "pieces of {piece}");
        }
        assert!(whole.loudness().is_some());
    }

    #[test]
    fn a_stream_begun_again_forgets_what_came_before() {
        let samples = sine(48_000, 1, 1000.0, 0.5, 1.0);
        let bytes = wav(48_000, 1, None, &samples);
        let mut p = Pass::new(vec![Kind::Loudness]);
        p.take(&bytes[..bytes.len() / 2]);
        p.again();
        p.take(&bytes);
        let mut q = Pass::new(vec![Kind::Loudness]);
        q.take(&bytes);
        assert_eq!(p.finish(&none()), q.finish(&none()));
    }

    #[test]
    fn a_kind_asked_for_records_its_method_though_nothing_read() {
        let mut p = Pass::new(vec![Kind::Loudness]);
        p.take(b"not audio at all");
        let a = p.finish(&none());
        assert_eq!(
            a.loudness.as_ref().map(|m| m.method.as_str()),
            Some(loudness::METHOD)
        );
        assert!(a.loudness().is_none());
        assert_eq!(a.stale(&[Kind::Loudness]), [] as [Kind; 0]);
        assert_eq!(
            Analysis::default().stale(&[Kind::Loudness]),
            [Kind::Loudness]
        );
        assert!(Pass::new(Vec::new()).finish(&none()).loudness.is_none());
    }

    #[test]
    fn the_pass_counts_the_frames_decoded_and_the_samples_no_number() {
        let mut samples = sine(8000, 2, 440.0, 0.5, 3.0);
        samples[10] = f32::NAN;
        samples[11] = f32::INFINITY;
        let bytes = wav(8000, 2, None, &samples);
        let measure = |cut: usize| {
            let mut p = Pass::new(vec![Kind::Integrity]);
            p.take(&bytes[..bytes.len() / 3]);
            p.again();
            for chunk in bytes.chunks(cut) {
                p.take(chunk);
            }
            *p.finish(&none()).integrity().unwrap()
        };
        let whole = measure(bytes.len());
        assert!((whole.decoded - 3.0).abs() < 1e-9, "{whole:?}");
        assert_eq!(whole.unfinite, 2);
        assert_eq!(whole.span, None, "no packets listed");
        assert_eq!(measure(7), whole, "however it is cut");
        let found = integrity::Found {
            said: Some(3.0),
            timeline: Some(integrity::Timeline {
                start: 0.0,
                end: 3.0,
                holes: 0.0,
            }),
        };
        let mut p = Pass::new(vec![Kind::Integrity, Kind::Loudness]);
        p.take(&bytes);
        let a = p.finish(&found);
        assert_eq!(a.integrity().unwrap().span, Some((0.0, 3.0)));
        assert_eq!(a.integrity().unwrap().damage(), None);
        assert!(a.loudness().is_some(), "every analyzer saw finite samples");
    }

    #[test]
    fn integrity_is_always_wanted_caught_up_once_and_never_of_a_mix() {
        let off = crate::settings::Loudness {
            mode: crate::settings::LoudnessMode::Off,
            ..crate::settings::Loudness::default()
        };
        assert_eq!(wanted(&off), [Kind::Integrity]);
        assert!(wanted(&crate::settings::Loudness::default()).contains(&Kind::Loudness));
        let old = Analysis {
            loudness: Some(Measured {
                method: loudness::METHOD.into(),
                value: None,
            }),
            integrity: None,
        };
        assert_eq!(
            old.stale(&[Kind::Integrity, Kind::Loudness]),
            [Kind::Integrity]
        );
        let mut p = Pass::new(vec![Kind::Integrity]);
        p.take(b"not audio at all");
        let unread = p.finish(&none());
        assert_eq!(unread.stale(&[Kind::Integrity]), [] as [Kind; 0]);
        assert!(unread.integrity().is_none());
        assert!(!Kind::Integrity.mixed() && Kind::Loudness.mixed());
    }
}
