//! Quantities written with a unit, in `songs.toml` and on the command
//! line: sizes, bitrates, lengths of time, frequencies and shares. Each
//! kind is read by one type here, so every setting of a kind takes the
//! same spellings, and is converted by `uom`, which names every unit of
//! its kind by abbreviation, singular and plural: `"2 gibibytes"`,
//! `"14 days"`, `"1.5 kHz"`.
//!
//! | Kind | Written as | A bare number |
//! |---|---|---|
//! | [`Size`] | `"32 GiB"`, `"700 MB"`, `"1.5G"` | Bytes |
//! | [`Bitrate`] | `"160 kb/s"`, `"160 kbps"`, `"160k"` | Refused |
//! | [`Time`] | `"14 days"`, `"2 s"`, `"500 ms"`, `"-120 ms"` | Refused |
//! | [`Frequency`] | `"500 Hz"`, `"1.5 kHz"` | Hertz |
//! | [`Share`] | `"1 %"`, `"0.1 %"` | A fraction: 0.01 is 1 % |
//!
//! `uom` reads a number, one space and a unit exactly as it spells it.
//! What muman read before it came stays readable on top: no space, any
//! case, `K`, `M`, `G` and `T` alone as binary sizes (`KiB`) where `uom`
//! has no such unit, `b` as a byte where `uom` takes it for a bit, and
//! `kbps`, `k` for kilobits a second, as ffmpeg writes them, and weeks.
//!
//! A bitrate and a time have no bare number: the settings they replaced
//! counted in their own units, days for one and milliseconds for another,
//! so `recheck = 30` or `opus_bitrate = 160` would be read wrongly.
//!
//! A value muman writes, as when it renames an old setting, is in the
//! largest unit that holds it whole: `"2 GiB"`, `"14 days"`, `"2 s"`.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use uom::si::f64::{Frequency as Hertz, Information, InformationRate, Ratio, Time as Seconds};
use uom::si::{frequency, information, information_rate, ratio, time};

/// A number of bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "Written")]
pub struct Size(pub u64);

/// A bitrate, in whole kbit/s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "Written")]
pub struct Bitrate(pub u32);

/// A length of time, in whole milliseconds; negative for an offset back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "Written")]
pub struct Time(pub i64);

/// A frequency, in whole hertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "Written")]
pub struct Frequency(pub u32);

/// A share of a whole, as a fraction: 0.01 is 1 %.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Deserialize)]
#[serde(try_from = "Written")]
pub struct Share(pub f64);

/// A value as TOML gives it: a number, or text with a unit.
#[derive(Deserialize)]
#[serde(untagged)]
enum Written {
    Whole(i64),
    Real(f64),
    Text(String),
}

/// A number and the unit after it, the space between them optional.
fn split(text: &str) -> (&str, &str) {
    let t = text.trim();
    let digits = t
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && matches!(c, '-' | '+'))))
        .map_or(t.len(), |(i, _)| i);
    let (number, unit) = t.split_at(digits);
    (number, unit.trim())
}

/// `text` read as a quantity of `Q`: as `uom` spells its units, else by
/// one of `aliases`, matched in any case, which names a unit `uom` knows
/// and how many of it one of the alias makes.
fn quantity<Q: std::str::FromStr + std::ops::Mul<f64, Output = Q>>(
    text: &str,
    aliases: &[(&str, &str, f64)],
) -> Option<Q> {
    let (number, unit) = split(text);
    if number.is_empty() {
        return None;
    }
    if let Ok(q) = format!("{number} {unit}").parse::<Q>() {
        return Some(q);
    }
    let lower = unit.to_lowercase();
    let &(_, to, times) = aliases.iter().find(|(from, _, _)| *from == lower)?;
    format!("{number} {to}")
        .parse::<Q>()
        .ok()
        .map(|q| q * times)
}

