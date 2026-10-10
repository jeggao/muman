//! `add` given lyrics, pictures and tag files: each matched to its songs.

use super::*;
use crate::ui::MockPrompter;

/// A FLAC's probe, named and as long as given.
fn flac(title: &str, artist: &str, album: &str, track: u32, seconds: u32, extra: &str) -> String {
    FLAC.replace(
        r#""duration": "200.0", "tags": {"TITLE": "Song", "ARTIST": "Artist", "ALBUM": "Record", "track": "2"}"#,
        &format!(
            r#""duration": "{seconds}.0", "tags": {{"TITLE": "{title}", "ARTIST": "{artist}", "ALBUM": "{album}", "track": "{track}"{extra}}}"#
        ),
    )
}

/// Three songs of one album and one of another, each its own recording.
fn library() -> Fake {
    Fake::default()
        .probe(
            "Lantern Weather.flac",
            &flac(
                "Lantern Weather",
                "Marlo Venn",
                "The Glass Orchards",
                1,
                221,
                "",
            ),
        )
        .probe(
            "Copper Moth.flac",
            &flac(
                "Copper Moth",
                "Marlo Venn",
                "The Glass Orchards",
                2,
                200,
                r#", "MUSICBRAINZ_TRACKID": "00000000-0000-4000-8000-000000000001""#,
            ),
        )
        .probe(
            "Lantern Weather (Live).flac",
            &flac(
                "Lantern Weather (Live)",
                "Marlo Venn",
                "Night Pier",
                1,
                260,
                "",
            ),
        )
        .probe(
            "Night Ferry.flac",
            &flac("Night Ferry", "Ada Fenn", "Harbour Lights", 3, 180, ""),
        )
}

impl Setup {
    fn songs_named(&self, fake: &Fake, names: &[&str]) {
        let files: Vec<String> = names
            .iter()
            .map(|n| self.file(&format!("rips/{n}.flac")))
            .collect();
        let mut args = vec!["add", "--new"];
        args.extend(files.iter().map(String::as_str));
        let (ok, text) = self.run(fake, &args);
        assert!(ok, "{text}");
    }

    fn asking(&self, fake: &Fake, args: &[&str], p: &mut MockPrompter) -> Result<(bool, String)> {
        let mut out = Vec::new();
        let ok = run_with(
            &self.job(args),
            fake,
            &Server::default(),
            Some(p),
            &mut out,
            &mut Vec::new(),
        )?;
        Ok((ok, crate::ui::plain(&String::from_utf8(out).unwrap())))
    }

