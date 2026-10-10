//! Whether two pictures are one design: perceptual hashes of covers.
//!
//! A cover reaches muman rescaled, recompressed, framed in a 16:9 video
//! thumbnail or cropped to a square, so its bytes say nothing; what
//! survives is its layout of light and dark. Each picture is hashed
//! twice, as image hashing libraries do:
//!
//! - **pHash**: the picture averaged down to 32×32, its two-dimensional
//!   DCT-II, and of the 8×8 lowest frequencies all but the mean, each a
//!   bit for whether it lies above their median. It survives scaling,
//!   recompression, gamma and brightness.
//! - **dHash**: the picture averaged down to 9×8, each bit whether a
//!   pixel is darker than the one to its right.
//!
//! Each is taken over the picture's content, inside the bars
//! `quality::image` trims, and over the centred square of that content,
//! which is the cover a thumbnail frames on a blurred background. Two
//! pictures are as far apart as the nearest of those pairs, in bits that
//! differ:
//!
//! | pHash bits apart | dHash bits apart | Verdict |
//! |---|---|---|
//! | 6 or fewer | 10 or fewer | One design |
//! | 12 or fewer | Any | Unsure |
//! | More | Any | Different |
//!
//! These are the bands Hackerfactor gives for 64-bit hashes (a variant
//! within about 5 to 10 bits, unrelated pictures beyond 10); two
//! unrelated pictures differ in 32 bits on average. A picture with little
//! contrast (a spread under 6 gray levels at 32×32) hashes like every
//! other such picture, so it is never more than Unsure. Hashes are of
//! gray pixels, so two colourings of one design are one design.
//!
//! The DCT's cosines come from `f64::cos`, whose last bit may differ
//! between platforms; that can flip a coefficient lying on the median,
//! a bit at most, far inside the bands.

use std::sync::LazyLock;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::quality::{self, Rect};

/// Names how pictures are hashed. A look taken by another is taken again.
pub const METHOD: &str = "picture/1";

const SIDE: usize = 32;
const LOW: usize = 8;
const SAME_P: u32 = 6;
const SAME_D: u32 = 10;
const UNSURE_P: u32 = 12;
const FLAT: f64 = 6.0;

/// A picture's two hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hashes {
    pub p: u64,
    pub d: u64,
}

impl Serialize for Hashes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        format!("{:016x}{:016x}", self.p, self.d).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Hashes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let half = |r: std::ops::Range<usize>| {
            text.get(r)
                .and_then(|h| u64::from_str_radix(h, 16).ok())
                .ok_or_else(|| serde::de::Error::custom("not 32 hex digits"))
        };
        Ok(Hashes {
            p: half(0..16)?,
            d: half(16..32)?,
        })
    }
}

/// How a picture looks, to tell it from others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Look {
    pub content: Hashes,
    /// The centred square, when the content is no square.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub square: Option<Hashes>,
    /// Too little contrast to be told apart.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flat: bool,
}

/// Whether two pictures are one design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Alike {
    Different,
    Unsure,
    Same,
}

/// How the gray picture `pixels`, `width` × `height`, looks, its
/// content inside `content`; `None` for a picture too small to hash.
#[must_use]
pub fn look(pixels: &[u8], width: u32, height: u32, content: Rect) -> Option<Look> {
    if content.width < 9 || content.height < 8 || pixels.len() < (width * height) as usize {
        return None;
    }
    let side = content.width.min(content.height);
    let square = (content.width != content.height).then(|| Rect {
        x: content.x + (content.width - side) / 2,
        y: content.y + (content.height - side) / 2,
        width: side,
        height: side,
    });
    let small = quality::downscale(pixels, width, content, SIDE, SIDE);
    Some(Look {
        content: hashes(pixels, width, content),
        square: square.map(|r| hashes(pixels, width, r)),
        flat: spread(&small) < FLAT,
    })
}

fn hashes(pixels: &[u8], width: u32, area: Rect) -> Hashes {
    Hashes {
        p: phash(&quality::downscale(pixels, width, area, SIDE, SIDE)),
        d: dhash(&quality::downscale(pixels, width, area, 9, 8)),
    }
}

/// `COSINES[u][x]`: the DCT-II basis at frequency `u` and sample `x`.
static COSINES: LazyLock<Vec<[f64; SIDE]>> = LazyLock::new(|| {
    (0..LOW)
        .map(|u| {
            let mut row = [0.0; SIDE];
            for (x, c) in row.iter_mut().enumerate() {
                let (u, x) = (real(u), real(x));
                *c = (std::f64::consts::PI * u * (2.0 * x + 1.0) / (2.0 * real(SIDE))).cos();
            }
            row
        })
        .collect()
});

fn phash(img: &[f64]) -> u64 {
    let mut coefficients = Vec::with_capacity(LOW * LOW);
    for u in 0..LOW {
        for v in 0..LOW {
            let mut sum = 0.0;
            for y in 0..SIDE {
                let row = &img[y * SIDE..(y + 1) * SIDE];
                let across: f64 = row.iter().zip(&COSINES[v]).map(|(p, c)| p * c).sum();
                sum += across * COSINES[u][y];
            }
            coefficients.push(sum);
        }
    }
    let ac = &coefficients[1..];
    let mut sorted = ac.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    ac.iter()
        .enumerate()
        .filter(|(_, c)| **c > median)
        .fold(0, |bits, (i, _)| bits | 1 << i)
}

fn dhash(img: &[f64]) -> u64 {
    let mut bits = 0;
    for y in 0..8 {
        for x in 0..8 {
            if img[y * 9 + x] < img[y * 9 + x + 1] {
                bits |= 1 << (y * 8 + x);
            }
        }
    }
    bits
}

