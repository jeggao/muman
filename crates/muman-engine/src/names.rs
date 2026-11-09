//! One spelling for each artist across the library.
//!
//! An artist's songs come from sources that each spell it their own way:
//! a record as its release credits it, a file as its tagger wrote it, an
//! upload by its channel's name, in one script or another. Measured on a
//! library of about two thousand songs, one artist was spelled four ways
//! in two scripts across its songs. Once every song is resolved, this
//! pass groups the names crediting one artist alone, across every song,
//! and writes one spelling for each group:
//!
//! - An artist is known by its MusicBrainz ID: a credit of one artist with
//!   one ID names it, and a record's credited artists give an ID its own
//!   name and its aliases ([`crate::facts::Facts::artists`]).
//! - A name is known by its folded key ([`similar::fold`]), never by being
//!   like another name: two spellings join only where they fold alike or
//!   one ID holds both. A key two IDs share, as two artists of one name,
//!   joins nothing.
//! - A group holding an ID is spelled as `[tags] script` says: the
//!   artist's own name, in its own script; or a name of theirs in Latin
//!   letters, its primary one first, then one no legal name, then one for
//!   an English locale. A search hint, a misspelling MusicBrainz keeps for
//!   search alone, neither spells an artist nor joins a name to it. With
//!   neither known, as most of the group's credits a record gave spell it
//!   in the script `[tags] script` names, any other script only where none
//!   spells it so; a credit spelled as another as common stays.
//! - A group with no ID is left as it is: nothing says its names are one
//!   artist's.
//!
//! Only a credit naming one artist is spelled so: no one name stands for
//! a credit of several. A tag set by hand is never changed.

use std::collections::{BTreeMap, BTreeSet};

use crate::album::Agreed;
use crate::clean::{Spelling, latin_only};
use crate::facts::Facts;
use crate::record::CreditedArtist;
use crate::resolve::Resolved;
use crate::similar;
use crate::source::SourceKey;
use crate::tags::Field;

/// The credits spelled, each with the field holding its artist's ID.
const CREDITS: [(Field, Field); 2] = [
    (Field::Artist, Field::MusicBrainzArtistId),
    (Field::AlbumArtist, Field::MusicBrainzAlbumArtistId),
];

/// One credit of one artist on one song.
struct Credit<'a> {
    n: usize,
    field: Field,
    name: &'a str,
    id: Option<&'a str>,
    /// Whether a MusicBrainz record gave it.
    by_record: bool,
    /// Whether the song lists it alone as its artists too, as Picard
    /// writes ARTISTS for one artist, which is spelled with it.
    listed: bool,
}

/// The spellings each song of `planned` takes, by its place in the song
/// list, from what `facts`' records say of the artists they credit.
#[must_use]
pub fn spell(
    planned: &[(usize, Resolved)],
    facts: &BTreeMap<SourceKey, Facts>,
    spelling: Spelling,
) -> BTreeMap<usize, Vec<Agreed>> {
    let credits: Vec<Credit<'_>> = planned
        .iter()
        .flat_map(|(n, r)| credits_of(*n, r))
        .collect();
    let known = known(facts);
    let (groups, _) = groups(&credits, &known);
    let mut out: BTreeMap<usize, Vec<Agreed>> = BTreeMap::new();
    for (id, members) in groups {
        let Some((name, from, ties)) = spelled(id, &members, &credits, known.get(id), spelling)
        else {
            continue;
        };
        for c in members.iter().map(|&i| &credits[i]) {
            if c.name != name && !ties.contains(&c.name) {
                let fields = std::iter::once(c.field).chain(c.listed.then_some(Field::Artists));
                for field in fields {
                    out.entry(c.n).or_default().push(Agreed {
                        field,
                        values: vec![name.clone()],
                        from: from.clone(),
                    });
                }
            }
        }
    }
    out
}

/// The names credited more than one way that no artist ID tells one
/// artist's, as `check` suggests them: each spelling with how many
/// credits spell it so, most first, then by name.
#[must_use]
pub fn suggestions(
    planned: &[(usize, Resolved)],
    facts: &BTreeMap<SourceKey, Facts>,
) -> Vec<Vec<(String, usize)>> {
    let credits: Vec<Credit<'_>> = planned
        .iter()
        .flat_map(|(n, r)| credits_of(*n, r))
        .collect();
    let (_, left) = groups(&credits, &known(facts));
    let mut by_key: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
    for c in left.iter().map(|&i| &credits[i]) {
        *by_key
            .entry(similar::fold(c.name))
            .or_default()
            .entry(c.name)
            .or_default() += 1;
    }
    by_key
        .into_values()
        .filter(|spellings| spellings.len() > 1)
        .map(|spellings| {
            let mut all: Vec<(String, usize)> = spellings
                .into_iter()
                .map(|(s, n)| (s.to_string(), n))
                .collect();
            all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            all
        })
        .collect()
}

/// Each artist the records in `facts` credit, by its ID.
fn known(facts: &BTreeMap<SourceKey, Facts>) -> BTreeMap<&str, &CreditedArtist> {
    let mut known: BTreeMap<&str, &CreditedArtist> = BTreeMap::new();
    for artist in facts.values().flat_map(|f| &f.artists) {
        known.entry(artist.id.as_str()).or_insert(artist);
    }
    known
}

