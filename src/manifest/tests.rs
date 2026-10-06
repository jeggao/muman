use super::*;

fn yt(id: &str) -> SourceKey {
    SourceKey::youtube(id)
}

fn text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(MANIFEST)).unwrap()
}

fn write(dir: &Path, body: &str) {
    std::fs::write(dir.join(MANIFEST), format!("version = 1\n{body}")).unwrap();
}

const TEMPLATE: &str = "[song.tags]\ntitle = \"\"\nartist = \"\"\nalbum = \"\"\nalbum_artist = \"\"\ngenre = \"\"\ndate = \"\"\n";

#[test]
fn a_new_list_carries_its_header_defaults_and_each_song() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = Manifest::load(dir.path()).unwrap();
    assert_eq!(m.lyrics, ["en"]);
    m.edit(Edit::Add {
        sources: vec![yt("aaaaaaaaaaa"), yt("bbbbbbbbbbb")],
        album: None,
    });
    m.save().unwrap();
    let t = text(dir.path());
    assert!(t.starts_with(&format!("{HEADER}{NEW}")), "{t}");
    assert!(
        t.ends_with(&format!(
            "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\", \"youtube:bbbbbbbbbbb\"]\n\n{TEMPLATE}"
        )),
        "{t}"
    );
    let again = Manifest::load(dir.path()).unwrap();
    assert_eq!(
        again.songs[0].sources,
        [yt("aaaaaaaaaaa"), yt("bbbbbbbbbbb")]
    );
}

#[test]
fn adding_a_listed_source_joins_its_song_and_sets_its_album_once() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"] # mine\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    let album = SourceKey::parse("youtubetab:OLAK5uy_abcdef").unwrap();
    m.edit(Edit::Add {
        sources: vec![yt("aaaaaaaaaaa"), SourceKey::Manual("x.flac".into())],
        album: Some((album.clone(), 3)),
    });
    m.edit(Edit::Add {
        sources: vec![yt("aaaaaaaaaaa")],
        album: Some((album.clone(), 9)),
    });
    m.save().unwrap();
    assert_eq!(m.songs.len(), 1);
    let song = &m.songs[0];
    assert_eq!(
        song.sources,
        [yt("aaaaaaaaaaa"), SourceKey::Manual("x.flac".into())]
    );
    assert_eq!((song.album.as_ref(), song.track), (Some(&album), Some(3)));
}

#[test]
fn comments_and_unknown_keys_survive_a_write() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(MANIFEST),
        "# mine\nversion = 1\nfuture = true\n\n[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"] # kept\nrating = 5\n",
    )
    .unwrap();
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Add {
        sources: vec![yt("bbbbbbbbbbb")],
        album: None,
    });
    m.save().unwrap();
    let t = text(dir.path());
    for kept in ["# mine", "future = true", "rating = 5"] {
        assert!(t.contains(kept), "{kept} lost:\n{t}");
    }
    assert!(t.starts_with(HEADER), "{t}");
}

#[test]
fn a_save_keeps_what_another_run_wrote_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = Manifest::load(dir.path()).unwrap();
    let mut second = Manifest::load(dir.path()).unwrap();
    first.edit(Edit::Add {
        sources: vec![yt("aaaaaaaaaaa")],
        album: None,
    });
    second.edit(Edit::Add {
        sources: vec![yt("bbbbbbbbbbb")],
        album: None,
    });
    first.save().unwrap();
    second.save().unwrap();
    assert_eq!(second.songs.len(), 2);
}

#[test]
fn pins_tags_and_offsets_are_read() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\", \"manual:a.flac\"]\naudio = \"manual:a.flac\"\nlyrics = false\nlyrics_offset_ms = -120\ntags = { title = \"T\", track = 3, artist = [\"A\", \"B\"], odd = true, empty = \"\" }\n",
    );
    let m = Manifest::load(dir.path()).unwrap();
    let s = &m.songs[0];
    assert_eq!(s.audio, Some(SourceKey::Manual("a.flac".into())));
    assert_eq!(s.lyrics, Some(LyricsPin::None));
    assert_eq!(s.lyrics_offset_ms, -120);
    assert_eq!(
        s.tags,
        vec![
            ("title".to_string(), vec!["T".to_string()]),
            ("track".to_string(), vec!["3".to_string()]),
            ("artist".to_string(), vec!["A".to_string(), "B".to_string()]),
        ]
    );
}

