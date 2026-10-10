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

const TEMPLATE: &str = "tags.title = \"\"\ntags.artist = \"\"\ntags.album = \"\"\ntags.album_artist = \"\"\ntags.genre = \"\"\ntags.date = \"\"\n";

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
            "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"youtube.com:bbbbbbbbbbb\"]\n{TEMPLATE}"
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
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"] # mine\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    let album = SourceKey::parse("youtube.com:playlist/OLAK5uy_abcdef").unwrap();
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
        "# mine\nversion = 1\nfuture = true\n\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"] # kept\nrating = 5\n",
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
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"manual:a.flac\"]\naudio = \"manual:a.flac\"\nlyrics = false\nlyrics_offset_ms = -120\ntags = { title = \"T\", track = 3, artist = [\"A\", \"B\"], odd = true, empty = \"\" }\n",
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
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ncover = \"manual:x.png\"\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("pins manual:x.png"), "{e:#}");
}

#[test]
fn a_source_listed_twice_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("both song 1 and song 2"), "{e:#}");
}

#[test]
fn a_bad_key_is_refused_rather_than_dropping_its_song() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "[[song]]\nsources = [\"youtube:short\"]\n");
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("[sites]"), "{e:#}");
}

#[test]
fn a_site_the_song_list_adds_names_its_keys() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[sites.\"tunes.example\"]\nextractors = { tunes = \"\" }\n\
         kinds = [{ id = \"^[0-9]+$\", fetch = \"https://tunes.example/t/{id}\" }]\n\
         [[song]]\nsources = [\"tunes.example:42\"]\n",
    );
    let m = Manifest::load(dir.path()).unwrap();
    let key = SourceKey::parse("tunes.example:42").unwrap();
    assert_eq!(
        m.settings.sites.fetch_url(&key).as_deref(),
        Some("https://tunes.example/t/42")
    );
    assert!(
        m.settings.sites.fetch_url(&yt("vid00000001")).is_some(),
        "the defaults stay"
    );
    write(
        dir.path(),
        "[sites.\"tunes.example\"]\nkinds = [{ id = \"^[0-9]+$\" }]\n\
        [[song]]\nsources = [\"tunes.example:x\"]\n",
    );
    assert!(Manifest::load(dir.path()).is_err());
}

#[test]
fn another_version_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MANIFEST), "version = 4\n").unwrap();
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
        "[[song]]\nsources = [\"lrclib:7\", \"youtube.com:aaaaaaaaaaa\"]\n\
         [[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\n",
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
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("listed by both"), "{e:#}");
}

#[test]
fn songs_share_a_picture_of_the_users_but_not_its_lyrics_in_version_3() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:added/cover.jpg\", \"manual:a.flac\"]\n\
         [[song]]\nsources = [\"manual:b.flac\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    assert_eq!(m.songs[0].id(), Some(&SourceKey::Manual("a.flac".into())));
    m.edit(Edit::Add {
        sources: vec![
            SourceKey::Manual("b.flac".into()),
            SourceKey::Manual("added/cover.jpg".into()),
        ],
        album: None,
    });
    m.save().unwrap();
    let m = Manifest::load(dir.path()).unwrap();
    assert_eq!(m.songs.len(), 2);
    assert!(
        m.songs
            .iter()
            .all(|s| s.has(&SourceKey::Manual("added/cover.jpg".into())))
    );
    assert!(
        text(dir.path()).contains("version = 3"),
        "{}",
        text(dir.path())
    );

    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:a.flac\", \"manual:added/a.lrc\"]\n\
         [[song]]\nsources = [\"manual:b.flac\", \"manual:added/a.lrc\"]\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("listed by both"), "{e:#}");
}

#[test]
fn a_song_with_no_source_of_its_own_is_refused_and_never_made() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(MANIFEST),
        "version = 3\n[[song]]\nsources = [\"manual:a.flac\", \"manual:added/c.jpg\"]\n\
         [[song]]\nsources = [\"manual:added/c.jpg\"]\n",
    )
    .unwrap();
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(
        format!("{e:#}").contains("song 2 lists no source of its own"),
        "{e:#}"
    );

    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:b.flac\", \"manual:added/c.jpg\"]\n\
         [[removed]]\nnote = \"gone\"\nsources = [\"manual:a.flac\", \"manual:added/c.jpg\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Add {
        sources: vec![SourceKey::Manual("a.flac".into())],
        album: None,
    });
    m.save().unwrap();
    let m = Manifest::load(dir.path()).unwrap();
    assert!(
        m.removed.is_empty(),
        "a tombstone left with the picture alone goes"
    );
    assert_eq!(m.songs.len(), 2);
}

