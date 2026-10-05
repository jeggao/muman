//! Whether two recordings are one, and how far apart they start: an
//! upload and its YouTube Music track, compared by their loudness over
//! time.
//!
//! Each side is decoded to 8 kHz mono and reduced to a log-energy
//! envelope at 100 frames a second. A cross-correlation over the whole
//! of both, coarse at a quarter of the rate and then refined, finds the
//! shift; ten-second windows of the upload, each searched again within
//! 1 s of it, say whether that one shift holds throughout. The two are
//! one recording when the whole correlates at 0.8 or more and at least
//! 80% of the windows agree, each within 50 ms and correlating at 0.5.
//!
//! Measured on uploads paired with their releases, one recording scored
//! 0.991 to 0.997 with 97% to 100% of its windows agreeing, a music video
//! starting 920 ms late and an upload with an outro included; an
//! unrelated song scored 0.289 with none.
//!
//! A single offset is the model: a cut that inserts a scene mid-song
//! lowers the windows' agreement and fails, where a piecewise map could
//! keep it.
//!
//! The same envelopes say how long a source plays sound outside the
//! stretch it shares with the other: frames within 40 dB of its median
//! loudness, so a video's intro or skit counts and silence padding the
//! same recording does not. That is what ranks a release over its music
//! video. Measured, music videos carried 1.2 to 17.9 s of it; an upload
//! 3.8 s longer than its release carried 60 ms.

use std::ffi::OsString;
use std::path::Path;

/// Names the method and its constants. A stored match made by another
/// is made again, so changing anything below means changing this.
pub const METHOD: &str = "envelope-xcorr/2";

const SAMPLE_RATE: usize = 8000;
/// Samples per envelope frame: 10 ms.
const HOP: usize = 80;
/// The longest intro or outro a music video may add, in frames.
const MAX_SHIFT: usize = 6000;
/// The coarse search averages this many frames into one.
const COARSE: usize = 4;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// How much later the upload plays a moment than the track does.
    pub offset_ms: i64,
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
pub fn audio_command(url: &str, id: &str, dir: &Path) -> Vec<OsString> {
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
    cmd.push("--".into());
    cmd.push(url.into());
    cmd
}

/// Milliseconds per envelope frame.
const FRAME_MS: i64 = 10;

/// A count as a float. Frame and sample counts stay far below 2^52.
#[allow(clippy::cast_precision_loss)]
fn real(count: usize) -> f64 {
    count as f64
}

/// The log-energy envelope of 16-bit little-endian samples.
#[must_use]
pub fn envelope(pcm: &[u8]) -> Vec<f64> {
    pcm.as_chunks::<{ HOP * 2 }>()
        .0
        .iter()
        .map(|frame| {
            let energy = frame
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| f64::from(i16::from_le_bytes([s[0], s[1]])).powi(2))
                .sum::<f64>()
                / real(HOP);
            energy.sqrt().ln_1p()
        })
        .collect()
}

/// Loudness within this much of a recording's median, in the envelope's
/// natural-log units, is sound; 4.6 is 40 dB.
const AUDIBLE_BELOW_MEDIAN: f64 = 4.6;

/// How long `upload` plays sound outside the stretch it shares with
/// `track` when it is `offset_ms` later: a video's intro, skit or outro,
/// where silence padding the same recording counts for nothing.
#[must_use]
pub fn audible_outside(upload: &[f64], track_len: usize, offset_ms: i64) -> i64 {
    let mut sorted = upload.to_vec();
    sorted.sort_by(f64::total_cmp);
    let Some(median) = sorted.get(sorted.len() / 2) else {
        return 0;
    };
    let floor = median - AUDIBLE_BELOW_MEDIAN;
    let lag = offset_ms / FRAME_MS;
    let start = lag.max(0);
    let end = i64::try_from(track_len)
        .unwrap_or(i64::MAX)
        .saturating_add(lag);
    let outside = upload
        .iter()
        .zip(0_i64..)
        .filter(|(level, at)| (*at < start || *at >= end) && **level > floor)
        .count();
    i64::try_from(outside).unwrap_or(i64::MAX) * FRAME_MS
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

fn decimate(x: &[f64]) -> Vec<f64> {
    x.as_chunks::<COARSE>()
        .0
        .iter()
        .map(|c| c.iter().sum::<f64>() / real(COARSE))
        .collect()
}

/// Where the upload's envelope sits against the track's; `None` when
/// they overlap too little to say.
#[must_use]
pub fn align(upload: &[f64], track: &[f64]) -> Option<Alignment> {
    let around = |at: isize, reach: usize| (at - reach.cast_signed())..=(at + reach.cast_signed());
    let (cu, ct) = (decimate(upload), decimate(track));
    let (coarse, _) = best(
        around(0, MAX_SHIFT / COARSE)
            .filter_map(|lag| Some((lag, score_at(&cu, &ct, lag, MIN_OVERLAP / COARSE)?))),
    )?;
    let (lag, score) = best(
        around(coarse * COARSE.cast_signed(), COARSE * 2)
            .filter_map(|lag| Some((lag, score_at(upload, track, lag, MIN_OVERLAP)?))),
    )?;

    let (mut considered, mut agreeing) = (0_usize, 0_usize);
    for at in (0..upload.len().saturating_sub(WINDOW - 1)).step_by(WINDOW_HOP) {
        let window = &upload[at..at + WINDOW];
        if window.iter().all(|v| (v - window[0]).abs() < 1e-9) {
            continue;
        }
        let found = best(around(lag, WINDOW_SEARCH).filter_map(|l| {
            let from = usize::try_from(at.cast_signed() - l).ok()?;
            Some((l, ncc(window, track.get(from..from + WINDOW)?)))
        }));
        let Some((l, s)) = found else { continue };
        considered += 1;
        if l.abs_diff(lag) <= WINDOW_TOLERANCE && s >= WINDOW_SCORE {
            agreeing += 1;
        }
    }
    let coverage = if considered == 0 {
        0.0
    } else {
        real(agreeing) / real(considered)
    };
    Some(Alignment {
        offset_ms: i64::try_from(lag).ok()? * FRAME_MS,
        score,
        coverage,
    })
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
    fn a_recording_with_an_intro_aligns_by_its_length() {
        let track = song(4000, 7);
        let mut upload = song(92, 99);
        upload.extend(&track);
        let a = align(&upload, &track).unwrap();
        assert_eq!(a.offset_ms, 920);
        assert!(a.score > 0.99, "{a:?}");
        assert!(a.fits(), "{a:?}");
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

    #[test]
    fn only_sound_outside_the_shared_stretch_counts() {
        // As loud as music is in the envelope; silence is zero.
        let loud = |e: Vec<f64>| e.into_iter().map(|v| v + 7.0).collect::<Vec<_>>();
        let track = loud(song(4000, 7));
        let mut intro = loud(song(500, 99));
        intro.extend(&track);
        assert_eq!(audible_outside(&intro, track.len(), 5000), 5000);
        let mut padded = vec![0.0; 500];
        padded.extend(&track);
        assert_eq!(audible_outside(&padded, track.len(), 5000), 0);
        assert_eq!(audible_outside(&track, intro.len(), -5000), 0);
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
