//! Whether a source's audio is all there: a [`crate::analysis`] kind,
//! measured in the decode every source has, from three lengths no header
//! can fake together.
//!
//! - **Said:** the audio stream's own length, [`crate::probe::Audio::duration`].
//!   Never the file's, which counts every stream: a `WebVTT` subtitle
//!   running a second or two past the audio would make every such video
//!   look cut off.
//! - **Span:** from where the stream's first packet starts to where its
//!   last ends ([`Timeline`]), as the packets list them without decoding.
//!   Not the sum of the packets' durations: Matroska leaves some packets'
//!   durations out, so healthy files sum about 2 s short, and corrupt
//!   packets can sum to more than the file runs.
//! - **Decoded:** the frames the decoder produced, counted as the
//!   analysis pass reads them.
//!
//! Two kinds of [`Damage`] follow. **Cut:** the packets end more than
//! [`SLACK_S`] before the stream says, as a copy or a download stopped
//! short leaves a file; copied, a song would break off mid-frame, so it is
//! encoded. **Missing:** less decodes than the packets span, by more than
//! [`SLACK_S`]: holes in the timeline, as fragments a download lost leave,
//! or packets too corrupt to decode. Encoding repairs neither, so the
//! source ranks below an intact copy by what is missing, and a fetched one
//! is fetched again.
//!
//! Measured over a library of 2,213 sources, span and decoded length
//! agree within 26 ms for 99% of them, Opus's pre-skip the most of it;
//! the two rules flagged exactly the three damaged files and nothing else,
//! where the file's length against the packets' end flagged 41, one of them
//! damaged.

use serde::{Deserialize, Serialize};

use crate::source::SourceKey;

/// Names the measure; a measure by another is taken again.
pub const METHOD: &str = "integrity/1";

/// Seconds the lengths may differ by before a source counts as damaged:
/// containers round lengths, and Opus skips its first few milliseconds.
pub const SLACK_S: f64 = 1.0;

/// A gap between packets longer than this is a hole: five Opus packets.
const HOLE_S: f64 = 0.1;

/// What a source's audio was found to hold, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    /// The audio stream's own length, where it says one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<f64>,
    pub decoded: f64,
    /// Where the first packet starts and the last ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<(f64, f64)>,
    /// The gaps between packets, summed.
    #[serde(default)]
    pub holes: f64,
    /// Samples that decoded as no number, NaN or infinite, which the
    /// analyzers took for silence.
    #[serde(default)]
    pub unfinite: u64,
}

/// What a damaged source lacks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Damage {
    /// Its packets end at `ends` where its stream says `said`.
    Cut { said: f64, ends: f64 },
    /// `seconds` within its packets' span did not decode, `holes` of them
    /// gaps between packets.
    Missing { seconds: f64, holes: f64 },
}

impl Measure {
    /// The damage the lengths show, if any: a cut first, as a file cut
    /// short may hold holes too.
    #[must_use]
    pub fn damage(&self) -> Option<Damage> {
        let (start, end) = self.span?;
        if let Some(said) = self.said.filter(|said| said - end > SLACK_S) {
            return Some(Damage::Cut { said, ends: end });
        }
        let seconds = end - start - self.decoded;
        (seconds > SLACK_S).then_some(Damage::Missing {
            seconds,
            holes: self.holes,
        })
    }
}

impl Damage {
    /// What it is, said of `key`.
    #[must_use]
    pub fn describe(&self, key: &SourceKey) -> String {
        match *self {
            Self::Cut { said, ends } => format!(
                "{key} holds {ends:.0} s of audio where its stream says {said:.0} s: \
                 cut off, as a copy or a download stopped"
            ),
            Self::Missing { seconds, holes } if holes > SLACK_S => format!(
                "{key} lacks {seconds:.0} s of its audio, {holes:.0} s of it in holes between \
                 its packets, as a download that lost pieces leaves"
            ),
            Self::Missing { seconds, .. } => format!(
                "{key} lacks {seconds:.0} s of its audio, which does not decode: \
                 its packets are damaged"
            ),
        }
    }

    /// Milliseconds of the recording it lacks: none for a cut, whose
    /// missing end comparing it with another source shows already.
    #[must_use]
    pub fn missing_ms(&self) -> i64 {
        match *self {
            Self::Cut { .. } => 0,
            Self::Missing { seconds, .. } => (seconds * 1000.0).round() as i64,
        }
    }
}

/// Where a stream's packets lie, from a `framecrc` list of them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timeline {
    pub start: f64,
    pub end: f64,
    pub holes: f64,
}

impl Timeline {
    /// The packets of a `framecrc` list: its time base from `#tb 0:`, then
    /// each line's presentation time and duration, the third and fourth
    /// fields. `None` for a list of no packet.
    #[must_use]
    pub fn parse(framecrc: &str) -> Option<Self> {
        let (num, den) = framecrc
            .lines()
            .find_map(|l| l.strip_prefix("#tb 0:"))?
            .trim()
            .split_once('/')?;
        let (num, den): (f64, f64) = (num.trim().parse().ok()?, den.trim().parse().ok()?);
        if den <= 0.0 {
            return None;
        }
        let mut packets: Vec<(i64, i64)> = framecrc
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| {
                let mut fields = l.split(',').skip(2).map(|f| f.trim().parse::<i64>().ok());
                Some((fields.next()??, fields.next()??.max(0)))
            })
            .collect();
        packets.sort_unstable();
        let seconds = |ticks: i64| ticks as f64 * num / den;
        let start = packets.first()?.0;
        let mut end = start;
        let mut holes = 0.0;
        for (pts, duration) in packets {
            let gap = seconds(pts - end);
            if gap > HOLE_S {
                holes += gap;
            }
            end = end.max(pts + duration);
        }
        Some(Self {
            start: seconds(start),
            end: seconds(end),
            holes,
        })
    }
}

