//! Which songs a new source is the same recording as, by Chromaprint:
//! one 32-bit word per ~0.12 s of audio.
//!
//! The print is computed in-process by `rusty-chromaprint`, a port of
//! Chromaprint, with the algorithm ffmpeg's `chromaprint` muxer and
//! `fpcalc` use by default (TEST2). ffmpeg only decodes: it writes the
//! audio as 16-bit mono at 11,025 Hz, the rate Chromaprint works at, so
//! nothing is resampled twice and any ffmpeg build will do, where the
//! muxer needed one built with Chromaprint. Prints are of the whole
//! track, as the muxer's were; `fpcalc` stops at two minutes.
//!
//! Two prints of one recording share long stretches of words that agree
//! to within a few bits; two songs that merely sound alike agree on bits
//! only on average, and a short print slid along a long one finds such a
//! stretch by chance. So a match is counted in words that nearly agree,
//! at the offset where most do, over the words that carry sound: a run
//! of words that never change is silence or a held note, alike in any
//! two songs. The offsets tried are those exactly equal half-words vote
//! for, so a comparison costs the prints' lengths, not their product.
//!
//! | Agreeing words, of the shorter print's sounding ones | Span of the longer | Verdict |
//! |---|---|---|
//! | 30% or more, and at least 48 (~6 s) | 80% or more | The same recording |
//! | 30% or more, and at least 48 | Under 80% | Unsure: an excerpt, or a video with a skit |
//! | 10% to 30%, and at least 48 | 80% or more | Unsure: another master or mix |
//! | Less | Any | A different song |
//!
//! The thresholds were calibrated over every pair of sources in a real
//! library, printed by ffmpeg's muxer with the same algorithm. Pairs the
//! user had joined as one song scored 42% to 100%, besides one other mix
//! at 6%; no two songs of different titles scored above 14%; and every
//! pair of releases of one master scoring 30% or more was one recording.
//!
//! The measure before this one, the share of bits agreeing at the best
//! offset, failed both ways: a 16 to 40 s song slid along a five-minute
//! one found 0.62 to 0.75 by chance, and two songs sharing a silence
//! agree on every bit of it. Counted in agreeing words, those pairs score
//! 0 to 20.

use std::collections::HashMap;
use std::ffi::OsString;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Names how a print is made. A stored print made by another is made
/// again; how prints are compared is no part of it.
pub const METHOD: &str = "chromaprint-rs/1";

/// The audio one word stands for.
const SECONDS_PER_WORD: f64 = 0.1238;

/// Words within this many bits of each other agree.
const NEAR_BITS: u32 = 4;
/// A run of at least this many words, each within a bit of the one
/// before, carries nothing to tell two songs apart.
const STATIC_RUN: usize = 8;
/// The fewest agreeing words, ~6 s, a match rests on.
const MIN_WORDS: usize = 48;
/// The share of the shorter print's sounding words that agree in a
/// match.
const SHARE: f64 = 0.3;
/// A certain match also spans this share of the longer print: an
/// excerpt, or a video with a long skit, is asked about.
const COMPLETE: f64 = 0.8;
/// Prints aligned over the whole of the longer, agreeing on less than
/// [`SHARE`] but this much, are asked about: another master or mix of
/// the song.
const AKIN: f64 = 0.1;
/// Offsets verified, the most voted for.
const CANDIDATES: usize = 8;
/// A half-word found more often than this in one print votes for
/// nothing: it is too common to place anything.
const COMMON: usize = 32;

/// A source's print, kept in the state file as base64 of its words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Print(pub Vec<u32>);

impl Print {
    /// Raw little-endian words, as `-fp_format raw` writes them.
    #[must_use]
    pub fn from_raw(bytes: &[u8]) -> Self {
        Self(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        )
    }

    fn to_raw(&self) -> Vec<u8> {
        self.0.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

impl Serialize for Print {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(self.to_raw()))
    }
}

impl<'de> Deserialize<'de> for Print {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD
            .decode(text)
            .map(|b| Self::from_raw(&b))
            .map_err(serde::de::Error::custom)
    }
}

/// The rate Chromaprint analyzes audio at.
const SAMPLE_RATE: u32 = 11_025;