    fn not_added(&self, fake: &Fake, args: &[&str]) -> String {
        let mut out = Vec::new();
        let e = run_with(
            &self.job(args),
            fake,
            &Server::default(),
            None,
            &mut out,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(e.downcast_ref::<change::Refused>().is_some(), "{e:#}");
        format!(
            "{e:#}\n{}",
            crate::ui::plain(&String::from_utf8(out).unwrap())
        )
    }

    fn song_with(&self, title_file: &str) -> crate::manifest::Song {
        Manifest::load(&self.dir.path().join("home"))
            .unwrap()
            .songs
            .into_iter()
            .find(|s| {
                s.has(&SourceKey::Manual(
                    format!("{title_file}.flac").as_str().into(),
                ))
            })
            .unwrap()
    }

    fn added(&self) -> Vec<String> {
        let dir = self.dir.path().join("home/sources/manual/added");
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|d| {
                d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

fn manual(rel: &str) -> SourceKey {
    SourceKey::Manual(rel.into())
}

#[test]
fn lyrics_named_as_a_song_s_file_go_to_it_and_into_the_library() {
    let s = Setup::new();
    let fake = library();
    s.songs_named(&fake, &["Lantern Weather", "Copper Moth"]);
    let lrc = s.written(
        "elsewhere/Lantern Weather.lrc",
        "[00:01.00]first light\n[01:00.00]second\n",
    );
    let (ok, text) = s.run(&fake, &["add", &lrc]);
    assert!(ok, "{text}");
    assert!(
        text.contains("the lyrics of Lantern Weather — Marlo Venn (named as its file"),
        "{text}"
    );
    assert!(
        s.song_with("Lantern Weather")
            .has(&manual("added/Lantern Weather.lrc"))
    );
    assert!(
        !s.song_with("Copper Moth")
            .has(&manual("added/Lantern Weather.lrc"))
    );
    assert_eq!(s.added(), ["Lantern Weather.lrc"]);
    assert!(
        crate::store::files_below(&s.dir.path().join("lib"), usize::MAX, |_| true)
            .unwrap()
            .iter()
            .any(|f| f.extension().is_some_and(|e| e == "lrc")),
        "the library has its lyrics"
    );
    let (_, again) = s.run(&fake, &["add", &lrc]);
    assert!(again.contains("already holds it"), "{again}");
    assert_eq!(s.added(), ["Lantern Weather.lrc"]);
}

#[test]
fn lyrics_tags_name_their_song_and_a_length_far_off_keeps_them_out() {
    let s = Setup::new();
    let fake = library();
    s.songs_named(&fake, &["Lantern Weather", "Copper Moth"]);
    let lrc = s.written(
        "dl/words.lrc",
        "[ti:Copper Moth]\n[ar:Marlo Venn]\n[length:03:20]\n[00:02.00]wings\n",
    );
    let (ok, text) = s.run(&fake, &["add", &lrc]);
    assert!(ok, "{text}");
    assert!(
        s.song_with("Copper Moth").has(&manual("added/words.lrc")),
        "{text}"
    );
    let far = s.written(
        "dl/far.lrc",
        "[ti:Copper Moth]\n[ar:Marlo Venn]\n[length:03:50]\n[00:02.00]wings\n",
    );
    let said = s.not_added(&fake, &["add", &far]);
    assert!(said.contains("1 file(s) were not added"), "{said}");
    assert!(said.contains("far.lrc matches no song"), "{said}");
    assert_eq!(s.added(), ["words.lrc"]);
}

#[test]
fn lyrics_that_may_be_two_songs_are_asked_about_and_searched_for() {
    let s = Setup::new();
    let fake = library();
    s.songs_named(
        &fake,
        &["Lantern Weather", "Lantern Weather (Live)", "Night Ferry"],
    );
    let lrc = s.written(
        "dl/words.lrc",
        "[ti:Lantern Weather]\n[ar:Marlo Venn]\n[00:02.00]rain\n",
    );
    let mut p = MockPrompter::new();
    p.push_select(Some(1));
    let (ok, text) = s.asking(&fake, &["add", &lrc], &mut p).unwrap();
    assert!(ok, "{text}");
    assert!(
        p.asked[0].starts_with("words.lrc: whose lyrics are these?"),
        "{:?}",
        p.asked
    );
    assert!(p.asked[0].contains("live version"), "{:?}", p.asked);
    assert!(
        s.song_with("Lantern Weather (Live)")
            .has(&manual("added/words.lrc"))
    );

    let other = s.written(
        "dl/other.lrc",
        "[ti:Lantern Weather]\n[ar:Marlo Venn]\n[00:03.00]sea\n",
    );
    let mut p = MockPrompter::new();
    p.push_select(Some(2))
        .push_text(Some("artist:fenn"))
        .push_select(Some(0));
    let (ok, _) = s.asking(&fake, &["add", &other], &mut p).unwrap();
    assert!(ok);
    assert!(p.asked[2].starts_with("1 song(s) match"), "{:?}", p.asked);
    assert!(s.song_with("Night Ferry").has(&manual("added/other.lrc")));

    let third = s.written(
        "dl/third.lrc",
        "[ti:Lantern Weather]\n[ar:Marlo Venn]\n[00:04.00]tide\n",
    );
    let mut p = MockPrompter::new();
    p.push_select(Some(3));
    let (ok, text) = s.asking(&fake, &["add", &third], &mut p).unwrap();
    assert!(ok, "declining is no failure");
    assert!(text.contains("third.lrc: not added"), "{text}");
    assert_eq!(s.added(), ["other.lrc", "words.lrc"]);
}

#[test]
fn yes_takes_a_song_that_leads_and_to_names_one() {
    let s = Setup::new();
    let fake = library();
    s.songs_named(&fake, &["Lantern Weather", "Copper Moth", "Night Ferry"]);
    let lrc = s.written("dl/moth.lrc", "[ti:Copper Moth]\n[00:02.00]wings\n");
    let said = s.not_added(&fake, &["add", &lrc]);
    assert!(said.contains("may be the lyrics of Copper Moth"), "{said}");
    let (ok, text) = s.run(&fake, &["add", "-y", &lrc]);
    assert!(
        ok && text.contains("likely the lyrics of Copper Moth"),
        "{text}"
    );

    let plain = s.written("dl/notes.txt", "a quiet harbour\nlights across the water\n");
    let (ok, text) = s.run(&fake, &["add", &plain, "--to", "ferry"]);
    assert!(ok, "{text}");
    assert!(s.song_with("Night Ferry").has(&manual("added/notes.lrc")));
    let said = s.not_added(&fake, &["add", &plain, "--to", "artist:venn"]);
    assert!(said.contains("2 songs match --to"), "{said}");
    let said = s.not_added(&fake, &["add", &plain, "--to", "nothing-like-it"]);
    assert!(said.contains("No song matches"), "{said}");
}

#[test]
fn a_picture_goes_to_every_song_whose_cover_it_looks_like() {
    let s = Setup::new();
    let mut fake = library();
    fake.looks = vec![
        ("Lantern Weather.flac".into(), 7),
        ("Copper Moth.flac".into(), 7),
        ("orchards.jpg".into(), 7),
    ];
    s.songs_named(&fake, &["Lantern Weather", "Copper Moth", "Night Ferry"]);
    let jpg = s.file("art/orchards.jpg");
    let (ok, text) = s.run(&fake, &["add", &jpg]);
    assert!(ok, "{text}");
    assert!(
        text.contains("alike to the covers of 2 songs of The Glass Orchards"),
        "{text}"
    );
    let cover = manual("added/orchards.jpg");
    assert!(s.song_with("Lantern Weather").has(&cover));
    assert!(s.song_with("Copper Moth").has(&cover));
    assert!(!s.song_with("Night Ferry").has(&cover));
    let list = std::fs::read_to_string(s.dir.path().join("home/songs.toml")).unwrap();
    assert!(list.contains("version = 3"), "{list}");

    let (ok, text) = s.run(&fake, &["remove", "-y", "manual:Copper Moth.flac"]);
    assert!(ok, "{text}");
    assert!(s.song_with("Lantern Weather").has(&cover));
    let (ok, text) = s.run(&fake, &["restore", "-y", "manual:Copper Moth.flac"]);
    assert!(ok, "{text}");
    assert!(s.song_with("Copper Moth").has(&cover));

    let other = s.file("art/unrelated.png");
    let said = s.not_added(&fake, &["add", &other]);
    assert!(said.contains("unrelated.png matches no song"), "{said}");
}

#[test]
fn a_folder_s_cover_goes_to_the_songs_given_from_it() {
    let s = Setup::new();
    let fake = library();
    let a = s.file("rips/A/Lantern Weather.flac");
    let b = s.file("rips/A/Copper Moth.flac");
    let cover = s.file("rips/A/cover.jpg");
    let (ok, text) = s.run(&fake, &["add", "--new", &a, &b, &cover]);
    assert!(ok, "{text}");
    let key = manual("added/cover.jpg");
    assert!(s.song_with("Lantern Weather").has(&key), "{text}");
    assert!(s.song_with("Copper Moth").has(&key), "{text}");
}

#[test]
fn a_tag_file_sets_hand_tags_but_none_that_describe_another_file() {
    let s = Setup::new();
    let fake = library();
    s.songs_named(&fake, &["Lantern Weather", "Copper Moth"]);
    let tags = s.written(
        "dl/tags.txt",
        "TITLE=Copper Moth\nARTIST=Marlo Venn\nTRACKNUMBER=2\nGENRE=Chamber Pop\nREPLAYGAIN_TRACK_GAIN=-3.10 dB\n",
    );
    let (ok, text) = s.run(&fake, &["add", &tags, "--genre", "Folk"]);
    assert!(ok, "{text}");
    let song = s.song_with("Copper Moth");
    let genre: Vec<&Vec<String>> = song
        .tags
        .iter()
        .filter(|(k, _)| k == "genre")
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        genre,
        [&vec!["Folk".to_string()]],
        "the command line wins: {:?}",
        song.tags
    );
    assert!(
        !song.tags.iter().any(|(k, _)| k.contains("replaygain")),
        "{:?}",
        song.tags
    );
    assert!(s.added().is_empty(), "a tag file is not kept");

    let meta = s.written(
        "dl/song.ffmeta",
        ";FFMETADATA1\ntitle=Lantern Weather\nartist=Marlo Venn\ntrack=1\ndate=2011\n",
    );
    let (ok, _) = s.run(&fake, &["add", &meta]);
    assert!(ok);
    assert!(
        s.song_with("Lantern Weather")
            .tags
            .iter()
            .any(|(k, v)| k == "date" && v == &["2011"])
    );

    let json = s.written("dl/rec.json", r#"{"title": "Some Other Name", "mb_trackid": "00000000-0000-4000-8000-000000000001", "mood": "x"}"#);
    let (ok, text) = s.run(&fake, &["add", &json]);
    assert!(ok, "{text}");
    assert!(text.contains("named by its ID"), "{text}");
}

#[test]
fn a_cue_sheet_tags_each_song_its_tracks_are() {
    let s = Setup::new();
    let fake = library();
    s.songs_named(&fake, &["Lantern Weather", "Copper Moth", "Night Ferry"]);
    let cue = s.written(
        "dl/album.cue",
        "REM GENRE \"Chamber Pop\"\nPERFORMER \"Marlo Venn\"\nTITLE \"The Glass Orchards\"\n\
         FILE \"Lantern Weather.flac\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"Lantern Weather\"\n    INDEX 01 00:00:00\n\
         FILE \"Copper Moth.flac\" WAVE\n  TRACK 02 AUDIO\n    TITLE \"Copper Moth\"\n    INDEX 01 00:00:00\n",
    );
    let (ok, text) = s.run(&fake, &["add", &cue]);
    assert!(ok, "{text}");
    for song in ["Lantern Weather", "Copper Moth"] {
        let tags = s.song_with(song).tags;
        assert!(
            tags.iter()
                .any(|(k, v)| k == "genre" && v == &["Chamber Pop"]),
            "{song}: {tags:?}"
        );
    }
    assert!(
        !s.song_with("Night Ferry")
            .tags
            .iter()
            .any(|(k, v)| k == "genre" && !v.is_empty())
    );
}

#[test]
fn a_song_file_and_its_lyrics_given_together_are_copied_once() {
    let s = Setup::new();
    let fake = library();
    let song = s.file("rips/Lantern Weather.flac");
    let lrc = s.written("rips/Lantern Weather.lrc", "[00:01.00]first light\n");
    let (ok, text) = s.run(&fake, &["add", &song, &lrc]);
    assert!(ok, "{text}");
    assert!(s.added().is_empty());
    assert!(
        s.dir
            .path()
            .join("home/sources/manual/Lantern Weather.lrc")
            .exists()
    );
}

#[test]
fn a_mistyped_loose_file_is_no_file() {
    let s = Setup::new();
    let e = run_with(
        &s.job(&["add", "dl/missing.cue"]),
        &library(),
        &Server::default(),
        None,
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("is no file"), "{e:#}");
}

#[test]
fn subtitles_are_given_as_the_lrc_they_convert_to() {
    let s = Setup::new();
    let fake = library();
    s.songs_named(&fake, &["Copper Moth"]);
    let srt = s.written(
        "dl/Copper Moth.srt",
        "1\n00:00:01,000 --> 00:00:02,000\nwings\n",
    );
    let (ok, text) = s.run(&fake, &["add", &srt]);
    assert!(ok, "{text}");
    assert!(
        s.song_with("Copper Moth")
            .has(&manual("added/Copper Moth.lrc")),
        "{text}"
    );
    let copied = std::fs::read_to_string(
        s.dir
            .path()
            .join("home/sources/manual/added/Copper Moth.lrc"),
    )
    .unwrap();
    assert!(copied.starts_with("[00:01.00]"), "{copied}");
}
