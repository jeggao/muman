//! Whether two recordings are one, and how far apart they start: an
//! upload and its YouTube Music track, compared by their loudness over
//! time.
//!
//! Each side is decoded to 8 kHz mono and cut into frames of 10 ms, each
//! split by a Haar wavelet into five octave bands, from 2 to 4 kHz down
//! to under 250 Hz, the lowest less the frame's own mean. Each band's
//! energy, averaged over the 50 ms about its frame, is taken as a log
//! level: an envelope of five bands at 100 frames a second. Comparing
//! correlates every band at once, each about its own mean; the frame's
//! own level over all five says what sounds outside the stretch two
//! recordings share (below).
//!
//! - **Bands.** One envelope of the whole band says little of audio whose
//!   loudness barely moves. Noise under a held tone moved it by 0.2 dB,
//!   as much as a codec's error moves it, and the Opus copy of such a
//!   track at 96 kbit/s scored 0.76 against it, and at 192 kbit/s 0.92. In
//!   bands, where the noise moves by decibels of its own, the two scored
//!   0.999 and 1.0, while two drones of other noise under other tones
//!   scored 0.14, and 0.05 by the whole band.
//! - **Each frame's own mean.** An old rip's converter may hold its sound
//!   off centre, and a codec may take the offset out, as Opus's encoder
//!   does. An MP3 whose offset wandered between 650 and 3400 second by
//!   second, then fell to digital silence, measured that silence as steady
//!   sound 24 dB down once its samples were centred on their whole mean,
//!   and each of its Opus copies scored 0.61 to 0.64 against it; centred
//!   frame by frame, 0.999.
//! - **50 ms.** A band's energy in one frame of noise-like sound is
//!   chance, and two copies whose frames fall a few ms apart, as an
//!   excerpt's and its song's do, draw it differently. Of the excerpts
//!   measured below, 163 of 306 fitted their song frame by frame, 260
//!   averaged over 30 ms, 282 over 50 ms and 294 over 90 ms, where the
//!   whole band's one envelope fitted 251; over 90 ms, other tracks scored
//!   up to 0.87 and 0.80 where 50 ms held them to 0.84 and 0.79. The
//!   average lends the two frames at a recording's edge some of what plays
//!   past it, as a video's intro before a release's first silent frame,
//!   which costs the pair a few hundredths of their score.
//!
//! A cross-correlation over the whole of both finds the shift: coarse, at
//! a quarter of the rate, over every shift that places the shorter within
//! the longer and up to a minute past either end, then refined. An
//! excerpt may start anywhere in its song, and a video's intro run past a
//! minute; so that hour-long files cost no more than a fraction of a
//! second, the coarse rate halves until the search correlates at most
//! 10^8 levels, a band's in a frame each, and the shift found is refined
//! at twice the rate in turn. Ten-second windows of the
//! upload, each searched again within 1 s of it, find their own, and a
//! line through them, the median of their pairwise slopes, says how the
//! shift drifts: a copy played 0.1% fast drifts 240 ms in four minutes.
//! The two are one recording when the whole, the drift undone, correlates
//! at 0.8 or more and at least 80% of the windows lie on the line, each
//! within 50 ms and correlating at 0.5. Only windows that shift places
//! inside the track count: one past its edge reaches the track only by
//! moving off the shift, and of a clip's few windows those were a third.
//!
//! Measured by the whole band's one envelope on uploads paired with their
//! releases, one recording scored 0.991 to 0.997 with 97% to 100% of its
//! windows agreeing, a music video starting 920 ms late and an upload
//! with an outro included; an unrelated song scored 0.289 with none. In
//! bands, 51 tracks of a public archive, live sets and netlabel releases,
//! each against its copies as Opus at 64 to 192 kbit/s, AAC at 128 and
//! 256, Vorbis at quality 5 and MP3 at 128, all 459 scored 0.998 or more
//! with every window agreeing, where the whole band had left one track's
//! five Opus copies apart; of the 2,550 ordered pairs of different tracks
//! none fitted, the highest scoring 0.79, and 0.84 by the whole band.
//!
//! **Short recordings.** Where the shorter of the two is under a minute,
//! the least overlap is three quarters of it, up to the 10 s it is for
//! longer ones, and its windows a quarter of it, up to 5 s, so an excerpt
//! of a few seconds is still judged by several windows. And both are
//! compared detrended: each frame less the mean loudness of the 2 s
//! around it, so what correlates is how the sound moves, not a swell or a
//! fade, which any two recordings share over a few seconds. Under 5 s
//! ([`SHORTEST`]) nothing is compared, and the two are neither one
//! recording nor two ([`comparable`]).
//!
//! Measured on 58 game soundtrack pieces, two excerpts of each at 30% and
//! 60% of its length, cut through 128 kbit/s MP3, each against its whole
//! piece, its own uncut excerpt, every other piece and every other
//! excerpt of its length: compared as longer ones are, excerpts of 12 s
//! joined 71 of 13,224 excerpts of other pieces, and of 16 to 45 s 21 to
//! 48, at scores up to 0.97. Detrended, excerpts of 5 to 45 s joined none
//! of 19,836 others, the highest scoring 0.88, and the whole pieces none
//! of the 1,653 pairs of them; 113 to 115 of the 116 excerpts of 6 s and
//! longer fitted their own piece, and every one its uncut excerpt. At 4 s,
//! 3 of 19,836 others joined, and at 3 s 17. In bands, of the 51 tracks'
//! excerpts of 6, 12 and 30 s cut so, 282 of 306 fitted their own track,
//! and 251 by the whole band; none of the 15,300 pairs with another track
//! fitted, the highest scoring 0.84, where by the whole band one counting
//! voice's excerpt joined another's; nor any of 30,600 pairs of excerpts
//! of one length from different tracks, the highest scoring 0.63, and
//! 0.84 by the whole band.
//!
//! A line is the model: a cut that inserts a scene mid-song lowers the
//! windows' agreement and fails, where a piecewise map could keep it. A
//! drift that moves the shift less than 50 ms over the whole upload is
//! none, and a line that leaves the two other recordings is not kept, so
//! anything but a drifting copy keeps the one shift it always had.
//!
//! The one piecewise map is a copy's holes ([`align_lacking`]). Audio a
//! damaged copy lost decodes not as silence but not at all, so all that
//! follows a hole plays that much earlier: a FLAC file missing 1.88 s
//! halfway scored 0.77 with 48% of its windows on one shift, and an MP3
//! file missing 2.61 s 0.75 with 50%, and both failed. Where
//! [`crate::integrity`] finds a side lacking audio, the windows are
//! searched again as far as it may lack, and the shift may step, down
//! past each hole in the upload and up past each in the track, its steps
//! spanning no more than the two lack together, nor 10 s ([`MAX_LOST`]):
//! whichever steps hold the most windows within 50 ms are kept. The whole
//! then correlates with each frame placed by the step of the nearest
//! window agreeing, and the two are one recording by the same 0.8 and 80%
//! as any. On one step each, the two scored 0.997 and 0.995, every window
//! agreeing. A radio edit,
//! whose cut verse is no damage, still fails, and a copy both drifting
//! and holed keeps its one shift. The offset kept is the first step's,
//! and the end is placed by the last, so what a hole moved earlier reads
//! neither as sound beyond the song nor as a cut end; what the holes
//! lack, [`crate::integrity`] measures.
//!
//! The same envelopes say how long a source plays sound outside the
//! stretch it shares with the other: frames within 40 dB of its median
//! loudness, so a video's intro or skit counts and silence padding the
//! same recording does not. That is what ranks a release over its music
//! video. Measured, music videos carried 1.2 to 17.9 s of it; an upload
//! 3.8 s longer than its release carried 60 ms.
//!
//! An excerpt is shaped as a release is against its video, the other
//! playing on past it, so what tells them apart is whether the other
//! plays the song on past the shorter one's end. From the last moment the
//! shorter one plays at full sound, within 20 dB of the median of the
//! stretch the two share, the longer one either keeps playing at full
//! sound to past that end, with no break longer than a 100 ms rest, or
//! falls quiet as the song does. It plays on past an excerpt cut
//! mid-sound or in a short rest, and past one an editor faded out, since
//! that fade is not in the song, which holds its level where the excerpt
//! falls away; a song's own end, a fade or a stop, is in every copy, and
//! the longer one falls quiet with it before any outro. Past a cut the
//! longer one plays the song on, which the cut one lacks; past any other
//! edge, sound beyond the song. Full sound is measured against the stretch
//! the two share, so a video whose quiet skit lowers its own median holds
//! a song's fading end no longer than the release does.
//!
//! A start is weighed so only in a recording cut at its end. Sound starts
//! at once but ends by dying away: a release may open on a full chord at
//! its first sample, a video's intro running straight into it, which its
//! loudness cannot tell from an excerpt cut where the song plays on; no
//! song ends at full sound with the song playing on, so a recording cut
//! at its end is an excerpt, and its start, where the other plays on
//! across it, is cut too. Any other start is the song's own, and the
//! other's sound before it is beyond the song.
//!
//! What this cannot tell apart: an excerpt from mid-song to the song's
//! end, starting at full sound, reads as a release on its first chord,
//! and the rest of the song before it as an intro; an outro beginning
//! within 100 ms of a release's last moment at full sound, as after one
//! that stops dead on its last sample or laid over its fade, reads as the
//! song played on; and an excerpt cut in a rest over 100 ms long reads as
//! whole.

