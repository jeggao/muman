use super::*;
use crate::manifest::{Edit, Manifest};

const SONG: &str =
    "version = 1\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"manual:a.flac\"]\n";

fn held(audio: &str, format: Option<&str>) -> Held {
    Held {
        audio: Some(audio.into()),
        format: format.map(str::to_string),
        size: format.map(|_| 3_456_789),
        ..Held::default()
    }
}

fn load(text: &str) -> (tempfile::TempDir, Manifest) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("songs.toml"), text).unwrap();
    let m = Manifest::load(dir.path()).unwrap();
    (dir, m)
}

#[test]
fn a_record_is_written_as_one_line_per_source_and_read_back() {
    let (dir, mut m) = load(SONG);
    let yt = SourceKey::youtube("aaaaaaaaaaa");
    m.edit(Edit::Hold {
        song: yt.clone(),
        key: yt.clone(),
        held: Held {
            tags: Some("fedcba9876543210".into()),
            ..held("0123456789abcdef", Some("251"))
        },
    });
    m.save().unwrap();
    let text = std::fs::read_to_string(dir.path().join("songs.toml")).unwrap();
    assert!(
        text.contains(
            "held.\"youtube.com:aaaaaaaaaaa\" = { audio = \"0123456789abcdef\", tags = \"fedcba9876543210\", format = \"251\", size = 3456789 }\n"
        ),
        "{text}"
    );
    let m = Manifest::load(dir.path()).unwrap();
    assert_eq!(
        m.songs[0].held[&yt].tags.as_deref(),
        Some("fedcba9876543210")
    );
    assert_eq!(m.songs[0].held[&yt].format.as_deref(), Some("251"));
}

#[test]
fn a_source_renamed_or_dropped_loses_its_record() {
    let text =
        format!("{SONG}held.\"youtube.com:aaaaaaaaaaa\" = {{ audio = \"0123456789abcdef\" }}\n");
    for edit in [
        Edit::Drop(SourceKey::youtube("aaaaaaaaaaa")),
        Edit::Rename {
            from: SourceKey::youtube("aaaaaaaaaaa"),
            to: SourceKey::youtube("bbbbbbbbbbb"),
        },
    ] {
        let (dir, mut m) = load(&text);
        m.edit(edit);
        m.save().unwrap();
        let after = std::fs::read_to_string(dir.path().join("songs.toml")).unwrap();
        assert!(!after.contains("held.\"youtube"), "{after}");
    }
}

#[test]
fn a_record_of_a_source_not_listed_is_ignored_and_one_unreadable_refused() {
    let (_d, m) = load(&format!(
        "{SONG}held.\"youtube.com:zzzzzzzzzzz\" = {{ audio = \"0123456789abcdef\" }}\n"
    ));
    assert!(m.songs[0].held.is_empty());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("songs.toml"),
        format!("{SONG}held.\"manual:a.flac\" = {{ audio = \"short\" }}\n"),
    )
    .unwrap();
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(
        format!("{e:#}").contains("`audio` must be at least 16 hex digits"),
        "{e:#}"
    );
    std::fs::write(
        dir.path().join("songs.toml"),
        format!("{SONG}held.\"manual:a.flac\" = {{ format = \"251\" }}\n"),
    )
    .unwrap();
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("records no digest"), "{e:#}");
}

#[test]
fn a_part_is_the_same_as_far_as_both_digests_go_and_one_not_recorded_no_change() {
    let short = held("0123456789abcdef", None);
    assert_eq!(
        short.changed(&held("0123456789abcdef0011223344556677", None)),
        []
    );
    assert_eq!(
        short.changed(&held("0123456789abcdee", None)),
        [Aspect::Audio]
    );
    let more = Held {
        cover: Some("1111111111111111".into()),
        tags: Some("2222222222222222".into()),
        ..held("0123456789abcdef", None)
    };
    assert!(short.changed(&more).is_empty(), "recorded, not a change");
    let fewer = Held {
        tags: Some("3333333333333333".into()),
        ..held("0123456789abcdef", None)
    };
    assert_eq!(more.changed(&fewer), [Aspect::Cover, Aspect::Tags]);
}