/// The ffmpeg output options that decode the audio stream `input:index`
/// for [`compute`]: raw 16-bit little-endian mono at Chromaprint's 11,025 Hz.
#[must_use]
pub fn output(input: usize, index: u32) -> Vec<OsString> {
    [
        "-map".to_string(),
        format!("{input}:{index}"),
        "-ac".to_string(),
        "1".to_string(),
        "-ar".to_string(),
        SAMPLE_RATE.to_string(),
        "-c:a".to_string(),
        "pcm_s16le".to_string(),
        "-f".to_string(),
        "s16le".to_string(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// The print of the audio [`output`] wrote to `pcm`, read in chunks so
/// an hour-long video never sits in memory whole.
pub fn compute(pcm: &std::path::Path) -> anyhow::Result<Print> {
    use std::io::Read;

    let config = rusty_chromaprint::Configuration::preset_test2();
    let mut printer = rusty_chromaprint::Fingerprinter::new(&config);
    printer
        .start(SAMPLE_RATE, 1)
        .map_err(|e| anyhow::anyhow!("starting the fingerprint: {e:?}"))?;
    let mut file = std::fs::File::open(pcm)?;
    let mut bytes = vec![0_u8; 1 << 16];
    let mut carry: Option<u8> = None;
    let mut samples = Vec::with_capacity(bytes.len() / 2);
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        samples.clear();
        let mut chunk = &bytes[..n];
        if let Some(low) = carry.take() {
            samples.push(i16::from_le_bytes([low, chunk[0]]));
            chunk = &chunk[1..];
        }
        let (pairs, rest) = chunk.as_chunks::<2>();
        samples.extend(pairs.iter().map(|p| i16::from_le_bytes(*p)));
        carry = rest.first().copied();
        printer.consume(&samples);
    }
    printer.finish();
    Ok(Print(printer.fingerprint().to_vec()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    /// Words that agree at the best offset.
    pub words: usize,
    /// Those over the shorter print's sounding words: how much of it is
    /// found in the other.
    pub share: f64,
    /// The overlap at that offset over the longer print's length.
    pub coverage: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Same,
    Unsure,
    Different,
}

impl Match {
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        let whole = self.coverage >= COMPLETE;
        if self.words < MIN_WORDS {
            Verdict::Different
        } else if self.share >= SHARE && whole {
            Verdict::Same
        } else if self.share >= SHARE || (self.share >= AKIN && whole) {
            Verdict::Unsure
        } else {
            Verdict::Different
        }
    }

    /// How alike, in words: the share and the seconds that agree.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{:.0}% alike, {:.0} s agreeing",
            self.share * 100.0,
            real(self.words) * SECONDS_PER_WORD
        )
    }
}

/// A count as a float. Print lengths stay far below 2^52.
#[allow(clippy::cast_precision_loss)]
fn real(n: usize) -> f64 {
    n as f64
}

/// Which words carry sound: not in a run of words that never change.
fn sounding(words: &[u32]) -> Vec<bool> {
    let mut keep = vec![true; words.len()];
    let mut start = 0;
    for i in 1..=words.len() {
        if i == words.len() || (words[i] ^ words[i - 1]).count_ones() > 1 {
            if i - start >= STATIC_RUN {
                keep[start..i].fill(false);
            }
            start = i;
        }
    }
    keep
}

/// The halves of a word, each a key of its own.
fn halves(word: u32) -> [u32; 2] {
    [word >> 16, (word & 0xffff) | 0x1_0000]
}

/// A print readied to be compared with many: which words sound, and
/// where each half-word of those is.
#[derive(Debug)]
pub struct Indexed<'a> {
    words: &'a [u32],
    keep: Vec<bool>,
    sounding: usize,
    at: HashMap<u32, Vec<usize>>,
}

impl<'a> Indexed<'a> {
    #[must_use]
    pub fn new(print: &'a Print) -> Self {
        let words = print.0.as_slice();
        let keep = sounding(words);
        let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, w) in words.iter().enumerate().filter(|(i, _)| keep[*i]) {
            for h in halves(*w) {
                at.entry(h).or_default().push(i);
            }
        }
        Self {
            words,
            sounding: keep.iter().filter(|k| **k).count(),
            keep,
            at,
        }
    }

    /// How much of this print and `other` is one recording, at the
    /// offset where most of their words agree; `None` when either has
    /// no sounding word.
    #[must_use]
    pub fn compare(&self, other: &Print) -> Option<Match> {
        let (a, b) = (self.words, other.0.as_slice());
        let keep_b = sounding(b);
        let sounding_b = keep_b.iter().filter(|k| **k).count();
        if self.sounding == 0 || sounding_b == 0 {
            return None;
        }
        let shorter = if a.len() <= b.len() {
            self.sounding
        } else {
            sounding_b
        };
        // `a[i]` against `b[i - offset]`, voted for by equal half-words.
        let mut votes: HashMap<isize, usize> = HashMap::new();
        for (j, w) in b.iter().enumerate().filter(|(j, _)| keep_b[*j]) {
            for h in halves(*w) {
                let Some(is) = self.at.get(&h).filter(|is| is.len() <= COMMON) else {
                    continue;
                };
                for i in is {
                    *votes.entry(i.cast_signed() - j.cast_signed()).or_default() += 1;
                }
            }
        }
        let mut ranked: Vec<(isize, usize)> = votes.into_iter().collect();
        ranked.sort_unstable_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
        let (words, overlap) = ranked
            .into_iter()
            .take(CANDIDATES)
            .map(|(offset, _)| self.agreeing(b, &keep_b, offset))
            .max_by_key(|(words, _)| *words)
            .unwrap_or((0, 0));
        Some(Match {
            words,
            share: real(words) / real(shorter),
            coverage: real(overlap) / real(a.len().max(b.len())),
        })
    }

    /// The words agreeing at `offset`, and the overlap's length.
    fn agreeing(&self, b: &[u32], keep_b: &[bool], offset: isize) -> (usize, usize) {
        let a = self.words;
        let lo = offset.max(0).cast_unsigned();
        let hi = a
            .len()
            .min((b.len().cast_signed() + offset).max(0).cast_unsigned());
        if hi <= lo {
            return (0, 0);
        }
        let words = (lo..hi)
            .filter(|&i| {
                let j = (i.cast_signed() - offset).cast_unsigned();
                self.keep[i] && keep_b[j] && (a[i] ^ b[j]).count_ones() <= NEAR_BITS
            })
            .count();
        (words, hi - lo)
    }
}