use std::borrow::Cow;
use std::ffi::OsString;
use std::path::Path;

/// Names the method and its constants. A stored match made by another
/// is made again, so changing anything below means changing this.
pub const METHOD: &str = "envelope-xcorr/11";

const SAMPLE_RATE: usize = 8000;
/// Samples per envelope frame: 10 ms.
const HOP: usize = 80;
/// How far past either end of the other one recording may play, in
/// frames: the longest intro or outro a music video adds beyond a release
/// that plays out past the video's other end.
const MAX_PAST: usize = 6000;
/// The coarse search averages this many frames into one, or more where
/// that would cost over [`COARSE_BUDGET`].
const COARSE: usize = 4;
/// The most levels the coarse search correlates, a frame of each band
/// each, over every shift.
const COARSE_BUDGET: usize = 100_000_000;
/// The least overlap a shift is scored on, in frames, where the shorter
/// recording holds more than [`Scale::of`] scales it to.
const MIN_OVERLAP: usize = 1000;
/// A window's length, in frames, where the shorter recording holds more
/// than [`Scale::of`] scales it to; windows step by half their length.
const WINDOW: usize = 1000;
/// The shortest recording compared, in frames: under it, comparing says
/// nothing ([`comparable`]).
const SHORTEST: usize = 500;
/// Where the shorter recording is under this many frames, both are
/// compared by how their loudness moves about its level over
/// [`DETREND`] frames either side, not by the level itself.
const DETRENDED_UNDER: usize = 6000;
const DETREND: usize = 100;
/// A window's length, in frames, where the two are detrended.
const DETRENDED_WINDOW: usize = 500;
/// How far a window's own best shift is looked for around the global one.
const WINDOW_SEARCH: usize = 100;
/// A window agrees within 50 ms of the global shift, and correlating.
const WINDOW_TOLERANCE: usize = 5;
const WINDOW_SCORE: f64 = 0.5;
const FIT_SCORE: f64 = 0.8;
const FIT_COVERAGE: f64 = 0.8;
/// The most an upload's speed may differ from the track's, as a share.
///
/// Measured on public-domain recordings, copies played 0.1% fast had 24%
/// to 72% of their windows on one shift and failed; on the line, all of
/// them, with a drift of 1001 ppm. Copies 0.5% fast had at most 11% on one
/// shift; on the line, 98% to 100%, at 4990 to 5025 ppm, and copies 0.9%
/// and 0.99% fast as many. Other recordings, another take by the same
/// orchestra among them, kept their one shift.
const MAX_STRETCH: f64 = 0.01;
/// The most audio, in frames, the shift steps over holes in all: each
/// second more costs every window's search as much again, a file cut off
/// lacks its end, which one shift finds, and a copy lacking more ranks
/// below every whole one by its damage alone.
const MAX_LOST: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// How much later the upload plays a moment than the track does; with
    /// holes, before the first.
    pub offset_ms: i64,
    /// How much longer the upload plays the recording than the track, in
    /// parts per million: negative when it runs fast. The offset then holds
    /// at the track's first moment.
    pub stretch_ppm: i64,
    /// How far the offset steps over holes by the last moment: negative
    /// where the upload lacks audio, positive where the track does.
    pub stepped_ms: i64,
    /// Normalized cross-correlation of the envelopes at that offset.
    pub score: f64,
    /// The share of the upload's windows that agree with the offset.
    pub coverage: f64,
}

impl Alignment {
    /// Whether the two are one recording, so the upload's subtitles,
    /// moved by the offset, fit the track.
    #[must_use]
    pub fn fits(&self) -> bool {
        self.score >= FIT_SCORE && self.coverage >= FIT_COVERAGE
    }

    /// How many times as long the upload plays the recording.
    #[must_use]
    pub fn stretch(&self) -> f64 {
        1.0 + crate::units::ratio_of_ppm(self.stretch_ppm as f64)
    }
}

/// One file's first audio stream as 8 kHz mono 16-bit samples on stdout.
#[must_use]
pub fn pcm_command(path: &Path) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin"]
        .into_iter()
        .map(OsString::from)
        .collect();
    cmd.extend(crate::ffmpeg::Input::from(path).args());
    let rate = SAMPLE_RATE.to_string();
    cmd.extend(
        [
            "-map", "0:a:0", "-ac", "1", "-ar", &rate, "-f", "s16le", "-",
        ]
        .into_iter()
        .map(OsString::from),
    );
    cmd
}

/// Milliseconds per envelope frame.
const FRAME_MS: i64 = 10;

/// A count as a float. Frame and sample counts stay far below 2^52.
fn real(count: usize) -> f64 {
    count as f64
}

/// The octave bands a frame is split into, as the module says: 2 to
/// 4 kHz, 1 to 2, 0.5 to 1, 0.25 to 0.5, and under 0.25 kHz less the
/// frame's mean.
const BANDS: usize = 5;

/// One frame's loudness in each of [`BANDS`].
type Frame = [f64; BANDS];

/// A band's energy is averaged over this many frames either side, as the
/// module says.
const SMOOTH: usize = 2;

/// A recording's loudness over time, a frame each 10 ms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Envelope {
    /// Each frame's loudness over every band, its mean taken out: what
    /// [`outside`] weighs.
    levels: Vec<f64>,
    /// Each frame's loudness in each band: what comparing correlates.
    bands: Vec<Frame>,
}

impl Envelope {
    /// How many frames it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// How long it plays, in ms.
    #[must_use]
    pub fn ms(&self) -> i64 {
        i64::try_from(self.len()).unwrap_or(i64::MAX / FRAME_MS) * FRAME_MS
    }
}

