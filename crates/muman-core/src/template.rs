//! The library path template: MiniJinja, the Jinja2 dialect, rendered
//! once per song into a path without its extension.
//!
//! | Variable | Holds |
//! |---|---|
//! | `title`, `album`, `album_artist`, `genre`, `date` | The resolved tag, or the fallback name for a missing title, album or album artist |
//! | `artist`, `artists` | The first artist, and every artist as a list |
//! | `year` | The first four digits of `date`, or empty |
//! | `original_year` | The year its album was first released, by `ORIGINALDATE`, else `year`: one for every edition of an album |
//! | `track`, `disc` | The track and disc numbers, or none, which writes nothing |
//! | `disc_track` | `"03 "` on a first disc, `"2-03 "` on a later one, `""` without a track |
//! | `composer`, `bpm`, `key` | The composer, the tempo and the musical key, or empty |
//! | `id` | The ID or file name of the song's audio source |
//!
//! A template is checked whole as it is read, before any song is named,
//! so a mistake stops the run up front rather than failing every song, or
//! some songs and not others: it must name a path, parse, name no
//! variable outside this table, apply no filter or test there is not,
//! give `pad` and `truncate` a whole number written in it, at most
//! [`MOST_WIDTH`], include no other template, and render for a song
//! with every field set and for one with none. Each refusal is
//! [`Refused`](crate::errors::Refused), which a query reading the songs
//! unnamed passes over. The filters and tests a template applies are
//! found in its compiled instructions, which MiniJinja shows only through
//! its `unstable_machinery`, so a MiniJinja that changes them fails to
//! build here rather than letting a mistake through. A value missing as
//! a song is named, as `title[100]` of a short title, writes nothing, as
//! a missing track does.
//!
//! Besides MiniJinja's own filters (`default`, `lower`, `upper`,
//! `replace`, `first`, `join`, …) there are `pad(n)`, a number with
//! leading zeros; `truncate(n)`, the first `n` characters; `asciify`,
//! the text transliterated to ASCII; and `the_suffix`, `The Orchards` as
//! `Orchards, The`. Every value arrives already safe for a path, so a `/` in
//! a title never makes a folder; only the template's own `/` does.
//!
//! `asciify` keeps that promise too. A value arrives with its `/` and `\`
//! written `⧸` and `⧹`, which `deunicode` writes back as `/` and `\`; it
//! writes others as a separator, `½` as `1/2` and `÷` as `/`. So the filter
//! transliterates one character at a time and writes a separator so made
//! as `_`, as `[library] restrict = "ascii"` does a tag's; a `/` or `\`
//! the text held before it stays.

use anyhow::{Context, Result};
use minijinja::machinery::Instruction;
use minijinja::value::Serde;
use minijinja::{Environment, ErrorKind, UndefinedBehavior, Value};
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
    pub original_year: String,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub disc_track: String,
    pub composer: String,
    pub bpm: String,
    pub key: String,
    pub id: String,
}

/// The names [`Fields`] gives a template.
pub const VARIABLES: [&str; 16] = [
    "title",
    "artist",
    "artists",
    "album",
    "album_artist",
    "genre",
    "date",
    "year",
    "original_year",
    "track",
    "disc",
    "disc_track",
    "composer",
    "bpm",
    "key",
    "id",
];

/// `<album artist>/<album>/<NN title>`, `<D-NN title>` from a second
/// disc on: a single on no album is named for its title.
pub const DEFAULT_TEMPLATE: &str = "{{ album_artist }}/{{ album }}/{{ disc_track }}{{ title }}";

/// The widest `pad(n)` and the longest `truncate(n)` a template may ask:
/// no filesystem's name holds more bytes, so more could only be cut.
pub const MOST_WIDTH: u64 = 255;

/// A compiled template.
#[derive(Debug)]
pub struct Template {
    env: Environment<'static>,
}

const NAME: &str = "path";

