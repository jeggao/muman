use super::*;

fn cand(title: &str, artist: &str, album: &str, seconds: i64) -> Candidate {
    let view = View {
        keys: vec![SourceKey::Manual(format!("{title}.flac").as_str().into())],
        tags: vec![
            ("TITLE".into(), vec![title.into()]),
            ("ARTIST".into(), vec![artist.into()]),
            ("ALBUM".into(), vec![album.into()]),
        ],
        ..View::default()
    };
    Candidate {
        id: view.keys[0].clone(),
        label: view.name(),
        album: album.into(),
        album_key: similar::fold(album),
        titles: vec![Name::new(title)],
        artists: vec![Name::new(artist)],
        albums: vec![Name::new(album)],
        track: None,
        length_ms: Some(seconds * 1000),
        stems: [similar::fold(title)].into(),
        ids: BTreeSet::new(),
        looks: Vec::new(),
        digests: BTreeSet::new(),
        lyrics: Vec::new(),
        streams: Vec::new(),
        view,
    }
}

fn said(title: &str, artist: Option<&str>) -> Clues {
    Clues {
        readings: vec![Fields {
            title: Some(Name::new(title)),
            artist: artist.map(Name::new),
            ..Fields::default()
        }],
        grace: (GRACE_MS, SPAN_MS),
        ..Clues::default()
    }
}

fn top(cands: &[Candidate], clues: &Clues, what: What) -> (usize, Verdict, Vec<String>) {
    let j = judge(cands, clues, what).remove(0);
    (j.song, j.verdict, j.reasons)
}

#[test]
fn a_title_and_artist_decide_but_a_title_alone_or_a_tie_does_not() {
    let cands = [
        cand("Lantern Weather", "Marlo Venn", "The Glass Orchards", 221),
        cand("Copper Moth", "Marlo Venn", "The Glass Orchards", 200),
    ];
    assert_eq!(
        top(
            &cands,
            &said("Copper Moth", Some("Marlo Venn")),
            What::Lyrics
        )
        .1,
        Verdict::Same
    );
    let (song, verdict, _) = top(&cands, &said("Copper Moth", None), What::Lyrics);
    assert_eq!(
        (song, verdict),
        (1, Verdict::Unsure),
        "a title weighs too little alone"
    );
    let twins = [
        cand("Copper Moth", "Marlo Venn", "The Glass Orchards", 200),
        cand("Copper Moth", "Marlo Venn", "Night Pier", 230),
    ];
    let (song, verdict, reasons) = top(
        &twins,
        &said("Copper Moth", Some("Marlo Venn")),
        What::Lyrics,
    );
    assert_eq!((song, verdict), (0, Verdict::Unsure), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.starts_with("as near as")));
}

#[test]
fn another_version_is_only_maybe_and_says_so() {
    let cands = [cand(
        "Lantern Weather (Live)",
        "Marlo Venn",
        "Night Pier",
        260,
    )];
    let (_, verdict, reasons) = top(
        &cands,
        &said("Lantern Weather", Some("Marlo Venn")),
        What::Lyrics,
    );
    assert_eq!(verdict, Verdict::Unsure);
    assert!(reasons.contains(&"live version".to_string()), "{reasons:?}");
}

#[test]
fn lyrics_that_outlast_a_song_or_state_another_length_are_not_its() {
    let cands = [cand("Copper Moth", "Marlo Venn", "The Glass Orchards", 200)];
    let mut clues = said("Copper Moth", Some("Marlo Venn"));
    clues.last_ms = Some(205_000);
    let (_, verdict, reasons) = top(&cands, &clues, What::Lyrics);
    assert_eq!(verdict, Verdict::Different);
    assert_eq!(reasons, ["its last line is at 3:25, past the song's 3:20"]);
    clues.last_ms = Some(190_000);
    clues.stated_ms = Some(230_000);
    let (_, verdict, reasons) = top(&cands, &clues, What::Lyrics);
    assert_eq!(verdict, Verdict::Different, "{reasons:?}");
    clues.stated_ms = Some(201_000);
    clues.length_ms = clues.stated_ms;
    let (_, verdict, reasons) = top(&cands, &clues, What::Lyrics);
    assert_eq!(verdict, Verdict::Same);
    assert!(reasons.contains(&"length agrees".to_string()));
}