/// The envelope of 16-bit little-endian samples: each frame split into
/// [`BANDS`] by a Haar wavelet, the lowest less the frame's mean, which
/// takes out a DC offset however it wanders, each band's energy as a
/// log level.
#[must_use]
pub fn envelope(pcm: &[u8]) -> Envelope {
    let (samples, _) = pcm.as_chunks::<2>();
    let level = |energy: f64| (energy / real(HOP)).sqrt().ln_1p();
    let mut found = Envelope::default();
    let mut heard: Vec<Frame> = Vec::new();
    let mut rest = [0.0; HOP];
    for frame in samples.as_chunks::<HOP>().0 {
        for (x, s) in rest.iter_mut().zip(frame) {
            *x = f64::from(i16::from_le_bytes(*s));
        }
        let mut energies = [0.0; BANDS];
        let mut len = HOP;
        for energy in &mut energies[..BANDS - 1] {
            len /= 2;
            for i in 0..len {
                let (a, b) = (rest[2 * i], rest[2 * i + 1]);
                *energy += (a - b).powi(2) / 2.0;
                rest[i] = (a + b) / std::f64::consts::SQRT_2;
            }
        }
        let low = &rest[..len];
        let mean = low.iter().sum::<f64>() / real(len);
        energies[BANDS - 1] = low.iter().map(|x| (x - mean).powi(2)).sum();
        found.levels.push(level(energies.iter().sum()));
        heard.push(energies);
    }
    let mut sums = vec![[0.0; BANDS]; heard.len() + 1];
    for (i, energies) in heard.iter().enumerate() {
        sums[i + 1] = std::array::from_fn(|b| sums[i][b] + energies[b]);
    }
    found.bands = (0..heard.len())
        .map(|i| {
            let (from, to) = (i.saturating_sub(SMOOTH), (i + SMOOTH + 1).min(heard.len()));
            std::array::from_fn(|b| level((sums[to][b] - sums[from][b]) / real(to - from)))
        })
        .collect();
    found
}

/// Loudness within this much of a recording's median, in the envelope's
/// natural-log units, is sound; 4.6 is 40 dB.
const AUDIBLE_BELOW_MEDIAN: f64 = 4.6;
/// Loudness within this much of the median of the stretch two recordings
/// share, 2.3 or 20 dB, is the song at full sound.
const FULL_BELOW_MEDIAN: f64 = 2.3;
/// The longest break in full sound that is a rest in the song, not its
/// end, in frames: 100 ms.
const REST: i64 = 10;

/// What two recordings play outside the stretch they share, each in
/// milliseconds of sound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outside {
    /// The upload's sound past an edge where the track ends of its own: a
    /// video's intro, skit or outro.
    pub extra_ms: i64,
    /// The track's sound past an edge where the upload is cut off: the
    /// song the upload lacks.
    pub lacks_ms: i64,
    /// The upload's sound past an edge where the track is cut off: the
    /// song the track lacks.
    pub track_lacks_ms: i64,
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// One recording's envelope, its median loudness, and the loudness over
/// which it plays the song it shares with the other at full sound.
struct Levels<'a> {
    frames: &'a [f64],
    median: f64,
    full: f64,
}

impl<'a> Levels<'a> {
    /// `frames`, playing the song at full sound within
    /// [`FULL_BELOW_MEDIAN`] of the median of those of them `shared` picks.
    fn new(frames: &'a [f64], shared: impl Fn(i64) -> bool) -> Self {
        let own = median(frames.to_vec()).unwrap_or(0.0);
        let song = frames
            .iter()
            .zip(0_i64..)
            .filter(|(_, at)| shared(*at))
            .map(|(level, _)| *level)
            .collect();
        Self {
            frames,
            median: own,
            full: median(song).unwrap_or(own) - FULL_BELOW_MEDIAN,
        }
    }

    fn len(&self) -> i64 {
        i64::try_from(self.frames.len()).unwrap_or(i64::MAX)
    }

    /// How long it sounds over the frames `outside` picks out.
    fn audible(&self, outside: impl Fn(i64) -> bool) -> i64 {
        let floor = self.median - AUDIBLE_BELOW_MEDIAN;
        let frames = self
            .frames
            .iter()
            .zip(0_i64..)
            .filter(|(level, at)| outside(*at) && **level > floor)
            .count();
        i64::try_from(frames).unwrap_or(i64::MAX) * FRAME_MS
    }

    /// Whether frame `at` plays at full sound; `None` past either end.
    fn is_full(&self, at: i64) -> Option<bool> {
        let level = self.frames.get(usize::try_from(at).ok()?)?;
        Some(*level > self.full)
    }

    /// Its first frame at full sound, or with `last` its last.
    fn full_edge(&self, last: bool) -> Option<i64> {
        let mut frames = 0..self.len();
        let full = |at: &i64| self.is_full(*at) == Some(true);
        if last {
            frames.rfind(full)
        } else {
            frames.find(full)
        }
    }

    /// Whether it plays on at full sound from frame `from`, stepping by
    /// `step` until frame `past` or beyond, with no break over [`REST`].
    fn plays_on(&self, from: i64, past: i64, step: i64) -> bool {
        let mut quiet = 0;
        let mut at = from + step;
        while let Some(full) = self.is_full(at) {
            if full && (at - past) * step >= 0 {
                return true;
            }
            quiet = if full { 0 } else { quiet + 1 };
            if quiet > REST {
                return false;
            }
            at += step;
        }
        false
    }
}

/// What `upload` and `track` play outside the stretch they share when `at`
/// places them, the start by its offset and the end by that stepped over
/// holes. Silence padding the same recording counts for nothing.
///
/// An end is a cut when the other plays the song on past it: from the
/// last moment the one ending there plays at full sound, the other keeps
/// playing at full sound, with no break longer than a rest, to past it. A
/// start is a cut so too, and only in a recording cut at its end. The
/// other's sound past a cut is the song, which the cut one lacks; past any
/// other edge it is beyond the song.
#[must_use]
pub fn outside(upload: &Envelope, track: &Envelope, at: &Alignment) -> Outside {
    let (upload, track) = (&upload.levels[..], &track.levels[..]);
    let lag = at.offset_ms / FRAME_MS;
    let stretch = at.stretch();
    let upload_at = |t: i64| lag as f64 + stretch * t as f64;
    let track_at = |u: i64| ((u - lag) as f64 / stretch).floor() as i64;
    let stepped = at.stepped_ms / FRAME_MS;
    let upload_end_at = |t: i64| upload_at(t) + stepped as f64;
    let track_end_at = |u: i64| track_at(u - stepped);
    let (u_len, t_len) = (
        i64::try_from(upload.len()).unwrap_or(i64::MAX),
        i64::try_from(track.len()).unwrap_or(i64::MAX),
    );
    let upload = Levels::new(upload, |u| (0..t_len).contains(&track_at(u)));
    let track = Levels::new(track, |t| (0.0..u_len as f64).contains(&upload_at(t)));
    let end = ((t_len as f64 * stretch).round() as i64).saturating_add(lag + stepped);
    let track_cut_end = track
        .full_edge(true)
        .is_some_and(|t| upload.plays_on(upload_end_at(t).round() as i64, end, 1));
    let track_cut_start = track_cut_end
        && track
            .full_edge(false)
            .is_some_and(|t| upload.plays_on(upload_at(t).round() as i64, lag - 1, -1));
    let upload_cut_end = upload
        .full_edge(true)
        .is_some_and(|u| track.plays_on(track_end_at(u), track_end_at(u_len), 1));
    let upload_cut_start = upload_cut_end
        && upload
            .full_edge(false)
            .is_some_and(|u| track.plays_on(track_at(u), track_at(-1), -1));
    let mut found = Outside::default();
    for (past, cut) in [
        (upload.audible(|u| u < lag), track_cut_start),
        (upload.audible(|u| u >= end), track_cut_end),
    ] {
        if cut {
            found.track_lacks_ms += past;
        } else {
            found.extra_ms += past;
        }
    }
    if upload_cut_start {
        found.lacks_ms += track.audible(|t| upload_at(t) < 0.0);
    }
    if upload_cut_end {
        found.lacks_ms += track.audible(|t| upload_end_at(t) >= u_len as f64);
    }
    found
}

