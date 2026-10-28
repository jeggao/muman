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
//! name each of theirs. Every other term is required, and case and
//! accents are ignored throughout, as is how a name was composed: `cafe`
//! matches "Café", written in either Unicode form.
//!
//! `lyrics:` is yes, none, timed or untimed, `cover:` yes, none, sharp
//! or small, `from:<provider>` holds when a source of the song is from
//! that provider, a torrent's file from `manual`, and `has:<tag>` when a
//! source or the song list gives the tag: an album artist muman takes
//! from the artist, or an album named for a single's title, is muman's
//! own and no given tag. Each compares its value whole, so `lyrics:timed`
//! never matches "untimed" nor `from:youtube` "youtube-music".
//!
//! A term is a key when what comes before its colon is no field's name
//! and a site or scheme some key in the song list uses, or one
//! [`crate::sites::Sites`] names, so `youtube.com:<id>` names a key
//! where `composer:quill` names a field; `youtube:<id>`, as keys were
//! once named, names the same. A song that does not resolve
//! yet, its sources not measured, offers its own `tags` and the tags of
//! its first measured source, so a query still finds it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use std::path::Path;

use anyhow::{Context, Result, bail};
use regex::{Regex, RegexBuilder};
use unicode_normalization::UnicodeNormalization;

use crate::facts::Facts;
use crate::manifest::{Manifest, Removed, Song};
use crate::provider::Provider;
use crate::reconcile;
use crate::resolve::Resolved;
use crate::source::SourceKey;
use crate::state::State;
use crate::tags::{self, Field};

/// Fields that are no tag: what the song is made of and written as.
const DERIVED: [&str; 7] = ["key", "path", "format", "cover", "lyrics", "from", "has"];

/// What a query reads of one song.
// Each is a fact of its own a query reads.
#[expect(clippy::struct_excessive_bools)]
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
    /// Whether its cover is measured to have less detail than
    /// [`crate::quality::SOFT_COVER`].
    pub soft_cover: bool,
    /// Whether its lyrics carry times.
    pub timed: bool,
    /// The providers its sources are from.
    pub from: Vec<Provider>,
    /// The Vorbis names of the tags a source or the song list gives it.
    pub given: Vec<String>,
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
            "from" => self.from.iter().map(ToString::to_string).collect(),
            "has" => self.given.iter().map(|k| k.to_ascii_lowercase()).collect(),
            // As the path template's `year`: a date's first four digits.
            "year" => self
                .values("date")
                .into_iter()
                .map(|d| d.chars().take(4).collect())
                .collect(),
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
) -> Result<Vec<(View, Option<Resolved>)>> {
    let (mut planned, _) = reconcile::plan(manifest, state, library, None, &mut std::io::sink())?;
    crate::limit::as_written(manifest, state, &mut planned);
    let mut plans: Vec<Option<Resolved>> = vec![None; manifest.songs.len()];
    for (n, r) in planned {
        plans[n] = Some(r);
    }
    Ok(manifest
        .songs
        .iter()
        .zip(plans)
        .map(|(song, r)| (view_of(song, r.as_ref(), &state.facts), r))
        .collect())
}

/// One song as a query reads it, from its plan where it resolves. A song
/// that does not resolve yet, its sources not measured, offers its own
/// tags and those of its first measured source.
#[must_use]
pub fn view_of(
    song: &Song,
    resolved: Option<&Resolved>,
    facts: &BTreeMap<SourceKey, Facts>,
) -> View {
    let from: BTreeSet<Provider> = song
        .sources
        .iter()
        .filter_map(|k| Provider::of(k, facts.get(k)))
        .collect();
    let from = from.into_iter().collect();
    let Some(r) = resolved else {
        let mut tags: Vec<(String, Vec<String>)> = song
            .tags
            .iter()
            .map(|(k, v)| (tags::vorbis_key(k), v.clone()))
            .collect();
        let offers = song.sources.iter().find_map(|k| facts.get(k));
        for field in Field::ALL {
            let offered = offers.and_then(|f| f.tags.get(&field));
            if let Some(offer) = offered
                && !tags.iter().any(|(k, _)| k == field.vorbis())
            {
                tags.push((field.vorbis().to_string(), offer.values.clone()));
            }
        }
        let given = tags.iter().map(|(k, _)| k.clone()).collect();
        return View {
            keys: song.sources.clone(),
            tags,
            from,
            given,
            ..View::default()
        };
    };
    let timed = r.plan.lyrics.as_ref().is_some_and(|l| {
        facts
            .get(&l.key)
            .and_then(|f| f.lyrics.iter().find(|x| x.at == l.at))
            .is_some_and(|x| x.timing.is_some())
    });
    let soft_cover = r.plan.cover.as_ref().is_some_and(|c| {
        facts
            .get(&c.key)
            .and_then(|f| f.covers.iter().find(|x| x.at == c.at))
            .and_then(|x| x.quality.as_ref())
            .is_some_and(|q| q.effective < crate::quality::SOFT_COVER)
    });
    let given = r
        .plan
        .tags
        .iter()
        .map(|(k, _)| k)
        .filter(|k| {
            !r.why
                .tags
                .iter()
                .any(|t| &t.key == *k && crate::resolve::derived(&t.from))
        })
        .cloned()
        .collect();
    View {
        keys: song.sources.clone(),
        tags: r.plan.tags.clone(),
        path: Some(reconcile::path_of(r)),
        format: Some(r.plan.format.extension().to_string()),
        cover: r.plan.cover.is_some(),
        lyrics: r.plan.lyrics.is_some(),
        soft_cover,
        timed,
        from,
        given,
    }
}

