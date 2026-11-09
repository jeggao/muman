//! The ways a video's title can be read as a song's title and artist,
//! each weighed, and whether one stands clear of the rest.
//!
//! An upload names its song in one line, `Marlo Venn - Night Ferry`,
//! `Night Ferry / Marlo Venn`, `「夜の船」Marlo Venn`, or the title alone.
//! Cleaning splits such a title by convention ([`crate::clean`]); here
//! every reading its separators allow is weighed by what the song's
//! other sources say, so a reading no evidence settles is told apart
//! from one it does, for `check` and `review` to name.
//!
//! The readings are the whole title, and at the first of each separator
//! (` - `, ` – `, ` — `, ` / `, ` | `, `：`), its left side as the artist
//! and its right as the title, or the other way round. Each scores, in
//! whole points:
//!
//! | Evidence | Points |
//! |---|---|
//! | The artist side, whole, is a name the song's sources credit | [`CREDITED`] |
//! | The artist side is the channel's name | [`CHANNEL`] |
//! | The artist side names someone featured, after `feat.` | [`FEATURING`] |
//! | The title side names a version, as `(Live)` | [`VERSIONED`] |
//! | The title side is quoted, as `「夜の船」` or `"Night Ferry"` | [`QUOTED`] |
//! | The title side is a title the song's sources give | [`TITLED`], or [`TITLED_ALIKE`] for one alike |
//! | The whole title, where nothing splits it | [`WHOLE`] |
//!
//! The reading scoring most is the song's where it leads the next by
//! [`MARGIN`] points or more; else the reading abstains. A title with
//! no separator reads one way and never abstains.

use crate::similar;

/// A side credited by the song's sources.
pub const CREDITED: i64 = 6;
/// A side that is the channel's name.
pub const CHANNEL: i64 = 4;
/// An artist side naming someone featured.
pub const FEATURING: i64 = 2;
/// A title side naming a version.
pub const VERSIONED: i64 = 2;
/// A quoted title side.
pub const QUOTED: i64 = 3;
/// A title side the song's sources give as its title, folded alike.
pub const TITLED: i64 = 5;
/// A title side alike to one the song's sources give, as one cut short.
pub const TITLED_ALIKE: i64 = 2;
/// The whole title, unsplit.
pub const WHOLE: i64 = 1;
/// How far the best reading must lead the next to be taken.
pub const MARGIN: i64 = 3;

/// What splits a video's title into its song's title and artist.
const SEPARATORS: [&str; 6] = [" - ", " – ", " — ", " / ", " | ", "："];

/// What quotes a title, open and close.
const QUOTES: [(char, char); 5] = [
    ('「', '」'),
    ('『', '』'),
    ('"', '"'),
    ('“', '”'),
    ('\'', '\''),
];

/// One way to read a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub title: String,
    pub artist: Option<String>,
    pub points: i64,
}

/// How a title reads: its readings, best first, and whether the best
/// stands clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub readings: Vec<Reading>,
    /// Whether the best reading leads the next by [`MARGIN`] or more, or
    /// is the only one.
    pub clear: bool,
}

impl Read {
    #[must_use]
    pub fn best(&self) -> Option<&Reading> {
        self.readings.first()
    }
}

/// What the song's other sources say, which a reading is weighed by.
#[derive(Debug, Clone, Copy, Default)]
pub struct Evidence<'a> {
    /// The names its sources credit.
    pub credited: &'a [&'a str],
    /// The titles its sources give.
    pub titles: &'a [&'a str],
    /// The channel's name, where an upload's.
    pub channel: Option<&'a str>,
}

/// `title` read every way its separators allow, weighed by `evidence`.
#[must_use]
pub fn read(title: &str, evidence: &Evidence<'_>) -> Read {
    let title = title.trim();
    let mut readings = vec![Reading {
        title: title.to_string(),
        artist: None,
        points: WHOLE + titled(title, evidence),
    }];
    for sep in SEPARATORS {
        let Some((left, right)) = title.split_once(sep) else {
            continue;
        };
        let (left, right) = (left.trim(), right.trim());
        if left.is_empty() || right.is_empty() {
            continue;
        }
        for (artist, song) in [(left, right), (right, left)] {
            readings.push(Reading {
                title: unquoted(song).to_string(),
                artist: Some(artist.to_string()),
                points: as_artist(artist, evidence) + as_title(song, evidence),
            });
        }
    }
    readings.sort_by(|a, b| {
        b.points
            .cmp(&a.points)
            .then_with(|| a.artist.is_some().cmp(&b.artist.is_some()))
            .then_with(|| a.title.cmp(&b.title))
    });
    let clear = match &readings[..] {
        [_] => true,
        [best, next, ..] => best.points - next.points >= MARGIN,
        [] => false,
    };
    Read { readings, clear }
}

