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

    /// `title` and `artist`, the song's, in this style, with the names
    /// moved into the artist's credit and out of it.
    #[must_use]
    pub fn apply(&self, title: &str, artist: &str) -> Styled {
        let mut styled = Styled {
            title: title.to_string(),
            artist: artist.to_string(),
            added: Vec::new(),
            removed: Vec::new(),
        };
        if self.keeps() {
            return styled;
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
        let mut changed = false;
        let mut decorations: Vec<String> = Vec::new();
        for d in &parsed.decorations {
            let context = d.kind == Kind::Other && is_context(&d.text);
            match d.kind {
                Kind::Featuring if self.featured == Featured::Artist => changed = true,
                _ if context && self.context == Context::Drop => changed = true,
                Kind::Version(_) | Kind::Soft if self.versions != Versions::Keep => {
                    let wanted = match self.versions {
                        Versions::Brackets => ('[', ']'),
                        _ => ('(', ')'),
                    };
                    changed |= d.brackets != Some(wanted);
                    decorations.push(format!("{}{}{}", wanted.0, d.text, wanted.1));
                }
                _ => decorations.push(written(&d.text, d.brackets)),
            }
        }
        match self.featured {
            Featured::Title if !in_credit.is_empty() => {
                let missing: Vec<&str> = (in_credit.iter())
                    .filter(|n| !in_title.iter().any(|t| crate::similar::caseless(t, n)))
                    .copied()
                    .collect();
                if !missing.is_empty() {
                    let all: Vec<&str> = (in_title.iter().map(String::as_str))
                        .chain(missing)
                        .collect();
                    let feat = format!("(feat. {})", all.join(", "));
                    let kept: Vec<&parts::Decoration> = (parsed.decorations.iter())
                        .filter(|d| {
                            let context = d.kind == Kind::Other && is_context(&d.text);
                            !(context && self.context == Context::Drop)
                        })
                        .collect();
                    match kept.iter().position(|d| d.kind == Kind::Featuring) {
                        Some(at) => decorations[at] = feat,
                        None => decorations.insert(0, feat),
                    }
                    changed = true;
                }
                if let Some(at) = credit.featured_at {
                    styled.artist = artist[..at].trim_end().to_string();
                }
                styled.removed = in_credit.iter().map(|n| (*n).to_string()).collect();
            }
            Featured::Artist if !in_title.is_empty() => {
                let missing: Vec<&str> = (in_title.iter())
                    .filter(|n| !credit.names.iter().any(|c| crate::similar::caseless(c, n)))
                    .map(String::as_str)
                    .collect();
                if !missing.is_empty() {
                    let joiner = if credit.featured.is_some() {
                        ","
                    } else {
                        " feat."
                    };
                    styled.artist = format!("{artist}{joiner} {}", missing.join(", "));
                    styled.added = missing.iter().map(|n| (*n).to_string()).collect();
                }
            }
            _ => {}
        }
        if changed {
            let core = parsed.core.split_whitespace().collect::<Vec<_>>().join(" ");
            styled.title = std::iter::once(core)
                .chain(decorations)
                .collect::<Vec<_>>()
                .join(" ");
        }
        styled
    }
}

/// A title and artist written in a [`Style`], and the names it moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Styled {
    pub title: String,
    pub artist: String,
    /// The featured names moved into the artist's credit.
    pub added: Vec<String>,
    /// The featured names moved out of it, into the title.
    pub removed: Vec<String>,
}

/// The names a featuring decoration gives, its opening word left out,
/// whether a space follows it or not, as `ft.Kiri Hoshino`.
fn featured_names(text: &str) -> &str {
    let lower = text.to_lowercase();
    ["featuring", "feat.", "feat", "ft.", "ft"]
        .iter()
        .find(|w| lower.starts_with(*w))
        .map_or(text, |w| &text[w.len()..])
        .trim()
}

/// Whether a decoration says where the song is from, as `From "Harbor
/// Lights"` or `from the film Harbor Lights`: one opening with `from` that
/// quotes a work or names what kind it is, so a subtitle as `From Me to
/// You` stays.
fn is_context(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    let opens = ["from ", "theme from ", "taken from "]
        .iter()
        .any(|o| lower.starts_with(o));
    let quotes = lower.contains(['"', '“', '”', '‘', '’', '「', '」', '『', '』']);
    let kinds = [
        "film",
        "movie",
        "soundtrack",
        "anime",
        "series",
        "game",
        "musical",
        "show",
    ];
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    opens && (quotes || kinds.iter().any(|k| words.contains(k)))
}

