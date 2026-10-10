//! Which songs a command acts on: a query over each song as it resolves
//! from what earlier runs measured, read without running or locking
//! anything.
//!
//! A bare word matches when the title, artist, album or album artist
//! holds it, or it is a key's ID or a manual file's name whole: never
//! part of a key, which would take `man` for every manual song.
//! `field:text` a field containing the text, `field:=text` one equal to
//! it, `field::regex` one matching it; `^` before any term negates it. A
//! field that is no tag muman knows and that no song has is refused, so
//! a misspelled one neither matches nothing nor, negated, every song.
//! A source key, as `youtube.com:<id>`, names its song exactly, and several
//! name each of theirs. Every other term is required, and case is
//! ignored throughout.
//!
//! A term is a key when what comes before its colon is no field's name
//! and a site or scheme some key in the song list uses, or one
//! [`crate::sites::Sites`] names, so `youtube.com:<id>` names a key
//! where `composer:quill` names a field; `youtube:<id>`, as keys were
//! once named, names the same. A song that does not resolve
//! yet, its sources not measured, offers its own `tags` and the tags of
//! its first measured source, so a query still finds it.

use std::collections::BTreeSet;
use std::path::PathBuf;

use std::path::Path;

use anyhow::{Context, Result, bail};
use regex::{Regex, RegexBuilder};

use crate::manifest::{Manifest, Removed};
use crate::reconcile;
use crate::source::SourceKey;
use crate::state::State;
use crate::tags::{self, Field};

/// Fields that are no tag: what the song is made of and written as.
const DERIVED: [&str; 5] = ["key", "path", "format", "cover", "lyrics"];

/// What a query reads of one song.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub keys: Vec<SourceKey>,
    /// Vorbis comments by their upper-case names.
    pub tags: Vec<(String, Vec<String>)>,
    /// Its file in the library, when it resolves.
    pub path: Option<PathBuf>,
    pub format: Option<String>,
    pub cover: bool,
    pub lyrics: bool,
}

impl View {
    /// The key naming the song, by [`crate::manifest::id_of`].
    #[must_use]
    pub fn id(&self) -> Option<&SourceKey> {
        crate::manifest::id_of(&self.keys)
    }

    /// The values of `field`, by any of its spellings.
    #[must_use]
    pub fn values(&self, field: &str) -> Vec<String> {
        let yes_no = |b: bool| vec![if b { "yes" } else { "none" }.to_string()];
        match field.to_ascii_lowercase().as_str() {
            "key" => self.keys.iter().map(ToString::to_string).collect(),
            "path" => self.path.iter().map(|p| crate::relpath::show(p)).collect(),
            "format" => self.format.iter().cloned().collect(),
            "cover" => yes_no(self.cover),
            "lyrics" => yes_no(self.lyrics),
            _ => {
                let name = tags::vorbis_key(field);
                self.tags
                    .iter()
                    .filter(|(k, _)| *k == name)
                    .flat_map(|(_, v)| v.iter().cloned())
                    .collect()
            }
        }
    }

    /// The first value of `field`, or nothing.
    #[must_use]
    pub fn first(&self, field: &str) -> String {
        self.values(field).into_iter().next().unwrap_or_default()
    }

    /// How a song is named in messages.
    #[must_use]
    pub fn name(&self) -> String {
        let title = self.first("title");
        let artist = self.first("artist");
        match (title.is_empty(), artist.is_empty()) {
            (false, false) => format!("{title} — {artist}"),
            (false, true) => title,
            _ => self.first("key"),
        }
    }
}

/// Every song as a query reads it, in song-list order.
pub fn views(manifest: &Manifest, state: &State, library: &Path) -> Result<Vec<View>> {
    Ok(resolved_views(manifest, state, library)?
        .into_iter()
        .map(|(view, _)| view)
        .collect())
}