fn as_artist(side: &str, evidence: &Evidence<'_>) -> i64 {
    // The side whole, never what joiners split out of it: a side holding
    // a credit and a title, as `Marlo Venn - Night Ferry`, names no one.
    let whole = similar::fold(side);
    let credited = (evidence.credited.iter()).any(|c| similar::credited(c).contains(&whole));
    let channel = evidence
        .channel
        .is_some_and(|c| similar::fold(c) == similar::fold(side));
    let featuring = crate::parts::credit(side).featured.is_some();
    i64::from(credited) * CREDITED + i64::from(channel) * CHANNEL + i64::from(featuring) * FEATURING
}

fn as_title(side: &str, evidence: &Evidence<'_>) -> i64 {
    let parsed = crate::parts::title(side);
    let versioned =
        (parsed.decorations.iter()).any(|d| matches!(d.kind, crate::parts::Kind::Version(_)));
    let quoted = unquoted(side) != side;
    i64::from(versioned) * VERSIONED + i64::from(quoted) * QUOTED + titled(unquoted(side), evidence)
}

/// [`TITLED`] where `side` is a title the song's sources give.
fn titled(side: &str, evidence: &Evidence<'_>) -> i64 {
    let folded = similar::fold(side);
    if (evidence.titles.iter()).any(|t| similar::fold(t) == folded) {
        return TITLED;
    }
    let mine = similar::Name::new(side);
    let alike = (evidence.titles.iter())
        .any(|t| similar::agree(&mine, &similar::Name::new(t)) == Some(true));
    i64::from(alike) * TITLED_ALIKE
}

/// `side` without the quotes around it, where it is quoted whole.
fn unquoted(side: &str) -> &str {
    for (open, close) in QUOTES {
        if let Some(inner) = side.strip_prefix(open).and_then(|s| s.strip_suffix(close))
            && !inner.trim().is_empty()
        {
            return inner.trim();
        }
    }
    side
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_the_song_s_sources_credit_one_side_of_reads_clear() {
        let credited = ["Marlo Venn"];
        let evidence = Evidence {
            credited: &credited,
            ..Evidence::default()
        };
        let r = read("Marlo Venn - Night Ferry", &evidence);
        assert!(r.clear);
        let best = r.best().unwrap();
        assert_eq!(
            (best.artist.as_deref(), best.title.as_str()),
            (Some("Marlo Venn"), "Night Ferry")
        );
        let flipped = read("Night Ferry / Marlo Venn", &evidence);
        assert_eq!(flipped.best().unwrap().title, "Night Ferry");
    }

    #[test]
    fn a_title_nothing_settles_abstains_and_one_with_no_separator_never_does() {
        let r = read("Paper Comets - Glass Orchard", &Evidence::default());
        assert!(!r.clear, "{r:?}");
        let whole = read("Night Ferry", &Evidence::default());
        assert!(whole.clear);
        assert_eq!(whole.best().unwrap().artist, None);
    }

    #[test]
    fn a_quoted_side_is_the_title_and_the_channel_the_artist() {
        let evidence = Evidence {
            channel: Some("Marlo Venn"),
            ..Evidence::default()
        };
        let r = read("Marlo Venn - 「夜の船」", &evidence);
        assert!(r.clear, "{r:?}");
        assert_eq!(r.best().unwrap().title, "夜の船");
    }

    #[test]
    fn a_title_the_sources_give_whole_and_a_credit_of_two_read_clear() {
        let titles = ["Night Ferry - Harbor Session"];
        let given = Evidence {
            titles: &titles,
            ..Evidence::default()
        };
        let r = read("Night Ferry - Harbor Session", &given);
        assert!(r.clear, "{r:?}");
        assert_eq!(r.best().unwrap().artist, None);
        let credited = ["Marlo Venn & Kiri Hoshino"];
        let both = Evidence {
            credited: &credited,
            ..Evidence::default()
        };
        let r = read("Marlo Venn - Kiri Hoshino - Night Ferry", &both);
        assert!(r.clear, "{r:?}");
        assert_eq!(r.best().unwrap().artist.as_deref(), Some("Marlo Venn"));
    }
}