#[test]
fn a_pin_outside_the_sources_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\ncover = \"manual:x.png\"\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("pins manual:x.png"), "{e:#}");
}

#[test]
fn a_source_listed_twice_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("both song 1 and song 2"), "{e:#}");
}

#[test]
fn a_bad_key_is_refused_rather_than_dropping_its_song() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "[[song]]\nsources = [\"youtube:short\"]\n");
    assert!(Manifest::load(dir.path()).is_err());
}

#[test]
fn another_version_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MANIFEST), "version = 3\n").unwrap();
    assert!(format!("{:#}", Manifest::load(dir.path()).unwrap_err()).contains("newer"));
    std::fs::write(dir.path().join(MANIFEST), "version = 1\n[[song]\n").unwrap();
    assert!(Manifest::load(dir.path()).is_err());
    assert_eq!(text(dir.path()), "version = 1\n[[song]\n");
}

#[test]
fn songs_share_a_kept_record_but_never_a_file() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"lrclib:7\", \"youtube:aaaaaaaaaaa\"]\n\
         [[song]]\nsources = [\"youtube:bbbbbbbbbbb\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    assert_eq!(m.songs[0].id(), Some(&SourceKey::youtube("aaaaaaaaaaa")));
    m.edit(Edit::Add {
        sources: vec![
            SourceKey::youtube("bbbbbbbbbbb"),
            SourceKey::parse("lrclib:7").unwrap(),
        ],
        album: None,
    });
    m.edit(Edit::Tag {
        key: SourceKey::youtube("bbbbbbbbbbb"),
        tags: vec![("genre".into(), vec!["Folk".into()])],
    });
    m.save().unwrap();
    let m = Manifest::load(dir.path()).unwrap();
    assert_eq!(
        m.songs.len(),
        2,
        "the record joins the second song, not the first"
    );
    assert!(
        m.songs
            .iter()
            .all(|s| s.has(&SourceKey::parse("lrclib:7").unwrap()))
    );
    assert!(
        text(dir.path()).contains("version = 2"),
        "{}",
        text(dir.path())
    );
    assert!(m.songs[0].tags.is_empty() || m.songs[0].tags.iter().all(|(_, v)| v.is_empty()));

    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("listed by both"), "{e:#}");
}

#[test]
fn a_rename_and_a_drop_follow_the_key_into_its_pins() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:old.flac\", \"youtube:aaaaaaaaaaa\"]\naudio = \"manual:old.flac\"\ncover = \"youtube:aaaaaaaaaaa\"\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Rename {
        from: SourceKey::Manual("old.flac".into()),
        to: SourceKey::Manual("new.flac".into()),
    });
    m.edit(Edit::Drop(yt("aaaaaaaaaaa")));
    m.save().unwrap();
    let s = &m.songs[0];
    assert_eq!(s.sources, [SourceKey::Manual("new.flac".into())]);
    assert_eq!(s.audio, Some(SourceKey::Manual("new.flac".into())));
    assert_eq!(s.cover, None);
}

#[test]
fn an_album_is_added_once_with_its_template() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = Manifest::load(dir.path()).unwrap();
    let key = SourceKey::parse("youtubetab:OLAK5uy_abcdef").unwrap();
    for _ in 0..2 {
        m.edit(Edit::AddAlbum {
            source: key.clone(),
            tracks: Some(12),
        });
    }
    m.save().unwrap();
    assert_eq!(m.albums.len(), 1);
    assert_eq!(m.albums[0].tracks, Some(12));
    assert!(
        text(dir.path()).contains("[album.tags]\nalbum = \"\""),
        "{}",
        text(dir.path())
    );
}

#[test]
fn nothing_recorded_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    Manifest::load(dir.path()).unwrap().save().unwrap();
    assert!(!dir.path().join(MANIFEST).exists());
}

#[test]
fn a_user_s_leading_comment_stays_below_the_header() {
    assert_eq!(
        with_header("# mine\nversion = 1\n"),
        Some(format!("{HEADER}# mine\nversion = 1\n"))
    );
    assert_eq!(with_header(&format!("{HEADER}version = 1\n")), None);
}