impl Template {
    pub fn new(source: &str) -> Result<Self> {
        let refused = |why: String| -> anyhow::Error { crate::errors::Refused(why).into() };
        if source.split('/').all(|part| part.trim().is_empty()) {
            return Err(refused(
                "[library] template names no path for a song".into(),
            ));
        }
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Chainable);
        // A missing track or disc writes nothing, not `none`.
        env.set_formatter(|out, state, value| {
            if value.is_none() {
                Ok(())
            } else {
                minijinja::escape_formatter(out, state, value)
            }
        });
        env.add_filter("pad", |n: Value, width: u64| {
            let width = usize::try_from(width.min(MOST_WIDTH)).unwrap_or_default();
            match n.as_i64() {
                Some(n) => format!("{n:0width$}"),
                None if n.is_none() || n.is_undefined() => String::new(),
                None => n.to_string(),
            }
        });
        env.add_filter("truncate", |s: String, n: u64| {
            s.chars()
                .take(usize::try_from(n).unwrap_or(usize::MAX))
                .collect::<String>()
        });
        env.add_filter("asciify", |s: String| asciify(&s));
        env.add_filter("the_suffix", |s: String| match s.strip_prefix("The ") {
            Some(rest) if !rest.is_empty() => format!("{rest}, The"),
            _ => s,
        });
        env.add_template_owned(NAME, source.to_string())
            .map_err(|e| refused(format!("the library template does not read: {}", said(&e))))?;
        let mut unknown: Vec<String> = env
            .get_template(NAME)?
            .undeclared_variables(false)
            .into_iter()
            .filter(|v| !VARIABLES.contains(&v.as_str()) && env.globals().all(|(g, _)| g != v))
            .collect();
        if !unknown.is_empty() {
            unknown.sort();
            return Err(refused(format!(
                "the library template names {}, which no song has; it may name {}",
                unknown.join(", "),
                VARIABLES.join(", ")
            )));
        }
        check(&env).map_err(|why| refused(format!("the library template {why}")))?;
        for fields in [Fields::default(), Fields::probe()] {
            env.get_template(NAME)?.render(Serde(fields)).map_err(|e| {
                refused(format!(
                    "the library template does not render: {}",
                    said(&e)
                ))
            })?;
        }
        Ok(Self { env })
    }

    pub fn render(&self, fields: &Fields) -> Result<String> {
        self.env
            .get_template(NAME)?
            .render(Serde(fields))
            .context("rendering the library template")
    }
}

/// `e` on one line: what went wrong, and where.
fn said(e: &minijinja::Error) -> String {
    let what = match e.detail() {
        Some(detail) => format!("{}: {detail}", e.kind()),
        None => e.kind().to_string(),
    };
    match e.line() {
        Some(n) => format!("{what}, on line {n}"),
        None => what,
    }
}

/// Why the template `env` holds cannot name every song, if it cannot: a
/// filter or test there is not, another template included, or `pad` or
/// `truncate` given what is no whole number up to [`MOST_WIDTH`].
fn check(env: &Environment<'_>) -> Result<(), String> {
    let template = env.get_template(NAME).map_err(|e| format!("{e:#}"))?;
    let compiled = minijinja::machinery::get_compiled_template(&template);
    let mut state = env.empty_state();
    for code in std::iter::once(&compiled.instructions).chain(compiled.blocks.values()) {
        let mut before: Option<&Instruction<'_>> = None;
        for at in 0.. {
            let Some(instruction) = code.get(at) else {
                break;
            };
            match instruction {
                Instruction::ApplyFilter(name, args, _) => {
                    let found = state.apply_filter(name, &[Value::UNDEFINED]);
                    if found.is_err_and(|e| e.kind() == ErrorKind::UnknownFilter) {
                        return Err(format!("applies `{name}`, which is no filter"));
                    }
                    let width = match (args, before) {
                        (Some(2), Some(Instruction::LoadConst(n))) => n.as_i64(),
                        _ => None,
                    };
                    let most = i64::try_from(MOST_WIDTH).unwrap_or(i64::MAX);
                    if ["pad", "truncate"].contains(name)
                        && !width.is_some_and(|n| (0..=most).contains(&n))
                    {
                        return Err(format!(
                            "gives `{name}` what is no whole number from 0 to {MOST_WIDTH}, \
                             as `{name}(3)`"
                        ));
                    }
                }
                Instruction::PerformTest(name, _, _) => {
                    let found = state.perform_test(name, &[Value::UNDEFINED]);
                    if found.is_err_and(|e| e.kind() == ErrorKind::UnknownTest) {
                        return Err(format!("tests `is {name}`, which is no test"));
                    }
                }
                Instruction::Include(_) | Instruction::LoadBlocks => {
                    return Err("names another template, and there is none".into());
                }
                _ => {}
            }
            before = Some(instruction);
        }
    }
    Ok(())
}

impl Fields {
    /// A song with every field set, as a template is tried on.
    fn probe() -> Self {
        let text = |s: &str| s.to_string();
        Self {
            title: text("Title"),
            artist: text("Artist"),
            artists: vec![text("Artist"), text("Other")],
            album: text("Album"),
            album_artist: text("Album Artist"),
            genre: text("Genre"),
            date: text("2000-01-02"),
            year: text("2000"),
            original_year: text("1999"),
            track: Some(1),
            disc: Some(2),
            disc_track: text("2-01 "),
            composer: text("Composer"),
            bpm: text("120"),
            key: text("C"),
            id: text("vid00000001"),
        }
    }
}

