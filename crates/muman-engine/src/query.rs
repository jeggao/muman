//! Which songs a command acts on: a query over each song as it resolves
//! from what earlier runs measured, read without running or locking
//! anything.
//!
//! A bare word matches when the title, artist, album or album artist
//! holds it, or it is a key's ID or a manual file's name whole: never
//! part of a key, which would take `man` for every manual song. Bare
//! text with spaces, as one argument quoted, is its words, each to match
//! as one alone would, so a script passing `"$query"` finds what the
//! words typed apart find; all of it may still name a file whole, as
//! `"02 tide.flac"`. Quotes around a term, or around a field's text, read
//! alike whether a shell took them, as the song list's queries are split,
//! or they reached muman in one argument. `field:text` a field containing
//! the text, spaces and all, `field:=text` one equal to it, `field::regex`
//! one matching it; `^` before any term negates it. A field, or a tag
//! `has:` asks for, that is no tag muman knows and that no song has is
//! refused, so a misspelled one neither matches nothing nor, negated,
//! every song.
//! A source key, as `youtube.com:<id>`, names its song exactly, and several
//! name each of theirs. Every other term is required, and case and
//! accents are ignored throughout, as is how a name was composed: `cafe`
//! matches "Café", written in either Unicode form. A regex is matched
//! against the field folded so, lower-cased and its accents taken off,
//! and its own accents are taken off too, so `title::^cafe` finds
//! "Café Noir"; `(?-i)\p{Lu}`, asking for upper case, finds nothing.
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
//! its first measured source, so a query still finds it. A song whose
//! file a sync leaves as it is, as built or as the user's, is read as
//! that file was made, at its name ([`reconcile::as_held`]), so its
//! `path` and tags are those of the file the library holds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use std::path::Path;

use anyhow::Result;