/// Pearson correlation of two runs over their common length, every band
/// at once, each about its own mean; 0 when either is flat.
fn ncc(left: &[Frame], right: &[Frame]) -> f64 {
    let len = left.len().min(right.len());
    if len == 0 {
        return 0.0;
    }
    let (left, right) = (&left[..len], &right[..len]);
    let mean = |run: &[Frame]| {
        let mut sums = [0.0; BANDS];
        for frame in run {
            for (sum, level) in sums.iter_mut().zip(frame) {
                *sum += level;
            }
        }
        sums.map(|sum| sum / real(len))
    };
    let (mean_l, mean_r) = (mean(left), mean(right));
    let (mut cross, mut sq_l, mut sq_r) = (0.0, 0.0, 0.0);
    for (l, r) in left.iter().zip(right) {
        for band in 0..BANDS {
            let (dl, dr) = (l[band] - mean_l[band], r[band] - mean_r[band]);
            cross += dl * dr;
            sq_l += dl * dl;
            sq_r += dr * dr;
        }
    }
    if sq_l == 0.0 || sq_r == 0.0 {
        0.0
    } else {
        cross / (sq_l * sq_r).sqrt()
    }
}

/// The correlation of `upload` against `track` with the upload `lag`
/// frames later, over their overlap; `None` below the least overlap.
fn score_at(upload: &[Frame], track: &[Frame], lag: isize, min_overlap: usize) -> Option<f64> {
    let start = lag.max(0);
    let end = upload
        .len()
        .cast_signed()
        .min(track.len().cast_signed() + lag);
    if end - start < min_overlap.cast_signed() {
        return None;
    }
    let u = upload.get(start.cast_unsigned()..end.cast_unsigned())?;
    let t = track.get((start - lag).cast_unsigned()..(end - lag).cast_unsigned())?;
    Some(ncc(u, t))
}

/// The highest-scoring of `scored`, the first on a tie.
fn best(scored: impl Iterator<Item = (isize, f64)>) -> Option<(isize, f64)> {
    scored.fold(None, |best, (lag, score)| match best {
        Some((_, b)) if b >= score => best,
        _ => Some((lag, score)),
    })
}

fn decimate(x: &[Frame], factor: usize) -> Vec<Frame> {
    x.chunks_exact(factor)
        .map(|c| {
            let mut sums = [0.0; BANDS];
            for frame in c {
                for (sum, level) in sums.iter_mut().zip(frame) {
                    *sum += level;
                }
            }
            sums.map(|sum| sum / real(factor))
        })
        .collect()
}

/// The shift among `lags`, in frames of `factor` averaged, at which
/// `upload` correlates best with `track`.
fn search(
    upload: &[Frame],
    track: &[Frame],
    factor: usize,
    lags: std::ops::RangeInclusive<isize>,
    scale: Scale,
) -> Option<(isize, f64)> {
    let (u, t) = (decimate(upload, factor), decimate(track, factor));
    best(lags.filter_map(|lag| Some((lag, score_at(&u, &t, lag, scale.overlap / factor)?))))
}

/// The shift `upload` plays `track` at: searched coarse over every shift
/// that places the shorter within the longer and up to [`MAX_PAST`] past
/// either end, as coarse as keeps that under `budget` levels correlated,
/// then refined at twice the rate in turn to the full one.
fn shift(upload: &[Frame], track: &[Frame], budget: usize, scale: Scale) -> Option<(isize, f64)> {
    let longer = upload.len().cast_signed() - track.len().cast_signed();
    let (lo, hi) = (
        longer.min(0) - MAX_PAST.cast_signed(),
        longer.max(0) + MAX_PAST.cast_signed(),
    );
    let span = (hi - lo).unsigned_abs();
    let shorter = upload.len().min(track.len());
    let mut factor = COARSE;
    while (span / factor) * (shorter / factor) * BANDS > budget {
        factor *= 2;
    }
    let step = factor.cast_signed();
    let mut found = search(
        upload,
        track,
        factor,
        lo.div_euclid(step)..=hi / step,
        scale,
    )?;
    while factor > 1 {
        let finer = if factor > COARSE { factor / 2 } else { 1 };
        let ratio = (factor / finer).cast_signed();
        let (lag, _) = found;
        found = search(
            upload,
            track,
            finer,
            lag * ratio - 2 * ratio..=lag * ratio + 2 * ratio,
            scale,
        )?;
        factor = finer;
    }
    Some(found)
}

/// One window of the upload and the shift it finds for itself.
struct Window {
    /// Its middle, in upload frames.
    at: f64,
    lag: isize,
    score: f64,
}

/// The upload's windows, each searched within `reach` frames of the
/// shift `guess` gives for its middle; flat windows say nothing, nor do
/// those that shift leaves outside the track.
fn windows(
    upload: &[Frame],
    track: &[Frame],
    reach: usize,
    scale: Scale,
    guess: impl Fn(f64) -> isize,
) -> Vec<Window> {
    let mut found = Vec::new();
    let length = scale.window;
    for at in (0..upload.len().saturating_sub(length - 1)).step_by(length / 2) {
        let window = &upload[at..at + length];
        let flat = |band: usize| {
            window
                .iter()
                .all(|v| (v[band] - window[0][band]).abs() < 1e-9)
        };
        if (0..BANDS).all(flat) {
            continue;
        }
        let middle = real(at + length / 2);
        let reach = reach.cast_signed();
        let centre = guess(middle);
        let inside = usize::try_from(at.cast_signed() - centre)
            .is_ok_and(|from| from + length <= track.len());
        if !inside {
            continue;
        }
        let best = best((centre - reach..=centre + reach).filter_map(|l| {
            let from = usize::try_from(at.cast_signed() - l).ok()?;
            Some((l, ncc(window, track.get(from..from + length)?)))
        }));
        if let Some((lag, score)) = best {
            found.push(Window {
                at: middle,
                lag,
                score,
            });
        }
    }
    found
}

/// The line through the windows that correlate: the median of their
/// pairwise slopes within [`MAX_STRETCH`], and the median intercept under
/// it, as shift = intercept + slope × upload frame.
fn line(windows: &[Window]) -> Option<(f64, f64)> {
    let sure: Vec<&Window> = windows.iter().filter(|w| w.score >= WINDOW_SCORE).collect();
    let mut slopes = Vec::new();
    for (i, a) in sure.iter().enumerate() {
        for b in &sure[i + 1..] {
            let slope = (b.lag - a.lag) as f64 / (b.at - a.at);
            if slope.abs() <= MAX_STRETCH {
                slopes.push(slope);
            }
        }
    }
    let slope = median(slopes)?;
    let intercept = median(sure.iter().map(|w| w.lag as f64 - slope * w.at).collect())?;
    Some((intercept, slope))
}