const SIZES: &[(&str, &str, f64)] = &[
    ("", "B", 1.0),
    ("b", "B", 1.0),
    ("k", "KiB", 1.0),
    ("kib", "KiB", 1.0),
    ("kb", "kB", 1.0),
    ("m", "MiB", 1.0),
    ("mib", "MiB", 1.0),
    ("mb", "MB", 1.0),
    ("g", "GiB", 1.0),
    ("gib", "GiB", 1.0),
    ("gb", "GB", 1.0),
    ("t", "TiB", 1.0),
    ("tib", "TiB", 1.0),
    ("tb", "TB", 1.0),
];

const BITRATES: &[(&str, &str, f64)] = &[
    ("k", "kb/s", 1.0),
    ("kbps", "kb/s", 1.0),
    ("kbit/s", "kb/s", 1.0),
    ("kb/s", "kb/s", 1.0),
    ("m", "Mb/s", 1.0),
    ("mbps", "Mb/s", 1.0),
    ("mbit/s", "Mb/s", 1.0),
    ("bps", "b/s", 1.0),
];

const TIMES: &[(&str, &str, f64)] = &[
    ("sec", "s", 1.0),
    ("secs", "s", 1.0),
    ("msec", "ms", 1.0),
    ("mins", "min", 1.0),
    ("hr", "h", 1.0),
    ("hrs", "h", 1.0),
    ("w", "d", 7.0),
    ("week", "d", 7.0),
    ("weeks", "d", 7.0),
];

const FREQUENCIES: &[(&str, &str, f64)] =
    &[("", "Hz", 1.0), ("hz", "Hz", 1.0), ("khz", "kHz", 1.0)];

const SHARES: &[(&str, &str, f64)] = &[("", "", 1.0)];

/// `value` rounded into `T`'s range, if it is a finite number there.
fn whole<T: TryFrom<i128>>(value: f64) -> Option<T> {
    let rounded = value.round();
    #[allow(clippy::cast_possible_truncation)]
    (rounded.is_finite() && rounded.abs() < 1e30)
        .then(|| T::try_from(rounded as i128).ok())
        .flatten()
}

/// A size as `songs.toml` and `--max-size` take it, in bytes.
pub fn parse_size(text: &str) -> Result<u64, String> {
    quantity::<Information>(text, SIZES)
        .and_then(|q| whole(q.get::<information::byte>()))
        .ok_or_else(|| format!("`{text}` is not a size, such as \"4 GiB\" or \"700 MB\""))
}

impl std::str::FromStr for Size {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        parse_size(text).map(Self)
    }
}

impl std::str::FromStr for Bitrate {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        quantity::<InformationRate>(text, BITRATES)
            .and_then(|q| whole(q.get::<information_rate::kilobit_per_second>()))
            .map(Self)
            .ok_or_else(|| format!("`{text}` is not a bitrate, such as \"160 kb/s\""))
    }
}

impl std::str::FromStr for Time {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        quantity::<Seconds>(text, TIMES)
            .and_then(|q| whole(q.get::<time::millisecond>()))
            .map(Self)
            .ok_or_else(|| {
                format!("`{text}` is not a length of time, such as \"14 days\" or \"2 s\"")
            })
    }
}

impl std::str::FromStr for Frequency {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        quantity::<Hertz>(text, FREQUENCIES)
            .and_then(|q| whole(q.get::<frequency::hertz>()))
            .map(Self)
            .ok_or_else(|| format!("`{text}` is not a frequency, such as \"500 Hz\""))
    }
}

impl std::str::FromStr for Share {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        quantity::<Ratio>(text, SHARES)
            .map(|q| (q.get::<ratio::ratio>() * 1e12).round() / 1e12)
            .filter(|f| f.is_finite())
            .map(Self)
            .ok_or_else(|| format!("`{text}` is not a share, such as \"1 %\" or 0.01"))
    }
}