#[test]
fn tags_set_replace_every_spelling_and_keep_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\ntags = { ARTIST = \"Old\", genre = \"Folk\" }\n[[song]]\nsources = [\"youtube:bbbbbbbbbbb\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    let artists = vec!["Marlo Venn".to_string(), "The Glass Orchards".to_string()];
    m.edit(Edit::Tag {
        key: yt("aaaaaaaaaaa"),
        tags: vec![
            ("artist".into(), artists.clone()),
            ("title".into(), vec!["Song".into()]),
        ],
    });
    m.save().unwrap();
    let tags =
        |n: usize| -> BTreeMap<String, Vec<String>> { m.songs[n].tags.iter().cloned().collect() };
    assert_eq!(tags(0)["artist"], artists);
    assert_eq!(tags(0)["title"], ["Song"]);
    assert_eq!(tags(0)["genre"], ["Folk"]);
    assert!(!tags(0).contains_key("ARTIST"));
    assert!(tags(1).is_empty(), "only the song listing the key");
    let t = text(dir.path());
    assert!(
        t.contains("artist = [\"Marlo Venn\", \"The Glass Orchards\"]"),
        "{t}"
    );
}

#[test]
fn every_cleaning_switch_is_listed_after_the_defaults_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "\n[defaults]\nlyrics = [\"en\"]\n\n[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n\n[clean.packaging]\nremaster = false # mine\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    assert!(!m.clean.is_on("packaging.remaster"));
    assert!(m.clean.is_on("packaging.album_version"));
    m.save().unwrap();
    let t = text(dir.path());
    let at = |s: &str| t.find(s).unwrap_or_else(|| panic!("{s} in {t}"));
    assert!(at("[defaults]") < at("[clean.tidy]"), "{t}");
    assert!(at("[clean.guesswork]") < at("\n[[song]]"), "{t}");
    assert!(t.contains("remaster = false # mine"), "{t}");
    assert!(t.contains("# Spacing and punctuation only"), "{t}");
    for (tier, key) in clean::switches() {
        assert!(t.contains(&format!("{key} = ")), "{tier}.{key} in {t}");
    }
    assert!(
        !Manifest::load(dir.path())
            .unwrap()
            .clean
            .is_on("packaging.remaster")
    );
}

#[test]
fn a_new_list_lists_its_cleaning_switches_before_its_songs() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Add {
        sources: vec![yt("aaaaaaaaaaa")],
        album: None,
    });
    m.save().unwrap();
    let t = text(dir.path());
    assert!(
        t.find("[clean.tidy]").unwrap() < t.find("\n[[song]]").unwrap(),
        "{t}"
    );
}

#[test]
fn a_cleaning_switch_must_be_true_or_false() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "[clean.tidy]\nspacing = \"no\"\n");
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("clean.tidy.spacing"), "{e:#}");
}

#[test]
fn an_unknown_cleaning_switch_is_kept_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[clean.tidy]\nspaceing = false\n\n[clean.other]\nx = true\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    assert_eq!(m.clean.unknown, ["clean.tidy.spaceing", "clean.other.x"]);
    assert!(m.clean.is_on("tidy.spacing"));
    m.save().unwrap();
    assert!(text(dir.path()).contains("spaceing = false"));
}

#[test]
fn a_removed_song_is_kept_whole_and_restored_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\", \"youtube:bbbbbbbbbbb\"]\naudio = \"youtube:bbbbbbbbbbb\"\n[song.tags]\ngenre = \"Pop\"\n[[song]]\nsources = [\"youtube:ccccccccccc\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Remove {
        key: yt("bbbbbbbbbbb"),
        note: "Song — Artist".into(),
    });
    m.save().unwrap();
    assert_eq!(m.songs.len(), 1);
    assert_eq!(m.removed.len(), 1);
    assert_eq!(Manifest::load(dir.path()).unwrap().removed.len(), 1);
    assert_eq!(m.removed[0].note, "Song — Artist");
    assert_eq!(m.removed_keys().len(), 2);
    assert!(text(dir.path()).contains("[[removed]]\nnote = \"Song — Artist\""));

    m.edit(Edit::Restore(yt("aaaaaaaaaaa")));
    m.save().unwrap();
    assert_eq!(m.removed, []);
    let song = &m.songs[1];
    assert_eq!(song.audio, Some(yt("bbbbbbbbbbb")));
    assert_eq!(song.tags, [("genre".to_string(), vec!["Pop".to_string()])]);
    assert!(!text(dir.path()).contains("note"));
}