#[test]
fn a_picture_one_song_lists_twice_is_shared_with_no_one() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:a.flac\", \"manual:added/c.jpg\", \"manual:added/c.jpg\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Tag {
        key: SourceKey::Manual("a.flac".into()),
        tags: vec![("genre".into(), vec!["Folk".into()])],
    });
    m.save().unwrap();
    assert!(
        text(dir.path()).contains("version = 1"),
        "{}",
        text(dir.path())
    );
}

#[test]
fn a_rename_and_a_drop_follow_the_key_into_its_pins() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:old.flac\", \"youtube.com:aaaaaaaaaaa\"]\naudio = \"manual:old.flac\"\ncover = \"youtube.com:aaaaaaaaaaa\"\n",
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
    let key = SourceKey::parse("youtube.com:playlist/OLAK5uy_abcdef").unwrap();
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
        text(dir.path()).contains("tags.album = \"\""),
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
    let once = with_header("# mine\nversion = 1\n").unwrap();
    assert_eq!(with_header(&once), None, "a second save keeps it too");
    let under = format!("{HEADER}# right under it\nversion = 1\n");
    assert_eq!(with_header(&under), None);
    let older = "# muman's song list: an older header.\n\
                 # Other keys muman does not know are kept.\n# mine\nversion = 1\n";
    assert_eq!(
        with_header(older),
        Some(format!("{HEADER}# mine\nversion = 1\n"))
    );
}

#[test]
fn tags_set_replace_every_spelling_and_keep_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\ntags = { ARTIST = \"Old\", genre = \"Folk\" }\n[[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\n",
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
        "\n[defaults]\nlyrics = [\"en\"]\n\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n\n[clean.packaging]\nremaster = false # mine\n",
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
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"youtube.com:bbbbbbbbbbb\"]\naudio = \"youtube.com:bbbbbbbbbbb\"\n[song.tags]\ngenre = \"Pop\"\n[[song]]\nsources = [\"youtube.com:ccccccccccc\"]\n",
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
        "[[removed]]\nnote = \"x\"\nsources = [\"youtube.com:aaaaaaaaaaa\", \"youtube.com:bbbbbbbbbbb\"]\n",
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
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\", \"youtube.com:bbbbbbbbbbb\"]\n[song.tags]\ngenre = \"x\"\n[[song]]\nsources = [\"youtube.com:ccccccccccc\"]\n[song.tags]\ngenre = \"y\"\n",
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
                    "sources = [\"youtube.com:ccccccccccc\", \"youtube.com:aaaaaaaaaaa\"]\n[song.tags]\ngenre = \"z\"",
                )),
            ),
            (yt("aaaaaaaaaaa"), Rewritten::Removed("gone".into())),
        ],
        new: vec![table("sources = [\"youtube.com:bbbbbbbbbbb\"]")],
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
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    let seen = m.songs.clone();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\nlyrics_offset_ms = 5\n",
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
fn the_new_song_list_sets_every_default_as_the_code_has_it() {
    // The file is the reference the docs point to, and the defaults an
    // older list is filled from: it must not drift from the code.
    let mut doc: DocumentMut = NEW.parse().unwrap();
    for table in crate::settings::TABLES {
        assert!(doc.contains_key(table), "{table} is not shown");
    }
    assert_eq!(
        crate::settings::read(&doc).unwrap(),
        crate::settings::Settings::default()
    );
    assert_eq!(
        doc.get("edition").and_then(Item::as_integer),
        Some(crate::settings::EDITION)
    );
    assert!(!crate::settings::fill(&mut doc, &crate::settings::EDITIONS));
}

#[test]
fn tags_tables_are_written_back_as_dotted_keys_with_their_comments() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "\n[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n\n# Checked by ear.\n[song.tags]\n\
         title = \"Paper Comets\" # as sung\nartist = \"Marlo Venn\"\n\n\
         [[song]]\nsources = [\"youtube.com:bbbbbbbbbbb\"]\ntags = { genre = \"Folk\" }\n\n\
         [[album]]\nsource = \"youtube.com:playlist/OLAK5uy_abcdef\"\n\n\
         [album.tags]\nalbum = \"Lantern Weather\"\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    let before = (m.songs.clone(), m.albums.clone());
    m.save().unwrap();
    let t = text(dir.path());
    assert!(
        t.contains(
            "[[song]]\nsources = [\"youtube.com:aaaaaaaaaaa\"]\n# Checked by ear.\n\
             tags.title = \"Paper Comets\" # as sung\ntags.artist = \"Marlo Venn\"\n\
             tags.album = \"\"\n"
        ),
        "{t}"
    );
    assert!(t.contains("tags.genre = \"Folk\"\n"), "{t}");
    assert!(t.contains("\ntags.album = \"Lantern Weather\"\n"), "{t}");
    assert!(
        !t.contains("[song.tags]") && !t.contains("[album.tags]"),
        "{t}"
    );
    let again = Manifest::load(dir.path()).unwrap();
    assert_eq!((again.songs, again.albums), before);
    Manifest::load(dir.path()).unwrap().save().unwrap();
    assert_eq!(text(dir.path()), t);
}

