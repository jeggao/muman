//! Whether two recordings are one, and how far apart they start: an
//! upload and its YouTube Music track, compared by their loudness over
//! time.
//!
//! Each side is decoded to 8 kHz mono and reduced to a log-energy
//! envelope at 100 frames a second, its samples centred on their mean
//! first: an old rip's converter may hold its silence off centre, which
//! would measure as steady sound 50 dB down. A cross-correlation over the
//! whole of both finds the shift: coarse, at a quarter of the rate, over
//! every shift that places the shorter within the longer and up to a
//! minute past either end, then refined. An excerpt may start anywhere in
//! its song, and a video's intro run past a minute; so that hour-long
//! files cost no more than a fraction of a second, the coarse rate halves
//! until the search correlates at most 10^8 frames, and the shift found
//! is refined at twice the rate in turn. Ten-second windows of the
//! upload, each searched again within 1 s of it, find their own, and a
//! line through them, the median of their pairwise slopes, says how the
//! shift drifts: a copy played 0.1% fast drifts 240 ms in four minutes.
//! The two are one recording when the whole, the drift undone, correlates
//! at 0.8 or more and at least 80% of the windows lie on the line, each
//! within 50 ms and correlating at 0.5. Only windows that shift places
//! inside the track count: one past its edge reaches the track only by
//! moving off the shift, and of a clip's few windows those were a third.
//!
//! Measured on uploads paired with their releases, one recording scored
//! 0.991 to 0.997 with 97% to 100% of its windows agreeing, a music video
//! starting 920 ms late and an upload with an outro included; an
//! unrelated song scored 0.289 with none.
//!
//! A line is the model: a cut that inserts a scene mid-song lowers the
//! windows' agreement and fails, where a piecewise map could keep it. A
//! drift that moves the shift less than 50 ms over the whole upload is
//! none, and a line that leaves the two other recordings is not kept, so
//! anything but a drifting copy keeps the one shift it always had.
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

use std::ffi::OsString;
use std::path::Path;

use crate::download::Passed;

/// Names the method and its constants. A stored match made by another
/// is made again, so changing anything below means changing this.
pub const METHOD: &str = "envelope-xcorr/8";

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
/// The most frames the coarse search correlates, over every shift.
const COARSE_BUDGET: usize = 100_000_000;
/// The least overlap a shift is scored on, in frames.
const MIN_OVERLAP: usize = 1000;
const WINDOW: usize = 1000;
const WINDOW_HOP: usize = 500;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// How much later the upload plays a moment than the track does.
    pub offset_ms: i64,
    /// How much longer the upload plays the recording than the track, in
    /// parts per million: negative when it runs fast. The offset then holds
    /// at the track's first moment.
    pub stretch_ppm: i64,
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
    let mut cmd: Vec<OsString> = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-i",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.push(path.as_os_str().to_os_string());
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

