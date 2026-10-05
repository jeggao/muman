//! Where a song lands in the library: `<album artist>/<album>/<NN title>`,
//! named from its resolved tags, `<D-NN title>` from a second disc on.

use std::path::{Path, PathBuf};

/// The longest folder name kept, leaving room for the files inside it.
const FOLDER_BYTES: usize = 120;
/// The longest file stem kept, under the filesystem's 255-byte limit
/// with the extension and a `.part` suffix.
const STEM_BYTES: usize = 200;

/// The library path of a song without its extension, relative to the
/// library folder. A missing name falls back so the path never has an
/// empty component.
#[must_use]
pub fn stem(
    album_artist: &str,
    album: &str,
    disc: Option<&str>,
    track: Option<&str>,
    title: &str,
) -> PathBuf {
    // Disc 1 keeps the plain number, so an album on one disc never moves.
    let number = match (disc.and_then(track_number), track.and_then(track_number)) {
        (Some(d), Some(n)) if d > 1 => format!("{d}-{n:02} "),
        (_, Some(n)) => format!("{n:02} "),
        (_, None) => String::new(),
    };
    let title = component(title, STEM_BYTES - number.len().max(3));
    let name = format!("{number}{}", or(&title, "Untitled"));
    [
        or(&component(album_artist, FOLDER_BYTES), "Unknown Artist"),
        or(&component(album, FOLDER_BYTES), "Unknown Album"),
        &name,
    ]
    .iter()
    .collect()
}

/// The longest ID a path is told apart by.
const ID_BYTES: usize = 64;

/// `stem` told apart by `id`, and by `n` when that is not enough:
/// `Song [id]`, `Song [id 2]`. The name is cut first, so the whole stays
/// within the stem limit.
#[must_use]
pub fn suffixed(stem: &Path, id: &str, n: u32) -> PathBuf {
    let id = cut(id, ID_BYTES);
    let suffix = if n > 1 {
        format!(" [{} {n}]", id.trim())
    } else {
        format!(" [{}]", id.trim())
    };
    let name = stem
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let name = component(&name, STEM_BYTES - suffix.len());
    stem.with_file_name(format!("{name}{suffix}"))
}

fn or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.is_empty() { fallback } else { s }
}

/// The number in a `TRACKNUMBER` or `DISCNUMBER` such as `3` or `3/12`.
fn track_number(track: &str) -> Option<u32> {
    track.split('/').next()?.trim().parse().ok()
}

/// A display name as one path component: no separator, no leading dot,
/// without control characters, cut at a character boundary within `limit`
/// bytes.
#[must_use]
pub fn component(name: &str, limit: usize) -> String {
    cut(name.trim().trim_start_matches('.'), limit)
        .trim_end()
        .to_string()
}

/// `text` without separators or control characters, cut at a character
/// boundary within `limit` bytes.
fn cut(text: &str, limit: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c == '/' { '⧸' } else { c };
        if c.is_control() {
            continue;
        }
        if out.len() + c.len_utf8() > limit {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_album_track_is_numbered() {
        assert_eq!(
            stem("Hoshi7ne", "Record", None, Some("3/12"), "Song"),
            PathBuf::from("Hoshi7ne/Record/03 Song")
        );
    }

    #[test]
    fn a_track_after_the_first_disc_carries_its_disc() {
        assert_eq!(
            stem("Marlo Venn", "Long Tides", Some("2/3"), Some("1"), "Song"),
            PathBuf::from("Marlo Venn/Long Tides/2-01 Song")
        );
        assert_eq!(
            stem("Marlo Venn", "Long Tides", Some("1"), Some("1"), "Song"),
            PathBuf::from("Marlo Venn/Long Tides/01 Song")
        );
    }

    #[test]
    fn a_single_is_named_by_its_title() {
        assert_eq!(
            stem("A", "Song", None, None, "Song"),
            PathBuf::from("A/Song/Song")
        );
    }

    #[test]
    fn separators_and_dots_never_make_new_components() {
        assert_eq!(
            stem("Moth/Lamp", "..Hidden", None, None, "a/b"),
            PathBuf::from("Moth⧸Lamp/Hidden/a⧸b")
        );
    }

    #[test]
    fn blank_names_fall_back() {
        assert_eq!(
            stem(" ", "", None, None, ""),
            PathBuf::from("Unknown Artist/Unknown Album/Untitled")
        );
    }

    #[test]
    fn a_suffix_cuts_the_name_to_fit() {
        let long = "雨".repeat(66);
        let stem = stem("A", "B", None, None, &long);
        let out = suffixed(&stem, &long, 2);
        let name = out.file_name().unwrap().to_str().unwrap();
        assert!(name.len() <= STEM_BYTES, "{}", name.len());
        assert!(name.starts_with('雨') && name.ends_with(" 2]"), "{name}");
        assert_eq!(out.parent(), Some(Path::new("A/B")));
        assert_eq!(
            suffixed(Path::new("A/B/Song"), "id", 1),
            PathBuf::from("A/B/Song [id]")
        );
    }

    #[test]
    fn a_long_name_is_cut_at_a_character_boundary() {
        let long = "雨".repeat(100);
        let cut = component(&long, FOLDER_BYTES);
        assert!(cut.len() <= FOLDER_BYTES);
        assert!(cut.chars().all(|c| c == '雨'));
    }
}