/// A bare number is refused for `what`, which needs a unit like `example`.
fn needs_unit(n: impl fmt::Display, example: &str) -> String {
    format!("{n} needs a unit, as in \"{n} {example}\"")
}

impl TryFrom<Written> for Size {
    type Error = String;

    fn try_from(w: Written) -> Result<Self, String> {
        match w {
            Written::Whole(n) => u64::try_from(n)
                .map(Self)
                .map_err(|_| format!("{n} is not a size")),
            Written::Real(n) => Err(format!("{n} bytes is not a whole number")),
            Written::Text(t) => t.parse(),
        }
    }
}

impl TryFrom<Written> for Bitrate {
    type Error = String;

    fn try_from(w: Written) -> Result<Self, String> {
        match w {
            Written::Whole(n) => Err(needs_unit(n, "kb/s")),
            Written::Real(n) => Err(needs_unit(n, "kb/s")),
            Written::Text(t) => t.parse(),
        }
    }
}

impl TryFrom<Written> for Time {
    type Error = String;

    fn try_from(w: Written) -> Result<Self, String> {
        match w {
            Written::Whole(n) => Err(needs_unit(n, "days")),
            Written::Real(n) => Err(needs_unit(n, "s")),
            Written::Text(t) => t.parse(),
        }
    }
}

impl TryFrom<Written> for Frequency {
    type Error = String;

    fn try_from(w: Written) -> Result<Self, String> {
        match w {
            Written::Whole(n) => u32::try_from(n)
                .map(Self)
                .map_err(|_| format!("{n} Hz is not a frequency")),
            Written::Real(n) => whole(n).map(Self).ok_or_else(|| format!("{n} Hz")),
            Written::Text(t) => t.parse(),
        }
    }
}

impl TryFrom<Written> for Share {
    type Error = String;

    fn try_from(w: Written) -> Result<Self, String> {
        match w {
            #[allow(clippy::cast_precision_loss)]
            Written::Whole(n) => Ok(Self(n as f64)),
            Written::Real(n) => Ok(Self(n)),
            Written::Text(t) => t.parse(),
        }
    }
}

/// Seconds in milliseconds.
#[must_use]
pub fn ms_of_seconds(seconds: f64) -> f64 {
    Seconds::new::<time::second>(seconds).get::<time::millisecond>()
}

/// Milliseconds in seconds.
#[must_use]
pub fn seconds_of_ms(ms: f64) -> f64 {
    Seconds::new::<time::millisecond>(ms).get::<time::second>()
}

/// Seconds in minutes.
#[must_use]
pub fn minutes_of_seconds(seconds: f64) -> f64 {
    Seconds::new::<time::second>(seconds).get::<time::minute>()
}

/// Hertz in kilohertz.
#[must_use]
pub fn khz_of_hz(hz: f64) -> f64 {
    Hertz::new::<frequency::hertz>(hz).get::<frequency::kilohertz>()
}

/// The bytes `seconds` of a stream at `kbps` kbit/s take.
#[must_use]
pub fn stream_bytes(kbps: f64, seconds: f64) -> f64 {
    let rate = InformationRate::new::<information_rate::kilobit_per_second>(kbps);
    let data: Information = (rate * Seconds::new::<time::second>(seconds)).into();
    data.get::<information::byte>()
}

/// Parts per million as a fraction.
#[must_use]
pub fn ratio_of_ppm(ppm: f64) -> f64 {
    Ratio::new::<ratio::part_per_million>(ppm).get::<ratio::ratio>()
}

/// A fraction in parts per million.
#[must_use]
pub fn ppm_of_ratio(fraction: f64) -> f64 {
    Ratio::new::<ratio::ratio>(fraction).get::<ratio::part_per_million>()
}

/// Readers for `#[serde(deserialize_with)]`: a setting whose value takes
/// a unit, kept as a number in the unit its field names.
pub mod serde_as {
    use serde::{Deserialize, Deserializer};

    use super::{Bitrate, Frequency, Share, Time};