/// The share of `windows` within [`WINDOW_TOLERANCE`] of `shift` and
/// correlating at [`WINDOW_SCORE`].
fn agreeing(windows: &[Window], shift: impl Fn(f64) -> f64) -> f64 {
    if windows.is_empty() {
        return 0.0;
    }
    let agree = windows
        .iter()
        .filter(|w| {
            w.score >= WINDOW_SCORE && (w.lag as f64 - shift(w.at)).abs() <= real(WINDOW_TOLERANCE)
        })
        .count();
    real(agree) / real(windows.len())
}

/// The correlation of `upload` against `track` played `stretch` times as
/// long and starting `start` frames into the upload, over their overlap.
fn stretched_score(
    upload: &[Frame],
    track: &[Frame],
    start: f64,
    stretch: f64,
    scale: Scale,
) -> Option<f64> {
    let (mut u, mut t) = (Vec::new(), Vec::new());
    for (i, level) in upload.iter().enumerate() {
        let at = (real(i) - start) / stretch;
        if at < 0.0 || at > real(track.len() - 1) {
            continue;
        }
        let k = at.floor() as usize;
        let next = track.get(k + 1).copied().unwrap_or(track[k]);
        let share = at - real(k);
        u.push(*level);
        t.push(std::array::from_fn(|b| {
            track[k][b] + (next[b] - track[k][b]) * share
        }));
    }
    (u.len() >= scale.overlap).then(|| ncc(&u, &t))
}

/// The overlap and windows two recordings are compared over, and whether
/// they are detrended, scaled to what the shorter holds.
#[derive(Debug, Clone, Copy)]
struct Scale {
    overlap: usize,
    window: usize,
    detrended: bool,
}

impl Scale {
    /// [`MIN_OVERLAP`] and [`WINDOW`], or where the shorter recording,
    /// `shorter` frames long, holds less, three quarters of it and a
    /// quarter of it, detrended under [`DETRENDED_UNDER`]; `None` under
    /// [`SHORTEST`].
    fn of(shorter: usize) -> Option<Self> {
        let detrended = shorter < DETRENDED_UNDER;
        let window = if detrended { DETRENDED_WINDOW } else { WINDOW };
        (shorter >= SHORTEST).then(|| Self {
            overlap: MIN_OVERLAP.min(shorter * 3 / 4),
            window: window.min(shorter / 4),
            detrended,
        })
    }

    /// `envelope`'s bands as they are compared at this scale.
    fn compared(self, envelope: &Envelope) -> Cow<'_, [Frame]> {
        if self.detrended {
            Cow::Owned(detrended(&envelope.bands))
        } else {
            Cow::Borrowed(&envelope.bands)
        }
    }
}

/// Each frame of `bands` less the mean of those within [`DETREND`] of
/// it, band by band.
fn detrended(bands: &[Frame]) -> Vec<Frame> {
    let mut sums = vec![[0.0; BANDS]; bands.len() + 1];
    for (i, frame) in bands.iter().enumerate() {
        sums[i + 1] = std::array::from_fn(|b| sums[i][b] + frame[b]);
    }
    bands
        .iter()
        .enumerate()
        .map(|(i, frame)| {
            let (from, to) = (
                i.saturating_sub(DETREND),
                (i + DETREND + 1).min(bands.len()),
            );
            std::array::from_fn(|b| frame[b] - (sums[to][b] - sums[from][b]) / real(to - from))
        })
        .collect()
}

/// Whether two recordings, `a_ms` and `b_ms` long, are long enough to
/// compare: where they are not, [`align`] says nothing, and neither is
/// known to be the other's recording or another.
#[must_use]
pub fn comparable(a_ms: i64, b_ms: i64) -> bool {
    let frames = usize::try_from(a_ms.min(b_ms) / FRAME_MS).unwrap_or(0);
    Scale::of(frames).is_some()
}

/// Where the upload's envelope sits against the track's, and how it
/// drifts; `None` where either is too short to compare ([`comparable`]).
#[must_use]
pub fn align(upload: &Envelope, track: &Envelope) -> Option<Alignment> {
    let scale = Scale::of(upload.len().min(track.len()))?;
    align_by(&scale.compared(upload), &scale.compared(track), scale)
}

fn align_by(upload: &[Frame], track: &[Frame], scale: Scale) -> Option<Alignment> {
    let (lag, score) = shift(upload, track, COARSE_BUDGET, scale)?;
    let one_shift = |_: f64| lag as f64;
    let flat = windows(upload, track, WINDOW_SEARCH, scale, |_| lag);
    let steady = Alignment {
        offset_ms: i64::try_from(lag).ok()? * FRAME_MS,
        stretch_ppm: 0,
        stepped_ms: 0,
        score,
        coverage: agreeing(&flat, one_shift),
    };
    let drifts = |slope: f64| (slope * real(upload.len())).abs() >= real(WINDOW_TOLERANCE);
    let Some((intercept, slope)) = line(&flat).filter(|(_, s)| drifts(*s)) else {
        return Some(steady);
    };
    // Searched again along the line, windows near the ends stay in reach
    // however far the shift has drifted from its value over the whole.
    let followed = windows(upload, track, WINDOW_SEARCH, scale, |at| {
        (intercept + slope * at).round() as isize
    });
    let Some((intercept, slope)) = line(&followed).filter(|(_, s)| drifts(*s)) else {
        return Some(steady);
    };
    let coverage = agreeing(&followed, |at| intercept + slope * at);
    // Upload frame u plays track frame t at u = start + stretch × t, and
    // the windows' shift is u − t.
    let stretch = 1.0 / (1.0 - slope);
    let start = intercept * stretch;
    let Some(score) = stretched_score(upload, track, start, stretch, scale) else {
        return Some(steady);
    };
    let drifting = Alignment {
        offset_ms: (start.round() as i64) * FRAME_MS,
        stretch_ppm: crate::units::ppm_of_ratio(stretch - 1.0).round() as i64,
        stepped_ms: 0,
        score,
        coverage,
    };
    // Between other recordings a line means nothing, and a pinned source's
    // lyrics are moved by whatever shift is kept.
    if drifting.fits() && drifting.coverage > steady.coverage {
        Some(drifting)
    } else {
        Some(steady)
    }
}

/// How much audio each side may have lost anywhere in it, in ms, as
/// [`crate::integrity`] finds a damaged source lacking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lost {
    pub upload_ms: i64,
    pub track_ms: i64,
}

/// [`align`], the shift stepping past the holes `lost` allows where one
/// shift does not fit: down where the upload lacks audio, up where the
/// track does.
#[must_use]
pub fn align_lacking(upload: &Envelope, track: &Envelope, lost: Lost) -> Option<Alignment> {
    let scale = Scale::of(upload.len().min(track.len()))?;
    let (upload, track) = (&scale.compared(upload)[..], &scale.compared(track)[..]);
    let steady = align_by(upload, track, scale)?;
    let frames = |ms: i64| usize::try_from(ms / FRAME_MS).unwrap_or(0);
    let (down, up) = (frames(lost.upload_ms), frames(lost.track_ms));
    if steady.fits() || down + up == 0 {
        return Some(steady);
    }
    let lag = isize::try_from(steady.offset_ms / FRAME_MS).ok()?;
    let span = (down + up).min(MAX_LOST) + WINDOW_TOLERANCE;
    let found = windows(upload, track, WINDOW_SEARCH + span, scale, |_| lag);
    let placed = steps(&found, down > 0, up > 0, span.cast_signed());
    let (Some((_, start)), Some((_, end)), Some(score)) = (
        placed.first(),
        placed.last(),
        stepped_score(upload, track, &placed, scale),
    ) else {
        return Some(steady);
    };
    let ms = |frames: isize| i64::try_from(frames).unwrap_or(0) * FRAME_MS;
    let stepped = Alignment {
        offset_ms: ms(*start),
        stretch_ppm: 0,
        stepped_ms: ms(end - start),
        score,
        coverage: real(placed.len()) / real(found.len()),
    };
    Some(if stepped.fits() && stepped.coverage > steady.coverage {
        stepped
    } else {
        steady
    })
}

