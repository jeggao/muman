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
//! Each bit is set only past a dead zone: a coefficient a millionth of
//! the largest above the median, a pixel a gray level darker. Without
//! it, a plain field, as behind a centred logo, and a design symmetric
//! about its middle, whose coefficients are all but zero, set their bits
//! by rounding: the 9×8 cells of a solid picture differ in their
//! thirteenth decimal, and one such cover set 16 to 32 bits otherwise at
//! another size.
//!
//! Each is taken over the picture's content, inside the bars
//! `quality::image` trims; over the centred square of that content,
//! which is the cover a thumbnail frames on a blurred background; and,
//! for a frame of another shape, over the centred square of the whole
//! frame, since a dark cover in black bars is trimmed into its own art.
//! Two pictures are one design when any pair of those is:
//!
//! | pHash bits apart | dHash bits apart | Verdict |
//! |---|---|---|
//! | 6 or fewer, both dense | Any | One design |
//! | 10 or fewer | 10 or fewer | One design |
//! | 12 or fewer, both dense | Any | Unsure |
//! | 12 or fewer | 20 or fewer | Unsure |
//! | More | Any | Different |
//!
//! A pHash is dense when it sets 24 of its 63 bits or more, as a median
//! split sets 31. One of a design symmetric about its middle, as bands
//! or a checkerboard, has nearly every coefficient on the median, so the
//! dead zone leaves it a few bits or none; two such hashes are near
//! whatever their designs, and only dHash tells them apart.
//!
//! The bands were measured over 50 generated covers, each under 29
//! changes. JPEG at quality 20, WebP, scaling from 120 to 2,000 px, bars
//! and blurred frames, gamma, brightness and noise kept 94 of 100 one
//! design, and of the 1,225 pairs of distinct covers, and 17,150 changed
//! covers against distinct originals, none was nearer than 16 bits of
//! pHash; distinct covers are 32 apart on average. Crops of a tenth and
//! a badge over a corner move pHash past the bands, as is its nature.
//! A picture with little contrast (a spread under 6 gray levels at
//! 32×32) hashes like every other such picture, so it is never more than
//! Unsure. Hashes are of gray pixels, so two colourings of one design are
//! one design, and a transparent field hashes as the colour it hides.
//!
//! The DCT's cosines come from `f64::cos`, whose last bit may differ
//! between platforms; the dead zone keeps such a difference from flipping
//! a bit.

use std::sync::LazyLock;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::quality::{self, Rect};

/// Names how pictures are hashed. A look taken by another is taken again.
pub const METHOD: &str = "picture/2";