/// One source alone as a query reads it: what its own facts hold, as a
/// lookup's record is judged by.
#[must_use]
pub fn source_view(key: &SourceKey, facts: Option<&Facts>) -> View {
    let tags: Vec<(String, Vec<String>)> = facts
        .map(|f| {
            Field::ALL
                .iter()
                .filter_map(|field| {
                    let offer = f.tags.get(field)?;
                    Some((field.vorbis().to_string(), offer.values.clone()))
                })
                .collect()
        })
        .unwrap_or_default();
    let covers = facts.map_or(&[][..], |f| f.covers.as_slice());
    let lyrics = facts.map_or(&[][..], |f| f.lyrics.as_slice());
    View {
        keys: vec![key.clone()],
        given: tags.iter().map(|(k, _)| k.clone()).collect(),
        tags,
        cover: !covers.is_empty(),
        soft_cover: !covers.is_empty()
            && covers.iter().all(|c| {
                c.quality
                    .as_ref()
                    .is_some_and(|q| q.effective < crate::quality::SOFT_COVER)
            }),
        lyrics: !lyrics.is_empty(),
        timed: lyrics.iter().any(|l| l.timing.is_some()),
        from: Provider::of(key, facts).into_iter().collect(),
        ..View::default()
    }
}

/// A removed song as a query reads it: its keys, and its note as its
/// title.
#[must_use]
/// A removed song as a query reads it: titled by its note, with the
/// tags it was given and those its sources offer, so `album:` finds it
/// as it found the song listed.
pub fn removed_view(removed: &Removed, state: &State) -> View {
    let mut tags: Vec<(String, Vec<String>)> = Vec::new();
    let given = removed
        .tags
        .iter()
        .map(|(k, v)| (tags::vorbis_key(k), v.clone()));
    let offers = removed.sources.iter().find_map(|k| state.facts.get(k));
    let offered = Field::ALL.iter().filter_map(|field| {
        let offer = offers.and_then(|f| f.tags.get(field))?;
        Some((field.vorbis().to_string(), offer.values.clone()))
    });
    for (key, values) in given.chain(offered) {
        if !tags.iter().any(|(k, _)| *k == key) {
            tags.push((key, values));
        }
    }
    // The name noted names it when nothing measured does; beside a title,
    // it would name the artist twice.
    if !tags.iter().any(|(k, _)| k == "TITLE") {
        tags.insert(0, ("TITLE".to_string(), vec![removed.note.clone()]));
    }
    View {
        keys: removed.sources.clone(),
        tags,
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
struct Term {
    negated: bool,
    test: Test,
}

/// A parsed query; with no terms it matches every song.
#[derive(Debug, Clone, Default)]
pub struct Query(Vec<Term>);

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
                        let re = RegexBuilder::new(pattern)
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
                 artist or album, or one of {}. The path template's own variables, as \
                 disc_track, are not fields",
                DERIVED.join(", ")
            ))
            .into());
        }
        rest = &after[close + 1..];
    }
    Ok(())
}

/// Whether `word`, lower-cased, is the key's ID or its manual or
/// torrent's file's name, with or without its extension.
fn names_key(key: &SourceKey, word: &str) -> bool {
    match key {
        SourceKey::Remote { id, .. } => id.to_lowercase() == word,
        SourceKey::Manual(_) | SourceKey::Torrent(_) => key.file().is_some_and(|path| {
            [path.file_name(), path.file_stem()]
                .into_iter()
                .flatten()
                .any(|n| fold(&n.to_string_lossy()) == word)
        }),
    }
}

/// `text` as a query compares it: lower case, in one Unicode form, its
/// accents taken off.
fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
        .nfc()
        .collect()
}

fn is_field(name: &str) -> bool {
    DERIVED.contains(&name.to_ascii_lowercase().as_str()) || Field::named(name).is_some()
}