/// `s` transliterated to ASCII, each separator a transliteration makes
/// written `_`.
fn asciify(s: &str) -> String {
    use deunicode::AsciiChars;
    let mut out = String::with_capacity(s.len());
    for (c, ascii) in s.chars().zip(s.ascii_chars()) {
        if c.is_ascii() {
            out.push(c);
        } else {
            out.extend(
                ascii
                    .unwrap_or("[?]")
                    .chars()
                    .map(|a| if matches!(a, '/' | '\\') { '_' } else { a }),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_number_writes_nothing() {
        let t = Template::new("{{ track }}-{{ disc | pad(2) }}-{{ title }}").unwrap();
        let none = Fields {
            title: "Tide".into(),
            ..Fields::default()
        };
        assert_eq!(t.render(&none).unwrap(), "--Tide");
    }

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
    fn an_empty_artist_falls_back_to_the_album_artist_with_or() {
        let t = Template::new("{{ artist or album_artist }}").unwrap();
        let none = Fields {
            artist: String::new(),
            ..fields()
        };
        assert_eq!(t.render(&none).unwrap(), "The Paper Comets");
        assert_eq!(t.render(&fields()).unwrap(), "Paper Comets");
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
    fn asciify_writes_a_separator_it_makes_as_an_underscore() {
        let t = Template::new("{{ title | asciify }}/{{ album | asciify }}").unwrap();
        let f = Fields {
            title: "Night⧸Day ⧹ Ça".into(),
            album: "a/b ½".into(),
            ..fields()
        };
        assert_eq!(t.render(&f).unwrap(), "Night_Day _ Ca/a/b 1_2");
    }

    #[test]
    fn an_unknown_variable_is_refused_as_the_template_is_read() {
        let e = Template::new("{{ albm }}/{{ title }}").unwrap_err();
        assert!(
            e.downcast_ref::<crate::errors::Refused>().is_some(),
            "{e:#}"
        );
        assert!(format!("{e:#}").contains("names albm,"), "{e:#}");
    }

    #[test]
    fn every_mistake_is_refused_as_the_template_is_read() {
        for (source, says) in [
            ("{{ title", "does not read"),
            ("{% if %}", "does not read"),
            ("", "names no path"),
            (" / ", "names no path"),
            ("{{ title | nofilter }}", "`nofilter`, which is no filter"),
            (
                "{% if title is nothing %}x{% endif %}{{ title }}",
                "`is nothing`",
            ),
            ("{{ track | pad(65536) }}", "`pad`"),
            ("{{ track | pad(-1) }}", "`pad`"),
            ("{{ track | pad(disc) }}", "`pad`"),
            ("{{ track | pad }}", "`pad`"),
            ("{{ title | truncate(-1) }}", "`truncate`"),
            ("{{ title | truncate(1.5) }}", "`truncate`"),
            (
                "{% if year %}{{ title | truncate('x') }}{% endif %}",
                "`truncate`",
            ),
            ("{% include 'other' %}", "another template"),
            ("{{ title + 1 }}", "does not render"),
        ] {
            let e = Template::new(source).unwrap_err();
            assert!(
                e.downcast_ref::<crate::errors::Refused>().is_some(),
                "{source}: {e:#}"
            );
            assert!(format!("{e:#}").contains(says), "{source}: {e:#}");
        }
    }

    #[test]
    fn the_widest_pad_renders_and_a_missing_item_writes_nothing() {
        let t = Template::new(&format!("{{{{ track | pad({MOST_WIDTH}) }}}}")).unwrap();
        assert_eq!(t.render(&fields()).unwrap().len(), 255);
        let t =
            Template::new("{{ title[100] }}x{{ title.nothing }}{{ artists[5] | upper }}").unwrap();
        assert_eq!(t.render(&fields()).unwrap(), "x");
    }

    #[test]
    fn names_a_template_sets_or_loops_over_are_its_own() {
        let t = Template::new(
            "{% set a = artists | join(', ') %}{{ a }}/\
             {% for x in range(2) %}{{ loop.index }}{{ x }}{% endfor %}{{ title }}",
        )
        .unwrap();
        assert!(t.render(&fields()).is_ok());
    }

    #[test]
    fn the_variables_are_the_fields() {
        let fields = Value::from(Serde(Fields::default()));
        let mut names: Vec<String> = fields.try_iter().unwrap().map(|k| k.to_string()).collect();
        names.sort();
        let mut variables = VARIABLES.map(str::to_string).to_vec();
        variables.sort();
        assert_eq!(names, variables);
    }
}
