//! Values that say nothing of a song, though a source's tags carry them:
//! a placeholder standing for no value, and a watermark, the name of
//! whoever shared the files, written into fields that name the work.
//! Either is dropped before the sources' offers are ranked, so it neither
//! wins its field nor counts for or against another source.
//!
//! Measured on a library of about two thousand songs, one torrent group
//! wrote its name into `WORK` on 1,414 songs and into `LABEL` and
//! `COPYRIGHT` on 149, across every album of three discographies;
//! MusicBrainz's special label `[no label]` stood on 104 songs, a video
//! site's category on 2 as their genre, and a soundtrack's short name,
//! as `Show OST 2`, on 38 as their artist.
//!
//! - A **placeholder** is a value `placeholders.txt` lists for its field,
//!   compared whole, case, spacing and typography aside, or a credit that names a soundtrack
//!   rather than anyone, ending in `OST` and perhaps its volume: files
//!   tagged by album, their artist filled with the album's short name.
//!   A release country that is no two-letter code, as a group's name
//!   written there, stands for none too.
//! - A **watermark** is a value of a field naming the work rather than
//!   the files ([`PER_WORK`]) that
//!   1. stands as a tag in its torrent's name, in brackets or after a
//!      final dash, as `GRP` in `Marlo Venn - Discography [GRP]` or
//!      `Night Ferry (2019) FLAC-GRP`;
//!   2. one source writes in two of those fields, a work and a label
//!      alike, but for a work and a grouping, or a label and a
//!      copyright, which name one thing ([`kind_of`]); or
//!   3. is the `WORK` of songs of [`ALBUMS`] albums or more and [`TITLES`]
//!      titles or more of one source group: a work is one piece, whose
//!      movements span one album's few titles, or a few albums' same
//!      titles, the discs of one album counting as one.
//!
//!   A value found so is a watermark in every such field of its group.
//!   A label shared by a discography's albums is no watermark: a label
//!   names a release, and only the first two tests hold it.
//!
//! A source group is the torrent a file came from, else the folder or the
//! site, as the caller says. Values are compared folded to their letters
//! and digits, case aside.
//!
//! Each test holds a value alike whether cleaning has run on it or not:
//! `tidy.typography` writes `[non-album tracks]` with the hyphen U+2010,
//! and the spacing rule makes a no-break space a space, so a placeholder
//! is compared with its spaces, quotes and hyphens as ASCII writes them,
//! and a watermark by its letters and digits alone.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use crate::tags::{Field, Offers};

/// The fields that name the work or the files rather than the recording
/// or the release, in which a watermark is written.
pub const PER_WORK: [Field; 5] = [
    Field::Work,
    Field::Label,
    Field::Copyright,
    Field::Comment,
    Field::Grouping,
];

/// How many albums of one group a `WORK` must span to be no work.
pub const ALBUMS: usize = 3;

/// How many titles of one group a `WORK` must span to be no work.
pub const TITLES: usize = 10;

/// The fields a soundtrack's short name stands in as a placeholder.
const CREDITS: [Field; 3] = [Field::Artist, Field::Artists, Field::AlbumArtist];

/// `placeholders.txt`, each field's placeholders lower-cased; `None` for
/// those of every field.
static PLACEHOLDERS: LazyLock<BTreeMap<Option<Field>, BTreeSet<String>>> = LazyLock::new(|| {
    let mut table: BTreeMap<Option<Field>, BTreeSet<String>> = BTreeMap::new();
    for line in include_str!("detect/placeholders.txt").lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let Some((field, value)) = line.split_once('\t') else {
            continue;
        };
        let field = if field == "*" {
            None
        } else {
            match Field::named(field) {
                Some(f) => Some(f),
                None => continue,
            }
        };
        table.entry(field).or_default().insert(plain(value));
    }
    table
});

/// `value` as a placeholder is compared: its spaces one space each, its
/// quotes and hyphens as ASCII writes them, lower-cased.
fn plain(value: &str) -> String {
    let words: Vec<&str> = value.split_whitespace().collect();
    words
        .join(" ")
        .chars()
        .map(crate::clean::plain_punctuation)
        .collect::<String>()
        .to_lowercase()
}

/// Whether `value` stands for no value in `field`.
#[must_use]
pub fn placeholder(field: Field, value: &str) -> bool {
    let plain = plain(value);
    let listed = [None, Some(field)]
        .iter()
        .any(|f| PLACEHOLDERS.get(f).is_some_and(|set| set.contains(&plain)));
    listed
        || (CREDITS.contains(&field) && names_a_soundtrack(value))
        || (field == Field::ReleaseCountry && !is_country(value.trim()))
}

/// Whether `code` is written as a country is: two letters, as `GB`, or
/// MusicBrainz's `XW` for the whole world.
fn is_country(code: &str) -> bool {
    code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())
}

/// Whether a credit names a soundtrack rather than anyone: `Show OST`,
/// `Show OST 2`, `Show OST III`.
fn names_a_soundtrack(value: &str) -> bool {
    let words: Vec<&str> = value.split_whitespace().collect();
    let numbered = |w: &str| {
        w.chars().all(|c| c.is_ascii_digit())
            || (!w.is_empty() && w.chars().all(|c| matches!(c, 'I' | 'V' | 'X')))
    };
    match words.as_slice() {
        [_, .., "OST"] => true,
        [_, .., "OST", n] => numbered(n),
        _ => false,
    }
}