/// What the pass is told of a stream beside its samples.
#[derive(Debug, Clone, Copy, Default)]
pub struct Found {
    /// The audio stream's own length.
    pub said: Option<f64>,
    pub timeline: Option<Timeline>,
}

/// The measure of `frames` decoded at `rate`, `unfinite` samples of them
/// no number, beside what `found` says; `None` where nothing decoded to
/// read a rate from.
#[must_use]
pub fn measure(rate: Option<u32>, frames: u64, unfinite: u64, found: &Found) -> Option<Measure> {
    let rate = rate.filter(|r| *r > 0)?;
    let decoded = frames as f64 / f64::from(rate);
    Some(Measure {
        said: found.said,
        decoded,
        span: found.timeline.map(|t| (t.start, t.end)),
        holes: found.timeline.map_or(0.0, |t| t.holes),
        unfinite,
    })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn list(packets: &[(i64, i64)]) -> String {
        let mut text = "#tb 0: 1/1000\n#media_type 0: audio\n".to_string();
        for (pts, duration) in packets {
            let _ = writeln!(text, "0, {pts}, {pts}, {duration}, 300, 0x00000000");
        }
        text
    }

    fn whole(seconds: i64) -> Vec<(i64, i64)> {
        (0..seconds * 50).map(|n| (n * 20 - 7, 20)).collect()
    }

    #[test]
    fn a_timeline_spans_its_packets_and_sums_the_holes_between_them() {
        let t = Timeline::parse(&list(&whole(10))).unwrap();
        assert!(
            (t.start + 0.007).abs() < 1e-9 && (t.end - 9.993).abs() < 1e-9,
            "{t:?}"
        );
        assert!(t.holes.abs() < 1e-9);
        let gapped: Vec<(i64, i64)> = whole(10)
            .into_iter()
            .filter(|(pts, _)| !(3000..6500).contains(pts))
            .collect();
        let t = Timeline::parse(&list(&gapped)).unwrap();
        assert!((t.holes - 3.5).abs() < 0.03, "{t:?}");
        assert!((t.end - 9.993).abs() < 1e-9);
        // Packets listed out of order, and some with no duration.
        let t = Timeline::parse(&list(&[(40, 0), (0, 20), (20, 20)])).unwrap();
        assert_eq!((t.start, t.end, t.holes), (0.0, 0.04, 0.0));
        assert_eq!(
            Timeline::parse("0, 0, 0, 20, 1, 0x0\n"),
            None,
            "no time base"
        );
        assert_eq!(Timeline::parse("#tb 0: 1/1000\n"), None, "no packet");
    }

    fn measured(said: Option<f64>, packets: &[(i64, i64)], decoded: f64) -> Measure {
        let found = Found {
            said,
            timeline: Timeline::parse(&list(packets)),
        };
        let frames = (decoded * 48_000.0).round() as u64;
        measure(Some(48_000), frames, 0, &found).unwrap()
    }

    #[test]
    fn damage_is_judged_by_the_stream_its_packets_and_what_decodes() {
        // A subtitle runs past the audio: the file says longer, the audio
        // stream does not, and nothing is damaged.
        assert_eq!(measured(Some(10.0), &whole(10), 9.98).damage(), None);
        assert_eq!(measured(None, &whole(10), 9.98).damage(), None);
        // Packets ending well before the stream says: cut off.
        assert!(matches!(
            measured(Some(358.5), &whole(90), 89.97).damage(),
            Some(Damage::Cut { ends, .. }) if (ends - 89.993).abs() < 1e-9
        ));
        // Holes in the timeline.
        let gapped: Vec<(i64, i64)> = whole(10)
            .into_iter()
            .filter(|(pts, _)| !(3000..6500).contains(pts))
            .collect();
        let Some(Damage::Missing { seconds, holes }) = measured(Some(10.0), &gapped, 6.5).damage()
        else {
            panic!("holes are missing audio");
        };
        assert!((seconds - 3.49).abs() < 0.03 && (holes - 3.5).abs() < 0.03);
        // Every packet there, but less decodes: corrupt packets.
        assert!(matches!(
            measured(Some(10.0), &whole(10), 7.0).damage(),
            Some(Damage::Missing { holes, .. }) if holes == 0.0
        ));
        assert_eq!(measured(Some(10.0), &whole(10), 10.4).damage(), None);
    }

    #[test]
    fn damage_is_said_by_its_kind_and_counted_only_when_missing() {
        let key = SourceKey::youtube("vid00000001");
        let cut = Damage::Cut {
            said: 359.0,
            ends: 90.0,
        };
        assert!(cut.describe(&key).contains("cut off"));
        assert_eq!(cut.missing_ms(), 0);
        let holes = Damage::Missing {
            seconds: 10.6,
            holes: 10.7,
        };
        assert!(
            holes.describe(&key).contains("holes"),
            "{}",
            holes.describe(&key)
        );
        assert_eq!(holes.missing_ms(), 10_600);
        let corrupt = Damage::Missing {
            seconds: 56.0,
            holes: 0.0,
        };
        assert!(corrupt.describe(&key).contains("does not decode"));
        assert_eq!(measure(None, 100, 0, &Found::default()), None);
    }
}
