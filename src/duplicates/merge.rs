//! Songs that are one recording made one song: which of them keeps its
//! `[[song]]`, the hand tags they set apart, and the edit that moves the
//! others' sources into it.
//!
//! The song kept is the one whose source the merged song's audio would
//! come from, so the merge changes no audio the sync after writes: the
//! songs' sources are resolved together as one song, as they will be
//! once merged, and the song listing the source picked keeps its table.
//! Its album and place, its pins and its lyrics offset stay; another's
//! are let go, and the plan says which. The others' sources join it with
//! what each held when built, so a fetched source's record of what its
//! song was built from is kept rather than taken again from what the
//! source holds now. A tag only one of them sets by hand is kept; one
//! they set apart is the user's to choose.

use anyhow::{Context, Result};

use crate::manifest::{Edit, Manifest, Rewritten, Song, SongTable, Tags};
use crate::reconcile::Resolver;
use crate::tags::vorbis_key;

/// The songs at `group` listed as one: every source once, in song-list
/// order, an audio pin only where a single song sets one, and nothing
/// set by hand.
pub(super) fn together(m: &Manifest, group: &[usize]) -> Song {
    let mut song = Song::default();
    for n in group {
        for key in &m.songs[*n].sources {
            if !song.sources.contains(key) {
                song.sources.push(key.clone());
            }
        }
    }
    let pinned: Vec<_> = group
        .iter()
        .filter_map(|n| m.songs[*n].audio.clone())
        .collect();
    if let [one] = pinned.as_slice() {
        song.audio = Some(one.clone());
    }
    song
}

/// The song of `among` the merged song's audio would come from; the
/// first when they do not resolve together.
pub(super) fn keeper(m: &Manifest, resolver: &Resolver<'_>, among: &[usize]) -> usize {
    let first = among.first().copied().unwrap_or_default();
    let Ok(merged) = resolver.resolve(&together(m, among)) else {
        return first;
    };
    among
        .iter()
        .copied()
        .find(|n| m.songs[*n].has(&merged.plan.audio.key))
        .unwrap_or(first)
}

/// A hand tag songs merged set apart.
#[derive(Debug)]
pub(super) struct Clash {
    /// Its Vorbis name.
    pub tag: String,
    /// Each value set, with the songs that set it.
    pub set: Vec<(Vec<String>, Vec<usize>)>,
}

/// Each hand tag two of `songs` set apart, in song-list order.
pub(super) fn clashes(m: &Manifest, songs: &[usize]) -> Vec<Clash> {
    let mut out: Vec<Clash> = Vec::new();
    for n in songs {
        for (name, values) in &m.songs[*n].tags {
            let tag = vorbis_key(name);
            let at = if let Some(at) = out.iter().position(|c| c.tag == tag) {
                at
            } else {
                out.push(Clash {
                    tag,
                    set: Vec::new(),
                });
                out.len() - 1
            };
            let set = &mut out[at].set;
            if let Some((_, by)) = set.iter_mut().find(|(v, _)| v == values) {
                by.push(*n);
            } else {
                set.push((values.clone(), vec![*n]));
            }
        }
    }
    out.retain(|c| c.set.len() > 1);
    out
}

/// What `song` lets go of when merged into another, in words: its album
/// and place, its pins and its lyrics offset.
pub(super) fn let_go(song: &Song) -> Vec<&'static str> {
    let mut gone = Vec::new();
    if song.album.is_some() || song.track.is_some() {
        gone.push("its album and place");
    }
    if song.audio.is_some() {
        gone.push("its audio pin");
    }
    if song.cover.is_some() {
        gone.push("its cover pin");
    }
    if song.lyrics.is_some() {
        gone.push("its lyrics pin");
    }
    if song.lyrics_offset_ms != 0 {
        gone.push("its lyrics offset");
    }
    gone
}