/// One source as the watermarks are found among: its group, the group's
/// own name where it has one, as a torrent's, and its offers.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub group: &'a str,
    pub name: Option<&'a str>,
    pub offers: &'a Offers,
}

/// The watermarks of every group, by group and folded value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watermarks(BTreeSet<(String, String)>);

/// What one group's sources show of one value in [`PER_WORK`].
#[derive(Default)]
struct Seen {
    albums: BTreeSet<String>,
    titles: BTreeSet<String>,
    as_work: bool,
}

impl Watermarks {
    /// The watermarks among `sources`, whatever order they come in.
    pub fn of<'a>(sources: impl IntoIterator<Item = Source<'a>>) -> Self {
        let mut seen: BTreeMap<(String, String), Seen> = BTreeMap::new();
        let mut found = BTreeSet::new();
        for source in sources {
            let first = |field: Field| {
                source
                    .offers
                    .get(&field)
                    .and_then(|o| o.values.first())
                    .map(|v| fold(v))
                    .unwrap_or_default()
            };
            let album = source
                .offers
                .get(&Field::Album)
                .and_then(|o| o.values.first())
                .map(|a| fold(crate::parts::album(a).name))
                .unwrap_or_default();
            let title = first(Field::Title);
            let tags = source.name.map(name_tags).unwrap_or_default();
            let mut kinds_of: BTreeMap<String, BTreeSet<Field>> = BTreeMap::new();
            for field in PER_WORK {
                let Some(offer) = source.offers.get(&field) else {
                    continue;
                };
                let values: BTreeSet<String> = offer
                    .values
                    .iter()
                    .map(|v| fold(v))
                    .filter(|v| !v.is_empty())
                    .collect();
                for value in values {
                    let key = (source.group.to_string(), value.clone());
                    if tags.contains(&value) {
                        found.insert(key.clone());
                    }
                    kinds_of.entry(value).or_default().insert(kind_of(field));
                    let s = seen.entry(key).or_default();
                    s.albums.insert(album.clone());
                    s.titles.insert(title.clone());
                    s.as_work |= field == Field::Work;
                }
            }
            for (value, kinds) in kinds_of {
                if kinds.len() >= 2 {
                    found.insert((source.group.to_string(), value));
                }
            }
        }
        for (key, s) in seen {
            if s.as_work && s.albums.len() >= ALBUMS && s.titles.len() >= TITLES {
                found.insert(key);
            }
        }
        Self(found)
    }

    /// Whether `value`, in a field of [`PER_WORK`], is a watermark of
    /// `group`.
    #[must_use]
    pub fn holds(&self, group: &str, value: &str) -> bool {
        self.0.contains(&(group.to_string(), fold(value)))
    }
}

/// The field of [`PER_WORK`] a value in `field` names the same thing as:
/// a grouping holds a work as taggers of classical music write it, and a
/// copyright names its label, so a value in both of one pair is no
/// watermark.
fn kind_of(field: Field) -> Field {
    match field {
        Field::Grouping => Field::Work,
        Field::Copyright => Field::Label,
        other => other,
    }
}

/// The tags a torrent's name carries, folded: each bracketed stretch, and
/// what follows a final dash with no space after it, as a release group
/// signs a name.
fn name_tags(name: &str) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    for (open, close) in [('[', ']'), ('{', '}')] {
        let mut rest = name;
        while let Some(start) = rest.find(open) {
            let inner = &rest[start + open.len_utf8()..];
            let end = inner.find(close).unwrap_or(inner.len());
            tags.insert(fold(&inner[..end]));
            rest = &inner[end..];
        }
    }
    if let Some((_, last)) = name.trim_end().rsplit_once('-')
        && !last.is_empty()
        && !last.contains(char::is_whitespace)
    {
        tags.insert(fold(last));
    }
    tags.remove("");
    tags
}