#[test]
fn words_sung_alike_decide_and_words_unlike_hold_a_name_back() {
    let dir = tempfile::tempdir().unwrap();
    let theirs = dir.path().join("theirs.lrc");
    let verse = "copper moth upon the glass\nwings of rust and amber light\n\
                 circle round the lantern twice\nfold your shadow into night\n\
                 every window holds a flame\nevery flame forgets its name\n";
    std::fs::write(&theirs, verse).unwrap();
    let mut cands = [cand("Copper Moth", "Marlo Venn", "The Glass Orchards", 200)];
    cands[0].lyrics = vec![theirs];
    let mut clues = Clues {
        grace: (GRACE_MS, SPAN_MS),
        ..Clues::default()
    };
    let words = |text: &str| {
        Some((
            similar::shingles(&similar::tokens(&lyrics::sung(text)), 3),
            similar::script_of(text),
        ))
    };
    clues.words = words(&verse.replace('\n', " \n"));
    assert_eq!(top(&cands, &clues, What::Lyrics).1, Verdict::Same);
    let mut named = said("Copper Moth", Some("Marlo Venn"));
    named.words = words(
        "harbour bells at early tide\nnets are mended on the pier\nsalt and cedar on the wind\n\
         ferries leaving, ferries near\nall the gulls have gone to sleep\nall the anchors settle deep\n",
    );
    let (_, verdict, reasons) = top(&cands, &named, What::Lyrics);
    assert_eq!(verdict, Verdict::Unsure, "{reasons:?}");
    assert!(reasons.contains(&"its words are not the song's".to_string()));
}

fn asking(cands: &[Candidate]) -> (Vec<u8>, BTreeSet<String>) {
    let _ = cands;
    (Vec::new(), BTreeSet::new())
}

fn picture_item(look: Look) -> Item {
    Item {
        path: PathBuf::from("art/cover.jpg"),
        label: "cover.jpg".into(),
        what: What::Picture,
        bytes: None,
        clues: Clues {
            look: Some(look),
            grace: (GRACE_MS, SPAN_MS),
            ..Clues::default()
        },
        sheet: None,
    }
}

#[test]
fn a_picture_alike_to_many_songs_of_many_albums_is_asked_about() {
    let look = Look {
        content: picture::Hashes {
            p: 0x0123_4567_89ab_cdef,
            d: 0xfedc_ba98_7654_3210,
        },
        square: None,
        flat: false,
    };
    let mut cands: Vec<Candidate> = (0..4)
        .map(|n| {
            cand(
                &format!("Upload {n}"),
                "Channel Seven",
                &format!("Album {n}"),
                100,
            )
        })
        .collect();
    for c in &mut cands {
        c.looks = vec![look];
    }
    let (mut out, extractors) = asking(&cands);
    let mut ask = Asking {
        cands: &cands,
        extractors: &extractors,
        yes: false,
        prompter: None,
        out: &mut out,
    };
    let decided = decide(&mut ask, &picture_item(look), None).unwrap();
    assert!(matches!(decided, Decision::LeftOut));
    for c in &mut cands {
        c.album = "One Album".into();
        c.album_key = similar::fold("One Album");
    }
    let mut ask = Asking {
        cands: &cands,
        extractors: &extractors,
        yes: false,
        prompter: None,
        out: &mut out,
    };
    let decided = decide(&mut ask, &picture_item(look), None).unwrap();
    assert!(matches!(decided, Decision::Listed(ref songs) if songs.len() == 4));
    let mut p = crate::ui::MockPrompter::new();
    p.push_choice([1]);
    for c in &mut cands[2..] {
        c.album = "Another".into();
        c.album_key = similar::fold("Another");
    }
    let mut ask = Asking {
        cands: &cands,
        extractors: &extractors,
        yes: false,
        prompter: Some(&mut p),
        out: &mut out,
    };
    let decided = decide(&mut ask, &picture_item(look), None).unwrap();
    assert!(matches!(decided, Decision::Listed(ref songs) if songs == &[1]));
}

