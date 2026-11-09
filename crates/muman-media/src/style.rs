//! The house style a song's title and artist are written in, once the
//! sources' values are cleaned and chosen, as `[tags]` asks.
//!
//! Sources write a featured artist, a version and a note on where a song
//! comes from each their own way: `Night Ferry (feat. Kiri Hoshino)`
//! beside `Marlo Venn feat. Kiri Hoshino`, `[Live]` beside `(Live)`, a
//! title closed by `(From "Harbor Lights")`. Read into parts
//! ([`crate::parts`]), each is written one way:
//!
//! - `featured`: `keep` as the source wrote it; `title`, a featured
//!   artist the artist credits moved into the title, as `(feat. …)`;
//!   `artist`, one the title names moved into the credit, after `feat.`.
//!   A name already where it goes is not written twice.
//! - `versions`: `keep`; or every version and soft version the title
//!   names written in `parentheses` or `brackets`.
//! - `context`: `keep`; or `drop` a decoration saying where the song is
//!   from, as `From "Harbor Lights"`, which names another work rather
//!   than the song.
//!
//! Each key keeps values as they are by default. A tag set by hand is
//! never restyled.

use serde::Deserialize;

use crate::parts::{self, Kind};

/// Where a featured artist is written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Featured {
    #[default]
    Keep,
    Title,
    Artist,
}

/// How a title's versions are bracketed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Versions {
    #[default]
    Keep,
    Parentheses,
    Brackets,
}

/// Whether a note on where a song is from stays in its title.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Context {
    #[default]
    Keep,
    Drop,
}

/// The house style, as `[tags]` sets it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub featured: Featured,
    pub versions: Versions,
    pub context: Context,
}

impl Style {
    /// Whether it changes nothing.
    #[must_use]
    pub fn keeps(&self) -> bool {
        *self == Self::default()
    }

    /// `title` and `artist`, the song's, in this style.
    #[must_use]
    pub fn apply(&self, title: &str, artist: &str) -> (String, String) {
        if self.keeps() {
            return (title.to_string(), artist.to_string());
        }
        let parsed = parts::title(title);
        let credit = parts::credit(artist);
        let in_title: Vec<String> = (parsed.decorations.iter())
            .filter(|d| d.kind == Kind::Featuring)
            .flat_map(|d| parts::credit(featured_names(&d.text)).names)
            .map(str::to_string)
            .collect();
        let in_credit: Vec<&str> = credit
            .featured
            .map_or(Vec::new(), |at| credit.names[at..].to_vec());
        let mut decorations: Vec<String> = Vec::new();
        for d in &parsed.decorations {
            let context = d.kind == Kind::Other && is_context(&d.text);
            match d.kind {
                Kind::Featuring if self.featured == Featured::Artist => {}
                _ if context && self.context == Context::Drop => {}
                Kind::Version(_) | Kind::Soft if self.versions != Versions::Keep => {
                    let (open, close) = match self.versions {
                        Versions::Brackets => ('[', ']'),
                        _ => ('(', ')'),
                    };
                    decorations.push(format!("{open}{}{close}", d.text));
                }
                _ => decorations.push(written(&d.text, d.brackets)),
            }
        }
        let mut artist = artist.to_string();
        match self.featured {
            Featured::Title if !in_credit.is_empty() => {
                let lead = credit
                    .featured
                    .map_or(&credit.names[..], |at| &credit.names[..at]);
                let joined = joined(lead, &credit.joiners);
                let missing: Vec<&str> = (in_credit.iter())
                    .filter(|n| !in_title.iter().any(|t| t.eq_ignore_ascii_case(n)))
                    .copied()
                    .collect();
                if !missing.is_empty() {
                    decorations.insert(0, format!("(feat. {})", missing.join(", ")));
                }
                artist = joined;
            }
            Featured::Artist if !in_title.is_empty() => {
                let missing: Vec<&String> = (in_title.iter())
                    .filter(|n| !credit.names.iter().any(|c| c.eq_ignore_ascii_case(n)))
                    .collect();
                if !missing.is_empty() {
                    let names: Vec<&str> = missing.iter().map(|n| n.as_str()).collect();
                    artist = format!("{artist} feat. {}", names.join(", "));
                }
            }
            _ => {}
        }
        let title = std::iter::once(parsed.core)
            .chain(decorations)
            .collect::<Vec<_>>()
            .join(" ");
        (title, artist)
    }
}