/// The edit that moves the sources of `others` into `keeper`, with what
/// each held when built, sets the tags only they set and `winners` for
/// those set apart, and takes the others out of the list.
pub(super) fn edit(m: &Manifest, keeper: usize, others: &[usize], winners: &Tags) -> Result<Edit> {
    let song = &m.songs[keeper];
    let id = song.id().context("a song lists no source")?.clone();
    let mut table = m
        .tables_by_id(std::slice::from_ref(&id))?
        .remove(&id)
        .with_context(|| format!("{id} is no longer listed"))?;
    let mut sources = song.sources.clone();
    for o in others {
        for key in &m.songs[*o].sources {
            if !sources.contains(key) {
                sources.push(key.clone());
            }
        }
    }
    table.insert("sources", crate::manifest::key_array(&sources));
    // After the sources: a record of a key the table does not list goes.
    for o in others {
        for (key, held) in &m.songs[*o].held {
            crate::held::set(&mut table, key, held);
        }
    }
    let set = |tags: &Tags, key: &str| tags.iter().any(|(n, _)| vorbis_key(n) == key);
    let mut tags: Tags = Vec::new();
    for o in others {
        for (name, values) in &m.songs[*o].tags {
            let key = vorbis_key(name);
            if !set(&song.tags, &key) && !set(&tags, &key) && !set(winners, &key) {
                tags.push((name.clone(), values.clone()));
            }
        }
    }
    tags.extend(winners.iter().cloned());
    crate::manifest::set_tags(&mut table, &tags);
    let mut songs = vec![(id, Rewritten::Table(SongTable(table)))];
    for o in others {
        let other = m.songs[*o].id().context("a song lists no source")?.clone();
        let note = other.to_string();
        songs.push((other, Rewritten::Removed(note)));
    }
    Ok(Edit::Rewrite {
        songs,
        new: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceKey;

    const SONGS: &str = r#"version = 1

[[song]]
sources = ["manual:a.flac"]
audio = "manual:a.flac"
tags = { genre = "Folk", title = "Tide" }

[[song]]
sources = ["youtube.com:vid00000001"]
track = 2
lyrics_offset = "120 ms"
tags = { genre = "Ambient", composer = "Ada Quill" }
held."youtube.com:vid00000001" = { audio = "7c1cfa040b3b82c0" }
"#;

    fn manifest(dir: &tempfile::TempDir) -> Manifest {
        std::fs::write(dir.path().join("songs.toml"), SONGS).unwrap();
        Manifest::load(dir.path()).unwrap()
    }

    #[test]
    fn songs_merged_keep_one_table_every_source_and_what_each_held() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = manifest(&dir);
        let clash = clashes(&m, &[0, 1]);
        assert_eq!(clash.len(), 1, "{clash:?}");
        assert_eq!(clash[0].tag, "GENRE");
        assert_eq!(
            let_go(&m.songs[1]),
            ["its album and place", "its lyrics offset"]
        );
        let winners = vec![("genre".to_string(), vec!["Ambient".to_string()])];
        let edit = edit(&m, 0, &[1], &winners).unwrap();
        m.trial(std::slice::from_ref(&edit)).unwrap();
        m.edit(edit);
        assert!(m.save_if_unchanged(&[]).unwrap().is_empty());
        let m = Manifest::load(dir.path()).unwrap();
        assert_eq!(m.songs.len(), 1, "{:?}", m.songs);
        assert!(m.removed.is_empty(), "a merged song leaves no tombstone");
        let song = &m.songs[0];
        let video = SourceKey::youtube("vid00000001");
        assert_eq!(
            song.sources,
            [SourceKey::Manual("a.flac".into()), video.clone()]
        );
        assert!(song.held.contains_key(&video), "{song:?}");
        assert_eq!(song.audio, Some(SourceKey::Manual("a.flac".into())));
        assert_eq!(song.lyrics_offset_ms, 0);
        let tag = |name: &str| {
            song.tags
                .iter()
                .find(|(n, _)| vorbis_key(n) == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(tag("GENRE"), Some(vec!["Ambient".to_string()]));
        assert_eq!(tag("COMPOSER"), Some(vec!["Ada Quill".to_string()]));
        assert_eq!(tag("TITLE"), Some(vec!["Tide".to_string()]));
    }

    #[test]
    fn songs_together_list_each_source_once_and_one_pin_alone() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest(&dir);
        let song = together(&m, &[0, 1]);
        assert_eq!(song.sources.len(), 2);
        assert_eq!(song.audio, Some(SourceKey::Manual("a.flac".into())));
        assert!(song.tags.is_empty());
    }
}
