use crate::ui::MockPrompter;

use super::*;
use crate::testing::{Fake, words};

struct Home {
    _dir: tempfile::TempDir,
    dirs: Dirs,
}

/// A home listing one song, `youtube.com:aaaaaaaaaaa`, and holding the
/// manual files named.
fn home(manual: &[&str]) -> Home {
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        home: dir.path().join("home"),
        library: dir.path().join("lib"),
    };
    let fetched = dirs.ytdlp().join("c/Song [aaaaaaaaaaa].mkv");
    std::fs::create_dir_all(fetched.parent().unwrap()).unwrap();
    std::fs::write(fetched, b"mkv").unwrap();
    for m in manual {
        let path = dirs.manual().join(m);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, m.as_bytes()).unwrap();
    }
    std::fs::write(
        dirs.manifest(),
        "version = 1\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
    )
    .unwrap();
    Home { _dir: dir, dirs }
}

fn proposal(path: &str) -> Proposal {
    Proposal {
        sources: vec![SourceKey::Manual(path.into())],
        album: None,
        label: path.into(),
    }
}

/// The listed song's print, and prints for the manual files: the same
/// recording, an excerpt of it, and another song.
fn fake() -> Fake {
    let song = words(1600, 1);
    let excerpt = song[400..900].to_vec();
    Fake::default()
        .probe("same.flac", crate::testing::FLAC)
        .probe("part.flac", crate::testing::FLAC)
        .probe("other.flac", crate::testing::FLAC)
        .print("aaaaaaaaaaa", song.clone())
        .print("same.flac", song)
        .print("part.flac", excerpt)
        .print("other.flac", words(1600, 2))
}

fn run(
    h: &Home,
    proposals: Vec<Proposal>,
    mode: Mode,
    prompter: Option<&mut dyn Prompter>,
) -> (Vec<Edit>, String) {
    let mut out = Vec::new();
    let edits = identify(&fake(), &h.dirs, proposals, mode, false, prompter, &mut out).unwrap();
    (edits, String::from_utf8(out).unwrap())
}

fn joined(edit: &Edit) -> bool {
    matches!(edit, Edit::Add { sources, .. } if sources.first() == Some(&SourceKey::youtube("aaaaaaaaaaa")))
}

#[test]
fn the_same_recording_joins_its_song_and_another_stands_alone() {
    let h = home(&["same.flac", "other.flac"]);
    let (edits, text) = run(
        &h,
        vec![proposal("same.flac"), proposal("other.flac")],
        Mode::Ask,
        None,
    );
    assert!(joined(&edits[0]), "{edits:?}");
    assert!(!joined(&edits[1]), "{edits:?}");
    assert!(text.contains("the same recording as"), "{text}");
    let state = State::load(&h.dirs.home).unwrap();
    assert!(
        state
            .facts
            .contains_key(&SourceKey::Manual("same.flac".into())),
        "measures are kept"
    );
}

#[test]
fn an_uncertain_match_is_asked_about() {
    let h = home(&["part.flac"]);
    let mut yes = MockPrompter::answering([true]);
    let (edits, _) = run(&h, vec![proposal("part.flac")], Mode::Ask, Some(&mut yes));
    assert!(joined(&edits[0]));
    let mut no = MockPrompter::answering([false]);
    let (edits, _) = run(&h, vec![proposal("part.flac")], Mode::Ask, Some(&mut no));
    assert!(!joined(&edits[0]));
}

#[test]
fn without_a_terminal_an_uncertain_match_stands_alone_and_says_how_to_merge() {
    let h = home(&["part.flac"]);
    let (edits, text) = run(&h, vec![proposal("part.flac")], Mode::Ask, None);
    assert!(!joined(&edits[0]));
    assert!(text.contains("-y"), "{text}");
}

#[test]
fn yes_joins_and_new_never_matches() {
    let h = home(&["part.flac", "same.flac"]);
    let (edits, _) = run(&h, vec![proposal("part.flac")], Mode::Yes, None);
    assert!(joined(&edits[0]));
    let (edits, _) = run(&h, vec![proposal("same.flac")], Mode::New, None);
    assert!(!joined(&edits[0]));
}

#[test]
fn two_new_files_of_one_song_become_one() {
    let h = home(&["other.flac", "b/other.flac"]);
    let mut fake = fake();
    fake.prints.push(("b/other.flac".into(), words(1600, 2)));
    let mut out = Vec::new();
    let edits = identify(
        &fake.probe("b/other.flac", crate::testing::FLAC),
        &h.dirs,
        vec![proposal("other.flac"), proposal("b/other.flac")],
        Mode::Ask,
        false,
        None,
        &mut out,
    )
    .unwrap();
    assert!(
        matches!(&edits[1], Edit::Add { sources, .. } if sources[0] == SourceKey::Manual("other.flac".into())),
        "{edits:?}"
    );
}

#[test]
fn verbose_says_how_close_each_candidate_came() {
    let h = home(&["other.flac"]);
    let mut out = Vec::new();
    identify(
        &fake(),
        &h.dirs,
        vec![proposal("other.flac")],
        Mode::Ask,
        true,
        None,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("other.flac against"), "{text}");
    assert!(text.contains("Different"), "{text}");
}
