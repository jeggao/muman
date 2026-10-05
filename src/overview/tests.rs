use super::*;
use crate::reconcile::{Options, reconcile};
use crate::testing::Fake;

fn home() -> (tempfile::TempDir, Dirs) {
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        home: dir.path().join("home"),
        library: dir.path().join("lib"),
    };
    std::fs::create_dir_all(&dirs.home).unwrap();
    for id in ["aaaaaaaaaaa", "bbbbbbbbbbb"] {
        let path = dirs.ytdlp().join(format!("chan/Song {id} [{id}].mkv"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, id).unwrap();
    }
    (dir, dirs)
}

fn songs(dirs: &Dirs, ids: &[&str]) {
    let body: String = ids
        .iter()
        .map(|id| format!("[[song]]\nsources = [\"youtube:{id}\"]\n"))
        .collect::<Vec<_>>()
        .concat();
    std::fs::write(dirs.manifest(), format!("version = 1\n{body}")).unwrap();
}

fn fake() -> Fake {
    ["aaaaaaaaaaa", "bbbbbbbbbbb"].iter().fold(Fake::default(), |f, id| {
        f.info(
            id,
            &format!(r#"{{"id": "{id}", "title": "Title {id}", "uploader": "Chan", "subtitles": {{"en": [{{"name": "English"}}]}}}}"#),
        )
    })
}

fn info_text(dirs: &Dirs, verbose: bool) -> String {
    let mut out = Vec::new();
    info(dirs, verbose, &mut out).unwrap();
    crate::ui::plain(&String::from_utf8(out).unwrap())
}

/// The value a label's row shows, colors and all.
fn value(text: &str, label: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with(label))
        .map_or_else(
            || panic!("no {label} in:\n{text}"),
            |l| l.trim_start()[label.len()..].trim().to_string(),
        )
}

#[test]
fn a_synced_library_counts_its_songs_and_is_up_to_date() {
    let (_dir, dirs) = home();
    songs(&dirs, &["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    let mut out = Vec::new();
    assert!(reconcile(&fake(), &dirs, Options::default(), None, &mut out).unwrap());
    let text = info_text(&dirs, false);
    assert_eq!(value(&text, "Songs"), "2 (0 on 0 albums, 2 singles)");
    assert_eq!(value(&text, "Up to date"), "2");
    assert_eq!(value(&text, "To write"), "0");
    assert_eq!(
        value(&text, "Formats"),
        "2 Opus (0 encoded), 0 FLAC (0 encoded)"
    );
    assert_eq!(value(&text, "Cannot be made"), "0");
    assert_eq!(value(&text, "No lyrics"), "0");
    assert!(
        !text.contains("Title aaaaaaaaaaa"),
        "names only when verbose"
    );
}

#[test]
fn what_a_sync_would_do_is_counted_and_named_when_verbose() {
    let (_dir, dirs) = home();
    songs(&dirs, &["aaaaaaaaaaa"]);
    let mut out = Vec::new();
    assert!(reconcile(&fake(), &dirs, Options::default(), None, &mut out).unwrap());
    songs(&dirs, &["bbbbbbbbbbb", "ccccccccccc"]);
    let text = info_text(&dirs, true);
    assert_eq!(value(&text, "To remove"), "1");
    assert_eq!(value(&text, "Sources missing"), "1");
    assert!(text.contains("youtube:ccccccccccc"), "{text}");
    assert_eq!(value(&text, "Sources to measure"), "1");
    assert!(text.contains("Chan/Title aaaaaaaaaaa"), "{text}");
}

#[test]
fn sizes_and_times_read_plainly() {
    assert_eq!(bytes(512), "512 B");
    assert_eq!(bytes(3 * 1024 * 1024 * 1024 / 2), "1.5 GiB");
    assert_eq!(duration(59.0), "1 min");
    assert_eq!(duration(3.0 * 3600.0 + 120.0), "3 h 2 min");
    assert_eq!(duration(50.0 * 3600.0), "2 d 2 h");
}