#[test]
fn a_drift_names_each_part_changed_and_the_format() {
    let was = Held {
        cover: Some("1111111111111111".into()),
        lyrics: Some("2222222222222222".into()),
        tags: Some("3333333333333333".into()),
        ..held("0123456789abcdef", Some("251"))
    };
    let now = Held {
        cover: Some("4444444444444444".into()),
        lyrics: None,
        tags: Some("3333333333333333".into()),
        ..held("fedcba9876543210", Some("140"))
    };
    let drift = Drift {
        song: 0,
        key: SourceKey::youtube("aaaaaaaaaaa"),
        changed: was.changed(&now),
        was,
        now,
    };
    assert_eq!(
        drift.show(),
        "youtube.com:aaaaaaaaaaa has changed since its song was built: \
         other audio and cover, no lyrics (format 251 → 140)"
    );
}

#[test]
fn only_a_youtube_format_is_asked_for_again() {
    let yt = SourceKey::youtube("aaaaaaaaaaa");
    let merged = held("0123456789abcdef", Some("399+251"));
    assert_eq!(
        merged.asked(&yt, "ba").as_deref(),
        Some("399+251/bv*+251/251/ba")
    );
    let single = held("0123456789abcdef", Some("140"));
    assert_eq!(single.asked(&yt, "ba").as_deref(), Some("140/ba"));
    let other = SourceKey::parse("archiveorg:item0001").unwrap();
    assert_eq!(single.asked(&other, "ba"), None, "numbered by place");
    assert_eq!(held("0123456789abcdef", None).asked(&yt, "ba"), None);
}

#[test]
fn a_site_serves_the_recorded_format_at_its_size_or_says_how_not() {
    let info = |formats: &str| {
        crate::info::parse(format!(r#"{{"id": "aaaaaaaaaaa", "formats": [{formats}]}}"#).as_bytes())
            .unwrap()
    };
    let opus = |size: u64| {
        format!(r#"{{"format_id": "251", "filesize": {size}, "acodec": "opus", "vcodec": "none"}}"#)
    };
    let video = r#"{"format_id": "399", "filesize": 9, "acodec": "none", "vcodec": "av01"}"#;
    let was = Held {
        size: Some(100),
        ..held("0123456789abcdef", Some("399+251"))
    };
    let at = |formats: &str| was.upstream(&info(formats));
    assert_eq!(at(&format!("{video}, {}", opus(100))), Some(Upstream::Same));
    assert_eq!(
        at(&format!("{video}, {}", opus(120))),
        Some(Upstream::Resized {
            format: "251".into(),
            was: 100,
            now: 120
        })
    );
    assert_eq!(
        at(r#"{"format_id": "251-0", "filesize": 100, "acodec": "opus", "vcodec": "none"}"#),
        Some(Upstream::Renumbered("251-0".into()))
    );
    assert_eq!(at(video), Some(Upstream::Withdrawn("251".into())));
    assert_eq!(
        at(r#"{"format_id": "251", "acodec": "opus", "vcodec": "none"}"#),
        Some(Upstream::Unsized("251".into()))
    );
}

#[test]
fn a_source_two_songs_share_is_held_by_each() {
    let (dir, mut m) = load(
        "version = 3\n[[song]]\nsources = [\"manual:a.flac\", \"manual:added/cover.jpg\"]\n\
         [[song]]\nsources = [\"manual:b.flac\", \"manual:added/cover.jpg\"]\n",
    );
    let cover = SourceKey::Manual("added/cover.jpg".into());
    for song in ["a.flac", "b.flac"] {
        m.edit(Edit::Hold {
            song: SourceKey::Manual(song.into()),
            key: cover.clone(),
            held: Held {
                cover: Some("0123456789abcdef".into()),
                ..Held::default()
            },
        });
    }
    m.save().unwrap();
    let again = Manifest::load(dir.path()).unwrap();
    assert!(
        again.songs.iter().all(|s| s.held.contains_key(&cover)),
        "{:?}",
        again.songs
    );
    let text = std::fs::read_to_string(dir.path().join("songs.toml")).unwrap();
    assert!(
        text.starts_with("version = 3") || text.contains("\nversion = 3\n"),
        "{text}"
    );
}
