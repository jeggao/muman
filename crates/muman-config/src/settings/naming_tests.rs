//! The naming of songs that `[library]` gives, built by
//! [`Library::naming`]: the path each song's tags make under the template,
//! made safe as `restrict` says, and cut to fit the limits. Here rather
//! than beside naming's own tests, as they read the table's defaults.

use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::naming::Tags;
use crate::naming::*;

fn naming(library: &Library) -> Naming {
    library.naming(Path::new("lib")).unwrap()
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
        // Windows itself refuses them whatever `restrict` says.
        if cfg!(windows) {
            path(&["A_ B", "x", "Why_"])
        } else {
            path(&["A: B", "x", "Why?"])
        }
    );
    assert_eq!(
        naming(&none).stem(&tags("CON", "LPT9.", "x. ")).unwrap(),
        if cfg!(windows) {
            path(&["CON_", "LPT9_", "x"])
        } else {
            path(&["CON", "LPT9.", "x."])
        }
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
fn blank_names_fall_back() {
    assert_eq!(
        naming(&Library::default())
            .stem(&tags(" ", "", ""))
            .unwrap(),
        path(&["Unknown Artist", "Unknown Album", "Untitled"])
    );
}

#[test]
fn the_original_year_is_the_album_s_first_else_the_song_s_own() {
    let custom = Library {
        template: "{{ original_year }} {{ year }}/{{ title }}".into(),
        ..Library::default()
    };
    let n = naming(&custom);
    let mut t = Tags {
        date: Some("2010-11-16"),
        original_date: Some("1971-03-02"),
        ..tags("x", "y", "Song")
    };
    assert_eq!(n.stem(&t).unwrap(), path(&["1971 2010", "Song"]));
    t.original_date = None;
    assert_eq!(n.stem(&t).unwrap(), path(&["2010 2010", "Song"]));
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
fn a_name_at_the_least_limit_keeps_a_grapheme_whole() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} Family";
    let short = Library {
        max_name_bytes: MIN_NAME_BYTES,
        ..Library::default()
    };
    let stem = naming(&short).stem(&tags("A", "B", family)).unwrap();
    let name = stem.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, "\u{1F468}\u{200D}\u{1F469}");
}

#[test]
fn a_title_of_nothing_visible_is_untitled_and_no_name_reorders() {
    let n = naming(&Library::default());
    let stem = n.stem(&tags("A", "B", "\u{200B}\u{FE0F}")).unwrap();
    assert_eq!(stem.file_name().unwrap(), "Untitled");
    let stem = n.stem(&tags("A", "B", "abc\u{202E}gpj.exe")).unwrap();
    assert_eq!(stem.file_name().unwrap(), "abcgpj.exe");
}

#[test]
fn a_path_limit_cuts_the_title_to_fit() {
    let limited = Library {
        max_path: Some(80),
        ..Library::default()
    };
    let n = limited.naming(&PathBuf::from("l".repeat(20))).unwrap();
    let stem = n.stem(&tags("Artist", "Album", &"t".repeat(100))).unwrap();
    assert!(20 + 1 + stem.to_string_lossy().chars().count() + SUFFIX_ROOM <= 80 + 8);
}

#[test]
fn a_name_takes_the_name_limit_and_a_suffix_fits_within_it() {
    let stem = naming(&Library::default())
        .stem(&tags("A", "B", &"t".repeat(300)))
        .unwrap();
    let name = stem.file_name().unwrap().to_str().unwrap().len();
    assert!(name > 120 && name <= 200 - SUFFIX_ROOM, "{name}");
    let long = suffixed(&stem, &"i".repeat(100), 3, (200, AUDIO_TAIL), None);
    let total = long.file_name().unwrap().to_str().unwrap().len() + ".flac.moving".len();
    assert!(total <= 200, "{total}");
}

#[test]
fn a_longer_file_beside_a_song_shortens_every_name_as_much() {
    let tail = ".zh-Hant.lrc".len();
    let n = naming(&Library::default()).with_tail(tail);
    let stem = n.stem(&tags("A", "B", &"t".repeat(300))).unwrap();
    let name = stem.file_name().unwrap().to_str().unwrap().len();
    assert_eq!(name, 200 - SUFFIX_ROOM - (tail - AUDIO_TAIL));
    let told = n.suffixed(&stem, &"i".repeat(100), 3);
    let total = told.file_name().unwrap().to_str().unwrap().len() + tail + ".moving".len();
    assert!(total <= 200, "{total}");
    assert_eq!(min_name_bytes(tail), MIN_NAME_BYTES + tail - AUDIO_TAIL);
    assert_eq!(min_name_bytes(".lrc".len()), MIN_NAME_BYTES);
}