impl Test {
    fn holds(&self, view: &View) -> bool {
        let any =
            |field: &str, f: &dyn Fn(&str) -> bool| view.values(field).iter().any(|v| f(&fold(v)));
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
            Self::Matches(f, re) => view
                .values(f)
                .iter()
                .any(|v| re.is_match(&v.nfc().collect::<String>())),
            Self::Is(c) => c.holds(view),
        }
    }
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

    /// Whether `view` meets it.
    #[must_use]
    pub fn holds(&self, view: &View) -> bool {
        let have = |h: Have, some: bool, better: bool| match h {
            Have::Some => some,
            Have::None => !some,
            Have::Better => some && better,
            Have::Worse => some && !better,
        };
        match self {
            Self::Lyrics(h) => have(*h, view.lyrics, view.timed),
            Self::Cover(h) => have(*h, view.cover, !view.soft_cover),
            Self::From(p) => view.from.contains(p),
            Self::Has(tag) => view.given.iter().any(|k| k == tag),
        }
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
            Self::Has(tag) => {
                let name = Field::named(tag)
                    .and_then(|x| x.aliases().first().copied())
                    .map_or_else(|| tag.to_ascii_lowercase(), str::to_string);
                write!(f, "has:{name}")
            }
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

/// The key schemes a query treats as keys: `manual`, every site, alias
/// and extractor `[sites]` names, built in or the song list's, as keys
/// were once named, and every site the song list names.
#[must_use]
pub fn extractors(manifest: &Manifest) -> BTreeSet<String> {
    let mut found: BTreeSet<String> = ["manual".to_string()].into();
    found.extend(crate::sites::built_in().schemes().map(str::to_string));
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
            ..View::default()
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
    fn accents_and_unicode_forms_are_ignored() {
        let composed = view("Caf\u{e9} Noir", "Lumo Fenn", "manual:a.flac");
        let decomposed = view("Cafe\u{301} Noir", "Lumo Fenn", "manual:b.flac");
        for term in [
            "caf\u{e9}",
            "cafe\u{301}",
            "cafe",
            "title:=\"CAF\u{c9} NOIR\"",
        ] {
            let q = query(&[term]);
            assert!(q.matches(&composed) && q.matches(&decomposed), "{term}");
        }
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
    fn closed_fields_compare_their_values_whole() {
        let mut v = view("Tide", "Ada Quill", "youtube.com:aaaaaaaaaaa");
        v.lyrics = true;
        v.timed = false;
        v.soft_cover = true;
        v.from = vec![Provider::YouTubeMusic];
        v.given = vec!["TITLE".into(), "TRACKNUMBER".into()];
        assert!(query(&["lyrics:untimed", "^lyrics:timed", "lyrics:yes"]).matches(&v));
        assert!(query(&["cover:small", "^cover:sharp", "cover:=yes"]).matches(&v));
        assert!(query(&["from:youtube-music", "^from:youtube"]).matches(&v));
        assert!(query(&["has:track", "has:TITLE", "^has:album"]).matches(&v));
        v.cover = false;
        assert!(query(&["cover:none", "^cover:small", "^cover:sharp"]).matches(&v));
        let none = BTreeSet::new();
        for bad in [
            "from:acoustid",
            "from:nowhere",
            "has:",
            "cover:big",
            "from::you",
        ] {
            assert!(Query::parse(&[bad.to_string()], &none).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_query_written_as_text_keeps_quoted_words_together() {
        let schemes = ["manual".to_string()].into();
        let q = parse_text(r#"album:="The Glass Orchards" ^genre:ambient"#, &schemes).unwrap();
        let mut v = view("Tide", "Ada Quill", "manual:a.flac");
        v.tags
            .push(("ALBUM".into(), vec!["The Glass Orchards".into()]));
        assert!(q.matches(&v));
        assert!(
            parse_text("  ", &schemes).is_err(),
            "an empty query matches every song"
        );
        assert_eq!(
            Closed::parse("has", "album_artist").unwrap().to_string(),
            "has:album_artist"
        );
    }

    #[test]
    fn a_field_no_tag_names_and_no_song_has_is_refused() {
        let v = [view(
            "One Lantern Hour",
            "Lumo Fenn",
            "youtube.com:aaaaaaaaaaa",
        )];
        for term in ["artst:fenn", "^artst:fenn", "gnere::x", "orchestra:quill"] {
            let e = query(&[term]).check_fields(&v).unwrap_err();
            assert!(
                e.downcast_ref::<crate::change::Refused>().is_some(),
                "{term}: {e:#}"
            );
        }
        for term in [
            "album:x",
            "^date:2001",
            "genre:house",
            "format:opus",
            "composer:quill",
        ] {
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
        assert!(
            query(&["artist:=\"Lumo Fenn\""]).matches(&v),
            "quotes that reach muman"
        );
        assert!(query(&["title:\"lantern hour\""]).matches(&v));
        let dated = View {
            tags: vec![("DATE".into(), vec!["2015-03-04".into()])],
            ..v.clone()
        };
        assert_eq!(dated.values("year"), ["2015"]);
        assert!(query(&["year:=2015"]).matches(&dated));
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