    pub fn kbps<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        Bitrate::deserialize(d).map(|b| b.0)
    }

    pub fn kbps_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
        Option::<Bitrate>::deserialize(d).map(|b| b.map(|b| b.0))
    }

    /// A length of time that cannot be negative, in milliseconds.
    pub fn ms<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        let t = Time::deserialize(d)?;
        u32::try_from(t.0).map_err(|_| {
            serde::de::Error::custom(format!(
                "`{}` must be 0 or more, and under 49 days",
                t.exact()
            ))
        })
    }

    /// A length of time that cannot be negative.
    pub fn duration<'de, D: Deserializer<'de>>(d: D) -> Result<std::time::Duration, D::Error> {
        let t = Time::deserialize(d)?;
        if t.0 < 0 {
            return Err(serde::de::Error::custom(format!(
                "`{}` must be 0 or more",
                t.exact()
            )));
        }
        Ok(t.duration())
    }

    pub fn hz<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        Frequency::deserialize(d).map(|f| f.0)
    }

    pub fn share<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Share::deserialize(d).map(|s| s.0)
    }

    pub fn shares<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
        Vec::<Share>::deserialize(d).map(|v| v.into_iter().map(|s| s.0).collect())
    }
}

/// `n` in the largest of `units` that holds it whole, each with how many
/// of the smallest it makes.
fn exact(n: i128, units: &[(i128, &str)]) -> String {
    let (size, name) = units
        .iter()
        .copied()
        .find(|(size, _)| n != 0 && n % size == 0)
        .unwrap_or(units[units.len() - 1]);
    format!("{} {name}", n / size)
}

impl Size {
    /// As a setting is written: `2 GiB`, `4 KiB`, `1000 B`.
    #[must_use]
    pub fn exact(self) -> String {
        let units = [
            (1 << 40, "TiB"),
            (1 << 30, "GiB"),
            (1 << 20, "MiB"),
            (1 << 10, "KiB"),
            (1, "B"),
        ];
        exact(i128::from(self.0), &units)
    }
}

impl fmt::Display for Size {
    /// In the largest binary unit that keeps it at least one, to a tenth.
    #[allow(clippy::cast_precision_loss)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let units = ["B", "KiB", "MiB", "GiB", "TiB"];
        let mut value = self.0 as f64;
        let mut unit = 0;
        while value >= 1024.0 && unit < units.len() - 1 {
            value /= 1024.0;
            unit += 1;
        }
        if unit == 0 {
            write!(f, "{} B", self.0)
        } else {
            write!(f, "{value:.1} {}", units[unit])
        }
    }
}

impl Bitrate {
    #[must_use]
    pub fn exact(self) -> String {
        format!("{} kb/s", self.0)
    }
}

impl Time {
    #[must_use]
    pub fn days(n: i64) -> Self {
        Self(n.saturating_mul(86_400_000))
    }

    /// The time as a wait, none when it is negative.
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_millis(u64::try_from(self.0).unwrap_or(0))
    }

    /// As a setting is written: `14 days`, `2 s`, `-120 ms`.
    #[must_use]
    pub fn exact(self) -> String {
        let units = [
            (86_400_000, "days"),
            (3_600_000, "h"),
            (60_000, "min"),
            (1000, "s"),
            (1, "ms"),
        ];
        exact(i128::from(self.0), &units)
    }
}