#[test]
fn a_key_named_by_its_extractor_is_written_by_its_site() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"youtube:aaaaaaaaaaa\", \"archiveorg:item0001\"] # mine\n\
         album = \"youtubetab:OLAK5uy_abcdef\"\naudio = \"archiveorg:item0001\"\n\
         held.\"youtube:aaaaaaaaaaa\" = { audio = \"0123456789abcdef\" }\n\n\
         [[album]]\nsource = \"youtubetab:OLAK5uy_abcdef\"\n\n\
         [[removed]]\nnote = \"Gone\"\nsources = [\"youtube:rrrrrrrrrrr\"]\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    assert_eq!(m.respelled.len(), 4);
    assert_eq!(
        m.respelled_notice().unwrap(),
        "4 source key(s) by their site, as archiveorg:item0001 → archive.org:item0001"
    );
    let archived = SourceKey::parse("archive.org:item0001").unwrap();
    assert_eq!(m.songs[0].sources, [yt("aaaaaaaaaaa"), archived.clone()]);
    assert_eq!(m.songs[0].audio.as_ref(), Some(&archived));
    assert!(m.songs[0].held.contains_key(&yt("aaaaaaaaaaa")));
    m.edit(Edit::Tag {
        key: archived,
        tags: vec![("genre".to_string(), vec!["Folk".to_string()])],
    });
    m.save().unwrap();
    let t = text(dir.path());
    assert!(
        t.contains(
            "sources = [\"youtube.com:aaaaaaaaaaa\", \"archive.org:item0001\"] # mine\n\
             album = \"youtube.com:playlist/OLAK5uy_abcdef\"\naudio = \"archive.org:item0001\"\n\
             held.\"youtube.com:aaaaaaaaaaa\" = { audio = \"0123456789abcdef\" }\n"
        ),
        "{t}"
    );
    assert!(t.contains("tags.genre = \"Folk\""), "an edit finds it: {t}");
    assert!(
        t.contains("source = \"youtube.com:playlist/OLAK5uy_abcdef\""),
        "{t}"
    );
    assert!(t.contains("sources = [\"youtube.com:rrrrrrrrrrr\"]"), "{t}");
    assert!(Manifest::load(dir.path()).unwrap().respelled.is_empty());
}

#[test]
fn a_song_with_no_list_of_sources_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    for (songs, said) in [
        (
            "[[song]]\nsources = \"manual:a.flac\"\n",
            "sources must be a list",
        ),
        ("[[song]]\nsources = []\n", "song 1 lists no source"),
        (
            "[[song]]\ntags.title = \"Lantern Weather\"\n",
            "song 1 lists no source",
        ),
    ] {
        write(dir.path(), songs);
        let e = Manifest::load(dir.path()).unwrap_err();
        assert!(format!("{e:#}").contains(said), "{songs}: {e:#}");
    }
}

#[test]
fn restoring_a_song_whose_sources_are_listed_again_lists_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:a.flac\"]\n\n[[removed]]\nsources = [\"manual:a.flac\"]\nnote = \"gone\"\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    m.edit(Edit::Restore(SourceKey::Manual("a.flac".into())));
    m.save().unwrap();
    let m = Manifest::load(dir.path()).unwrap();
    assert_eq!(m.songs.len(), 1);
    assert_eq!(m.removed.len(), 1);
}

#[test]
fn a_record_of_what_a_source_held_goes_when_the_song_lists_it_no_more() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:a.flac\"]\nheld.\"manual:a.flac\" = { audio = \"0000000000000001\" }\nheld.\"manual:b.flac\" = { audio = \"0000000000000002\" }\n",
    );
    let mut m = Manifest::load(dir.path()).unwrap();
    m.save().unwrap();
    let t = text(dir.path());
    assert!(t.contains("held.\"manual:a.flac\""), "{t}");
    assert!(!t.contains("manual:b.flac"), "{t}");
}

#[test]
fn lyrics_moved_more_than_an_hour_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "[[song]]\nsources = [\"manual:a.flac\"]\nlyrics_offset = \"2 h\"\n",
    );
    let e = Manifest::load(dir.path()).unwrap_err();
    assert!(format!("{e:#}").contains("more than an hour"), "{e:#}");
}