/// `value`'s letters and digits, lower-cased.
fn fold(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::Offer;

    fn offers(pairs: &[(Field, &str)]) -> Offers {
        pairs
            .iter()
            .map(|(f, v)| {
                (
                    *f,
                    Offer {
                        values: vec![(*v).to_string()],
                        structured: true,
                        also: Vec::new(),
                    },
                )
            })
            .collect()
    }

    fn found(sources: &[(&str, Option<&str>, Offers)]) -> Watermarks {
        Watermarks::of(sources.iter().map(|(group, name, offers)| Source {
            group,
            name: *name,
            offers,
        }))
    }

    #[test]
    fn a_placeholder_is_listed_for_its_field_and_compared_whole() {
        assert!(placeholder(Field::Label, "[no label]"));
        assert!(placeholder(Field::Title, "[Unknown]"), "every field's");
        assert!(placeholder(Field::Artist, "Unknown Artist"));
        assert!(!placeholder(Field::Title, "Unknown"), "a title of its own");
        assert!(!placeholder(Field::AlbumArtist, "Various Artists"));
        assert!(placeholder(Field::Artist, "Various Artists"));
        assert!(!placeholder(Field::Label, "Glass Orchard Records"));
        assert!(placeholder(Field::ReleaseCountry, "GRP"), "no country");
        assert!(!placeholder(Field::ReleaseCountry, "XW"));
    }

    #[test]
    fn a_placeholder_is_one_however_cleaning_spelled_it() {
        for value in [
            "[non-album tracks]",
            "[non\u{2010}album tracks]",
            "[Non\u{2011}Album  Tracks]",
            "[non-album\u{a0}tracks]",
        ] {
            assert!(placeholder(Field::Album, value), "{value}");
        }
        assert!(placeholder(Field::Artist, "Unknown\u{a0}Artist"));
        assert!(!placeholder(Field::Album, "[non album tracks]"));
    }

    #[test]
    fn a_soundtrack_s_short_name_credits_no_one() {
        for name in [
            "Night Ferry OST",
            "Night Ferry OST 2",
            "Night Ferry OST III",
        ] {
            assert!(placeholder(Field::Artist, name), "{name}");
        }
        for name in [
            "OST",
            "Night Ferry OST Band",
            "Ostrich",
            "Night Ferry Ost 2",
        ] {
            assert!(!placeholder(Field::Artist, name), "{name}");
        }
        assert!(
            !placeholder(Field::Title, "Night Ferry OST"),
            "credits alone"
        );
    }

    #[test]
    fn a_value_tagging_its_torrent_s_name_is_a_watermark_in_every_field() {
        let name = Some("Marlo Venn - Discography [FLAC] [GRP]");
        let w = found(&[
            ("t1", name, offers(&[(Field::Work, "GRP")])),
            (
                "t1",
                name,
                offers(&[(Field::Label, "Glass Orchard Records")]),
            ),
        ]);
        assert!(w.holds("t1", "GRP"));
        assert!(w.holds("t1", "grp"), "case aside");
        assert!(!w.holds("t1", "Glass Orchard Records"));
        assert!(!w.holds("t2", "GRP"), "another group's");
        let signed = found(&[(
            "t1",
            Some("Night Ferry (2019) FLAC-GRP"),
            offers(&[(Field::Copyright, "GRP")]),
        )]);
        assert!(signed.holds("t1", "GRP"));
    }

    #[test]
    fn one_value_in_two_fields_of_one_source_is_a_watermark() {
        let w = found(&[(
            "t1",
            None,
            offers(&[(Field::Work, "GRP"), (Field::Copyright, "GRP")]),
        )]);
        assert!(w.holds("t1", "GRP"));
        let named_alike = found(&[(
            "t1",
            None,
            offers(&[
                (Field::Work, "Night Ferry Suite"),
                (Field::Grouping, "Night Ferry Suite"),
                (Field::Label, "Glass Orchard Records"),
                (Field::Copyright, "Glass Orchard Records"),
            ]),
        )]);
        assert_eq!(
            named_alike,
            Watermarks::default(),
            "a grouping and a copyright"
        );
    }

    #[test]
    fn a_work_across_albums_and_titles_is_none_and_a_label_across_them_is_one() {
        let mut sources = Vec::new();
        for n in 0..12 {
            let album = format!("Album {}", n % 4);
            let title = format!("Song {n}");
            sources.push((
                "t1",
                None,
                offers(&[
                    (Field::Album, album.as_str()),
                    (Field::Title, title.as_str()),
                    (Field::Work, "GRP"),
                    (Field::Label, "Glass Orchard Records"),
                ]),
            ));
        }
        let w = found(&sources);
        assert!(w.holds("t1", "GRP"));
        assert!(!w.holds("t1", "Glass Orchard Records"));
        let movements: Vec<_> = (1..=4)
            .map(|n| {
                (
                    "t1",
                    None,
                    offers(&[
                        (Field::Album, "Night Ferry Symphony"),
                        (Field::Title, ["I", "II", "III", "IV"][n - 1]),
                        (Field::Work, "Symphony No. 2"),
                    ]),
                )
            })
            .collect();
        assert!(!found(&movements).holds("t1", "Symphony No. 2"));
        let scenes: Vec<_> = (0..12)
            .map(|n| {
                let album = format!("Night Ferry Opera CD {}", n % 3 + 1);
                let title = format!("Scene {n}");
                let tags = offers(&[
                    (Field::Album, album.as_str()),
                    (Field::Title, title.as_str()),
                    (Field::Work, "Night Ferry Opera"),
                ]);
                ("t1", None, tags)
            })
            .collect();
        assert!(
            !found(&scenes).holds("t1", "Night Ferry Opera"),
            "one album's discs"
        );
    }

    #[test]
    fn the_order_sources_come_in_changes_nothing() {
        let a = (
            "t1",
            None,
            offers(&[(Field::Label, "GRP"), (Field::Work, "GRP")]),
        );
        let b = (
            "t1",
            None,
            offers(&[(Field::Label, "Glass Orchard Records")]),
        );
        assert_eq!(found(&[a.clone(), b.clone()]), found(&[b, a]));
    }
}