fn spread(img: &[f64]) -> f64 {
    let n = real(img.len());
    let mean = img.iter().sum::<f64>() / n;
    (img.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt()
}

#[allow(clippy::cast_precision_loss)]
fn real(n: usize) -> f64 {
    n as f64
}

/// How many bits apart the nearest of two looks' hashes are: pHash,
/// then dHash.
#[must_use]
pub fn distance(a: &Look, b: &Look) -> (u32, u32) {
    let sides = |l: &Look| {
        std::iter::once(l.content)
            .chain(l.square)
            .collect::<Vec<_>>()
    };
    let (ours, theirs) = (sides(a), sides(b));
    ours.iter()
        .flat_map(|x| {
            theirs
                .iter()
                .map(move |y| ((x.p ^ y.p).count_ones(), (x.d ^ y.d).count_ones()))
        })
        .min()
        .unwrap_or((64, 64))
}

/// Whether `a` and `b` are one design, by the bands above.
#[must_use]
pub fn alike(a: &Look, b: &Look) -> Alike {
    let (p, d) = distance(a, b);
    let verdict = if p <= SAME_P && d <= SAME_D {
        Alike::Same
    } else if p <= UNSURE_P {
        Alike::Unsure
    } else {
        Alike::Different
    };
    if a.flat || b.flat {
        verdict.min(Alike::Unsure)
    } else {
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An invented cover: rings and a bar, on `[0, 1)²`.
    fn art(x: f64, y: f64, seed: f64) -> f64 {
        let ring = ((x - 0.4).hypot(y - 0.55) * (14.0 + seed)).sin();
        let bar = if (0.2..0.35).contains(&y) && x > 0.1 * seed {
            0.8
        } else {
            0.0
        };
        let wave = (x * 9.0 * seed + y * 4.0).cos() * 0.5;
        (128.0 + 60.0 * ring + 50.0 * bar * 2.0 - 1.0 + 30.0 * wave).clamp(0.0, 255.0)
    }

    fn render(w: u32, h: u32, f: impl Fn(u32, u32) -> f64) -> Vec<u8> {
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                out.push(f(x, y).round().clamp(0.0, 255.0) as u8);
            }
        }
        out
    }

    fn cover(side: u32, seed: f64) -> Vec<u8> {
        render(side, side, |x, y| {
            art(
                f64::from(x) / f64::from(side),
                f64::from(y) / f64::from(side),
                seed,
            )
        })
    }

    fn look_of(pixels: &[u8], w: u32, h: u32) -> Look {
        let q = quality::image(pixels, w, h).unwrap();
        look(pixels, w, h, q.content).unwrap()
    }

    #[test]
    fn a_cover_rescaled_framed_brightened_or_noised_is_one_design() {
        let original = look_of(&cover(600, 1.0), 600, 600);
        let small = look_of(&cover(240, 1.0), 240, 240);
        assert_eq!(
            alike(&original, &small),
            Alike::Same,
            "{:?}",
            distance(&original, &small)
        );
        let boxed = render(640, 360, |x, y| {
            if (140..500).contains(&x) {
                art(f64::from(x - 140) / 360.0, f64::from(y) / 360.0, 1.0)
            } else {
                0.0
            }
        });
        let boxed = look_of(&boxed, 640, 360);
        assert_eq!(
            alike(&original, &boxed),
            Alike::Same,
            "{:?}",
            distance(&original, &boxed)
        );
        let blurred_sides = render(640, 360, |x, y| {
            if (140..500).contains(&x) {
                art(f64::from(x - 140) / 360.0, f64::from(y) / 360.0, 1.0)
            } else {
                90.0 + 20.0 * (f64::from(y) / 40.0).sin()
            }
        });
        let framed = look_of(&blurred_sides, 640, 360);
        assert!(framed.square.is_some());
        assert_eq!(
            alike(&original, &framed),
            Alike::Same,
            "{:?}",
            distance(&original, &framed)
        );
        let noisy = render(600, 600, |x, y| {
            let n = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 8;
            art(f64::from(x) / 600.0, f64::from(y) / 600.0, 1.0) + 20.0 + f64::from(n) - 3.5
        });
        let noisy = look_of(&noisy, 600, 600);
        assert_eq!(
            alike(&original, &noisy),
            Alike::Same,
            "{:?}",
            distance(&original, &noisy)
        );
    }

    #[test]
    fn other_designs_are_apart_and_flat_ones_never_sure() {
        let a = look_of(&cover(300, 1.0), 300, 300);
        let b = look_of(&cover(300, 2.7), 300, 300);
        assert_eq!(alike(&a, &b), Alike::Different, "{:?}", distance(&a, &b));
        assert!(distance(&a, &b).0 > UNSURE_P);
        let flat = look_of(&render(64, 64, |x, _| 100.0 + f64::from(x % 2)), 64, 64);
        assert!(flat.flat);
        assert!(alike(&flat, &flat) <= Alike::Unsure);
        assert_eq!(
            look(
                &[0; 16],
                4,
                4,
                Rect {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 4
                }
            ),
            None
        );
    }

    #[test]
    fn hashes_read_back_as_they_were_written() {
        let a = look_of(&cover(300, 1.0), 300, 300);
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains(&format!("{:016x}", a.content.p)));
        assert_eq!(serde_json::from_str::<Look>(&json).unwrap(), a);
        assert!(serde_json::from_str::<Look>(r#"{"content":"zz"}"#).is_err());
    }
}