/// One video's audio alone, the smallest download that serves a
/// comparison, into `dir` as `<id>.<ext>`; prints the path.
#[must_use]
pub fn audio_command(url: &str, id: &str, dir: &Path, passed: &Passed) -> Vec<OsString> {
    let mut cmd: Vec<OsString> = [
        "yt-dlp",
        "--ignore-config",
        "--quiet",
        "--no-warnings",
        "--no-playlist",
        "--format",
        "bestaudio[acodec=opus]/bestaudio",
        "--print",
        "after_move:filepath",
        "--output",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    cmd.push(dir.join(format!("{id}.%(ext)s")).into_os_string());
    crate::download::ending(cmd, passed, url)
}

/// Milliseconds per envelope frame.
const FRAME_MS: i64 = 10;

/// A count as a float. Frame and sample counts stay far below 2^52.
fn real(count: usize) -> f64 {
    count as f64
}

/// The log-energy envelope of 16-bit little-endian samples, centred on
/// their mean first, which takes out a DC offset.
#[must_use]
pub fn envelope(pcm: &[u8]) -> Vec<f64> {
    let (samples, _) = pcm.as_chunks::<2>();
    let value = |s: &[u8; 2]| f64::from(i16::from_le_bytes(*s));
    let mean = samples.iter().map(value).sum::<f64>() / real(samples.len().max(1));
    samples
        .as_chunks::<HOP>()
        .0
        .iter()
        .map(|frame| {
            let energy = frame.iter().map(|s| (value(s) - mean).powi(2)).sum::<f64>() / real(HOP);
            energy.sqrt().ln_1p()
        })
        .collect()
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
/// places them. Silence padding the same recording counts for nothing.
///
/// An end is a cut when the other plays the song on past it: from the
/// last moment the one ending there plays at full sound, the other keeps
/// playing at full sound, with no break longer than a rest, to past it. A
/// start is a cut so too, and only in a recording cut at its end. The
/// other's sound past a cut is the song, which the cut one lacks; past any
/// other edge it is beyond the song.
#[must_use]
pub fn outside(upload: &[f64], track: &[f64], at: &Alignment) -> Outside {
    let lag = at.offset_ms / FRAME_MS;
    let stretch = at.stretch();
    let upload_at = |t: i64| lag as f64 + stretch * t as f64;
    let track_at = |u: i64| ((u - lag) as f64 / stretch).floor() as i64;
    let (u_len, t_len) = (
        i64::try_from(upload.len()).unwrap_or(i64::MAX),
        i64::try_from(track.len()).unwrap_or(i64::MAX),
    );
    let upload = Levels::new(upload, |u| (0..t_len).contains(&track_at(u)));
    let track = Levels::new(track, |t| (0.0..u_len as f64).contains(&upload_at(t)));
    let end = ((t_len as f64 * stretch).round() as i64).saturating_add(lag);
    let on_upload = |t: i64| upload_at(t).round() as i64;
    let track_cut_end = track
        .full_edge(true)
        .is_some_and(|t| upload.plays_on(on_upload(t), end, 1));
    let track_cut_start = track_cut_end
        && track
            .full_edge(false)
            .is_some_and(|t| upload.plays_on(on_upload(t), lag - 1, -1));
    let upload_cut_end = upload
        .full_edge(true)
        .is_some_and(|u| track.plays_on(track_at(u), track_at(u_len), 1));
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
        found.lacks_ms += track.audible(|t| upload_at(t) >= u_len as f64);
    }
    found
}

/// Pearson correlation of two runs over their common length; 0 when
/// either is flat.
fn ncc(left: &[f64], right: &[f64]) -> f64 {
    let len = left.len().min(right.len());
    if len == 0 {
        return 0.0;
    }
    let (left, right) = (&left[..len], &right[..len]);
    let mean = |run: &[f64]| run.iter().sum::<f64>() / real(len);
    let (mean_l, mean_r) = (mean(left), mean(right));
    let (mut cross, mut sq_l, mut sq_r) = (0.0, 0.0, 0.0);
    for (l, r) in left.iter().zip(right) {
        let (dl, dr) = (l - mean_l, r - mean_r);
        cross += dl * dr;
        sq_l += dl * dl;
        sq_r += dr * dr;
    }
    if sq_l == 0.0 || sq_r == 0.0 {
        0.0
    } else {
        cross / (sq_l * sq_r).sqrt()
    }
}

/// The correlation of `upload` against `track` with the upload `lag`
/// frames later, over their overlap; `None` below the least overlap.
fn score_at(upload: &[f64], track: &[f64], lag: isize, min_overlap: usize) -> Option<f64> {
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

fn decimate(x: &[f64], factor: usize) -> Vec<f64> {
    x.chunks_exact(factor)
        .map(|c| c.iter().sum::<f64>() / real(factor))
        .collect()
}

/// The shift among `lags`, in frames of `factor` averaged, at which
/// `upload` correlates best with `track`.
fn search(
    upload: &[f64],
    track: &[f64],
    factor: usize,
    lags: std::ops::RangeInclusive<isize>,
) -> Option<(isize, f64)> {
    let (u, t) = (decimate(upload, factor), decimate(track, factor));
    best(lags.filter_map(|lag| Some((lag, score_at(&u, &t, lag, MIN_OVERLAP / factor)?))))
}

/// The shift `upload` plays `track` at: searched coarse over every shift
/// that places the shorter within the longer and up to [`MAX_PAST`] past
/// either end, as coarse as keeps that under `budget` frames correlated,
/// then refined at twice the rate in turn to the full one.
fn shift(upload: &[f64], track: &[f64], budget: usize) -> Option<(isize, f64)> {
    let longer = upload.len().cast_signed() - track.len().cast_signed();
    let (lo, hi) = (
        longer.min(0) - MAX_PAST.cast_signed(),
        longer.max(0) + MAX_PAST.cast_signed(),
    );
    let span = (hi - lo).unsigned_abs();
    let shorter = upload.len().min(track.len());
    let mut factor = COARSE;
    while (span / factor) * (shorter / factor) > budget {
        factor *= 2;
    }
    let step = factor.cast_signed();
    let mut found = search(upload, track, factor, lo.div_euclid(step)..=hi / step)?;
    while factor > 1 {
        let finer = if factor > COARSE { factor / 2 } else { 1 };
        let scale = (factor / finer).cast_signed();
        let (lag, _) = found;
        found = search(
            upload,
            track,
            finer,
            lag * scale - 2 * scale..=lag * scale + 2 * scale,
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

/// The upload's windows, each searched within [`WINDOW_SEARCH`] of the
/// shift `guess` gives for its middle; flat windows say nothing, nor do
/// those that shift leaves outside the track.
fn windows(upload: &[f64], track: &[f64], guess: impl Fn(f64) -> isize) -> Vec<Window> {
    let mut found = Vec::new();
    for at in (0..upload.len().saturating_sub(WINDOW - 1)).step_by(WINDOW_HOP) {
        let window = &upload[at..at + WINDOW];
        if window.iter().all(|v| (v - window[0]).abs() < 1e-9) {
            continue;
        }
        let middle = real(at + WINDOW / 2);
        let reach = WINDOW_SEARCH.cast_signed();
        let centre = guess(middle);
        let inside = usize::try_from(at.cast_signed() - centre)
            .is_ok_and(|from| from + WINDOW <= track.len());
        if !inside {
            continue;
        }
        let best = best((centre - reach..=centre + reach).filter_map(|l| {
            let from = usize::try_from(at.cast_signed() - l).ok()?;
            Some((l, ncc(window, track.get(from..from + WINDOW)?)))
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
fn stretched_score(upload: &[f64], track: &[f64], start: f64, stretch: f64) -> Option<f64> {
    let (mut u, mut t) = (Vec::new(), Vec::new());
    for (i, level) in upload.iter().enumerate() {
        let at = (real(i) - start) / stretch;
        if at < 0.0 || at > real(track.len() - 1) {
            continue;
        }
        let k = at.floor() as usize;
        let next = track.get(k + 1).copied().unwrap_or(track[k]);
        u.push(*level);
        t.push(track[k] + (next - track[k]) * (at - real(k)));
    }
    (u.len() >= MIN_OVERLAP).then(|| ncc(&u, &t))
}

/// Where the upload's envelope sits against the track's, and how it
/// drifts; `None` when they overlap too little to say.
#[must_use]
pub fn align(upload: &[f64], track: &[f64]) -> Option<Alignment> {
    let (lag, score) = shift(upload, track, COARSE_BUDGET)?;
    let one_shift = |_: f64| lag as f64;
    let flat = windows(upload, track, |_| lag);
    let steady = Alignment {
        offset_ms: i64::try_from(lag).ok()? * FRAME_MS,
        stretch_ppm: 0,
        score,
        coverage: agreeing(&flat, one_shift),
    };
    let drifts = |slope: f64| (slope * real(upload.len())).abs() >= real(WINDOW_TOLERANCE);
    let Some((intercept, slope)) = line(&flat).filter(|(_, s)| drifts(*s)) else {
        return Some(steady);
    };
    // Searched again along the line, windows near the ends stay in reach
    // however far the shift has drifted from its value over the whole.
    let followed = windows(upload, track, |at| {
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
    let Some(score) = stretched_score(upload, track, start, stretch) else {
        return Some(steady);
    };
    let drifting = Alignment {
        offset_ms: (start.round() as i64) * FRAME_MS,
        stretch_ppm: crate::units::ppm_of_ratio(stretch - 1.0).round() as i64,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let (lag, score) = shift(&upload, &track, 100_000).unwrap();
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

    /// As loud as music is in the envelope; silence is zero.
    fn loud(e: Vec<f64>) -> Vec<f64> {
        e.into_iter().map(|v| v + 7.0).collect()
    }

    fn at(offset_ms: i64) -> Alignment {
        Alignment {
            offset_ms,
            stretch_ppm: 0,
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
        assert_eq!(outside(&upload, &track, &at(3000)), Outside::default());
    }

    fn song_levels() -> Vec<f64> {
        song(4000, 7).into_iter().map(|l| 0.5 + l).collect()
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
        assert!(e.iter().all(|v| *v == 0.0));
    }
}
