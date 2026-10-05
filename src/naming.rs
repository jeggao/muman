//! Where a song lands in the library: its tags made safe for a path, put
//! through the `[library]` template, each folder and the file name cut
//! to fit.
//!
//! A name is made safe the same way on every system, so a library copies
//! between them: by default anything Windows, FAT or a phone refuses is
//! replaced, `/` by `⧸` and the rest by their full-width lookalikes as
//! yt-dlp does (`：？＊＂＜＞｜⧹`); control characters go; leading dots
//! (hidden files) and trailing dots and spaces (which Windows drops) are
//! trimmed; and a reserved device name such as `CON` or `nul.txt` gets a
//! `_`. Limits are counted in UTF-8 bytes, which no filesystem's own
//! count exceeds.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::settings::{Library, Restrict};
use crate::template::{Fields, Template};

/// What Windows refuses in a name, besides control characters.
const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

const LOOKALIKES: [(&str, &str); 9] = [
    ("/", "⧸"),
    ("\\", "⧹"),
    (":", "："),
    ("*", "＊"),
    ("?", "？"),
    ("\"", "＂"),
    ("<", "＜"),
    (">", "＞"),
    ("|", "｜"),
];

/// Room kept in a file name for a collision suffix and the extension.
const SUFFIX_ROOM: usize = 24;

/// The longest ID a path is told apart by.
const ID_BYTES: usize = 64;

/// How songs are named, from the `[library]` settings.
#[derive(Debug)]
pub struct Naming {
    template: Template,
    replace: Vec<(String, String)>,
    restrict: Restrict,
    max_name: usize,
    max_folder: usize,
    /// Characters the whole path may take after the library folder.
    room: Option<usize>,
    unknown_artist: String,
    unknown_album: String,
    untitled: String,
}

/// A song's tags as they come, before they are made safe.
#[derive(Debug, Clone, Default)]
pub struct Tags<'a> {
    pub title: Option<&'a str>,
    pub artists: Vec<&'a str>,
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub date: Option<&'a str>,
    pub track: Option<&'a str>,
    pub disc: Option<&'a str>,
    pub id: &'a str,
}

impl Naming {
    /// Naming for a library in `library`, whose length counts against
    /// `max_path`.
    pub fn new(settings: &Library, library: &Path) -> Result<Self> {
        let mut replace: Vec<(String, String)> = match settings.restrict {
            Restrict::None => vec![("/".into(), "⧸".into())],
            Restrict::Windows | Restrict::Ascii => LOOKALIKES
                .iter()
                .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
                .collect(),
        };
        for (from, to) in &settings.replace {
            replace.retain(|(f, _)| f != from);
            replace.push((from.clone(), to.clone()));
        }
        // Longest first, so a replacement of `...` is tried before `.`.
        replace.sort_by_key(|(from, _)| std::cmp::Reverse(from.chars().count()));
        Ok(Self {
            template: Template::new(&settings.template)?,
            replace,
            restrict: settings.restrict,
            max_name: settings.max_name_bytes.saturating_sub(SUFFIX_ROOM).max(16),
            max_folder: settings.max_folder_bytes.max(16),
            room: settings
                .max_path
                .map(|max| max.saturating_sub(library.to_string_lossy().chars().count() + 1)),
            unknown_artist: settings.unknown_artist.clone(),
            unknown_album: settings.unknown_album.clone(),
            untitled: settings.untitled.clone(),
        })
    }