#[test]
fn adding_a_removed_key_lifts_it_from_its_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[removed]]\nnote = \"x\"\nsources = [\"youtube:aaaaaaaaaaa\", \"youtube:bbbbbbbbbbb\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Add {
        sources: vec![yt("aaaaaaaaaaa")],
        album: None,
    });
    m.save().unwrap();
    assert_eq!(m.songs[0].sources, [yt("aaaaaaaaaaa")]);
    assert_eq!(m.removed[0].sources, [yt("bbbbbbbbbbb")]);
    m.edit(Edit::Add {
        sources: vec![yt("bbbbbbbbbbb")],
        album: None,
    });
    m.save().unwrap();
    assert!(m.removed.is_empty(), "an emptied tombstone goes");
    assert_eq!(Manifest::load(dir.path()).unwrap().removed, []);
    assert!(!text(dir.path()).contains("note = "));
}

#[test]
fn a_rewrite_finds_every_song_before_changing_any() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\", \"youtube:bbbbbbbbbbb\"]\n[song.tags]\ngenre = \"x\"\n[[song]]\nsources = [\"youtube:ccccccccccc\"]\n[song.tags]\ngenre = \"y\"\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    let table = |body: &str| {
        let doc: DocumentMut = format!("[[song]]\n{body}").parse().unwrap();
        SongTable(
            doc["song"]
                .as_array_of_tables()
                .unwrap()
                .get(0)
                .unwrap()
                .clone(),
        )
    };
    m.edit(Edit::Rewrite {
        songs: vec![
            (
                yt("ccccccccccc"),
                Rewritten::Table(table(
                    "sources = [\"youtube:ccccccccccc\", \"youtube:aaaaaaaaaaa\"]\n[song.tags]\ngenre = \"z\"",
                )),
            ),
            (yt("aaaaaaaaaaa"), Rewritten::Removed("gone".into())),
        ],
        new: vec![table("sources = [\"youtube:bbbbbbbbbbb\"]")],
    });
    m.save().unwrap();
    let lists: Vec<Vec<SourceKey>> = m.songs.iter().map(|s| s.sources.clone()).collect();
    assert_eq!(
        lists,
        [
            vec![yt("ccccccccccc"), yt("aaaaaaaaaaa")],
            vec![yt("bbbbbbbbbbb")]
        ]
    );
    assert_eq!(m.removed[0].note, "gone");
    let again = Manifest::load(dir.path()).unwrap();
    assert_eq!(again.songs.len(), 2, "{}", text(dir.path()));
    assert_eq!(
        again.songs[0].tags,
        [("genre".to_string(), vec!["z".to_string()])]
    );
}

#[test]
fn edits_are_not_saved_over_a_song_changed_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    let seen = m.songs.clone();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\"]\nlyrics_offset_ms = 5\n",
    );
    m.edit(Edit::Remove {
        key: yt("aaaaaaaaaaa"),
        note: "x".into(),
    });
    assert_eq!(m.save_if_unchanged(&seen).unwrap(), [yt("aaaaaaaaaaa")]);
    assert!(text(dir.path()).contains("lyrics_offset_ms = 5"));
    assert!(m.has_edits(), "the edits wait for another try");
    let now = Manifest::load(dir.path()).unwrap().songs;
    assert_eq!(m.save_if_unchanged(&now).unwrap(), []);
    assert!(m.songs.is_empty() && m.removed.len() == 1);
}

#[test]
fn the_new_song_list_lists_every_default_as_the_code_has_it() {
    // Uncommenting every setting must change nothing: the file is the
    // reference, and must not drift from the defaults it shows.
    let mut tables = false;
    let mut uncommented = String::new();
    for l in NEW.lines() {
        tables = (tables || l.starts_with("# [")) && (l.is_empty() || l.starts_with('#'));
        let l = if tables {
            l.strip_prefix("# ").unwrap_or(l)
        } else {
            l
        };
        uncommented.push_str(l);
        uncommented.push('\n');
    }
    let doc: toml_edit::DocumentMut = uncommented.parse().unwrap();
    for table in crate::settings::TABLES {
        assert!(doc.contains_key(table), "{table} is not shown");
    }
    assert_eq!(
        crate::settings::read(&doc).unwrap(),
        crate::settings::Settings::default()
    );
}
