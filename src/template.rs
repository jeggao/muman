//! The library path template: MiniJinja, the Jinja2 dialect, rendered
//! once per song into a path without its extension.
//!
//! | Variable | Holds |
//! |---|---|
//! | `title`, `album`, `album_artist`, `genre`, `date` | The resolved tag, or the fallback name for a missing title, album or album artist |
//! | `artist`, `artists` | The first artist, and every artist as a list |
//! | `year` | The first four digits of `date`, or empty |
//! | `track`, `disc` | The track and disc numbers, or none |
//! | `disc_track` | `"03 "` on a first disc, `"2-03 "` on a later one, `""` without a track |
//! | `id` | The ID or file name of the song's audio source |
//!
//! Besides MiniJinja's own filters (`default`, `lower`, `upper`,
//! `replace`, `first`, `join`, …) there are `pad(n)`, a number with
//! leading zeros; `truncate(n)`, the first `n` characters; `asciify`,
//! the text transliterated to ASCII; and `the_suffix`, `The Orchards` as
//! `Orchards, The`. Every value arrives already safe for a path, so a `/` in
//! a title never makes a folder; only the template's own `/` does.

use anyhow::{Context, Result};
use minijinja::{Environment, UndefinedBehavior, Value};
use serde::Serialize;

/// What a song's path is made from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Fields {
    pub title: String,
    pub artist: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub date: String,
    pub year: String,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub disc_track: String,
    pub id: String,
}

/// A compiled template.
#[derive(Debug)]
pub struct Template {
    env: Environment<'static>,
}

const NAME: &str = "path";

impl Template {
    pub fn new(source: &str) -> Result<Self> {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.set_keep_trailing_newline(false);
        env.add_filter("pad", |n: Value, width: usize| match n.as_i64() {
            Some(n) => format!("{n:0width$}"),
            None => n.to_string(),
        });
        env.add_filter("truncate", |s: String, n: usize| {
            s.chars().take(n).collect::<String>()
        });
        env.add_filter("asciify", |s: String| deunicode::deunicode(&s));
        env.add_filter("the_suffix", |s: String| match s.strip_prefix("The ") {
            Some(rest) if !rest.is_empty() => format!("{rest}, The"),
            _ => s,
        });
        env.add_template_owned(NAME, source.to_string())
            .context("reading the library template")?;
        Ok(Self { env })
    }

    pub fn render(&self, fields: &Fields) -> Result<String> {
        self.env
            .get_template(NAME)?
            .render(fields)
            .context("rendering the library template")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::DEFAULT_TEMPLATE;

    fn fields() -> Fields {
        Fields {
            title: "Lantern Weather".into(),
            artist: "Paper Comets".into(),
            artists: vec!["Paper Comets".into(), "Ada Quill".into()],
            album: "Harbor Lights".into(),
            album_artist: "The Paper Comets".into(),
            date: "2024-05-01".into(),
            year: "2024".into(),
            track: Some(3),
            disc: Some(2),
            disc_track: "2-03 ".into(),
            id: "vid00000001".into(),
            ..Fields::default()
        }
    }

    #[test]
    fn the_default_template_reads_artist_album_and_numbered_title() {
        let t = Template::new(DEFAULT_TEMPLATE).unwrap();
        assert_eq!(
            t.render(&fields()).unwrap(),
            "The Paper Comets/Harbor Lights/2-03 Lantern Weather"
        );
    }

    #[test]
    fn filters_and_conditions_shape_a_path() {
        let t = Template::new(
            "{{ album_artist | the_suffix }}/{% if year %}{{ year }} - {% endif %}{{ album }}/\
             {{ track | pad(3) }} {{ artists | join(', ') | truncate(20) }} - {{ title | asciify }}",
        )
        .unwrap();
        assert_eq!(
            t.render(&fields()).unwrap(),
            "Paper Comets, The/2024 - Harbor Lights/003 Paper Comets, Ada Qu - Lantern Weather"
        );
    }

    #[test]
    fn an_unknown_variable_is_an_error_not_a_blank() {
        let t = Template::new("{{ albm }}/{{ title }}").unwrap();
        assert!(t.render(&fields()).is_err());
        assert!(Template::new("{{ title").is_err());
    }
}