impl Frequency {
    #[must_use]
    pub fn exact(self) -> String {
        format!("{} Hz", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_takes_every_unit_uom_names_and_the_old_spellings() {
        assert_eq!(parse_size("4 GiB"), Ok(4 << 30));
        assert_eq!(parse_size("4GiB"), Ok(4 << 30));
        assert_eq!(parse_size("2 gibibytes"), Ok(2 << 30));
        assert_eq!(parse_size("700 MB"), Ok(700_000_000));
        assert_eq!(parse_size("700mb"), Ok(700_000_000));
        assert_eq!(parse_size("1.5G"), Ok(3 << 29));
        assert_eq!(parse_size("4k"), Ok(4096));
        assert_eq!(parse_size("32768"), Ok(32768));
        assert_eq!(parse_size("4 G"), Ok(4 << 30));
        assert_eq!(parse_size("1.5k"), Ok(1536));
        assert!(parse_size("4 XB").is_err());
        assert!(parse_size("GB").is_err());
        assert!(parse_size("lots").is_err());
        assert!(parse_size("-1 MB").is_err());
    }

    #[test]
    fn a_size_is_shown_to_a_tenth_and_written_whole() {
        assert_eq!(Size(512).to_string(), "512 B");
        assert_eq!(Size(85_181_845).to_string(), "81.2 MiB");
        assert_eq!(Size(2 << 30).exact(), "2 GiB");
        assert_eq!(Size(1000).exact(), "1000 B");
        assert_eq!(Size(0).exact(), "0 B");
    }

    #[test]
    fn a_bitrate_is_in_kilobits_a_second_however_spelled() {
        for text in [
            "160 kb/s",
            "160kbps",
            "160 kilobits per second",
            "160k",
            "0.16 Mb/s",
        ] {
            assert_eq!(text.parse(), Ok(Bitrate(160)), "{text}");
        }
        assert!(
            Bitrate::try_from(Written::Whole(160))
                .unwrap_err()
                .contains("160 kb/s")
        );
    }

    #[test]
    fn a_time_needs_its_unit_and_may_be_negative() {
        assert_eq!("14 days".parse(), Ok(Time::days(14)));
        assert_eq!("2 weeks".parse(), Ok(Time::days(14)));
        assert_eq!("2 s".parse(), Ok(Time(2000)));
        assert_eq!("2000ms".parse(), Ok(Time(2000)));
        assert_eq!("1.5 min".parse(), Ok(Time(90_000)));
        assert_eq!("-120 ms".parse(), Ok(Time(-120)));
        assert!("7".parse::<Time>().is_err());
        assert!(
            Time::try_from(Written::Whole(7))
                .unwrap_err()
                .contains("7 days")
        );
        assert_eq!(Time::days(14).exact(), "14 days");
        assert_eq!(Time(2000).exact(), "2 s");
        assert_eq!(Time(-120).exact(), "-120 ms");
        assert_eq!(Time(-5).duration(), Duration::ZERO);
    }

    #[test]
    fn conversions_are_uoms() {
        assert!((ms_of_seconds(2.5) - 2500.0).abs() < 1e-9);
        assert!((seconds_of_ms(2500.0) - 2.5).abs() < 1e-12);
        assert!((minutes_of_seconds(90.0) - 1.5).abs() < 1e-12);
        assert!((khz_of_hz(19_810.0) - 19.81).abs() < 1e-12);
        assert!((stream_bytes(160.0, 60.0) - 1_200_000.0).abs() < 1e-6);
        assert!((ratio_of_ppm(1500.0) - 0.0015).abs() < 1e-15);
        assert!((ppm_of_ratio(0.0015) - 1500.0).abs() < 1e-9);
    }

    #[test]
    fn a_frequency_is_in_hertz_and_a_share_a_fraction_or_percent() {
        assert_eq!("500 Hz".parse(), Ok(Frequency(500)));
        assert_eq!("1.5 kHz".parse(), Ok(Frequency(1500)));
        assert_eq!(Frequency::try_from(Written::Whole(250)), Ok(Frequency(250)));
        assert_eq!("1 %".parse(), Ok(Share(0.01)));
        assert_eq!("0.1 %".parse(), Ok(Share(0.001)));
        assert_eq!("10%".parse(), Ok(Share(0.1)));
        assert_eq!(Share::try_from(Written::Real(0.03)), Ok(Share(0.03)));
        assert!("loud".parse::<Frequency>().is_err());
    }
}