/// [`Indexed::compare`] for one pair.
#[must_use]
pub fn compare(a: &Print, b: &Print) -> Option<Match> {
    Indexed::new(a).compare(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize, seed: u64) -> Vec<u32> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..n)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                u32::try_from(state >> 32).unwrap()
            })
            .collect()
    }

    /// `w` with `flips` bits flipped in one half of every word, as an
    /// encoder's noise leaves a print.
    fn noisy(w: &[u32], flips: u32) -> Vec<u32> {
        w.iter()
            .enumerate()
            .map(|(i, v)| {
                let i = u32::try_from(i).unwrap();
                (0..flips).fold(*v, |v, f| v ^ 1 << ((i % 2) * 16 + (i + f * 5) % 16))
            })
            .collect()
    }

    #[test]
    fn a_noisy_offset_copy_is_the_same_song() {
        let song = words(1500, 1);
        let mut video = words(150, 9);
        video.extend(noisy(&song, 3));
        let m = compare(&Print(video), &Print(song)).unwrap();
        assert!(m.share > 0.95, "{m:?}");
        assert_eq!(m.verdict(), Verdict::Same, "{m:?}");
    }

    #[test]
    fn another_song_is_different() {
        let m = compare(&Print(words(1500, 1)), &Print(words(1400, 2))).unwrap();
        assert!(m.share < 0.05, "{m:?}");
        assert_eq!(m.verdict(), Verdict::Different);
    }

    #[test]
    fn an_excerpt_is_only_unsure() {
        let song = words(1600, 3);
        let excerpt = song[600..920].to_vec();
        let m = compare(&Print(excerpt), &Print(song)).unwrap();
        assert!(m.share > 0.99, "{m:?}");
        assert_eq!(m.verdict(), Verdict::Unsure);
    }

    #[test]
    fn a_shared_intro_is_too_little() {
        let intro = words(100, 4);
        let mut a = intro.clone();
        a.extend(words(1000, 5));
        let mut b = intro;
        b.extend(words(1000, 6));
        let m = compare(&Print(a), &Print(b)).unwrap();
        assert_eq!(m.verdict(), Verdict::Different, "{m:?}");
    }

    // A micro song against a long track scored 0.745 of bits agreeing
    // at its best offset; no word agreed within a few bits.
    #[test]
    fn bits_agreeing_on_average_are_no_match() {
        let long = words(4000, 7);
        let short = noisy(&long[1000..1300], 8);
        let m = compare(&Print(short), &Print(long)).unwrap();
        assert_eq!(m.words, 0, "{m:?}");
        assert_eq!(m.verdict(), Verdict::Different);
    }

    #[test]
    fn a_shared_silence_is_no_match() {
        let silence = vec![0x256d_f977_u32; 60];
        let mut a = words(200, 8);
        a.extend(&silence);
        a.extend(words(200, 9));
        let mut b = words(900, 10);
        b.extend(&silence);
        b.extend(words(900, 11));
        let m = compare(&Print(a), &Print(b)).unwrap();
        assert!(m.words < 4, "only chance words: {m:?}");
        assert_eq!(m.verdict(), Verdict::Different);
    }

    #[test]
    fn another_master_of_the_whole_song_is_asked_about() {
        let song = words(1500, 12);
        let master: Vec<u32> = song
            .iter()
            .enumerate()
            .map(|(i, w)| if i % 5 == 0 { *w } else { w ^ 0x0f0f_0f0f })
            .collect();
        let m = compare(&Print(master), &Print(song)).unwrap();
        assert!((0.15..0.25).contains(&m.share), "{m:?}");
        assert_eq!(m.verdict(), Verdict::Unsure);
    }

    #[test]
    fn prints_round_trip_through_json() {
        let p = Print(words(10, 7));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Print>(&json).unwrap(), p);
    }

    #[test]
    fn an_empty_print_compares_to_nothing() {
        assert!(compare(&Print::default(), &Print(words(5, 1))).is_none());
    }
}
