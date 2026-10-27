//! Songs listed apart that are one recording, by the prints of the audio
//! each is written from: the copy of a track a compilation repeats, or
//! one file of an album ripped twice.
//!
//! Every pair of songs is compared as `identify` compares a new source
//! with a song, and a pair called the same recording links the two;
//! linked songs make a group. A pair is compared only when the shorter
//! print is at least `RATIO` of the longer: the same recording spans
//! at least that share of the longer print, and the overlap of two prints
//! is no longer than the shorter, so no pair left out could be one. A
//! real library of 1,546 songs took 8 s, and found 12 groups on one album
//! and 144 across albums.
//!
//! A group with an album holding it twice is most likely a file there
//! twice, whatever other albums hold it too. An album is told by its
//! album artist with its name, so two albums of one name by two artists
//! are two, and a song on none is on no album. A group on several albums,
//! once on each, is a song and its copies elsewhere, which a library of
//! whole albums keeps. Nothing is changed: merging is for the user, by
//! moving the sources into one `[[song]]` or removing the others.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use anyhow::Result;

use crate::dirs::Dirs;
use crate::fingerprint::{Indexed, Verdict};
use crate::manifest::Manifest;
use crate::parallel;
use crate::query::{self, Query, View};
use crate::state::State;

/// The least share of the longer print the shorter is, for two to be
/// compared: the coverage the same recording needs (`fingerprint`).
const RATIO: f64 = 0.8;

/// Say each group of songs the query matches any of that are one
/// recording to `report`, and what is left unknown to `out`.
pub fn report<W: Write, D: Write>(
    dirs: &Dirs,
    terms: &[String],
    out: &mut W,
    report: &mut D,
) -> Result<()> {
    let manifest = Manifest::load(&dirs.home)?;
    let state = State::load(&dirs.home)?;
    let (views, resolved): (Vec<_>, Vec<_>) =
        query::resolved_views(&manifest, &state, &dirs.library)?
            .into_iter()
            .unzip();
    let query = Query::parse(terms, &query::extractors(&manifest))?;
    query.check_fields(&views)?;
    let planned: Vec<(usize, &crate::resolve::Resolved)> = resolved
        .iter()
        .enumerate()
        .filter_map(|(n, r)| Some((n, r.as_ref()?)))
        .collect();
    let (mut printed, mut short, mut still) = (Vec::new(), 0_usize, 0_usize);
    for (n, r) in &planned {
        match state.facts.get(&r.plan.audio.key).map(|f| f.print.as_ref()) {
            Some(Some(print)) => {
                let indexed = Indexed::new(print);
                if indexed.is_empty() {
                    still += 1;
                } else {
                    printed.push((*n, indexed));
                }
            }
            Some(None) => short += 1,
            None => {}
        }
    }
    let unmade = manifest.songs.len() - planned.len();
    for (count, why) in [
        (
            unmade,
            "cannot be made, so have no print; `status` says why",
        ),
        (short, "are too short to print, 2 s or less"),
        (
            still,
            "hold only silence or held notes, which any two songs share",
        ),
    ] {
        if count > 0 {
            crate::ui::info(
                out,
                &format!("{count} song(s) {why}; they are compared with none"),
            )?;
        }
    }
    printed.sort_by_key(|(_, p)| p.len());
    let step = crate::progress::step("Comparing prints", Some(printed.len() as u64));
    let pairs = parallel::map(
        &(0..printed.len()).collect::<Vec<_>>(),
        parallel::builds(),
        |&i| {
            let (_, a) = &printed[i];
            let linked = printed[i + 1..]
                .iter()
                .take_while(|(_, b)| ratio(a.len(), b.len()) >= RATIO)
                .filter(|(_, b)| {
                    a.compare_indexed(b)
                        .is_some_and(|m| m.verdict() == Verdict::Same)
                })
                .map(|(m, _)| (printed[i].0, *m))
                .collect::<Vec<_>>();
            step.advance(1);
            linked
        },
    );
    // Done before the report, so a plain progress line does not follow it.
    drop(step);
    let groups = groups(manifest.songs.len(), pairs.into_iter().flatten());
    let shown: Vec<&Vec<usize>> = groups
        .iter()
        .filter(|g| g.iter().any(|n| query.matches(&views[*n])))
        .collect();
    let album = |n: usize| views[n].first("album");
    let ids = |g: &[usize]| g.iter().map(|n| album_id(&views[*n])).collect::<Vec<_>>();
    let (alone, across): (Vec<&Vec<usize>>, Vec<&Vec<usize>>) =
        shown.into_iter().partition(|g| twice(&ids(g)).is_some());
    for group in alone.iter().chain(&across) {
        let ids = ids(group);
        let albums = ids.iter().collect::<BTreeSet<_>>().len();
        let on = match (albums, twice(&ids)) {
            (1, _) => format!("on one album, {}", album(group[0])),
            (n, Some(at)) => format!("on {n} albums, twice on {}", album(group[at])),
            (n, None) => format!("on {n} albums"),
        };
        crate::ui::line(
            report,
            &format!("{} songs, one recording, {on}:", group.len()),
        )?;
        for n in *group {
            let view = &views[*n];
            let key = view.id().map(ToString::to_string).unwrap_or_default();
            crate::ui::line(report, &format!("  {key}\t{}\t{}", view.name(), album(*n)))?;
        }
    }
    summary(report, (alone.len(), across.len()), groups.len())
}