const SIDE: usize = 32;
const LOW: usize = 8;
const SAME_P: u32 = 6;
const NEAR_P: u32 = 10;
const NEAR_D: u32 = 10;
const UNSURE_P: u32 = 12;
/// The dHash bits a sparse pair may differ by and still be Unsure.
const UNSURE_D: u32 = 20;
/// The pHash bits set, of 63, that make a hash dense enough to decide by
/// itself; a median split sets 31.
const DENSE: u32 = 24;
const FLAT: f64 = 6.0;
/// How far past the median a coefficient lies to set its bit, of the
/// largest coefficient.
const TIE: f64 = 1e-6;
/// How much darker, in gray levels, a pixel is to set its bit.
const STEP: f64 = 1.0;

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
        if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(serde::de::Error::custom("not 32 hex digits"));
        }
        let half = |r: std::ops::Range<usize>| {
            u64::from_str_radix(&text[r], 16).map_err(serde::de::Error::custom)
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
    /// The centred square of the whole frame, when the frame is no square.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Hashes>,
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

fn centred_square(r: Rect) -> Option<Rect> {
    let side = r.width.min(r.height);
    (r.width != r.height).then(|| Rect {
        x: r.x + (r.width - side) / 2,
        y: r.y + (r.height - side) / 2,
        width: side,
        height: side,
    })
}

/// How the gray picture `pixels`, `width` × `height`, looks, its
/// content inside `content`; `None` for a picture too small to hash, or
/// a content that is not inside it.
#[must_use]
pub fn look(pixels: &[u8], width: u32, height: u32, content: Rect) -> Option<Look> {
    let inside = u64::from(content.x) + u64::from(content.width) <= u64::from(width)
        && u64::from(content.y) + u64::from(content.height) <= u64::from(height);
    let size = usize::try_from(u64::from(width) * u64::from(height)).ok()?;
    if content.width < 9 || content.height < 8 || !inside || pixels.len() < size {
        return None;
    }
    let whole = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let frame = centred_square(whole)
        .filter(|r| r.width >= 9 && Some(*r) != centred_square(content) && *r != content);
    let (hashed, small) = hashes(pixels, width, content);
    Some(Look {
        content: hashed,
        square: centred_square(content).map(|r| hashes(pixels, width, r).0),
        frame: frame.map(|r| hashes(pixels, width, r).0),
        flat: spread(&small) < FLAT,
    })
}

/// The hashes of `area`, and the 32×32 it was averaged to.
fn hashes(pixels: &[u8], width: u32, area: Rect) -> (Hashes, Vec<f64>) {
    let small = quality::downscale(pixels, width, area, SIDE, SIDE);
    let hashed = Hashes {
        p: phash(&small),
        d: dhash(&quality::downscale(pixels, width, area, 9, 8)),
    };
    (hashed, small)
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
    let largest = ac.iter().map(|c| c.abs()).fold(0.0, f64::max);
    let tie = largest * TIE;
    ac.iter()
        .enumerate()
        .filter(|(_, c)| **c > median + tie)
        .fold(0, |bits, (i, _)| bits | 1 << i)
}

fn dhash(img: &[f64]) -> u64 {
    let mut bits = 0;
    for y in 0..8 {
        for x in 0..8 {
            if img[y * 9 + x] + STEP < img[y * 9 + x + 1] {
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

fn sides(l: &Look) -> Vec<Hashes> {
    std::iter::once(l.content)
        .chain(l.square)
        .chain(l.frame)
        .collect()
}

/// The verdict on two sides `p` and `d` bits apart, `dense` when both
/// pHashes set enough bits to decide alone.
fn verdict(p: u32, d: u32, dense: bool) -> Alike {
    if (dense && p <= SAME_P) || (p <= NEAR_P && d <= NEAR_D) {
        Alike::Same
    } else if p <= UNSURE_P && (dense || d <= UNSURE_D) {
        Alike::Unsure
    } else {
        Alike::Different
    }
}

fn judged(x: &Hashes, y: &Hashes) -> (Alike, u32, u32) {
    let (p, d) = ((x.p ^ y.p).count_ones(), (x.d ^ y.d).count_ones());
    let dense = x.p.count_ones() >= DENSE && y.p.count_ones() >= DENSE;
    (verdict(p, d, dense), p, d)
}

/// How many bits apart the nearest of two looks' hashes are, pHash
/// then dHash: the pair of their sides that is most alike.
#[must_use]
pub fn distance(a: &Look, b: &Look) -> (u32, u32) {
    most_alike(a, b).map_or((64, 64), |(_, p, d)| (p, d))
}

fn most_alike(a: &Look, b: &Look) -> Option<(Alike, u32, u32)> {
    let (ours, theirs) = (sides(a), sides(b));
    ours.iter()
        .flat_map(|x| theirs.iter().map(move |y| judged(x, y)))
        .min_by_key(|&(v, p, d)| (std::cmp::Reverse(v), p, d))
}

/// Whether `a` and `b` are one design, by the bands above, judged on
/// the pair of their sides most alike.
#[must_use]
pub fn alike(a: &Look, b: &Look) -> Alike {
    let verdict = most_alike(a, b).map_or(Alike::Different, |(v, _, _)| v);
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

    /// A gray field with a centred white block, rendered at `side`.
    fn plain_logo(side: u32) -> Vec<u8> {
        render(side, side, |x, y| {
            let (u, v) = (
                f64::from(x) / f64::from(side),
                f64::from(y) / f64::from(side),
            );
            if (0.2..0.8).contains(&u) && (0.4..0.6).contains(&v) {
                255.0
            } else {
                64.0
            }
        })
    }

    #[test]
    fn a_plain_field_sets_no_bit_by_rounding_at_any_size() {
        let at = |side| look_of(&plain_logo(side), side, side);
        let original = at(600);
        for side in [300, 500, 900, 1000, 1200] {
            assert_eq!(
                alike(&original, &at(side)),
                Alike::Same,
                "{side}: {:?}",
                distance(&original, &at(side))
            );
        }
        let solid = look_of(&render(64, 64, |_, _| 128.0), 64, 64);
        assert_eq!(solid.content.d, 0);
    }

    #[test]
    fn a_dark_cover_in_black_bars_is_still_its_cover() {
        let disk = |u: f64, v: f64| {
            if (u - 0.5).hypot(v - 0.5) < 0.25 {
                235.0
            } else {
                0.0
            }
        };
        let cover = render(600, 600, |x, y| {
            disk(f64::from(x) / 600.0, f64::from(y) / 600.0)
        });
        let original = look_of(&cover, 600, 600);
        let framed = render(640, 360, |x, y| {
            if (140..500).contains(&x) {
                disk(f64::from(x - 140) / 360.0, f64::from(y) / 360.0)
            } else {
                0.0
            }
        });
        let framed = look_of(&framed, 640, 360);
        assert!(framed.frame.is_some());
        assert_eq!(
            alike(&original, &framed),
            Alike::Same,
            "{:?}",
            distance(&original, &framed)
        );
    }

    #[test]
    fn the_most_alike_pair_of_sides_decides() {
        let h = |p, d| Hashes { p, d };
        let a = Look {
            content: h(0, 0),
            square: Some(h(u64::MAX, u64::MAX)),
            frame: None,
            flat: false,
        };
        let b = Look {
            content: h(0xFF, (1 << 14) - 1),
            square: Some(h(u64::MAX >> 10, u64::MAX >> 3)),
            frame: None,
            flat: false,
        };
        assert_eq!(alike(&a, &b), Alike::Same, "{:?}", distance(&a, &b));
    }

    #[test]
    fn hashes_read_only_as_32_hex_digits_and_a_look_only_inside_its_picture() {
        let bad = [
            r#"{"content":"0123456789abcdef0123456789abcdefzz"}"#,
            r#"{"content":"+123456789abcdef0123456789abcdef"}"#,
        ];
        for json in bad {
            assert!(serde_json::from_str::<Look>(json).is_err(), "{json}");
        }
        let outside = Rect {
            x: 1,
            y: 0,
            width: 64,
            height: 64,
        };
        assert_eq!(look(&[0; 64 * 64], 64, 64, outside), None);
        assert_eq!(
            look(
                &[0; 16],
                65536,
                65536,
                Rect {
                    x: 0,
                    y: 0,
                    width: 16,
                    height: 16
                }
            ),
            None
        );
    }

    #[test]
    fn designs_symmetric_about_their_middle_are_told_apart_by_dhash() {
        let ramp = render(300, 300, |x, _| 100.0 + f64::from(x) / 12.0);
        let ramp = look_of(&ramp, 300, 300);
        let rotated = look_of(
            &render(300, 300, |_, y| 100.0 + f64::from(y) / 12.0),
            300,
            300,
        );
        let mirrored = look_of(
            &render(300, 300, |x, _| 125.0 - f64::from(x) / 12.0),
            300,
            300,
        );
        assert_ne!(
            alike(&ramp, &rotated),
            Alike::Same,
            "{:?}",
            distance(&ramp, &rotated)
        );
        assert_ne!(
            alike(&ramp, &mirrored),
            Alike::Same,
            "{:?}",
            distance(&ramp, &mirrored)
        );
        let checks = render(300, 300, |x, y| {
            if (x / 50 + y / 50) % 2 == 0 {
                30.0
            } else {
                220.0
            }
        });
        let bands = render(
            300,
            300,
            |_, y| if (y / 50) % 2 == 0 { 30.0 } else { 220.0 },
        );
        let (checks, bands) = (look_of(&checks, 300, 300), look_of(&bands, 300, 300));
        assert_eq!(
            alike(&checks, &bands),
            Alike::Different,
            "{:?}",
            distance(&checks, &bands)
        );
    }
}
