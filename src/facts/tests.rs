use super::*;
use crate::source::SourceKey;
use crate::testing::{FLAC, Fake};

fn located(dir: &Path, rel: &str) -> Located {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"x").unwrap();
    Located {
        key: SourceKey::Manual(rel.into()),
        path,
        kind: Kind::Media,
        lyrics: None,
        covers: Vec::new(),
    }
}

#[test]
fn an_original_s_facts_take_three_runs() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let fake = Fake::default();
    let facts = gather(&fake, &source, &dir.path().join("scratch")).unwrap();
    assert_eq!(fake.calls().len(), 3, "{:#?}", fake.calls());
    let audio = facts.audio.unwrap();
    assert_eq!((audio.index, audio.codec.as_str()), (1, "opus"));
    assert!(audio.quality.unwrap().bandwidth_hz > 20_000.0);
    assert_eq!(facts.print.unwrap().words().len(), 1600);
    assert_eq!(facts.covers.len(), 1);
    assert_eq!(facts.covers[0].at, CoverAt::Attachment { ordinal: 1 });
    assert_eq!(facts.covers[0].quality.unwrap().width, 64);
    assert_eq!(facts.lyrics.len(), 1);
    assert_eq!(facts.lyrics[0].at, LyricsAt::Stream { index: 2 });
    assert_eq!(facts.lyrics[0].language, Language::Codes(vec!["en".into()]));
    assert_eq!(facts.lyrics[0].timing.unwrap().lines, 2);
    assert_eq!(facts.tags[&tags::Field::Artist].values, ["Hoshi7ne"]);
    assert_eq!(facts.duration, Some(200.0));
    assert_eq!(audio.bytes, Some(3_200_000), "listed in the same run");
}

#[test]
fn packet_sizes_add_up_and_an_old_source_is_listed_alone() {
    let text = "#tb 0: 1/48000\n0, -312, -312, 960, 307, 0xd64e9e1c, S=1, Skip Samples, 10, 0x1\n\
                0, 648, 648, 960, 168, 0x8757553d\n";
    assert_eq!(packet_bytes(text), Some(475));
    assert_eq!(packet_bytes("#tb 0: 1/48000\n"), None);
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.flac");
    let fake = Fake {
        packets: vec![("Song.flac".into(), 31_000_000)],
        ..Fake::default()
    };
    let bytes = audio_bytes(&fake, &source, 0, &dir.path().join("scratch")).unwrap();
    assert_eq!(bytes, 31_000_000);
    assert_eq!(fake.calls().len(), 1);
    assert!(fake.ran("framecrc") && !fake.ran("pcm_s16le"));
}

#[test]
fn a_manual_file_brings_its_tags_cover_and_lrc() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = located(dir.path(), "A/Song.flac");
    let lrc = dir.path().join("A/Song.lrc");
    std::fs::write(&lrc, "[00:05.00]sung\n[00:06.00]♪\n").unwrap();
    let cover = dir.path().join("A/cover.png");
    std::fs::write(&cover, b"png").unwrap();
    source.lyrics = Some(lrc);
    source.covers = vec![cover];
    let fake = Fake::default().probe("Song.flac", FLAC);
    let facts = gather(&fake, &source, &dir.path().join("scratch")).unwrap();
    assert_eq!(
        fake.calls().len(),
        3,
        "no attachments to dump, the cover alone"
    );
    assert_eq!(facts.tags[&tags::Field::Album].values, ["Record"]);
    assert_eq!(facts.tags[&tags::Field::Track].values, ["2"]);
    let at: Vec<&CoverAt> = facts.covers.iter().map(|c| &c.at).collect();
    assert_eq!(at, [&CoverAt::Picture { index: 1 }, &CoverAt::Sidecar(0)]);
    assert_eq!(facts.covers[1].mimetype, "image/png");
    assert_eq!(facts.lyrics.len(), 1);
    assert_eq!(facts.lyrics[0].at, LyricsAt::Sidecar);
    assert_eq!(
        facts.lyrics[0].timing.unwrap().lines,
        1,
        "the cue is no lyric"
    );
    assert!(facts.audio.unwrap().is_lossless());
}

