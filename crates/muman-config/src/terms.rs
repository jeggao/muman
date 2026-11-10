//! A query's terms as written: the grammar `query.rs` matches songs by,
//! read where the song list's settings hold a query, as a provider's
//! `until` and `skip` do. `query.rs` says what each term asks.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use regex::{Regex, RegexBuilder};
use unicode_normalization::UnicodeNormalization;

use crate::provider::Provider;
use crate::source::SourceKey;
use crate::tags::{self, Field};

/// Fields that are no tag: what the song is made of and written as.
pub const DERIVED: [&str; 7] = ["key", "path", "format", "cover", "lyrics", "from", "has"];

#[derive(Debug, Clone)]
pub enum Test {
    /// Bare text, folded: each of its words, or all of it as a key's ID
    /// or a file's name.
    Word(String),
    Key(SourceKey),
    Contains(String, String),
    Equals(String, String),
    Matches(String, Regex),
    /// A closed field's value, compared whole.
    Is(Closed),
}

/// What a closed field's term asks of a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    Lyrics(Have),
    Cover(Have),
    From(Provider),
    /// A tag, by its Vorbis name, that a source or the song list gives.
    Has(String),
}

/// How much of its lyrics or cover a song has: `Better` timed lyrics or a
/// sharp cover, `Worse` untimed lyrics or a small cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Have {
    Some,
    None,
    Better,
    Worse,
}

#[derive(Debug, Clone)]
pub struct Term {
    pub negated: bool,
    pub test: Test,
}

/// A parsed query; with no terms it matches every song.
#[derive(Debug, Clone, Default)]
pub struct Query(pub Vec<Term>);

/// `text` without one pair of double quotes around it, so that
/// `album:="The Glass Orchards"` reads alike whether a shell took its
/// quotes or they reached muman.
fn unquoted(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text)
}