/// Every song as a query reads it, in song-list order, with its plan
/// where it resolves.
pub fn resolved_views(
    manifest: &Manifest,
    state: &State,
    library: &Path,
) -> Result<Vec<(View, Option<crate::resolve::Resolved>)>> {
    let (mut planned, _) = reconcile::plan(manifest, state, library, None, &mut std::io::sink())?;
    crate::limit::as_written(manifest, state, &mut planned);
    let mut views: Vec<View> = manifest
        .songs
        .iter()
        .map(|song| {
            let mut tags: Vec<(String, Vec<String>)> = song
                .tags
                .iter()
                .map(|(k, v)| (tags::vorbis_key(k), v.clone()))
                .collect();
            let offers = song.sources.iter().find_map(|k| state.facts.get(k));
            for field in Field::ALL {
                let offered = offers.and_then(|f| f.tags.get(&field));
                if let Some(offer) = offered
                    && !tags.iter().any(|(k, _)| k == field.vorbis())
                {
                    tags.push((field.vorbis().to_string(), offer.values.clone()));
                }
            }
            View {
                keys: song.sources.clone(),
                tags,
                ..View::default()
            }
        })
        .collect();
    let mut plans: Vec<Option<crate::resolve::Resolved>> = vec![None; views.len()];
    for (n, r) in planned {
        let view = &mut views[n];
        view.tags.clone_from(&r.plan.tags);
        view.path = Some(reconcile::path_of(&r));
        view.format = Some(r.plan.format.extension().to_string());
        view.cover = r.plan.cover.is_some();
        view.lyrics = r.plan.lyrics.is_some();
        plans[n] = Some(r);
    }
    Ok(views.into_iter().zip(plans).collect())
}

/// A removed song as a query reads it: its keys, and its note as its
/// title.
#[must_use]
pub fn removed_view(removed: &Removed) -> View {
    View {
        keys: removed.sources.clone(),
        tags: vec![("TITLE".to_string(), vec![removed.note.clone()])],
        ..View::default()
    }
}

#[derive(Debug, Clone)]
enum Test {
    Word(String),
    Key(SourceKey),
    Contains(String, String),
    Equals(String, String),
    Matches(String, Regex),
}

#[derive(Debug, Clone)]
struct Term {
    negated: bool,
    test: Test,
}

/// A parsed query; with no terms it matches every song.
#[derive(Debug, Clone, Default)]
pub struct Query(Vec<Term>);

impl Query {
    /// Parse the terms of a query. `extractors` are the key schemes in
    /// use, so `youtube.com:<id>` names a key where `composer:quill` names
    /// a field.
    pub fn parse(terms: &[String], extractors: &BTreeSet<String>) -> Result<Self> {
        Self::parse_terms(terms, extractors)
            .map_err(|e| crate::change::Refused(format!("{e:#}")).into())
    }