#[test]
fn a_file_copied_in_twice_is_one_file_and_a_name_taken_gets_another() {
    let dir = tempfile::tempdir().unwrap();
    let manual = dir.path().join("manual");
    let from = dir.path().join("words.txt");
    std::fs::write(&from, "first light\n").unwrap();
    let item = |path: &Path| Item {
        path: path.to_path_buf(),
        label: "words.txt".into(),
        what: What::Lyrics,
        bytes: None,
        clues: Clues::default(),
        sheet: None,
    };
    let mut none = BTreeSet::new();
    let key = copy_in(&manual, &item(&from), &mut none).unwrap();
    assert_eq!(key, SourceKey::Manual("added/words.lrc".into()));
    assert_eq!(
        copy_in(&manual, &item(&from), &mut BTreeSet::new()).unwrap(),
        key
    );
    let mut taken = BTreeSet::from([key.clone()]);
    assert_eq!(
        copy_in(&manual, &item(&from), &mut taken).unwrap(),
        SourceKey::Manual("added/words (2).lrc".into()),
        "another song's own copy is not given to a second song"
    );
    std::fs::write(&from, "second light\n").unwrap();
    assert_eq!(
        copy_in(&manual, &item(&from), &mut BTreeSet::new()).unwrap(),
        SourceKey::Manual("added/words (3).lrc".into())
    );
    assert_eq!(song_list_name("ALBUMARTIST"), "album_artist");
    assert_eq!(song_list_name("MOOD"), "mood");
}

#[test]
fn files_are_sorted_by_what_they_hold() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    };
    assert_eq!(
        sort(&write("a.lrc", "[00:01.00]x\n")).unwrap(),
        Some(What::Lyrics)
    );
    assert_eq!(
        sort(&write("a.txt", "plain words\n")).unwrap(),
        Some(What::Lyrics)
    );
    assert_eq!(
        sort(&write("b.txt", "TITLE=Copper Moth\nARTIST=Marlo Venn\n")).unwrap(),
        Some(What::Tags)
    );
    assert_eq!(
        sort(&write("c.cue", "TRACK 01 AUDIO\n")).unwrap(),
        Some(What::Tags)
    );
    assert_eq!(sort(&write("d.png", "png")).unwrap(), Some(What::Picture));
    assert_eq!(sort(&write("e.flac", "flac")).unwrap(), None);
    assert!(sort(&write("f.docx", "doc")).is_err());
    assert_eq!(sort(dir.path()).unwrap(), None);
}

#[test]
fn the_likeliest_songs_lyrics_are_read_from_their_streams() {
    let dir = tempfile::tempdir().unwrap();
    let verse = "[00:01.00]copper moth upon the glass\n[00:05.00]wings of rust and amber light\n\
                 [00:09.00]circle round the lantern twice\n[00:13.00]fold your shadow into night\n";
    let fake = crate::testing::Fake {
        lrc: Some(verse.into()),
        ..crate::testing::Fake::default()
    };
    let mut cands = vec![cand("Moth", "Marlo Venn", "The Glass Orchards", 200)];
    cands[0].streams = vec![(dir.path().join("Moth [aaaaaaaaaaa].mkv"), 2)];
    let item = Item {
        path: PathBuf::from("words.lrc"),
        label: "words.lrc".into(),
        what: What::Lyrics,
        bytes: None,
        clues: Clues {
            words: Some((
                similar::shingles(&similar::tokens(&lyrics::sung(verse)), 3),
                similar::script_of(verse),
            )),
            grace: (GRACE_MS, SPAN_MS),
            ..Clues::default()
        },
        sheet: None,
    };
    read_streams(&fake, &mut cands, std::slice::from_ref(&item), dir.path());
    assert_eq!(cands[0].lyrics.len(), 1);
    assert!(fake.ran("lrc"));
    let (_, verdict, reasons) = top(&cands, &item.clues, What::Lyrics);
    assert_eq!(verdict, Verdict::Same, "{reasons:?}");
    assert!(reasons.contains(&"its words are the song's".to_string()));
}