impl Query {
    /// Parse the terms of a query. `extractors` are the key schemes in
    /// use, so `youtube.com:<id>` names a key where `composer:quill` names
    /// a field.
    pub fn parse(terms: &[String], extractors: &BTreeSet<String>) -> Result<Self> {
        Self::parse_terms(terms, extractors)
            .map_err(|e| crate::errors::Refused(format!("{e:#}")).into())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many keys the query names.
    #[must_use]
    pub fn keys_named(&self) -> usize {
        self.0
            .iter()
            .filter(|t| !t.negated && matches!(t.test, Test::Key(_)))
            .count()
    }

    /// Whether the query is keys alone, each naming its song.
    #[must_use]
    pub fn names_keys(&self) -> bool {
        !self.0.is_empty()
            && self
                .0
                .iter()
                .all(|t| !t.negated && matches!(t.test, Test::Key(_)))
    }
    pub fn parse_terms(terms: &[String], extractors: &BTreeSet<String>) -> Result<Self> {
        let mut parsed = Vec::new();
        for raw in terms {
            let (negated, body) = match raw.strip_prefix('^') {
                Some(rest) => (true, rest),
                None => (false, raw.as_str()),
            };
            if body.trim().is_empty() {
                bail!("`{raw}` is an empty term");
            }
            let test = match body.split_once(':') {
                None => Test::Word(fold(body)),
                Some(("", _)) => bail!("`{raw}` names no field"),
                Some((scheme, rest))
                    if extractors.contains(&scheme.to_ascii_lowercase()) && !is_field(scheme) =>
                {
                    Test::Key(SourceKey::parse(&format!(
                        "{}:{rest}",
                        scheme.to_ascii_lowercase()
                    ))?)
                }
                Some((field, rest)) if closed_field(field) && !rest.starts_with(':') => {
                    let text = rest.strip_prefix('=').unwrap_or(rest);
                    Test::Is(
                        Closed::parse(field, unquoted(text)).with_context(|| format!("`{raw}`"))?,
                    )
                }
                Some((field, rest)) => {
                    let field = field.to_string();
                    if ["from", "has"].contains(&field.to_ascii_lowercase().as_str()) {
                        bail!("`{raw}`: {field} takes a name, not a pattern");
                    }
                    if let Some(pattern) = rest.strip_prefix(':') {
                        let re = RegexBuilder::new(&fold_pattern(pattern))
                            .case_insensitive(true)
                            .build()
                            .with_context(|| format!("`{raw}`"))?;
                        Test::Matches(field, re)
                    } else {
                        let (exact, text) = match rest.strip_prefix('=') {
                            Some(text) => (true, fold(unquoted(text))),
                            None => (false, fold(unquoted(rest))),
                        };
                        if exact {
                            Test::Equals(field, text)
                        } else {
                            Test::Contains(field, text)
                        }
                    }
                }
            };
            parsed.push(Term { negated, test });
        }
        Ok(Self(parsed))
    }
}

/// `text` as a query compares it: lower case, in one Unicode form, its
/// accents taken off, the punctuation typography respells written
/// plain, so `twenty-five` finds `Twenty‐Five`, and on one line, so
/// `comment:"one two"` finds a comment of two lines.
#[must_use]
pub fn fold(text: &str) -> String {
    let plain: String = tags::joined(text)
        .chars()
        .map(crate::clean::plain_punctuation)
        .collect();
    plain
        .nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
        .nfc()
        .collect()
}

/// A regular expression as [`fold`] folds the text it is matched
/// against, its accents taken off and its typography written plain, its
/// case kept: lower-cased, `\D` would read as `\d`, and the pattern is
/// matched ignoring case anyway.
fn fold_pattern(pattern: &str) -> String {
    let plain: String = pattern
        .chars()
        .map(crate::clean::plain_punctuation)
        .collect();
    plain
        .nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .nfc()
        .collect()
}

#[must_use]
pub fn is_field(name: &str) -> bool {
    DERIVED.contains(&name.to_ascii_lowercase().as_str()) || Field::named(name).is_some()
}

/// Whether `field` takes one of a few values, compared whole.
fn closed_field(field: &str) -> bool {
    ["lyrics", "cover", "from", "has"].contains(&field.to_ascii_lowercase().as_str())
}

impl Closed {
    /// The term `field:text` of a closed field.
    pub fn parse(field: &str, text: &str) -> Result<Self> {
        let have = |what: &str, better: &str, worse: &str| {
            Ok(match fold(text).as_str() {
                "yes" => Have::Some,
                "none" => Have::None,
                t if t == better => Have::Better,
                t if t == worse => Have::Worse,
                _ => bail!("{what} is yes, none, {better} or {worse}"),
            })
        };
        Ok(match field.to_ascii_lowercase().as_str() {
            "lyrics" => Self::Lyrics(have("lyrics", "timed", "untimed")?),
            "cover" => Self::Cover(have("cover", "sharp", "small")?),
            "from" => {
                let p = Provider::named(&text.to_ascii_lowercase())?;
                if p == Provider::AcoustId {
                    bail!("no source is from acoustid; its finds are from musicbrainz");
                }
                Self::From(p)
            }
            "has" if text.trim().is_empty() => bail!("has names no tag"),
            "has" => Self::Has(tags::vorbis_key(text.trim())),
            _ => bail!("`{field}` is no closed field"),
        })
    }
}

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let have = |h: &Have, better: &'static str, worse: &'static str| match h {
            Have::Some => "yes",
            Have::None => "none",
            Have::Better => better,
            Have::Worse => worse,
        };
        match self {
            Self::Lyrics(h) => write!(f, "lyrics:{}", have(h, "timed", "untimed")),
            Self::Cover(h) => write!(f, "cover:{}", have(h, "sharp", "small")),
            Self::From(p) => write!(f, "from:{p}"),
            Self::Has(tag) => write!(f, "has:{}", tags::song_list_name(tag)),
        }
    }
}

/// A query written as one string, as the song list's settings hold one:
/// split into terms as a shell would, so `album:="The Glass Orchards"` is
/// one term. An empty one, which would match every song, is refused.
pub fn parse_text(text: &str, extractors: &BTreeSet<String>) -> Result<Query> {
    let terms = shell_words::split(text).with_context(|| format!("`{text}`"))?;
    if terms.is_empty() {
        bail!("an empty query would match every song");
    }
    Query::parse_terms(&terms, extractors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typography_folds_as_what_it_respelled() {
        assert_eq!(
            fold("Twenty\u{2010}Five \u{2019}n\u{2019} \u{201c}x\u{201d}"),
            fold("twenty-five 'n' \"x\"")
        );
    }
}