/// The credits of one artist `r` writes, but those set by hand.
fn credits_of(n: usize, r: &Resolved) -> Vec<Credit<'_>> {
    let tag = |f: Field| {
        r.plan
            .tags
            .iter()
            .zip(&r.why.tags)
            .find(|((k, _), _)| k == f.vorbis())
            .map(|((_, v), why)| (v.as_slice(), why.from.as_str()))
    };
    CREDITS
        .iter()
        .filter_map(|&(field, ids)| {
            let (values, from) = tag(field)?;
            let [name] = values else {
                return None;
            };
            // An album artist taken from an artist set by hand is too.
            let by_hand = from.ends_with("song.tags") || from.ends_with("album.tags");
            if by_hand || similar::names_in(name).len() != 1 {
                return None;
            }
            let id = match tag(ids).map(|(v, _)| v) {
                Some([id]) => Some(id.as_str()),
                _ => None,
            };
            let by_record = SourceKey::parse(from).is_ok_and(|k| {
                matches!(k, SourceKey::Remote { site, .. } if site == crate::provider::MUSICBRAINZ)
            });
            let listed = field == Field::Artist
                && tag(Field::Artists).is_some_and(|(v, _)| matches!(v, [one] if one == name));
            Some(Credit {
                n,
                field,
                name,
                id,
                by_record,
                listed,
            })
        })
        .collect()
}

/// The credits of each artist ID, by their place in `credits`: those
/// naming it, and those whose name folds as a name it is known by does,
/// where no other ID is known by that name; and the credits of none.
fn groups<'c>(
    credits: &[Credit<'c>],
    known: &BTreeMap<&'c str, &'c CreditedArtist>,
) -> (BTreeMap<&'c str, Vec<usize>>, Vec<usize>) {
    let mut ids_of: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for c in credits {
        if let Some(id) = c.id {
            ids_of.entry(similar::fold(c.name)).or_default().insert(id);
        }
    }
    for (id, artist) in known {
        let names = std::iter::once(&artist.name).chain(
            (artist.aliases.iter())
                .filter(|a| !is_hint(a))
                .map(|a| &a.name),
        );
        for name in names.filter(|n| !n.is_empty()) {
            ids_of.entry(similar::fold(name)).or_default().insert(*id);
        }
    }
    let (mut groups, mut left): (BTreeMap<&str, Vec<usize>>, Vec<usize>) = Default::default();
    for (i, c) in credits.iter().enumerate() {
        let id = c.id.or_else(|| match ids_of.get(&similar::fold(c.name)) {
            Some(ids) if ids.len() == 1 => ids.first().copied(),
            _ => None,
        });
        match id {
            Some(id) => groups.entry(id).or_default().push(i),
            None => left.push(i),
        }
    }
    (groups, left)
}

/// Whether `alias` is a search hint, a misspelling MusicBrainz keeps for
/// search alone, which neither spells an artist nor joins a name to it.
fn is_hint(alias: &crate::record::Alias) -> bool {
    alias.kind.as_deref() == Some("Search hint")
}

/// How the group of artist `id` is spelled, why, and the spellings as
/// common among its records, which a credit keeps; none where no
/// spelling is known.
fn spelled<'a>(
    id: &str,
    members: &[usize],
    credits: &[Credit<'a>],
    artist: Option<&&CreditedArtist>,
    spelling: Spelling,
) -> Option<(String, String, Vec<&'a str>)> {
    if let Some(artist) = artist.filter(|a| !a.name.is_empty()) {
        if spelling == Spelling::Latin && !latin_only(&artist.name) {
            let latin = artist
                .aliases
                .iter()
                .filter(|a| !a.ended && !is_hint(a) && latin_only(&a.name))
                .min_by_key(|a| {
                    let english = a.locale.as_deref().is_some_and(|l| l.starts_with("en"));
                    let legal = a.kind.as_deref() == Some("Legal name");
                    (!a.primary, legal, !english, a.name.as_str())
                });
            if let Some(alias) = latin {
                let why = format!("musicbrainz artist {id}'s name in Latin letters");
                return Some((alias.name.clone(), why, Vec::new()));
            }
        }
        let why = format!("musicbrainz artist {id}'s own name");
        return Some((artist.name.clone(), why, Vec::new()));
    }
    let by_record: Vec<&Credit<'a>> = members
        .iter()
        .map(|&i| &credits[i])
        .filter(|c| c.by_record)
        .collect();
    let mut counts: BTreeMap<&'a str, usize> = BTreeMap::new();
    for c in &by_record {
        *counts.entry(c.name).or_default() += 1;
    }
    let wanted = |name: &str| (spelling == Spelling::Latin) == latin_only(name);
    if counts.keys().any(|name| wanted(name)) {
        counts.retain(|name, _| wanted(name));
    }
    let most = counts.values().copied().max()?;
    let ties: Vec<&'a str> = counts
        .into_iter()
        .filter(|(_, n)| *n == most)
        .map(|(name, _)| name)
        .collect();
    let why = format!(
        "as {most} of {} records crediting musicbrainz artist {id} spell it",
        by_record.len()
    );
    Some((ties[0].to_string(), why, ties))
}

#[cfg(test)]
mod tests;