#[test]
fn a_failed_measure_costs_only_itself() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let fake = Fake {
        failing: vec!["lrc".into()],
        ..Fake::default()
    };
    let facts = gather(&fake, &source, &dir.path().join("scratch")).unwrap();
    assert_eq!(facts.lyrics, []);
    assert!(facts.print.is_some() && facts.audio.unwrap().quality.is_some());
    assert!(fake.calls().len() > 3, "the outputs were retried alone");
}

#[test]
fn a_standalone_lrc_is_lyrics_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = located(dir.path(), "x.lrc");
    std::fs::write(&source.path, "[00:01.00]a\n").unwrap();
    source.kind = Kind::Lyrics;
    let fake = Fake::default();
    let facts = gather(&fake, &source, &dir.path().join("scratch")).unwrap();
    assert_eq!(fake.calls(), Vec::<Vec<String>>::new());
    assert_eq!(facts.lyrics[0].at, LyricsAt::File);
    assert!(facts.audio.is_none());
}

#[test]
fn segments_stay_inside_the_recording() {
    assert_eq!(segment_starts(None), [0.0]);
    assert_eq!(segment_starts(Some(10.0)), [0.0]);
    assert_eq!(segment_starts(Some(200.0)), [50.0, 100.0, 150.0]);
}

#[test]
fn facts_round_trip_through_json() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let facts = gather(&Fake::default(), &source, &dir.path().join("scratch")).unwrap();
    let back: Facts = serde_json::from_str(&serde_json::to_string(&facts).unwrap()).unwrap();
    assert_eq!(
        (&back.covers[0].at, &back.lyrics, &back.tags, &back.print),
        (
            &facts.covers[0].at,
            &facts.lyrics,
            &facts.tags,
            &facts.print
        )
    );
    assert!(back.holds_for(&source.rev()));
}

#[test]
fn a_file_cut_off_is_as_long_as_its_packets_and_measured_within_them() {
    let real = "#tb 0: 1/44100\n#media_type 0: audio\n\
                0,    2195456,    2195456,     4096,     5610, 0x1517d997\n\
                0,    2199552,    2199552,     4096,     2234, 0x74706144\n";
    let held = packet_seconds(real).unwrap();
    assert!((held - 49.97).abs() < 0.01, "{held}");
    assert_eq!(
        packet_seconds("0, 0, 0, 960, 1, 0x0\n"),
        None,
        "no time base"
    );

    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.flac");
    let fake = Fake {
        held: vec![("Song.flac".into(), 50)],
        ..Fake::default()
    }
    .probe("Song.flac", crate::testing::FLAC);
    let facts = gather(&fake, &source, &dir.path().join("scratch")).unwrap();
    assert_eq!(facts.duration, Some(50.0));
    assert_eq!(facts.cut_from, Some(200.0));
    assert!(facts.audio.unwrap().quality.is_some());
    let whole = Fake::default().probe("Song.flac", crate::testing::FLAC);
    let facts = gather(&whole, &source, &dir.path().join("whole")).unwrap();
    assert_eq!((facts.duration, facts.cut_from), (Some(200.0), None));
}

/// A `CUESHEET` block of tracks with these flags and one index each.
fn cuesheet(flags: &[u8]) -> Vec<u8> {
    let mut block = vec![0; 395];
    block.push(u8::try_from(flags.len()).unwrap());
    for f in flags {
        let mut track = vec![0; 36];
        track[21] = *f;
        track[35] = 1;
        block.extend(track);
        block.extend([0; 12]);
    }
    block
}

#[test]
fn a_cue_sheet_flag_marks_pre_emphasis() {
    assert!(!cuesheet_emphasis(&cuesheet(&[0, 0])));
    assert!(cuesheet_emphasis(&cuesheet(&[0, 0x40])));
    assert!(!cuesheet_emphasis(&cuesheet(&[0x40])[..400]), "cut short");
}

#[test]
fn a_cue_sheet_beside_a_file_is_noted() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("Lantern Hours.flac");
    std::fs::write(&image, b"fLaC").unwrap();
    assert!(unkept(&image).is_empty());
    std::fs::write(
        dir.path().join("Lantern Hours.cue"),
        "FILE \"Lantern Hours.flac\" WAVE\n  TRACK 01 AUDIO\n    FLAGS DCP PRE\n",
    )
    .unwrap();
    assert_eq!(unkept(&image), [Unkept::PreEmphasis, Unkept::CueSheet]);
}