#[test]
fn a_blank_title_never_takes_the_album_folders_place() {
    assert_eq!(
        naming(&Library::default())
            .stem(&tags("A", "Album", ". ."))
            .unwrap(),
        path(&["A", "Album", "Untitled"])
    );
}

#[test]
fn a_decomposed_tag_names_the_composed_path() {
    assert_eq!(
        naming(&Library::default())
            .stem(&tags("A", "B", "Cafe\u{301}"))
            .unwrap(),
        path(&["A", "B", "Caf\u{e9}"])
    );
}

#[test]
fn what_the_template_spells_is_made_safe_too() {
    let custom = Library {
        template: "{{ album_artist }}: {{ title }}?".into(),
        ..Library::default()
    };
    assert_eq!(
        naming(&custom).stem(&tags("A", "x", "T")).unwrap(),
        path(&["A： T？"])
    );
}

#[test]
fn a_path_limit_holds_even_for_deep_templates() {
    let deep = Library {
        template: "{{ album_artist }}/{{ album }}/{{ genre }}x/{{ title }}".into(),
        max_path: Some(60),
        ..Library::default()
    };
    let n = deep.naming(Path::new("lib")).unwrap();
    let long = "w".repeat(80);
    let stem = n.stem(&tags(&long, &long, &long)).unwrap();
    let chars = 4 + stem.to_string_lossy().chars().count();
    assert!(chars + SUFFIX_ROOM <= 60, "{chars}");
}

#[test]
fn a_name_composed_longer_still_fits_its_bytes() {
    let n = naming(&Library::default());
    // U+0958 composes from its parts' 6 bytes to none, NFC keeping them
    // apart: a cut before composing let the name grow past its limit.
    let title = "\u{0958}".repeat(100);
    let stem = n
        .stem(&tags("Marlo Venn", &"\u{0958}".repeat(60), &title))
        .unwrap();
    for part in &stem {
        let part = part.to_string_lossy();
        assert_eq!(part.nfc().collect::<String>(), part);
        assert!(
            part.len() <= Library::default().max_name_bytes,
            "{}",
            part.len()
        );
    }
    let album = stem
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy();
    assert!(
        album.len() <= Library::default().max_folder_bytes,
        "{}",
        album.len()
    );
}

#[test]
fn a_path_limit_counts_as_windows_does_and_keeps_characters_whole() {
    let limited = Library {
        max_path: Some(100),
        ..Library::default()
    };
    let n = limited.naming(&PathBuf::from("l".repeat(10))).unwrap();
    let clef = "\u{1D11E}".repeat(40);
    let stem = n.stem(&tags(&clef, &clef, &clef)).unwrap();
    let whole = 11 + units(&stem.to_string_lossy());
    assert!(whole + SUFFIX_ROOM <= 100, "{whole} units");
    let family = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}".repeat(20);
    let flags = "\u{1F1EF}\u{1F1F5}".repeat(40);
    for title in [family.as_str(), flags.as_str()] {
        let stem = n.stem(&tags("A", "B", title)).unwrap();
        let name = stem.file_name().unwrap().to_string_lossy().into_owned();
        let one = title.graphemes(true).next().unwrap();
        assert!(
            !name.is_empty() && name.graphemes(true).all(|g| g == one),
            "{name}"
        );
    }
}

#[test]
fn asciify_never_makes_a_folder_of_a_separator_in_a_value() {
    for restrict in [Restrict::None, Restrict::Windows] {
        let ascii = Library {
            template: "{{ album_artist | asciify }}/{{ title | asciify }}".into(),
            restrict,
            ..Library::default()
        };
        let n = naming(&ascii);
        assert_eq!(
            n.stem(&tags("Moth/Lamp", "B", "Night\\Day")).unwrap(),
            path(&["Moth_Lamp", "Night_Day"]),
            "{restrict:?}"
        );
        assert_eq!(
            n.stem(&tags("Ça", "B", "½ Tide ÷ 2")).unwrap(),
            path(&["Ca", "1_2 Tide _ 2"]),
            "{restrict:?}"
        );
    }
}

#[test]
fn a_backslash_never_names_a_folder_on_another_system() {
    let none = Library {
        restrict: Restrict::None,
        ..Library::default()
    };
    let stem = naming(&none).stem(&tags("A", "B", "a\\b")).unwrap();
    assert_eq!(stem.file_name().unwrap(), "a⧹b");
}