/// The windows agreeing with the steps that hold the most of them, each
/// with its step: a shift held over runs of windows, stepping down only
/// with `down` and up only with `up`, its steps within `span` frames.
fn steps(windows: &[Window], down: bool, up: bool, span: isize) -> Vec<(f64, isize)> {
    let sure: Vec<&Window> = windows.iter().filter(|w| w.score >= WINDOW_SCORE).collect();
    let mut levels: Vec<isize> = sure.iter().map(|w| w.lag).collect();
    levels.sort_unstable();
    levels.dedup();
    let near = |w: &Window, level: isize| w.lag.abs_diff(level) <= WINDOW_TOLERANCE;
    let mut best: Vec<(f64, isize)> = Vec::new();
    for &lo in &levels {
        let allowed: Vec<isize> = levels
            .iter()
            .copied()
            .filter(|l| (lo..=lo + span).contains(l))
            .collect();
        let n = allowed.len();
        // held[c]: the windows so far agreeing with the best steps ending on
        // allowed[c]; back[i][c]: the step window i was on before them.
        let mut held = vec![0_usize; n];
        let mut back: Vec<Vec<usize>> = Vec::with_capacity(sure.len());
        for w in &sure {
            let more = |a: usize, b: usize| if held[b] > held[a] { b } else { a };
            let mut below = Vec::with_capacity(n);
            for c in 0..n {
                below.push(below.last().map_or(c, |&b| more(b, c)));
            }
            let mut above = vec![n - 1; n];
            for c in (0..n - 1).rev() {
                above[c] = more(above[c + 1], c);
            }
            let from: Vec<usize> = (0..n)
                .map(|c| {
                    let fell = (down && c + 1 < n).then(|| above[c + 1]);
                    let rose = (up && c > 0).then(|| below[c - 1]);
                    fell.into_iter().chain(rose).fold(c, more)
                })
                .collect();
            held = (0..n)
                .map(|c| held[from[c]] + usize::from(near(w, allowed[c])))
                .collect();
            back.push(from);
        }
        let Some(mut c) = (0..n).reduce(|b, c| if held[c] > held[b] { c } else { b }) else {
            continue;
        };
        if held[c] <= best.len() {
            continue;
        }
        let mut placed = Vec::with_capacity(held[c]);
        for (w, from) in sure.iter().zip(&back).rev() {
            if near(w, allowed[c]) {
                placed.push((w.at, allowed[c]));
            }
            c = from[c];
        }
        placed.reverse();
        best = placed;
    }
    best
}