/// A decoration as it was written: in its brackets, else after a dash.
fn written(text: &str, brackets: Option<(char, char)>) -> String {
    match brackets {
        Some((open, close)) => format!("{open}{text}{close}"),
        None => format!("- {text}"),
    }
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

    fn written(s: Style, title: &str, artist: &str) -> (String, String) {
        let styled = s.apply(title, artist);
        (styled.title, styled.artist)
    }

    #[test]
    fn the_default_style_keeps_every_value_as_it_is() {
        let s = Style::default();
        let (title, artist) = ("Night Ferry [Live] (feat. Kiri Hoshino)", "Marlo Venn");
        assert_eq!(
            written(s, title, artist),
            (title.to_string(), artist.to_string())
        );
    }

    #[test]
    fn a_featured_artist_moves_where_the_style_puts_it_once() {
        let to_artist = style(Featured::Artist, Versions::Keep, Context::Keep);
        let moved = to_artist.apply("Night Ferry (feat. Ada Quill)", "Salt & Pepper");
        assert_eq!(moved.title, "Night Ferry");
        assert_eq!(moved.artist, "Salt & Pepper feat. Ada Quill");
        assert_eq!(moved.added, ["Ada Quill"]);
        assert_eq!(
            written(
                to_artist,
                "Night Ferry (feat. Ada Quill)",
                "Marlo Venn feat. Kiri Hoshino"
            )
            .1,
            "Marlo Venn feat. Kiri Hoshino, Ada Quill",
            "one featured list"
        );
        assert_eq!(
            written(
                to_artist,
                "Night Ferry (feat. Kiri Hoshino)",
                "Marlo Venn feat. Kiri Hoshino"
            )
            .1,
            "Marlo Venn feat. Kiri Hoshino",
            "never twice"
        );
        assert_eq!(
            written(to_artist, "Night Ferry (ft.Kiri Hoshino)", "Marlo Venn").1,
            "Marlo Venn feat. Kiri Hoshino"
        );
        let subtitle = ("Stay Close (With You)", "Marlo Venn");
        assert_eq!(
            written(to_artist, subtitle.0, subtitle.1),
            (subtitle.0.to_string(), subtitle.1.to_string()),
            "a subtitle, no one featured"
        );
        let to_title = style(Featured::Title, Versions::Keep, Context::Keep);
        let moved = to_title.apply("Night Ferry", "Kilo/Vex feat. Kiri Hoshino");
        assert_eq!(
            (moved.title.as_str(), moved.artist.as_str()),
            ("Night Ferry (feat. Kiri Hoshino)", "Kilo/Vex"),
            "the lead credit as written"
        );
        assert_eq!(moved.removed, ["Kiri Hoshino"]);
        assert_eq!(
            written(
                to_title,
                "Night Ferry (feat. Ada Quill)",
                "Marlo Venn feat. Kiri Hoshino"
            )
            .0,
            "Night Ferry (feat. Ada Quill, Kiri Hoshino)"
        );
    }

    #[test]
    fn versions_take_one_bracket_and_a_note_on_where_it_is_from_may_go() {
        let s = style(Featured::Keep, Versions::Parentheses, Context::Drop);
        assert_eq!(
            written(
                s,
                "Night Ferry [Live] (From \"Harbor Lights\") - Radio Edit",
                "Marlo Venn"
            )
            .0,
            "Night Ferry (Live) (Radio Edit)"
        );
        let brackets = style(Featured::Keep, Versions::Brackets, Context::Keep);
        assert_eq!(
            written(
                brackets,
                "Night Ferry (Acoustic) (Harbor Session)",
                "Marlo Venn"
            )
            .0,
            "Night Ferry [Acoustic] (Harbor Session)"
        );
        let drop = style(Featured::Keep, Versions::Keep, Context::Drop);
        for kept in [
            "Night Ferry (Live) Reprise",
            "Night Ferry (From Me to You)",
            "「夜の船」",
        ] {
            assert_eq!(
                written(drop, kept, "Marlo Venn").0,
                kept,
                "nothing to change"
            );
        }
    }
}