use crate::facts::Facts;
use crate::manifest::{Manifest, Note, Removed, Song};
use crate::provider::Provider;
use crate::providers::ProviderRules;
use crate::reconcile;
use crate::resolve::Resolved;
use crate::source::SourceKey;
use crate::state::State;
use crate::tags::{self, Field};
pub use crate::terms::{Closed, Have, Query, parse_text};
use crate::terms::{DERIVED, Term, Test, fold, is_field};

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

    /// How the song is named when it is removed: as in messages, its
    /// title and artist kept apart too.
    #[must_use]
    pub fn note(&self) -> Note {
        let field = |name: &str| Some(self.first(name)).filter(|v| !v.is_empty());
        Note {
            text: self.name(),
            title: field("title"),
            artist: field("artist"),
        }
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
/// where it resolves: at the format its file was written in, and a song a
/// sync leaves as it is as its file holds it, by the decision the sync
/// makes ([`reconcile::as_held`]).
pub fn resolved_views(
    manifest: &Manifest,
    state: &State,
    library: &Path,
) -> Result<Vec<(View, Option<Resolved>)>> {
    // A sync refuses a plan the settings cannot make, as a template naming
    // no field; a query still reads each song, unresolved.
    let mut planned = match reconcile::plan(manifest, state, library, None, &mut std::io::sink()) {
        Ok((planned, _)) => planned,
        Err(e) if e.downcast_ref::<crate::errors::Refused>().is_some() => Vec::new(),
        Err(e) => return Err(e),
    };
    crate::limit::as_written(manifest, state, &mut planned);
    let decided = reconcile::foresee(manifest, state, &planned, library, (false, false)).decided;
    let shown = reconcile::as_held(&planned, &decided, &state.outputs);
    Ok(views_of(manifest, &state.facts, &shown))
}

/// Every song of `manifest` as a query reads it, in song-list order, by
/// its plan in `planned` where it has one.
#[must_use]
pub fn views_of(
    manifest: &Manifest,
    facts: &BTreeMap<SourceKey, Facts>,
    planned: &[(usize, Resolved)],
) -> Vec<(View, Option<Resolved>)> {
    let mut plans: Vec<Option<Resolved>> = vec![None; manifest.songs.len()];
    for (n, r) in planned {
        plans[*n] = Some(r.clone());
    }
    manifest
        .songs
        .iter()
        .zip(plans)
        .map(|(song, r)| (view_of(song, r.as_ref(), facts), r))
        .collect()
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
    // A tag one of the song's sources offers is given, whatever muman
    // writes for it: a measured gain stands in for the one a file holds.
    let offered = |k: &str| {
        song.sources.iter().filter_map(|s| facts.get(s)).any(|f| {
            Field::ALL
                .iter()
                .any(|field| field.vorbis() == k && f.tags.contains_key(field))
        })
    };
    let given = r
        .plan
        .tags
        .iter()
        .map(|(k, _)| k)
        .filter(|k| {
            offered(k)
                || r.why
                    .tags
                    .iter()
                    .any(|t| &t.key == *k && !crate::resolve::derived(&t.from))
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

/// A removed song as a query reads it: with the tags it was given, the
/// title and artist it was removed as, and the tags its sources offer,
/// so `album:` finds it as it found the song listed and `artist:` once
/// its sources are purged; titled by its note when nothing else names
/// it.
#[must_use]
pub fn removed_view(removed: &Removed, state: &State) -> View {
    let mut tags: Vec<(String, Vec<String>)> = Vec::new();
    let named = [("TITLE", &removed.title), ("ARTIST", &removed.artist)]
        .into_iter()
        .filter_map(|(k, v)| Some((k.to_string(), vec![v.clone()?])));
    let given = removed
        .tags
        .iter()
        .map(|(k, v)| (tags::vorbis_key(k), v.clone()))
        .chain(named);
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

/// A query held against the songs it is asked of, as they are shown.
/// A trait, as [`Query`] is the song list's and a [`View`] the engine's.
pub trait QueryMatch {
    /// Whether `view` is one of the songs the keys name, if any do, and
    /// satisfies every other term.
    #[must_use]
    fn matches(&self, view: &View) -> bool;

    /// Refuse a field no tag muman knows is named and no song in `views`
    /// has: a misspelling, which would match nothing, or negated, every
    /// song.
    fn check_fields(&self, views: &[View]) -> Result<()>;
}

impl QueryMatch for Query {
    fn matches(&self, view: &View) -> bool {
        let named = |t: &&Term| !t.negated && matches!(t.test, Test::Key(_));
        let mut keys = self.0.iter().filter(named).peekable();
        let keyed = keys.peek().is_none() || keys.any(|t| holds(&t.test, view));
        keyed
            && self
                .0
                .iter()
                .filter(|t| !named(t))
                .all(|t| t.negated != holds(&t.test, view))
    }

    fn check_fields(&self, views: &[View]) -> Result<()> {
        let had = |name: &str| views.iter().any(|v| v.tags.iter().any(|(k, _)| k == name));
        match self.unknown_field(had) {
            Some(field) => Err(crate::errors::Refused(crate::terms::no_field(&field)).into()),
            None => Ok(()),
        }
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
            return Err(crate::errors::Refused(format!(
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

/// Whether `view` meets `term`.
fn holds(term: &Test, view: &View) -> bool {
    let any =
        |field: &str, f: &dyn Fn(&str) -> bool| view.values(field).iter().any(|v| f(&fold(v)));
    match term {
        Test::Word(text) => {
            let word = |w: &str| {
                ["title", "artist", "album", "album_artist"]
                    .iter()
                    .any(|f| any(f, &|v| v.contains(w)))
                    || view.keys.iter().any(|k| names_key(k, w))
            };
            view.keys.iter().any(|k| names_key(k, text)) || text.split_whitespace().all(word)
        }
        Test::Key(k) => view.keys.contains(k),
        Test::Contains(f, text) => any(f, &|v| v.contains(text.as_str())),
        Test::Equals(f, text) => any(f, &|v| v == text),
        Test::Matches(f, re) => view.values(f).iter().any(|v| re.is_match(&fold(v))),
        Test::Is(c) => c.holds(view),
    }
}

/// A closed term held against a song as it is shown. A trait, as
/// [`Closed`] is the song list's and a [`View`] the engine's.
pub trait Holds {
    /// Whether `view` meets it.
    #[must_use]
    fn holds(&self, view: &View) -> bool;
}

impl Holds for Closed {
    fn holds(&self, view: &View) -> bool {
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

/// The key schemes a query treats as keys: every scheme that names no
/// site ([`crate::source::schemes`]), every site, alias and extractor
/// `[sites]` names, built in or the song list's, as keys were once
/// named, and every site the song list names.
#[must_use]
pub fn extractors(manifest: &Manifest) -> BTreeSet<String> {
    let mut found: BTreeSet<String> = crate::source::schemes().map(str::to_string).collect();
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
        let flat: Vec<String> = values.iter().map(|v| tags::escaped(v)).collect();
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
    fn a_regex_is_matched_against_the_field_folded() {
        let composed = view("Caf\u{e9} Noir", "Lumo Fenn", "manual:a.flac");
        let plain = view("Cafe Noir", "Lumo Fenn", "manual:b.flac");
        for term in [
            "title::^cafe",
            "title::^caf\u{e9}",
            "title::^CAFE\\s",
            "title::^cafe\u{301}",
        ] {
            let q = query(&[term]);
            assert!(q.matches(&composed) && q.matches(&plain), "{term}");
        }
        assert!(
            !query(&["title::(?-i)\\p{Lu}afe"]).matches(&plain),
            "lower-cased"
        );
        let quoted = view("Don\u{2019}t Wait", "Lumo Fenn", "manual:c.flac");
        assert!(query(&["title::don't"]).matches(&quoted));
    }

    #[test]
    fn bare_text_with_spaces_is_each_of_its_words() {
        let v = view(
            "Paper Comets",
            "Marlo Venn",
            "manual:Marlo Venn/02 Tide.flac",
        );
        for term in ["comets paper", "venn paper", "  paper   comets "] {
            assert!(query(&[term]).matches(&v), "{term}");
        }
        assert!(!query(&["paper lantern"]).matches(&v));
        assert!(query(&["^paper lantern"]).matches(&v));
        assert!(query(&["02 tide.flac"]).matches(&v), "a file's name whole");
        assert!(
            !query(&["title:\"comets paper\""]).matches(&v),
            "a field's text is a phrase"
        );
        assert!(Query::parse(&["  ".to_string()], &BTreeSet::new()).is_err());
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
            e.downcast_ref::<crate::errors::Refused>().is_some(),
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
            e.downcast_ref::<crate::errors::Refused>().is_some(),
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
    fn every_key_a_song_can_list_names_it_in_a_query() {
        let home = tempfile::tempdir().unwrap();
        let schemes = extractors(&Manifest::load(home.path()).unwrap());
        for key in [
            "manual:Tide/a.flac",
            "torrent:0123456789abcdef0123456789abcdef01234567/Glass Hours/01.flac",
            "lrclib:123",
            "musicbrainz:00000000-0000-0000-0000-000000000001",
            "coverart:00000000-0000-0000-0000-000000000002",
            "youtube.com:vid00000001",
        ] {
            let q = Query::parse(&[key.to_string()], &schemes).unwrap_or_else(|e| panic!("{e:#}"));
            assert!(q.names_keys(), "{key}");
            let v = view("Tide", "Ada Quill", key);
            assert!(q.matches(&v), "{key}");
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
                e.downcast_ref::<crate::errors::Refused>().is_some(),
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
            "manual:Ada\\nQuill/02 Tide.flac\tTide"
        );
    }
}
