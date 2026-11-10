//! Whether a source's audio is all there: a [`crate::analysis`] kind,
//! measured in the decode every source has, from three lengths no header
//! can fake together, and a fourth from outside the file.
//!
//! - **Said:** the audio stream's own length, [`crate::probe::Audio::duration`].
//!   Never the file's, which counts every stream: a `WebVTT` subtitle
//!   running a second or two past the audio would make every such video
//!   look cut off. Never a length ffmpeg guessed from the bitrate either,
//!   for a file that states none ([`crate::probe::Probed::guessed`]): a
//!   whole VBR MP3 with no Xing header, its first frames quieter than the
//!   rest, said 62 s for 40 s. Guessed from the bytes there are, such a
//!   length never shows a cut, and only makes one up, so the file is
//!   judged by its span and its decode alone.
//! - **Span:** from where the stream's first packet starts to where its
//!   last ends ([`Timeline`]), as the packets list them without decoding.
//!   Not the sum of the packets' durations: Matroska leaves some packets'
//!   durations out, so healthy files sum about 2 s short, and corrupt
//!   packets can sum to more than the file runs. A duration no packet can
//!   last, wrapped around from a negative one ([`WRAP`]) or past
//!   [`LONGEST_PACKET_S`], ends nothing: one such packet at the end of a
//!   whole Ogg file spanned 27 hours.
//! - **Decoded:** the frames the decoder produced, counted as the
//!   analysis pass reads them.
//! - **Stated:** the length the site gave, for what yt-dlp fetched, kept
//!   with what it served ([`crate::facts::Served`]); not measured, so a
//!   source measured before it was kept is judged by the three alone.
//!
//! Three kinds of [`Damage`] follow. **Cut:** the packets end more than
//! [`SLACK_S`] before the stream says, as a copy or a download stopped
//! short leaves a file; copied, a song would break off mid-frame, so it is
//! encoded. **Missing:** less decodes than the packets span, by more than
//! [`SLACK_S`]: holes in the timeline, as fragments a download lost leave,
//! or packets too corrupt to decode. Encoding repairs neither, so the
//! source ranks below an intact copy by what is missing, and a fetched one
//! is fetched again. **Short:** the file is whole in itself, but its
//! packets end well before the length its site stated, as ffmpeg leaves
//! a remux it is stopped in: every length the file holds was written by
//! the program that stopped, and agrees. It is fetched again as a cut
//! file is, but copies cleanly.
//!
//! Measured over a library of 2,213 sources, span and decoded length
//! agree within 26 ms for 99% of them, Opus's pre-skip the most of it;
//! the two rules flagged exactly the three damaged files and nothing else,
//! where the file's length against the packets' end flagged 41, one of them
//! damaged.

use serde::{Deserialize, Serialize};

use crate::source::SourceKey;

/// Names the measure; a measure by another is taken again.
pub const METHOD: &str = "integrity/3";

/// Seconds the lengths may differ by before a source counts as damaged:
/// containers round lengths, and Opus skips its first few milliseconds.
pub const SLACK_S: f64 = 1.0;

/// Seconds, and the share of the length, a source may fall short of the
/// length its site states, the more of the two: sites round to the
/// second, and a long stream's stated length is its video's.
const STATED_SLACK: (f64, f64) = (2.0, 0.01);

/// A gap between packets longer than this is a hole: five Opus packets.
const HOLE_S: f64 = 0.1;

/// Ticks past which a packet's listed duration wrapped around 32 bits: a
/// negative one, as an Ogg page whose granule ends inside its last packet
/// trims that packet, listed unsigned. A Vorbis file remuxed with
/// `-c copy` listed its last packet as 4,294,966,620 ticks, 2^32 less 676,
/// and read as lacking 97,392 s of its 207 s.
const WRAP: i64 = 1 << 32;

/// Seconds no audio packet lasts: the longest frames of any codec are a
/// FLAC block of 65,535 samples, 1.5 s at 44.1 kHz. A packet listed as
/// longer says nothing of where it ends, and counts as one listing no
/// duration.
const LONGEST_PACKET_S: f64 = 10.0;