/// The correlation of `upload` against `track`, each upload frame placed
/// by the step of the nearest of `placed`.
fn stepped_score(
    upload: &[Frame],
    track: &[Frame],
    placed: &[(f64, isize)],
    scale: Scale,
) -> Option<f64> {
    let (mut u, mut t) = (Vec::new(), Vec::new());
    let mut nearest = 0;
    for (i, level) in upload.iter().enumerate() {
        let at = real(i);
        while placed
            .get(nearest + 1)
            .is_some_and(|next| next.0 - at < at - placed[nearest].0)
        {
            nearest += 1;
        }
        let from = i.cast_signed() - placed.get(nearest)?.1;
        if let Some(heard) = usize::try_from(from).ok().and_then(|k| track.get(k)) {
            u.push(*level);
            t.push(*heard);
        }
    }
    (u.len() >= scale.overlap).then(|| ncc(&u, &t))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An envelope whose every band moves as `levels` do.
    fn flat(levels: &[f64]) -> Envelope {
        Envelope {
            levels: levels.to_vec(),
            bands: levels.iter().map(|level| [*level; BANDS]).collect(),
        }
    }

    fn align(upload: &[f64], track: &[f64]) -> Option<Alignment> {
        super::align(&flat(upload), &flat(track))
    }

    fn align_lacking(upload: &[f64], track: &[f64], lost: Lost) -> Option<Alignment> {
        super::align_lacking(&flat(upload), &flat(track), lost)
    }

    fn outside(upload: &[f64], track: &[f64], at: &Alignment) -> Outside {
        super::outside(&flat(upload), &flat(track), at)
    }

    /// A deterministic envelope, smoothed noise unique to its seed.
    fn song(frames: usize, seed: u64) -> Vec<f64> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut level = 0.0;
        (0..frames)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let noise = f64::from(u32::try_from(state >> 33).unwrap()) / f64::from(u32::MAX);
                level = 0.8 * level + 0.2 * noise;
                level
            })
            .collect()
    }

    #[test]
    fn a_short_track_inside_a_longer_upload_counts_only_its_own_windows() {
        let track = song(3400, 11);
        let mut upload = song(520, 12);
        upload.extend(&track);
        upload.extend(song(480, 13));
        let a = align(&upload, &track).unwrap();
        assert_eq!(a.offset_ms, 5200);
        assert!(a.fits(), "{a:?}");
    }

    #[test]
    fn a_recording_with_an_intro_aligns_by_its_length() {
        let track = song(4000, 7);
        let mut upload = song(92, 99);
        upload.extend(&track);
        let a = align(&upload, &track).unwrap();
        assert_eq!(a.offset_ms, 920);
        assert_eq!(a.stretch_ppm, 0);
        assert!(a.score > 0.99, "{a:?}");
        assert!(a.fits(), "{a:?}");
    }

    /// `envelope` played `stretch` times as long, by linear interpolation.
    fn stretched(envelope: &[f64], stretch: f64) -> Vec<f64> {
        let frames = (real(envelope.len() - 1) * stretch) as usize;
        (0..frames)
            .map(|i| {
                let at = real(i) / stretch;
                let k = at.floor() as usize;
                envelope[k] + (envelope[k + 1] - envelope[k]) * (at - real(k))
            })
            .collect()
    }

    #[test]
    fn a_shift_past_the_first_minute_is_found_however_long_the_two() {
        let track = song(3000, 7);
        let mut upload = song(7000, 99);
        upload.extend(&track);
        upload.extend(song(500, 98));
        let a = align(&upload, &track).unwrap();
        assert_eq!(a.offset_ms, 70_000);
        assert!(a.fits(), "{a:?}");
        let excerpt = upload[8000..9500].to_vec();
        assert_eq!(align(&upload, &excerpt).unwrap().offset_ms, 80_000);
        let scale = Scale::of(track.len()).unwrap();
        let (lag, score) =
            shift(&flat(&upload).bands, &flat(&track).bands, 500_000, scale).unwrap();
        assert_eq!(lag, 7000);
        assert!(score > 0.99, "{score}");
    }

    #[test]
    fn a_copy_played_fast_aligns_by_its_drift() {
        let track = song(24_000, 7);
        let mut upload = song(300, 99);
        upload.extend(stretched(&track, 0.999));
        let a = align(&upload, &track).unwrap();
        assert!(a.fits(), "{a:?}");
        assert!((a.stretch_ppm + 1000).abs() <= 50, "{a:?}");
        assert!((a.offset_ms - 3000).abs() <= 20, "{a:?}");
    }

    #[test]
    fn another_song_does_not_fit() {
        let a = align(&song(4000, 1), &song(4000, 2)).unwrap();
        assert!(!a.fits(), "{a:?}");
    }

    #[test]
    fn a_different_cut_in_the_middle_lowers_coverage() {
        let track = song(4000, 7);
        let mut upload = track[..2000].to_vec();
        upload.extend(song(600, 5));
        upload.extend(&track[2000..]);
        let a = align(&upload, &track).unwrap();
        assert!(a.coverage < 0.8, "{a:?}");
    }

    /// `track` with `frames` lost at each of `at`, as a damaged copy decodes.
    fn holed(track: &[f64], at: &[usize], frames: usize) -> Vec<f64> {
        let mut copy = Vec::new();
        let mut from = 0;
        for &hole in at {
            copy.extend(&track[from..hole]);
            from = hole + frames;
        }
        copy.extend(&track[from..]);
        copy
    }

    #[test]
    fn a_copy_with_holes_fits_by_steps_as_far_as_it_lacks() {
        let track = song(21_000, 7);
        let copy = holed(&track, &[10_900], 190);
        assert!(!align(&copy, &track).unwrap().fits());
        let lost = |upload_ms, track_ms| Lost {
            upload_ms,
            track_ms,
        };
        let a = align_lacking(&copy, &track, lost(1900, 0)).unwrap();
        assert!(a.fits(), "{a:?}");
        assert_eq!((a.offset_ms, a.stepped_ms), (0, -1900));
        assert_eq!(outside(&copy, &track, &a), Outside::default());
        let back = align_lacking(&track, &copy, lost(0, 1900)).unwrap();
        assert!(back.fits(), "{back:?}");
        assert_eq!((back.offset_ms, back.stepped_ms), (0, 1900));
        assert_eq!(outside(&track, &copy, &back), Outside::default());
        assert!(!align_lacking(&copy, &track, lost(0, 1900)).unwrap().fits());
        assert!(!align_lacking(&copy, &track, lost(500, 0)).unwrap().fits());
        let twice = holed(&track, &[5000, 15_000], 100);
        assert!(align_lacking(&twice, &track, lost(2000, 0)).unwrap().fits());
    }

    #[test]
    fn a_cut_verse_is_no_hole() {
        let track = song(21_000, 7);
        let edit = holed(&track, &[10_000], 2000);
        let lost = Lost {
            upload_ms: 1900,
            track_ms: 0,
        };
        assert!(!align_lacking(&edit, &track, lost).unwrap().fits());
    }

    /// As loud as music is in the envelope; silence is zero.
    fn loud(e: Vec<f64>) -> Vec<f64> {
        e.into_iter().map(|v| v + 7.0).collect()
    }

    fn at(offset_ms: i64) -> Alignment {
        Alignment {
            offset_ms,
            stretch_ppm: 0,
            stepped_ms: 0,
            score: 1.0,
            coverage: 1.0,
        }
    }

    /// A whole song, `frames` of sound between 100 ms of quiet at each end.
    fn whole(frames: usize, seed: u64) -> Vec<f64> {
        let mut track = vec![0.0; 10];
        track.extend(loud(song(frames, seed)));
        track.extend([0.0; 10]);
        track
    }

    #[test]
    fn only_sound_outside_the_shared_stretch_counts() {
        let track = whole(4000, 7);
        let mut intro = loud(song(500, 99));
        intro.extend(&track);
        let found = outside(&intro, &track, &at(5000));
        assert_eq!((found.extra_ms, found.track_lacks_ms), (5000, 0));
        let mut padded = vec![0.0; 500];
        padded.extend(&track);
        assert_eq!(outside(&padded, &track, &at(5000)), Outside::default());
        assert_eq!(outside(&track, &intro, &at(-5000)), Outside::default());
        let sudden = loud(song(4000, 7));
        let mut paused = loud(song(500, 99));
        paused.extend([0.0; 20]);
        paused.extend(&sudden);
        assert_eq!(outside(&paused, &sudden, &at(5200)).extra_ms, 5000);
    }

    #[test]
    fn past_an_excerpt_s_cut_the_song_plays_on() {
        let full = whole(4000, 7);
        let excerpt = full[..2000].to_vec();
        let found = outside(&full, &excerpt, &at(0));
        assert_eq!(found.extra_ms, 0);
        assert_eq!(found.track_lacks_ms, 20_100);
        let back = outside(&excerpt, &full, &at(0));
        assert_eq!((back.extra_ms, back.lacks_ms), (0, found.track_lacks_ms));
        let middle = full[1000..3000].to_vec();
        let found = outside(&full, &middle, &at(10_000));
        assert_eq!(found.extra_ms, 0);
        assert_eq!(found.track_lacks_ms, 20_000);
    }

    /// `levels` falling away over their last `frames`, to 52 dB down.
    fn faded(mut levels: Vec<f64>, frames: usize) -> Vec<f64> {
        let from = levels.len() - frames;
        for (i, level) in levels[from..].iter_mut().enumerate() {
            *level -= 6.0 * real(i + 1) / real(frames);
        }
        levels
    }

    #[test]
    fn an_edit_s_fade_or_a_cut_in_a_rest_is_a_cut() {
        let full = whole(4000, 7);
        let fade = faded(full[..2000].to_vec(), 100);
        assert_eq!(outside(&fade, &full, &at(0)).lacks_ms, 20_100);
        let mut rested = full.clone();
        rested[1998..2000].copy_from_slice(&[0.0, 0.0]);
        let found = outside(&rested[..2000], &rested, &at(0));
        assert_eq!(found.lacks_ms, 20_100);
    }

    #[test]
    fn a_song_s_own_start_or_end_is_no_cut_however_sudden() {
        // A release on its first chord at its first sample, its video's
        // intro running straight into it.
        let mut release = loud(song(4000, 7));
        release.extend([0.0; 10]);
        let mut video = loud(song(500, 99));
        video.extend(&release);
        let found = outside(&video, &release, &at(5000));
        assert_eq!((found.extra_ms, found.track_lacks_ms), (5000, 0));
        assert_eq!(outside(&release, &video, &at(-5000)), Outside::default());
        // One stopping dead on its last sample, the outro after a breath.
        let mut stop = vec![0.0; 10];
        stop.extend(loud(song(4000, 7)));
        let mut video = stop.clone();
        video.extend([0.0; 20]);
        video.extend(loud(song(500, 99)));
        assert_eq!(outside(&video, &stop, &at(0)).extra_ms, 5000);
        // One fading out, in its video too, the outro straight after.
        let fade = faded(stop, 200);
        let mut video = fade.clone();
        video.extend(loud(song(500, 99)));
        assert_eq!(outside(&video, &fade, &at(0)).extra_ms, 5000);
    }

    /// 8 kHz samples of `levels`, a square wave a frame, each around
    /// `offset`.
    fn samples(levels: &[f64], offset: i16) -> Vec<u8> {
        levels
            .iter()
            .flat_map(|l| {
                let a = (l * 6000.0) as i16;
                [offset + a, offset - a].repeat(HOP / 2)
            })
            .flat_map(i16::to_le_bytes)
            .collect()
    }

    #[test]
    fn silence_held_off_centre_by_a_dc_offset_counts_for_nothing() {
        let mut song = vec![0.0; 10];
        song.extend(song_levels());
        song.extend([0.0; 10]);
        // An old rip's 3 s lead-in, its converter 50 dB off centre.
        let mut padded = vec![0.0; 300];
        padded.extend(&song);
        let track = envelope(&samples(&song, 0));
        let upload = envelope(&samples(&padded, 100));
        assert_eq!(
            super::outside(&upload, &track, &at(3000)),
            Outside::default()
        );
    }

    /// 8 kHz samples of `seconds` of noise unique to `seed` at `noise`,
    /// under a 250 Hz tone at `tone`, each sample off centre by what
    /// `drift` gives for its second.
    fn heard(
        seconds: usize,
        seed: u64,
        (tone, noise): (f64, f64),
        drift: &dyn Fn(usize) -> f64,
    ) -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut last = 0.0;
        (0..seconds * SAMPLE_RATE)
            .map(|i| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let white = f64::from(u32::try_from(state >> 33).unwrap()) / f64::from(u32::MAX);
                let hiss = white - last;
                last = white;
                let phase = std::f64::consts::TAU * 250.0 * real(i) / real(SAMPLE_RATE);
                let sample = tone * phase.sin() + noise * hiss + drift(i / SAMPLE_RATE);
                sample.round() as i16
            })
            .flat_map(i16::to_le_bytes)
            .collect()
    }

    /// `pcm` with each sample moved by up to `by` either way, as a codec's
    /// error moves it.
    fn coded(pcm: &[u8], by: u16) -> Vec<u8> {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let (samples, _) = pcm.as_chunks::<2>();
        samples
            .iter()
            .flat_map(|s| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let error = i32::try_from((state >> 33) % u64::from(2 * by + 1)).unwrap();
                let moved = i32::from(i16::from_le_bytes(*s)) + error - i32::from(by);
                i16::try_from(moved).unwrap().to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn noise_under_a_held_tone_aligns_by_its_bands() {
        let still = |_: usize| 0.0;
        let song = heard(70, 1, (8000.0, 2500.0), &still);
        let track = envelope(&song);
        let copy = envelope(&coded(&song, 300));
        let a = super::align(&copy, &track).unwrap();
        assert!(a.fits(), "{a:?}");
        let other = envelope(&heard(70, 2, (8000.0, 2500.0), &still));
        let a = super::align(&other, &track).unwrap();
        assert!(!a.fits(), "{a:?}");
    }

    #[test]
    fn an_excerpt_cut_between_frames_fits_its_song() {
        // Noise whose loudness moves as a song's does, every 30 ms.
        let levels = song(2400, 5);
        let noise = heard(70, 4, (0.0, 20_000.0), &|_| 0.0);
        let song: Vec<u8> = noise
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                let gain = 0.1 + levels[i / (3 * HOP)];
                ((f64::from(i16::from_le_bytes(*s)) * gain).round() as i16).to_le_bytes()
            })
            .collect();
        let from = 2 * (20 * SAMPLE_RATE + HOP / 2);
        let excerpt = &song[from..from + 2 * 30 * SAMPLE_RATE];
        let a = super::align(&envelope(excerpt), &envelope(&song)).unwrap();
        assert!(a.fits(), "{a:?}");
        assert!((a.offset_ms + 20_000).abs() <= 10, "{a:?}");
    }

    #[test]
    fn a_dc_offset_that_wanders_is_no_sound() {
        // The offset an old converter holds, changing second by second,
        // then digital silence, which a codec that takes out the offset
        // leaves as it is.
        let wandering = |second: usize| [2400.0, 700.0, 3400.0, 1500.0][second % 4];
        let mut song = heard(30, 3, (6000.0, 1500.0), &wandering);
        song.extend(vec![0; 2 * 3 * SAMPLE_RATE]);
        let mut centred = heard(30, 3, (6000.0, 1500.0), &|_| 0.0);
        centred.extend(vec![0; 2 * 3 * SAMPLE_RATE]);
        let (track, copy) = (envelope(&song), envelope(&centred));
        let a = super::align(&copy, &track).unwrap();
        assert!(a.fits() && a.score > 0.99, "{a:?}");
        assert!(track.levels[3000..].iter().all(|level| *level == 0.0));
    }

    fn song_levels() -> Vec<f64> {
        song(4000, 7).into_iter().map(|l| 0.5 + l).collect()
    }

    #[test]
    fn short_copies_and_excerpts_of_one_recording_fit_down_to_five_seconds() {
        let track = song(4000, 7);
        for frames in [500, 800, 900] {
            let excerpt = track[1200..1200 + frames].to_vec();
            let a = align(&excerpt, &track).unwrap();
            assert!(a.fits(), "{frames}: {a:?}");
            assert_eq!(a.offset_ms, -12_000);
            assert!(align(&excerpt, &excerpt.clone()).unwrap().fits());
        }
    }

    #[test]
    fn short_excerpts_of_other_recordings_do_not_fit() {
        let track = song(4000, 7);
        for seed in 20..60 {
            let other = song(600, seed);
            let a = align(&other, &track).unwrap();
            assert!(!a.fits(), "{seed}: {a:?}");
            assert!(!align(&other, &track[1000..1600]).unwrap().fits());
        }
    }

    #[test]
    fn under_five_seconds_is_too_short_to_compare() {
        let track = song(4000, 7);
        assert!(align(&track[..499], &track).is_none());
        assert!(align(&track, &track[..499]).is_none());
        assert!(!comparable(4_990, 200_000));
        assert!(comparable(5_000, 5_000));
    }

    #[test]
    fn too_little_overlap_is_no_answer() {
        assert!(align(&song(100, 1), &song(100, 1)).is_none());
    }

    #[test]
    fn the_envelope_has_one_frame_per_10_ms() {
        let pcm = vec![0_u8; SAMPLE_RATE * 2];
        let e = envelope(&pcm);
        assert_eq!(e.len(), 100);
        assert!(
            e.levels
                .iter()
                .chain(e.bands.as_flattened())
                .all(|v| *v == 0.0)
        );
    }

    /// What comparing makes of a copy with an intro, off centre and
    /// played fast, of an excerpt, and of a copy with a hole, against
    /// their track, with [`METHOD`].
    const WRITTEN: (&str, &str) = (
        "envelope-xcorr/11",
        "2d5ce0e954f8c9b43003bf3af5aeb8c98247da24a3d3008d90cf6ab056b7cd94",
    );

    #[test]
    fn the_method_rises_with_what_it_makes() {
        let levels = song_levels();
        let mut fast: Vec<f64> = song(300, 99).into_iter().map(|l| 0.5 + l).collect();
        fast.extend(stretched(&levels, 0.995));
        let track = envelope(&samples(&levels, 0));
        let mut digest = crate::testing::Digest::default();
        let lost = Lost {
            upload_ms: 1500,
            track_ms: 0,
        };
        for upload in [
            samples(&fast, 100),
            samples(&levels[1000..3000], 0),
            samples(&holed(&levels, &[2000], 150), 0),
        ] {
            let upload = envelope(&upload);
            let found = super::align_lacking(&upload, &track, lost);
            assert!(found.is_some_and(|a| a.fits()), "{found:?}");
            digest
                .debug(&upload)
                .debug(&found)
                .debug(&found.map(|a| super::outside(&upload, &track, &a)));
        }
        crate::testing::check(WRITTEN, METHOD, &digest.hex());
    }
}
