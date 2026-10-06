use std::cell::Cell;

use super::*;
use crate::manifest::Manifest;

struct Home {
    _dir: tempfile::TempDir,
    dirs: Dirs,
}

fn home(songs: &str) -> Home {
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        home: dir.path().join("home"),
        library: dir.path().join("lib"),
    };
    for name in ["a", "b", "c"] {
        let file = dirs.manual().join(format!("{name}.flac"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, name).unwrap();
    }
    std::fs::write(dirs.manifest(), format!("version = 1\n{songs}")).unwrap();
    Home { _dir: dir, dirs }
}

const TWO: &str = "[[song]]\nsources = [\"manual:a.flac\", \"manual:b.flac\"]\n\n[[song]]\nsources = [\"manual:c.flac\"]\n";

/// What one opening of the editor makes of the file.
type Step = Box<dyn Fn(&str) -> String>;

/// Edit with each step in turn, one per time the editor opens.
fn edit_with(h: &Home, steps: &[Step]) -> (bool, String, Vec<String>) {
    let mut prompter = crate::ui::MockPrompter::new();
    let seen = std::cell::RefCell::new(Vec::new());
    let step = Cell::new(0);
    let mut editor = |path: &Path| -> Result<()> {
        let text = std::fs::read_to_string(path)?;
        seen.borrow_mut().push(text.clone());
        let n = step.get();
        step.set(n + 1);
        let f = steps.get(n).expect("the editor opened once too often");
        std::fs::write(path, f(&text))?;
        Ok(())
    };
    let mut out = Vec::new();
    let changed = edit(
        &h.dirs,
        &[],
        false,
        Some(&mut prompter),
        &mut editor,
        &mut out,
    )
    .unwrap();
    (changed, String::from_utf8(out).unwrap(), seen.into_inner())
}

fn songs(h: &Home) -> Manifest {
    Manifest::load(&h.dirs.home).unwrap()
}

#[test]
fn a_tag_set_in_the_editor_is_applied() {
    let h = home(TWO);
    let (changed, text, seen) = edit_with(
        &h,
        &[Box::new(|t| {
            t.replacen("[[song]]\nid = \"manual:c.flac\"\nsources = [\"manual:c.flac\"]",
                "[[song]]\nid = \"manual:c.flac\"\nsources = [\"manual:c.flac\"]\ntags = { title = \"Sea\" }", 1)
        })],
    );
    assert!(changed, "{text}");
    assert!(seen[0].contains("id = \"manual:a.flac\""), "{}", seen[0]);
    let m = songs(&h);
    assert_eq!(
        m.songs[1].tags,
        [("title".to_string(), vec!["Sea".to_string()])]
    );
    assert_eq!(m.songs[0].sources.len(), 2, "the song left alone stays");
}

#[test]
fn nothing_changed_changes_nothing() {
    let h = home(TWO);
    let before = std::fs::read_to_string(h.dirs.manifest()).unwrap();
    let (changed, text, _) = edit_with(&h, &[Box::new(str::to_string)]);
    assert!(!changed);
    assert!(text.contains("Nothing changed"), "{text}");
    assert_eq!(std::fs::read_to_string(h.dirs.manifest()).unwrap(), before);
}

#[test]
fn a_mistake_reopens_with_what_is_wrong_and_a_fix_applies() {
    let h = home(TWO);
    let (changed, _, seen) = edit_with(
        &h,
        &[
            Box::new(|t| t.replace("\"manual:c.flac\"]", "\"manual:nowhere.flac\"]")),
            Box::new(|t| {
                t.replace(
                    "\"manual:nowhere.flac\"]",
                    "\"manual:c.flac\"]\nlyrics_offset_ms = 5",
                )
            }),
        ],
    );
    assert!(changed);
    assert!(seen[1].starts_with(PROBLEM), "{}", seen[1]);
    assert!(seen[1].contains("not in the store"), "{}", seen[1]);
    assert_eq!(songs(&h).songs[1].lyrics_offset_ms, 5);
}

#[test]
fn a_comment_inside_a_song_survives_and_changes_nothing() {
    let commented = "[[song]]\n# ripped from the second pressing\nsources = [\"manual:a.flac\", \"manual:b.flac\"]\n\n[[song]]\nsources = [\"manual:c.flac\"]\n";
    let h = home(commented);
    let (changed, text, seen) = edit_with(&h, &[Box::new(str::to_string)]);
    assert!(
        seen[0].contains("# ripped from the second pressing"),
        "{}",
        seen[0]
    );
    assert!(!changed, "{text}");
    assert!(
        std::fs::read_to_string(h.dirs.manifest())
            .unwrap()
            .contains("# ripped from the second pressing")
    );
}

#[test]
fn deleting_every_song_opened_removes_them() {
    let h = home(TWO);
    let (changed, text, _) = edit_with(
        &h,
        &[Box::new(|t| t[..t.find("\n# ").unwrap()].to_string())],
    );
    assert!(changed, "{text}");
    let m = songs(&h);
    assert_eq!(m.songs, []);
    assert_eq!(m.removed.len(), 2);
}

#[test]
fn a_deleted_song_is_removed_and_a_moved_key_split_off() {
    let h = home(TWO);
    let (changed, text, _) = edit_with(
        &h,
        &[Box::new(|t| {
            let at = t.find("\n# ").unwrap();
            let c = t.rfind("[[song]]").unwrap();
            let head = &t[..at];
            let first = &t[at..c];
            format!(
                "{head}{}\n[[song]]\nsources = [\"manual:b.flac\"]\n",
                first.replace(", \"manual:b.flac\"", "")
            )
        })],
    );
    assert!(changed, "{text}");
    let m = songs(&h);
    let lists: Vec<Vec<String>> = m
        .songs
        .iter()
        .map(|s| s.sources.iter().map(ToString::to_string).collect())
        .collect();
    assert_eq!(lists, [vec!["manual:a.flac"], vec!["manual:b.flac"]]);
    assert_eq!(m.removed.len(), 1);
    assert_eq!(m.removed[0].sources[0].to_string(), "manual:c.flac");
    assert!(text.contains("Removed:"), "{text}");
}

#[test]
fn a_song_changed_meanwhile_is_said_and_saving_again_applies() {
    let h = home(TWO);
    let manifest = h.dirs.manifest();
    let (changed, _, seen) = edit_with(
        &h,
        &[
            Box::new(move |t| {
                let now = std::fs::read_to_string(&manifest).unwrap();
                std::fs::write(
                    &manifest,
                    now.replace(
                        "[\"manual:c.flac\"]",
                        "[\"manual:c.flac\"]\nlyrics_offset_ms = 9",
                    ),
                )
                .unwrap();
                t.replace(
                    "[\"manual:c.flac\"]",
                    "[\"manual:c.flac\"]\nlyrics_offset_ms = 1",
                )
            }),
            Box::new(str::to_string),
        ],
    );
    assert!(changed);
    assert!(seen[1].starts_with(NOTICE), "{}", seen[1]);
    assert_eq!(songs(&h).songs[1].lyrics_offset_ms, 1);
}
