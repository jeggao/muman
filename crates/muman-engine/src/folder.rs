//! The cover file a folder of songs holds once, as each `[[cover.out]]`
//! of `scope = "folder"` names it: decided once every song is resolved,
//! from all of the folder's songs, and held by one of them.
//!
//! A folder is the songs whose files the template puts in it, its name
//! folded as a filesystem blind to case folds it. Its songs are of one
//! album when [`crate::album::of_each`] puts them on one, as they are
//! levelled ([`crate::levels`]); a single, or a song on no album, is an
//! album of its own. A folder of one album takes its cover, but for the
//! library folder itself, which holds the songs a template puts in no
//! folder: its cover would stand for the whole library, said. A folder of
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
//!
//! A song that moves carries the folder's file with it, as it carries
//! its own, so a folder a path template renames keeps its cover, the
//! user's change to it with it, as their change to a song's `.lrc`; only
//! where it holds its new folder's file, so songs of several folders
//! moving into one bring no file there of each, in one another's way;
//! each one carried is said moved, as it is named for no song. But one
//! the user changed stays where it is when the song that holds it moves
//! for a change to the song, its plan not the one its file was made from,
//! as every song of an album retagged to another album moves, or while
//! songs are left in its folder, as when the song that holds it moves to
//! another album: it is that folder's cover, not the song's. Carried off,
//! it would sit in another album's folder at a name the song there may
//! not write, its own folder given a new cover in its place, and `undo`,
//! which takes out what the run wrote but leaves a file of the user's,
//! could not put the two back. Left, it is a file of the user's own,
//! said so, to its folder's next holder, which writes beside it, and the
//! song's new folder gets a cover of its own.
//!
//! A song held at its last plan while it fails, while a sync leaves it as
//! built, or while a file of the user's keeps it unwritten
//! ([`crate::reconcile`]), is one of its folder's songs still, by that
//! plan: it counts toward the albums, the majority and the cover,
//! and one that holds the folder's file keeps it, its file kept with it.
//! So such a song moves no cover, makes none appear and takes none away,
//! and no two songs list one file. One a file of the user's keeps from
//! being written with no file of its own counts by its plan and holds
//! no folder's file, which another song writes.

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
    /// The folder is the library's own.
    Root,
}

/// An album of a folder, or for a song on none, the song's place.
type Album = (Option<crate::album::Album>, Option<usize>);

/// Give each folder's cover, as the folder outputs of `cover` say, to
/// the song of `planned` that holds it, the songs of `held` counted
/// among theirs, by their plan there alone; each folder's file not
/// written, and why.
pub fn assign(
    planned: &mut [(usize, Resolved)],
    held: &[(usize, Resolved)],
    cover: &Cover,
    facts: &BTreeMap<SourceKey, Facts>,
) -> Vec<Unplaced> {
    let outs: Vec<&CoverOut> = cover.of(Scope::Folder).collect();
    if outs.is_empty() {
        return Vec::new();
    }
    // Planned songs held by no other plan first, so an index below
    // `free.len()` is one that can be given the file.
    let mut given = Vec::new();
    let free: Vec<usize> = (0..planned.len())
        .filter(|&i| held.iter().all(|(n, _)| *n != planned[i].0))
        .collect();
    let unplaced = {
        let all: Vec<&(usize, Resolved)> = free.iter().map(|&i| &planned[i]).chain(held).collect();
        assigned(&all, free.len(), &outs, facts, &mut given)
    };
    for (i, placed) in given {
        planned[free[i]].1.plan.folder = Some(placed);
    }
    unplaced
}

/// [`assign`] of `all`, the first `planned` of which can hold a file:
/// each holder and its file into `given`.
fn assigned(
    all: &[&(usize, Resolved)],
    planned: usize,
    outs: &[&CoverOut],
    facts: &BTreeMap<SourceKey, Facts>,
    given: &mut Vec<(usize, CoverRef)>,
) -> Vec<Unplaced> {
    let paths: Vec<PathBuf> = all.iter().map(|(_, r)| path_of(r)).collect();
    let album_of = crate::album::of_each(all.iter().map(|(_, r)| r));
    let taken: BTreeSet<String> = all
        .iter()
        .zip(&paths)
        .flat_map(|((_, r), p)| {
            std::iter::once(p.clone()).chain(crate::artifact::parts(&r.plan).files(p))
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
        if folder.as_os_str().is_empty() {
            unplaced.push(Unplaced {
                folder,
                why: Why::Root,
            });
            continue;
        }
        let mut albums: BTreeMap<Album, Vec<usize>> = BTreeMap::new();
        for &i in songs.iter() {
            let album = album_of[i].clone();
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
        // A held song keeps the file it holds; else the first song by path
        // that can be given it holds it.
        if album
            .iter()
            .any(|&i| i >= planned && all[i].1.plan.folder.is_some())
        {
            continue;
        }
        let Some(&holder) = album.iter().find(|&&i| i < planned) else {
            continue;
        };
        let Some(base) = album_cover(all, &album) else {
            continue;
        };
        let q = facts
            .get(&base.key)
            .and_then(|f| f.covers.iter().find(|x| x.at == base.at))
            .and_then(|x| x.quality);
        let mut places = Vec::new();
        for out in admitted {
            let place = crate::resolve::place(out, &base.mimetype, q, out.name.clone());
            let file = crate::outputs::folder_file(&all[holder].1.stem, &place);
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
            given.push((holder, crate::resolve::placed(&base, places)));
        }
    }
    unplaced
}

/// The cover most of `album`'s songs of `planned`, in the order of their
/// paths, take, a tie going to the first; none where none takes one.
fn album_cover(planned: &[&(usize, Resolved)], album: &[usize]) -> Option<CoverRef> {
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
    if unplaced.iter().any(|u| u.why == Why::Root) {
        lines.push(
            "No folder cover in the library folder itself, where the template puts songs \
             in no folder of their own"
                .to_string(),
        );
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