    /// The song's library path without its extension, relative to the
    /// library folder; never empty, never outside it.
    pub fn stem(&self, tags: &Tags<'_>) -> Result<PathBuf> {
        // Leading dots would hide a file or climb out of a folder.
        let safe = |s: Option<&str>| {
            let v = self.value(s.unwrap_or_default());
            v.trim().trim_start_matches('.').trim().to_string()
        };
        let or = |s: String, fallback: &str| {
            if s.is_empty() {
                self.value(fallback)
            } else {
                s
            }
        };
        let track = tags.track.and_then(number);
        let disc = tags.disc.and_then(number);
        let artists: Vec<String> = tags.artists.iter().map(|a| self.value(a)).collect();
        let date = safe(tags.date);
        let fields = Fields {
            title: or(safe(tags.title), &self.untitled),
            artist: artists.first().cloned().unwrap_or_default(),
            album: or(safe(tags.album), &self.unknown_album),
            album_artist: or(safe(tags.album_artist), &self.unknown_artist),
            genre: safe(tags.genre),
            year: date.chars().take(4).filter(char::is_ascii_digit).collect(),
            date,
            // Disc 1 keeps the plain number, so an album on one disc never
            // moves when a later edition adds a second.
            disc_track: match (disc, track) {
                (Some(d), Some(n)) if d > 1 => format!("{d}-{n:02} "),
                (_, Some(n)) => format!("{n:02} "),
                (_, None) => String::new(),
            },
            track,
            disc,
            artists,
            id: self.value(tags.id),
        };
        let rendered = self.template.render(&fields)?;
        let mut parts: Vec<String> = rendered
            .split('/')
            .map(|p| self.component(p, self.max_folder))
            .filter(|p| !p.is_empty())
            .collect();
        let name = parts.pop().unwrap_or_default();
        let folders: usize = parts.iter().map(|p| p.chars().count() + 1).sum();
        let name_limit = match self.room {
            Some(room) => self
                .max_name
                .min(room.saturating_sub(folders + SUFFIX_ROOM).max(8)),
            None => self.max_name,
        };
        let name = match self.component(&name, name_limit) {
            n if n.is_empty() => self.component(&self.untitled, name_limit),
            n => n,
        };
        Ok(parts
            .iter()
            .map(String::as_str)
            .chain([name.as_str()])
            .collect())
    }

    /// One tag value made safe for a path.
    fn value(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (from, to) in &self.replace {
            if !from.is_empty() {
                out = out.replace(from.as_str(), to);
            }
        }
        if self.restrict == Restrict::Ascii {
            out = deunicode::deunicode(&out);
        }
        out.chars()
            .filter(|c| !c.is_control())
            .map(|c| match self.restrict {
                Restrict::None if c == '/' => '_',
                Restrict::Windows | Restrict::Ascii if FORBIDDEN.contains(&c) => '_',
                _ => c,
            })
            .collect()
    }

    /// A display name as one path component within `limit` bytes.
    fn component(&self, name: &str, limit: usize) -> String {
        let trimmed = name.trim().trim_start_matches('.');
        let mut out = cut(trimmed, limit);
        if self.restrict != Restrict::None {
            out = out.trim_end_matches(['.', ' ']).to_string();
            if reserved(&out) {
                let at = out.find('.').unwrap_or(out.len());
                out.insert(at, '_');
            }
        }
        out.trim_end().to_string()
    }
}

/// A name Windows keeps for a device, extension or not: `CON`, `nul.txt`.
fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && stem.chars().count() == 4
            && stem
                .chars()
                .nth(3)
                .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)))
}

/// The number in a `TRACKNUMBER` or `DISCNUMBER` such as `3` or `3/12`.
fn number(text: &str) -> Option<u32> {
    text.split('/').next()?.trim().parse().ok()
}

/// `stem` told apart by `id`, and by `n` when that is not enough:
/// `Song [id]`, `Song [id 2]`. The suffix fits in the room every name
/// keeps for it.
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
    let name = cut(&name, name.len().min(ID_BYTES * 8));
    stem.with_file_name(format!("{name}{suffix}"))
}

