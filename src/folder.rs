//! The cover file a folder of songs holds once, as each `[[cover.out]]`
//! of `scope = "folder"` names it: decided once every song is resolved,
//! from all of the folder's songs, and held by one of them.
//!
//! A folder is the songs whose files the template puts in it, its name
//! folded as a filesystem blind to case folds it. Its songs are of one
//! album when they share an album artist and album, as an album is
//! levelled ([`crate::loudness`]); a single, or a song on no album, is an
//! album of its own. A folder of one album takes its cover. A folder of
//! several takes none, said, unless an output's `mixed = "majority"`
//! takes the album most of its songs are on, where one album's songs
//! outnumber every other's. An album's cover is the one most of its
//! songs in the folder take, a tie going to the first song's by its path,
//! so a cover pinned by hand counts as any other.
//!
//! The file is held by the album's first song there, by its path:
//! written with that song's files, recorded beside it and removed with
//! it, so every file in the library stays one song's, kept for `undo`,
//! checked, counted toward the size limit and exported as the song's. A
//! song added before it, or the song removed, hands the file to another,
//! which writes it over; a file another song's plan writes is never
//! stale to the song it leaves ([`crate::reconcile`]). A name one of the
//! songs' own files takes is left to the song, said.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::facts::Facts;
use crate::reconcile::path_of;
use crate::relpath::folded;
use crate::resolve::{CoverRef, Resolved};
use crate::settings::{Cover, CoverOut, Mixed, Scope};
use crate::source::SourceKey;

/// A folder's file not written, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unplaced {
    /// The folder, as the library holds it.
    pub folder: PathBuf,
    pub why: Why,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// Its songs are of this many albums.
    Albums(usize),
    /// Of this many albums, none has the most songs.
    Tie(usize),
    /// A song's own file takes the name.
    Taken(PathBuf),
}

/// An album of a folder: its artist and name, or for a song on none, the
/// song's place.
type Album = (Option<(String, String)>, Option<usize>);

/// Give each folder's cover, as the folder outputs of `cover` say, to
/// the song of `planned` that holds it; each folder's file not written,
/// and why.
pub fn assign(
    planned: &mut [(usize, Resolved)],
    cover: &Cover,
    facts: &BTreeMap<SourceKey, Facts>,
) -> Vec<Unplaced> {
    let outs: Vec<&CoverOut> = cover.of(Scope::Folder).collect();
    if outs.is_empty() {
        return Vec::new();
    }
    let paths: Vec<PathBuf> = planned.iter().map(|(_, r)| path_of(r)).collect();
    let taken: BTreeSet<String> = planned
        .iter()
        .zip(&paths)
        .flat_map(|((_, r), p)| {
            std::iter::once(p.clone()).chain(crate::outputs::planned(p, &r.plan))
        })
        .map(|p| folded(&p))
        .collect();
    let mut folders: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, path) in paths.iter().enumerate() {
        folders.entry(folded(parent(path))).or_default().push(i);
    }
    let mut unplaced = Vec::new();
    for songs in folders.values_mut() {
        songs.sort_by_key(|&i| (folded(&paths[i]), paths[i].clone()));
        let folder = parent(&paths[songs[0]]).to_path_buf();
        let mut albums: BTreeMap<Album, Vec<usize>> = BTreeMap::new();
        for &i in songs.iter() {
            let album = crate::loudness::album_of(&planned[i].1);
            let own = album.is_none().then_some(i);
            albums.entry((album, own)).or_default().push(i);
        }
        let most = albums.values().map(Vec::len).max().unwrap_or(0);
        let leaders: Vec<&Vec<usize>> = albums.values().filter(|s| s.len() == most).collect();
        let chosen = match leaders[..] {
            [one] => Some(one.clone()),
            _ => None,
        };
        let several = albums.len() > 1;
        let admitted: Vec<&CoverOut> = outs
            .iter()
            .copied()
            .filter(|o| !several || (o.mixed == Mixed::Majority && chosen.is_some()))
            .collect();
        if admitted.len() < outs.len() {
            let tie = chosen.is_none() && outs.iter().any(|o| o.mixed == Mixed::Majority);
            let why = if tie {
                Why::Tie(albums.len())
            } else {
                Why::Albums(albums.len())
            };
            unplaced.push(Unplaced {
                folder: folder.clone(),
                why,
            });
        }
        let Some(album) = chosen.filter(|_| !admitted.is_empty()) else {
            continue;
        };
        let Some(base) = album_cover(planned, &album) else {
            continue;
        };
        let q = facts
            .get(&base.key)
            .and_then(|f| f.covers.iter().find(|x| x.at == base.at))
            .and_then(|x| x.quality);
        let holder = album[0];
        let mut places = Vec::new();
        for out in admitted {
            let place = crate::resolve::place(out, &base.mimetype, q, out.name.clone());
            let file = crate::outputs::folder_file(&planned[holder].1.stem, &place);
            if let Some(file) = file.filter(|f| taken.contains(&folded(f))) {
                unplaced.push(Unplaced {
                    folder: folder.clone(),
                    why: Why::Taken(file),
                });
                continue;
            }
            places.push(place);
        }
        if !places.is_empty() {
            planned[holder].1.plan.folder = Some(crate::resolve::placed(&base, places));
        }
    }
    unplaced
}

/// The cover most of `album`'s songs of `planned`, in the order of their
/// paths, take, a tie going to the first; none where none takes one.
fn album_cover(planned: &[(usize, Resolved)], album: &[usize]) -> Option<CoverRef> {
    let mut counts: Vec<(CoverRef, usize)> = Vec::new();
    for &i in album {
        let Some(c) = &planned[i].1.plan.cover else {
            continue;
        };
        let base = CoverRef {
            out: None,
            made: None,
            ..c.clone()
        };
        match counts.iter_mut().find(|(b, _)| *b == base) {
            Some((_, n)) => *n += 1,
            None => counts.push((base, 1)),
        }
    }
    let most = counts.iter().map(|(_, n)| *n).max()?;
    counts.into_iter().find(|(_, n)| *n == most).map(|(b, _)| b)
}

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

/// What `unplaced` says, a line for each reason, naming a few folders.
#[must_use]
pub fn said(unplaced: &[Unplaced]) -> Vec<String> {
    const NAMED: usize = 3;
    let named = |folders: &[&Path]| {
        let mut list: Vec<String> = folders
            .iter()
            .take(NAMED)
            .map(|f| crate::relpath::show(f))
            .collect();
        if folders.len() > NAMED {
            list.push(format!("and {} more", folders.len() - NAMED));
        }
        list.join(", ")
    };
    let of = |want: fn(&Why) -> bool| -> Vec<&Path> {
        unplaced
            .iter()
            .filter(|u| want(&u.why))
            .map(|u| u.folder.as_path())
            .collect()
    };
    let mut lines = Vec::new();
    let mixed = of(|w| matches!(w, Why::Albums(_)));
    if !mixed.is_empty() {
        lines.push(format!(
            "No folder cover where songs of several albums share a folder: {} \
             ([[cover.out]] mixed = \"majority\" takes the album most of its songs are on)",
            named(&mixed)
        ));
    }
    let tied = of(|w| matches!(w, Why::Tie(_)));
    if !tied.is_empty() {
        lines.push(format!(
            "No folder cover where no album has the most of a folder's songs: {}",
            named(&tied)
        ));
    }
    for u in unplaced {
        if let Why::Taken(file) = &u.why {
            lines.push(format!(
                "No folder cover at {}: a song's own file takes the name",
                crate::relpath::show(file)
            ));
        }
    }
    lines
}