/// The names a featuring decoration gives, its opening word left out.
fn featured_names(text: &str) -> &str {
    text.split_once(char::is_whitespace)
        .map_or("", |(_, names)| names)
        .trim()
}

/// Whether a decoration says where the song is from, as `From "Harbor
/// Lights"` or `from the film Harbor Lights`.
fn is_context(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.starts_with("from ")
        || lower.starts_with("theme from ")
        || lower.starts_with("taken from ")
}

/// A decoration as it was written: in its brackets, else after a dash.
fn written(text: &str, brackets: Option<(char, char)>) -> String {
    match brackets {
        Some((open, close)) => format!("{open}{text}{close}"),
        None => format!("- {text}"),
    }
}

/// `names` joined by the joiners between them, as the credit wrote them.
fn joined(names: &[&str], joiners: &[&str]) -> String {
    let mut out = String::new();
    for (at, name) in names.iter().enumerate() {
        if at > 0 {
            let joiner = joiners.get(at - 1).copied().unwrap_or(",");
            match joiner {
                "," | "、" | ";" => out.push_str(joiner),
                _ => {
                    out.push(' ');
                    out.push_str(joiner);
                }
            }
            out.push(' ');
        }
        out.push_str(name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(featured: Featured, versions: Versions, context: Context) -> Style {
        Style {
            featured,
            versions,
            context,
        }
    }

    #[test]
    fn the_default_style_keeps_every_value_as_it_is() {
        let s = Style::default();
        assert_eq!(
            s.apply("Night Ferry [Live] (feat. Kiri Hoshino)", "Marlo Venn"),
            (
                "Night Ferry [Live] (feat. Kiri Hoshino)".to_string(),
                "Marlo Venn".to_string()
            )
        );
    }

    #[test]
    fn a_featured_artist_moves_where_the_style_puts_it_once() {
        let to_artist = style(Featured::Artist, Versions::Keep, Context::Keep);
        assert_eq!(
            to_artist.apply("Night Ferry (feat. Kiri Hoshino)", "Marlo Venn"),
            (
                "Night Ferry".to_string(),
                "Marlo Venn feat. Kiri Hoshino".to_string()
            )
        );
        assert_eq!(
            to_artist
                .apply(
                    "Night Ferry (feat. Kiri Hoshino)",
                    "Marlo Venn feat. Kiri Hoshino"
                )
                .1,
            "Marlo Venn feat. Kiri Hoshino",
            "never twice"
        );
        let to_title = style(Featured::Title, Versions::Keep, Context::Keep);
        assert_eq!(
            to_title.apply("Night Ferry", "Marlo Venn & Ada Quill feat. Kiri Hoshino"),
            (
                "Night Ferry (feat. Kiri Hoshino)".to_string(),
                "Marlo Venn & Ada Quill".to_string()
            )
        );
    }

    #[test]
    fn versions_take_one_bracket_and_a_note_on_where_it_is_from_may_go() {
        let s = style(Featured::Keep, Versions::Parentheses, Context::Drop);
        assert_eq!(
            s.apply(
                "Night Ferry [Live] (From \"Harbor Lights\") - Radio Edit",
                "Marlo Venn"
            )
            .0,
            "Night Ferry (Live) (Radio Edit)"
        );
        let brackets = style(Featured::Keep, Versions::Brackets, Context::Keep);
        assert_eq!(
            brackets
                .apply("Night Ferry (Acoustic) (Harbor Session)", "Marlo Venn")
                .0,
            "Night Ferry [Acoustic] (Harbor Session)"
        );
    }
}