/// What tells the album `view`'s song is on from another: its album
/// artist, else its artist, with its name; `None` on no album.
fn album_id(view: &View) -> Option<(String, String)> {
    let name = view.first("album");
    if name.is_empty() {
        return None;
    }
    let mut by = view.first("album_artist");
    if by.is_empty() {
        by = view.first("artist");
    }
    Some((by, name))
}

/// Where in `albums`, one per song of a group, an album holds it a
/// second time.
fn twice(albums: &[Option<(String, String)>]) -> Option<usize> {
    albums
        .iter()
        .enumerate()
        .find(|(n, a)| a.is_some() && albums[..*n].contains(a))
        .map(|(n, _)| n)
}

/// The line after the groups shown, `alone` with an album holding it
/// twice and `across` albums, of `found` in all.
fn summary<D: Write>(report: &mut D, (alone, across): (usize, usize), found: usize) -> Result<()> {
    if found == 0 {
        crate::ui::success(report, "No two songs are one recording")?;
    } else if alone + across == 0 {
        crate::ui::info(
            report,
            "No group of one recording holds a song the query matches",
        )?;
    } else {
        crate::ui::info(
            report,
            &format!(
                "{alone} group(s) an album holds twice, {across} across albums. To keep one song of a \
                 group, move the others' sources into its [[song]] with `muman edit`, or \
                 remove them"
            ),
        )?;
    }
    Ok(())
}

/// The shorter of two lengths over the longer.
fn ratio(a: usize, b: usize) -> f64 {
    a.min(b) as f64 / a.max(b).max(1) as f64
}

/// The groups of more than one that `links` make of `count` songs, each
/// in song-list order, ordered by their first song.
fn groups(count: usize, links: impl IntoIterator<Item = (usize, usize)>) -> Vec<Vec<usize>> {
    let mut root: Vec<usize> = (0..count).collect();
    for (a, b) in links {
        let (a, b) = (root_of(&mut root, a), root_of(&mut root, b));
        root[a.max(b)] = a.min(b);
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for n in 0..count {
        let r = root_of(&mut root, n);
        by_root.entry(r).or_default().push(n);
    }
    by_root.into_values().filter(|g| g.len() > 1).collect()
}

/// The first song of `n`'s group, shortening the path to it on the way.
fn root_of(root: &mut [usize], mut n: usize) -> usize {
    while root[n] != n {
        root[n] = root[root[n]];
        n = root[n];
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_album_holding_a_recording_twice_is_found_among_others() {
        let on = |by: &str, name: &str| Some((by.to_string(), name.to_string()));
        let both = [
            on("Ada Quill", "Lantern Weather"),
            on("Ada Quill", "Best Of"),
            on("Ada Quill", "Lantern Weather"),
        ];
        assert_eq!(twice(&both), Some(2));
        assert_eq!(twice(&both[..2]), None);
    }

    #[test]
    fn albums_of_one_name_by_two_artists_or_no_album_hold_nothing_twice() {
        let on = |by: &str, name: &str| Some((by.to_string(), name.to_string()));
        assert_eq!(
            twice(&[on("Ada Quill", "Best Of"), on("Marlo Venn", "Best Of")]),
            None
        );
        assert_eq!(twice(&[None, None]), None);
        let mut view = View::default();
        assert_eq!(album_id(&view), None);
        view.tags = vec![
            ("ARTIST".into(), vec!["Marlo Venn".into()]),
            ("ALBUM".into(), vec!["Best Of".into()]),
        ];
        assert_eq!(album_id(&view), on("Marlo Venn", "Best Of"));
    }

    #[test]
    fn linked_songs_make_one_group() {
        assert_eq!(
            groups(6, [(4, 1), (1, 3), (2, 5)]),
            [vec![1, 3, 4], vec![2, 5]]
        );
        assert_eq!(groups(3, []), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn prints_too_unalike_in_length_are_never_one_recording() {
        assert!(ratio(800, 1000) >= RATIO);
        assert!(ratio(799, 1000) < RATIO);
    }
}