    fn parse_terms(terms: &[String], extractors: &BTreeSet<String>) -> Result<Self> {
        let mut parsed = Vec::new();
        for raw in terms {
            let (negated, body) = match raw.strip_prefix('^') {
                Some(rest) => (true, rest),
                None => (false, raw.as_str()),
            };
            if body.is_empty() {
                bail!("`{raw}` is an empty term");
            }
            let test = match body.split_once(':') {
                None => Test::Word(body.to_lowercase()),
                Some(("", _)) => bail!("`{raw}` names no field"),
                Some((scheme, rest))
                    if extractors.contains(&scheme.to_ascii_lowercase()) && !is_field(scheme) =>
                {
                    Test::Key(SourceKey::parse(&format!(
                        "{}:{rest}",
                        scheme.to_ascii_lowercase()
                    ))?)
                }
                Some((field, rest)) => {
                    let field = field.to_string();
                    if let Some(pattern) = rest.strip_prefix(':') {
                        let re = RegexBuilder::new(pattern)
                            .case_insensitive(true)
                            .build()
                            .with_context(|| format!("`{raw}`"))?;
                        Test::Matches(field, re)
                    } else {
                        let (exact, text) = match rest.strip_prefix('=') {
                            Some(text) => (true, text.to_lowercase()),
                            None => (false, rest.to_lowercase()),
                        };
                        let yes_no =
                            ["cover", "lyrics"].contains(&field.to_ascii_lowercase().as_str());
                        if yes_no && text != "yes" && text != "none" {
                            bail!("`{raw}`: {field} is yes or none");
                        }
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

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `view` is one of the songs the keys name, if any do, and
    /// satisfies every other term.
    #[must_use]
    pub fn matches(&self, view: &View) -> bool {
        let named = |t: &&Term| !t.negated && matches!(t.test, Test::Key(_));
        let mut keys = self.0.iter().filter(named).peekable();
        let keyed = keys.peek().is_none() || keys.any(|t| t.test.holds(view));
        keyed
            && self
                .0
                .iter()
                .filter(|t| !named(t))
                .all(|t| t.negated != t.test.holds(view))
    }

    /// Refuse a field no tag muman knows is named and no song in `views`
    /// has: a misspelling, which would match nothing, or negated, every
    /// song.
    pub fn check_fields(&self, views: &[View]) -> Result<()> {
        for term in &self.0 {
            let (Test::Contains(field, _) | Test::Equals(field, _) | Test::Matches(field, _)) =
                &term.test
            else {
                continue;
            };
            let name = tags::vorbis_key(field);
            let had = views.iter().any(|v| v.tags.iter().any(|(k, _)| *k == name));
            if !is_field(field) && !had {
                return Err(crate::change::Refused(format!(
                    "No song has a field `{field}`: a tag such as title, artist, album, \
                     album_artist, genre or date, or one of {}",
                    DERIVED.join(", ")
                ))
                .into());
            }
        }
        Ok(())
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
}

/// Refuse a `list` format naming a field that is no tag muman knows and
/// that no song has, as [`Query::check_fields`] refuses a query's.
pub fn check_format(template: &str, views: &[View]) -> Result<()> {
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let field = &after[..close];
        let name = tags::vorbis_key(field);
        let had = views.iter().any(|v| v.tags.iter().any(|(k, _)| *k == name));
        if !is_field(field) && !had {
            return Err(crate::change::Refused(format!(
                "The format names `{{{field}}}`, which no song has: a tag such as title, \
                 artist or album, or one of {}",
                DERIVED.join(", ")
            ))
            .into());
        }
        rest = &after[close + 1..];
    }
    Ok(())
}

/// Whether `word`, lower-cased, is the key's ID or its manual file's
/// name, with or without its extension.
fn names_key(key: &SourceKey, word: &str) -> bool {
    match key {
        SourceKey::Remote { id, .. } => id.to_lowercase() == word,
        SourceKey::Manual(key) => [key.path().file_name(), key.path().file_stem()]
            .into_iter()
            .flatten()
            .any(|n| n.to_string_lossy().to_lowercase() == word),
    }
}

fn is_field(name: &str) -> bool {
    DERIVED.contains(&name.to_ascii_lowercase().as_str()) || Field::named(name).is_some()
}

impl Test {
    fn holds(&self, view: &View) -> bool {
        let any = |field: &str, f: &dyn Fn(&str) -> bool| {
            view.values(field).iter().any(|v| f(&v.to_lowercase()))
        };
        match self {
            Self::Word(w) => {
                ["title", "artist", "album", "album_artist"]
                    .iter()
                    .any(|f| any(f, &|v| v.contains(w.as_str())))
                    || view.keys.iter().any(|k| names_key(k, w))
            }
            Self::Key(k) => view.keys.contains(k),
            Self::Contains(f, text) => any(f, &|v| v.contains(text.as_str())),
            Self::Equals(f, text) => any(f, &|v| v == text),
            Self::Matches(f, re) => view.values(f).iter().any(|v| re.is_match(v)),
        }
    }
}

/// The key schemes a query treats as keys: `manual`, every site and
/// extractor `[sites]` names and keys were once named by
/// ([`crate::source::LEGACY`]), and every site the song list names.
#[must_use]
pub fn extractors(manifest: &Manifest) -> BTreeSet<String> {
    let mut found: BTreeSet<String> = ["manual", "youtubetab"].map(String::from).into();
    for (extractor, site) in crate::source::LEGACY {
        found.insert(extractor.to_string());
        found.insert(site.to_string());
    }
    found.extend(manifest.settings.sites.schemes().map(str::to_string));
    let keys = manifest
        .songs
        .iter()
        .flat_map(|s| &s.sources)
        .chain(manifest.removed.iter().flat_map(|r| &r.sources));
    for key in keys {
        if let SourceKey::Remote { site, .. } = key {
            found.insert(site.clone());
        }
    }
    found
}

/// `template` with each `{field}` replaced by the song's values, joined
/// by `; `; `\t` and `\n` are a tab and a new line.
#[must_use]
pub fn format(template: &str, view: &View) -> String {
    let template = template.replace("\\t", "\t").replace("\\n", "\n");
    let mut out = String::new();
    let mut rest = template.as_str();
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push('{');
            rest = after;
            continue;
        };
        let field = &after[..close];
        let values = if field == "key" {
            vec![view.first("key")]
        } else {
            view.values(field)
        };
        // One song a line, whatever its values hold, a file's name too.
        let flat: Vec<String> = values
            .iter()
            .map(|v| v.replace(['\t', '\n', '\r'], " "))
            .collect();
        out.push_str(&flat.join("; "));
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(title: &str, artist: &str, key: &str) -> View {
        View {
            keys: vec![SourceKey::parse(key).unwrap()],
            tags: vec![
                ("TITLE".into(), vec![title.into()]),
                ("ARTIST".into(), vec![artist.into()]),
                ("GENRE".into(), vec!["House".into(), "French".into()]),
            ],
            path: Some("Lumo Fenn/Lanternfall/01 One Lantern Hour.opus".into()),
            format: Some("opus".into()),
            cover: true,
            lyrics: false,
        }
    }

    fn query(terms: &[&str]) -> Query {
        let terms: Vec<String> = terms.iter().map(ToString::to_string).collect();
        let schemes = ["manual", "youtube.com", "youtube"].map(String::from);
        Query::parse(&terms, &schemes.into()).unwrap()
    }

    #[test]
    fn bare_words_are_all_required_and_ignore_case() {
        let v = view("One Lantern Hour", "Lumo Fenn", "youtube.com:aaaaaaaaaaa");
        assert!(query(&["lumo", "hour"]).matches(&v));
        assert!(!query(&["lumo", "around"]).matches(&v));
        assert!(query(&[]).matches(&v));
    }

    #[test]
    fn a_bare_word_names_a_key_only_whole() {
        let video = view("One Lantern Hour", "Lumo Fenn", "youtube.com:aaaaaaaaaaa");
        let file = view("Tide", "Ada Quill", "manual:Ada Quill/02 Tide.flac");
        for part in ["man", "tube", "you", "aaa"] {
            assert!(!query(&[part]).matches(&video), "{part}");
            assert!(!query(&[part]).matches(&file), "{part}");
        }
        assert!(query(&["aaaaaaaaaaa"]).matches(&video));
        assert!(query(&["02 tide.flac"]).matches(&file));
        assert!(query(&["ada"]).matches(&file), "names still match in part");
    }

    #[test]
    fn a_key_names_its_song_whatever_the_case_of_its_scheme() {
        let file = view("Tide", "Ada Quill", "manual:Ada Quill/02 Tide.flac");
        assert!(query(&["MANUAL:Ada Quill/02 Tide.flac"]).matches(&file));
        let terms = ["title::(".to_string()];
        let e = Query::parse(&terms, &BTreeSet::new()).unwrap_err();
        assert!(
            e.downcast_ref::<crate::change::Refused>().is_some(),
            "{e:#}"
        );
    }

    #[test]
    fn cover_and_lyrics_are_yes_or_none_and_a_format_names_known_fields() {
        let none = BTreeSet::new();
        assert!(Query::parse(&["lyrics:maybe".to_string()], &none).is_err());
        assert!(Query::parse(&["lyrics:=none".to_string()], &none).is_ok());
        assert!(Query::parse(&["cover:yes".to_string()], &none).is_ok());
        let v = [view("Tide", "Ada Quill", "manual:a.flac")];
        assert!(check_format("{key}\\t{title} {genre}", &v).is_ok());
        let e = check_format("{key} {nosuch}", &v).unwrap_err();
        assert!(
            e.downcast_ref::<crate::change::Refused>().is_some(),
            "{e:#}"
        );
    }

    #[test]
    fn a_field_no_tag_names_and_no_song_has_is_refused() {
        let v = [view(
            "One Lantern Hour",
            "Lumo Fenn",
            "youtube.com:aaaaaaaaaaa",
        )];
        for term in ["artst:fenn", "^artst:fenn", "gnere::x", "composer:quill"] {
            let e = query(&[term]).check_fields(&v).unwrap_err();
            assert!(
                e.downcast_ref::<crate::change::Refused>().is_some(),
                "{term}: {e:#}"
            );
        }
        for term in ["album:x", "^date:2001", "genre:house", "format:opus"] {
            query(&[term]).check_fields(&v).unwrap();
        }
        let mut composed = v[0].clone();
        composed
            .tags
            .push(("COMPOSER".into(), vec!["Ada Quill".into()]));
        query(&["^composer:quill"])
            .check_fields(&[composed])
            .unwrap();
    }

    #[test]
    fn fields_match_by_text_exactly_or_by_pattern() {
        let v = view("One Lantern Hour", "Lumo Fenn", "youtube.com:aaaaaaaaaaa");
        assert!(query(&["artist:fenn"]).matches(&v));
        assert!(!query(&["artist:=fenn"]).matches(&v));
        assert!(query(&["artist:=LUMO FENN"]).matches(&v));
        assert!(query(&["title::^one"]).matches(&v));
        assert!(query(&["genre:=french"]).matches(&v), "any value matches");
        assert!(query(&["lyrics:none", "cover:yes", "format:opus"]).matches(&v));
        assert!(query(&["path:lanternfall"]).matches(&v));
    }

    #[test]
    fn a_caret_negates_and_a_key_names_its_song() {
        let v = view("One Lantern Hour", "Lumo Fenn", "youtube.com:aaaaaaaaaaa");
        assert!(!query(&["^lumo"]).matches(&v));
        assert!(query(&["^lyrics:yes"]).matches(&v));
        let q = query(&["youtube.com:aaaaaaaaaaa"]);
        assert!(q.matches(&v) && q.names_keys());
        assert!(
            query(&["youtube:aaaaaaaaaaa"]).matches(&v),
            "as keys were once named"
        );
        assert!(!query(&["youtube.com:bbbbbbbbbbb"]).matches(&v));
        let both = query(&["youtube.com:bbbbbbbbbbb", "youtube.com:aaaaaaaaaaa"]);
        assert!(
            both.matches(&v) && both.names_keys(),
            "keys name any of theirs"
        );
        assert!(!query(&["youtube.com:aaaaaaaaaaa", "^lumo"]).matches(&v));
        assert!(
            !query(&["composer:quill"]).names_keys(),
            "an unknown scheme is a field"
        );
    }

    #[test]
    fn a_bad_term_is_refused() {
        let parse = |t: &str| Query::parse(&[t.to_string()], &BTreeSet::new());
        assert!(parse("title::(").is_err());
        assert!(parse(":x").is_err());
        assert!(parse("^").is_err());
    }

    #[test]
    fn a_format_names_fields_and_escapes() {
        let v = view("One Lantern Hour", "Lumo Fenn", "youtube.com:aaaaaaaaaaa");
        assert_eq!(
            format("{key}\\t{artist} - {title} [{genre}] {", &v),
            "youtube.com:aaaaaaaaaaa\tLumo Fenn - One Lantern Hour [House; French] {"
        );
        let odd = view("Tide", "Ada Quill", "manual:Ada\nQuill/02 Tide.flac");
        assert_eq!(
            format("{key}\t{title}", &odd),
            "manual:Ada Quill/02 Tide.flac\tTide"
        );
    }
}