/// `text` cut at a character boundary within `limit` bytes.
fn cut(text: &str, limit: usize) -> String {
    let mut end = 0;
    for (i, c) in text.char_indices() {
        if i + c.len_utf8() > limit {
            break;
        }
        end = i + c.len_utf8();
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naming(library: &Library) -> Naming {
        Naming::new(library, Path::new("lib")).unwrap()
    }

    fn tags<'a>(album_artist: &'a str, album: &'a str, title: &'a str) -> Tags<'a> {
        Tags {
            title: Some(title),
            album: Some(album),
            album_artist: Some(album_artist),
            id: "vid00000001",
            ..Tags::default()
        }
    }

    fn path(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    #[test]
    fn an_album_track_is_numbered() {
        let t = Tags {
            track: Some("3/12"),
            ..tags("Hoshi7ne", "Record", "Song")
        };
        assert_eq!(
            naming(&Library::default()).stem(&t).unwrap(),
            path(&["Hoshi7ne", "Record", "03 Song"])
        );
    }

    #[test]
    fn a_track_after_the_first_disc_carries_its_disc() {
        let n = naming(&Library::default());
        let mut t = Tags {
            disc: Some("2/3"),
            track: Some("1"),
            ..tags("Marlo Venn", "Long Tides", "Song")
        };
        assert_eq!(
            n.stem(&t).unwrap(),
            path(&["Marlo Venn", "Long Tides", "2-01 Song"])
        );
        t.disc = Some("1");
        assert_eq!(
            n.stem(&t).unwrap(),
            path(&["Marlo Venn", "Long Tides", "01 Song"])
        );
    }

    #[test]
    fn separators_and_dots_never_make_new_components() {
        assert_eq!(
            naming(&Library::default())
                .stem(&tags("Moth/Lamp", "..Hidden", "a/b"))
                .unwrap(),
            path(&["Moth⧸Lamp", "Hidden", "a⧸b"])
        );
        assert_eq!(
            naming(&Library::default())
                .stem(&tags("..", ".", "/"))
                .unwrap(),
            path(&["Unknown Artist", "Unknown Album", "⧸"])
        );
    }

    #[test]
    fn what_windows_refuses_is_replaced_on_every_system() {
        assert_eq!(
            naming(&Library::default())
                .stem(&tags("A: B", "Why? <Live>", "con. "))
                .unwrap(),
            path(&["A： B", "Why？ ＜Live＞", "con_"])
        );
        let none = Library {
            restrict: Restrict::None,
            ..Library::default()
        };
        assert_eq!(
            naming(&none).stem(&tags("A: B", "x", "Why?")).unwrap(),
            path(&["A: B", "x", "Why?"])
        );
    }

    #[test]
    fn ascii_transliterates_and_replacements_override_the_defaults() {
        let ascii = Library {
            restrict: Restrict::Ascii,
            replace: [(":".to_string(), " -".to_string())].into(),
            ..Library::default()
        };
        let stem = naming(&ascii)
            .stem(&tags("星屑ラジオ", "Ça: va", "Ünïcode?"))
            .unwrap();
        assert_eq!(stem, path(&["Xing Xie razio", "Ca - va", "Unicode_"]));
    }

    #[test]
    fn reserved_device_names_get_an_underscore() {
        assert!(reserved("CON") && reserved("nul.txt") && reserved("COM1") && reserved("lpt²"));
        assert!(!reserved("CONTACT") && !reserved("COM") && !reserved("Console.opus"));
    }

    #[test]
    fn blank_names_fall_back() {
        assert_eq!(
            naming(&Library::default())
                .stem(&tags(" ", "", ""))
                .unwrap(),
            path(&["Unknown Artist", "Unknown Album", "Untitled"])
        );
    }

    #[test]
    fn a_template_lays_out_the_library() {
        let custom = Library {
            template: "{{ genre | default('Unsorted', true) }}/{{ artist }} - {{ title }}".into(),
            ..Library::default()
        };
        let t = Tags {
            artists: vec!["Ada Quill", "Marlo Venn"],
            ..tags("x", "y", "Song")
        };
        assert_eq!(
            naming(&custom).stem(&t).unwrap(),
            path(&["Unsorted", "Ada Quill - Song"])
        );
    }

    #[test]
    fn a_long_name_is_cut_at_a_character_boundary() {
        let long = "雨".repeat(100);
        let stem = naming(&Library::default())
            .stem(&tags(&long, "B", &long))
            .unwrap();
        let folder = stem
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap();
        assert!(folder.len() <= 120 && folder.chars().all(|c| c == '雨'));
        let name = stem.file_name().unwrap().to_str().unwrap();
        assert!(name.len() <= 200 - SUFFIX_ROOM, "{}", name.len());
    }

    #[test]
    fn a_path_limit_cuts_the_title_to_fit() {
        let limited = Library {
            max_path: Some(80),
            ..Library::default()
        };
        let n = Naming::new(&limited, &PathBuf::from("l".repeat(20))).unwrap();
        let stem = n.stem(&tags("Artist", "Album", &"t".repeat(100))).unwrap();
        assert!(20 + 1 + stem.to_string_lossy().chars().count() + SUFFIX_ROOM <= 80 + 8);
    }

    #[test]
    fn a_suffix_tells_two_songs_apart() {
        assert_eq!(
            suffixed(&path(&["A", "B", "Song"]), "id", 1),
            path(&["A", "B", "Song [id]"])
        );
        assert_eq!(
            suffixed(&path(&["A", "B", "Song"]), "id", 2),
            path(&["A", "B", "Song [id 2]"])
        );
    }
}