/// What a source's audio was found to hold, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    /// The audio stream's own length, where it states one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<f64>,
    /// Whether the file states no length, so that the one ffprobe gave
    /// was a guess, left out of `said`, and the audio plays to where its
    /// packets end.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub guessed: bool,
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
    /// Whole in itself, its packets end at `ends` where its site states
    /// `stated`: a download stopped while it was written out, as ffmpeg
    /// ends a remux it is told to stop.
    Short { stated: f64, ends: f64 },
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

    /// [`Self::damage`], and a source whose site `stated` a length it
    /// falls well short of.
    #[must_use]
    pub fn damage_against(&self, stated: Option<f64>) -> Option<Damage> {
        self.damage().or_else(|| {
            let (_, ends) = self.span?;
            let stated = stated.filter(|s| s.is_finite())?;
            let slack = STATED_SLACK.0.max(stated * STATED_SLACK.1);
            (stated - ends > slack).then_some(Damage::Short { stated, ends })
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
            Self::Short { stated, ends } => format!(
                "{key} holds {ends:.0} s of audio where its site says {stated:.0} s: \
                 cut short, as a download stopped while it was written out"
            ),
        }
    }

    /// Milliseconds of the recording it lacks: none for a cut, whose
    /// missing end comparing it with another source shows already.
    #[must_use]
    pub fn missing_ms(&self) -> i64 {
        match *self {
            Self::Cut { .. } | Self::Short { .. } => 0,
            Self::Missing { seconds, .. } => (seconds * 1000.0).round() as i64,
        }
    }

    /// Milliseconds of its audio that may be lost anywhere in it, which
    /// comparing it lets it lack ([`crate::align::align_lacking`]): what is
    /// missing, or what a cut stream says past its packets' end, since
    /// packets a demuxer skips as unreadable end the stream that much
    /// early; and [`SLACK_S`] more, as far as the lengths may be off. An
    /// MP3 file with a run of its bytes zeroed said 2.51 s past its end and
    /// had lost 2.61 s.
    #[must_use]
    pub fn lost_ms(&self) -> i64 {
        let lost = match *self {
            Self::Cut { said, ends } => said - ends,
            Self::Short { .. } => return 0,
            Self::Missing { seconds, .. } => seconds,
        };
        ((lost + SLACK_S) * 1000.0).round() as i64
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
        let seconds = |ticks: i64| ticks as f64 * num / den;
        let length = |ticks: i64| {
            let ticks = if (WRAP / 2..WRAP).contains(&ticks) {
                ticks - WRAP
            } else {
                ticks
            };
            if seconds(ticks) > LONGEST_PACKET_S {
                0
            } else {
                ticks.max(0)
            }
        };
        let mut packets: Vec<(i64, i64)> = framecrc
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| {
                let mut fields = l.split(',').skip(2).map(|f| f.trim().parse::<i64>().ok());
                Some((fields.next()??, length(fields.next()??)))
            })
            .collect();
        packets.sort_unstable();
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
    /// The audio stream's own length, where the file states one.
    pub said: Option<f64>,
    /// Whether ffprobe only guessed it ([`crate::probe::Probed::guessed`]).
    pub guessed: bool,
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
        guessed: found.guessed,
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
    fn a_file_whole_in_itself_but_short_of_its_sites_length_is_cut_short() {
        let measure = Measure {
            said: Some(143.78),
            guessed: false,
            decoded: 143.78,
            span: Some((0.0, 143.78)),
            holes: 0.0,
            unfinite: 0,
        };
        assert_eq!(measure.damage(), None, "every length the file holds agrees");
        assert_eq!(
            measure.damage_against(Some(198.0)),
            Some(Damage::Short {
                stated: 198.0,
                ends: 143.78
            })
        );
        assert_eq!(measure.damage_against(Some(145.0)), None, "a site rounds");
        assert_eq!(measure.damage_against(None), None);
        let stream = Measure {
            said: Some(10_750.0),
            decoded: 10_750.0,
            span: Some((0.0, 10_750.0)),
            ..measure
        };
        assert_eq!(
            stream.damage_against(Some(10_800.0)),
            None,
            "a share of a long one"
        );
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

    #[test]
    fn a_duration_no_packet_lasts_ends_nothing() {
        // The last packet of a remuxed Ogg file, trimmed by its page's
        // granule, listed 7 ms short of 2^32 ticks.
        let mut trimmed = whole(10);
        trimmed.push((9993, WRAP - 7));
        let t = Timeline::parse(&list(&trimmed)).unwrap();
        assert!((t.end - 9.993).abs() < 1e-9, "{t:?}");
        assert_eq!(measured(Some(10.0), &trimmed, 10.0).damage(), None);
        let mut long = whole(10);
        long.push((9993, 86_400_000));
        assert!((Timeline::parse(&list(&long)).unwrap().end - 9.993).abs() < 1e-9);
        let blocks: Vec<(i64, i64)> = (0..10).map(|n| (n * 1486, 1486)).collect();
        assert!(
            (Timeline::parse(&list(&blocks)).unwrap().end - 14.86).abs() < 1e-9,
            "FLAC's longest blocks"
        );
    }

    fn measured(said: Option<f64>, packets: &[(i64, i64)], decoded: f64) -> Measure {
        let found = Found {
            said,
            guessed: false,
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
        assert_eq!((cut.lost_ms(), holes.lost_ms()), (270_000, 11_600));
        let short = Damage::Short {
            stated: 200.0,
            ends: 150.0,
        };
        assert_eq!(
            short.lost_ms(),
            0,
            "it lacks its end, which comparing finds"
        );
        let corrupt = Damage::Missing {
            seconds: 56.0,
            holes: 0.0,
        };
        assert!(corrupt.describe(&key).contains("does not decode"));
        assert_eq!(measure(None, 100, 0, &Found::default()), None);
    }

    /// What the pass measures of audio holding samples that are no number,
    /// beside packets whole, gapped, out of order, listed with durations
    /// no packet lasts, or none, and a length stated or guessed, with
    /// [`METHOD`].
    const WRITTEN: (&str, &str) = (
        "integrity/3",
        "d0259807b05e7a7aae0d2eac016784282e4f2a5ab217283013f2ccc18429789d",
    );

    #[test]
    fn the_method_rises_with_what_it_makes() {
        use crate::analysis::{Kind, Pass};

        let mut samples = crate::testing::sine(8000, 2, 440.0, 0.5, 3.0);
        samples[100] = f32::NAN;
        samples[2001] = f32::INFINITY;
        let wav = crate::testing::wav(8000, 2, None, &samples);
        let gapped: Vec<(i64, i64)> = whole(3)
            .into_iter()
            .filter(|(pts, _)| !(1000..1150).contains(pts) && !(2000..2080).contains(pts))
            .collect();
        let mut trimmed = whole(3);
        trimmed.push((2993, WRAP - 7));
        trimmed.push((3013, 86_400_000));
        let packets = [
            Some(whole(3)),
            Some(gapped),
            Some(vec![(60, 0), (0, 20), (20, 20), (500, 20)]),
            None,
            Some(trimmed),
        ];
        let mut digest = crate::testing::Digest::default();
        for (said, packets) in [Some(3.5), None, Some(2.0), Some(3.0), Some(3.0)]
            .into_iter()
            .zip(packets)
        {
            let found = Found {
                said,
                guessed: said.is_none(),
                timeline: packets.and_then(|p| Timeline::parse(&list(&p))),
            };
            let mut pass = Pass::new(vec![Kind::Integrity]);
            pass.take(&wav);
            digest.debug(&pass.finish(&found).integrity);
        }
        crate::testing::check(WRITTEN, METHOD, &digest.hex());
    }
}
