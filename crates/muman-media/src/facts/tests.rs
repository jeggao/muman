use std::fmt::Write as _;

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
    let facts = gather(
        &fake,
        &source,
        &dir.path().join("scratch"),
        &[analysis::Kind::Loudness],
    )
    .unwrap();
    assert_eq!(fake.calls().len(), 3, "{:#?}", fake.calls());
    assert!(
        fake.ran("-err_detect:v"),
        "a picture cut short fails rather than measuring sharp"
    );
    assert!(
        fake.ran("-max_pixels:v"),
        "a picture too large to measure fails before it is decoded"
    );
    let audio = facts.audio.unwrap();
    assert!(
        audio.analysis.loudness().is_some(),
        "analyzed in the same run"
    );
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
fn a_picture_past_the_most_pixels_is_no_picture_and_is_not_read() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let pam = |name: &str, side: u64, padded: u64| {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        write!(
            file,
            "P7\nWIDTH {side}\nHEIGHT {side}\nDEPTH 2\nMAXVAL 255\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n"
        )
        .unwrap();
        let pixels: Vec<u8> = (0..side * side)
            .flat_map(|i| [(i * 37 % 256) as u8, 255])
            .collect();
        file.write_all(&pixels).unwrap();
        if padded > 0 {
            file.set_len(padded).unwrap();
        }
        path
    };
    let (quality, look) = measure_image(&pam("small.pam", 16, 0));
    assert_eq!(quality.unwrap().width, 16);
    assert!(look.is_some());
    let past = pam("large.pam", 8, 2 * quality::MOST_PIXELS + 1024);
    assert!(matches!(measure_image(&past), (None, None)));
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
    let facts = gather(&fake, &source, &dir.path().join("scratch"), &[]).unwrap();
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

/// A FLAC at `path` holding `picture`'s bytes as its front cover,
/// labelled `label`.
fn with_cover(path: &Path, picture: &[u8], label: lofty::picture::MimeType) {
    use lofty::config::WriteOptions;
    use lofty::picture::{Picture, PictureType};
    use lofty::tag::{Tag, TagExt, TagType};

    std::fs::write(path, crate::testing::SILENCE_FLAC).unwrap();
    let mut tag = Tag::new(TagType::VorbisComments);
    tag.push_picture(
        Picture::unchecked(picture.to_vec())
            .pic_type(PictureType::CoverFront)
            .mime_type(label)
            .build(),
    );
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

#[test]
fn a_picture_labelled_otherwise_than_its_bytes_is_read_by_them() {
    use lofty::picture::MimeType;

    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.flac");
    let scratch = dir.path().join("scratch");
    let covers = |json: &str| {
        let fake = Fake::default().probe("Song.flac", json);
        let facts = gather(&fake, &source, &scratch, &[]).unwrap();
        let at: Vec<(CoverAt, String)> = (facts.covers.into_iter())
            .map(|c| (c.at, c.mimetype))
            .collect();
        at
    };
    let as_png = FLAC.replace(r#""mjpeg""#, r#""png""#);
    let unshown = FLAC.replace(r#""codec_type": "video""#, r#""codec_type": "data""#);

    with_cover(&source.path, crate::testing::PIXEL_JPEG, MimeType::Png);
    let atom = vec![(CoverAt::Atom { index: 0 }, "image/jpeg".to_string())];
    assert_eq!(
        covers(&as_png),
        atom,
        "ffprobe shows a PNG it cannot decode"
    );
    with_cover(
        &source.path,
        crate::testing::PIXEL,
        MimeType::Unknown("image/x-png".into()),
    );
    let atom = vec![(CoverAt::Atom { index: 0 }, "image/png".to_string())];
    assert_eq!(covers(&unshown), atom, "ffprobe shows none");
    with_cover(&source.path, crate::testing::PIXEL_JPEG, MimeType::Jpeg);
    assert_eq!(
        covers(FLAC),
        [(CoverAt::Picture { index: 1 }, "image/jpeg".to_string())],
        "labelled as it is, read as ffprobe shows it"
    );
}

#[test]
fn a_picture_that_does_not_decode_is_said() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.flac");
    let mut fake = Fake::default().probe("Song.flac", FLAC);
    fake.failing = vec!["image2".into()];
    let facts = gather(&fake, &source, &dir.path().join("scratch"), &[]).unwrap();
    assert!(facts.covers.is_empty());
    assert_eq!(facts.unkept, [Unkept::Picture]);
}

#[test]
fn a_file_s_own_lyrics_tag_is_lyrics_of_it_measured_hashed_and_read_again() {
    use lofty::config::WriteOptions;
    use lofty::tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType};

    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.flac");
    std::fs::write(&source.path, crate::testing::SILENCE_FLAC).unwrap();
    let lrc = "[00:01.00]first invented line\n[00:03.00][Chorus]\n[00:05.00]last line\n";
    let mut tag = Tag::new(TagType::VorbisComments);
    tag.push(TagItem::new(ItemKey::Lyrics, ItemValue::Text(lrc.into())));
    tag.save_to_path(&source.path, WriteOptions::default())
        .unwrap();
    let fake = Fake::default().probe("Song.flac", FLAC);
    let scratch = dir.path().join("scratch");
    let facts = gather(&fake, &source, &scratch, &[]).unwrap();
    let own = facts.lyrics.iter().find(|l| l.at == LyricsAt::Tag).unwrap();
    assert_eq!(own.timing.unwrap().lines, 2, "the cue is no lyric");
    assert_eq!(own.language, Language::Unstated);
    assert!(own.digest.is_some());
    let (digests, _) = hash(&fake, &source, &scratch).unwrap();
    assert_eq!(digests.lyrics(&LyricsAt::Tag), own.digest);
    let again = retag(&fake, &source, &scratch).unwrap().lyrics;
    assert_eq!(again.as_ref(), Some(own));
    let found = relyric(&fake, &source, &[LyricsAt::Tag], &scratch).unwrap();
    assert_eq!(found, [(LyricsAt::Tag, true)]);
}

#[test]
fn a_failed_measure_costs_only_itself() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let fake = Fake {
        failing: vec!["lrc".into()],
        ..Fake::default()
    };
    let facts = gather(&fake, &source, &dir.path().join("scratch"), &[]).unwrap();
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
    let facts = gather(&fake, &source, &dir.path().join("scratch"), &[]).unwrap();
    assert_eq!(fake.calls(), Vec::<Vec<String>>::new());
    assert_eq!(facts.lyrics[0].at, LyricsAt::File);
    assert!(facts.audio.is_none());
    assert!(facts.tags_hold(Kind::Lyrics), "never read again");
}

#[test]
fn a_sidecar_is_named_as_its_song_in_any_case() {
    let song = Path::new("a/Back Home.flac");
    assert!(named_as(Path::new("a/back home.JPG"), song));
    assert!(!named_as(Path::new("a/back.jpg"), song));
}

#[test]
fn segments_stay_inside_the_recording() {
    assert_eq!(segment_starts(None), [0.0]);
    assert_eq!(segment_starts(Some(10.0)), [0.0]);
    assert_eq!(segment_starts(Some(200.0)), [50.0, 100.0, 150.0]);
}

#[test]
fn segments_that_fell_silent_are_taken_where_it_sounds() {
    let mut sounding = vec![true; 10];
    sounding.extend([false; 30]);
    let starts = sounding_starts(&sounding).unwrap();
    assert!(
        starts
            .iter()
            .all(|s| s + quality::SEGMENT_SECONDS / 2.0 <= 10.0),
        "{starts:?}"
    );
    assert_eq!(sounding_starts(&[false; 40]), None);
    let late = [vec![false; 300], vec![true; 3]].concat();
    assert_eq!(
        sounding_starts(&late).unwrap(),
        [303.0 - quality::SEGMENT_SECONDS],
        "a hidden track at the end"
    );
}

#[test]
fn facts_of_the_measure_before_hold_unless_they_found_no_sound() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let mut f = Facts::unreadable(source.rev());
    f.method = before_sound();
    f.audio = Some(AudioFacts {
        quality: Some(AudioQuality {
            bandwidth_hz: 20_000.0,
            incoherence: 0.5,
            clipping: 0.0,
            bits: 16,
        }),
        codec: "flac".into(),
        ..AudioFacts::default()
    });
    assert!(f.holds_for(&source.rev()));
    f.audio.as_mut().unwrap().quality = None;
    assert!(!f.holds_for(&source.rev()), "measured again");
}

#[test]
fn facts_of_lossy_audio_before_the_lowest_wall_are_measured_again() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    for method in [
        before_lowest(),
        before_square(),
        before_alpha(),
        before_sound(),
    ] {
        let mut f = Facts::unreadable(source.rev());
        f.method.clone_from(&method);
        f.audio = Some(AudioFacts {
            quality: Some(AudioQuality {
                bandwidth_hz: 20_437.0,
                incoherence: 0.5,
                clipping: 0.0,
                bits: 0,
            }),
            codec: "flac".into(),
            ..AudioFacts::default()
        });
        assert!(f.holds_for(&source.rev()), "{method}: lossless");
        f.audio.as_mut().unwrap().codec = "opus".into();
        assert!(!f.holds_for(&source.rev()), "{method}: measured again");
    }
}

#[test]
fn facts_from_before_alpha_hold_unless_a_picture_had_no_detail() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let mut f = Facts::unreadable(source.rev());
    f.method = before_alpha();
    let flat = |effective| CoverFacts {
        at: CoverAt::File,
        mimetype: "image/png".into(),
        quality: Some(ImageQuality {
            width: 64,
            height: 64,
            content: quality::Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            effective,
            blockiness: 0.0,
        }),
        digest: None,
        look: None,
        other: false,
    };
    f.covers = vec![flat(64)];
    assert!(f.holds_for(&source.rev()));
    f.covers = vec![flat(0)];
    assert!(!f.holds_for(&source.rev()), "measured again");
}

#[test]
fn facts_of_older_measures_of_a_picture_past_the_largest_measured_do_not_hold() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let picture = |side: u32| CoverFacts {
        at: CoverAt::File,
        mimetype: "image/png".into(),
        quality: Some(ImageQuality {
            width: side,
            height: side,
            content: quality::Rect {
                x: 0,
                y: 0,
                width: side,
                height: side,
            },
            effective: side,
            blockiness: 0.0,
        }),
        digest: None,
        look: None,
        other: false,
    };
    for method in [before_alpha(), before_sound()] {
        let mut f = Facts::unreadable(source.rev());
        f.method.clone_from(&method);
        f.audio = Some(AudioFacts {
            quality: Some(AudioQuality {
                bandwidth_hz: 20_000.0,
                incoherence: 0.5,
                clipping: 0.0,
                bits: 16,
            }),
            codec: "flac".into(),
            ..AudioFacts::default()
        });
        f.covers = vec![picture(4000)];
        assert!(f.holds_for(&source.rev()), "{method}");
        f.covers = vec![picture(9000)];
        assert!(!f.holds_for(&source.rev()), "{method}: measured again");
    }
}

#[test]
fn facts_of_older_measures_of_a_frame_trimmed_to_no_square_do_not_hold() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let framed = |x: u32, width: u32| CoverFacts {
        at: CoverAt::File,
        mimetype: "image/png".into(),
        quality: Some(ImageQuality {
            width: 1280,
            height: 720,
            content: quality::Rect {
                x,
                y: 0,
                width,
                height: 720,
            },
            effective: 512,
            blockiness: 0.0,
        }),
        digest: None,
        look: None,
        other: false,
    };
    for method in [before_square(), before_alpha(), before_sound()] {
        let mut f = Facts::unreadable(source.rev());
        f.method.clone_from(&method);
        f.covers = vec![framed(280, 720)];
        assert!(f.holds_for(&source.rev()), "{method}: its square art");
        f.covers = vec![framed(0, 1280)];
        assert!(f.holds_for(&source.rev()), "{method}: a frame untrimmed");
        f.covers = vec![framed(320, 640)];
        assert!(!f.holds_for(&source.rev()), "{method}: measured again");
    }
}

#[test]
fn facts_round_trip_through_json() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let facts = gather(&Fake::default(), &source, &dir.path().join("scratch"), &[]).unwrap();
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
fn a_file_cut_off_is_judged_by_its_stream_and_measured_within_its_packets() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.flac");
    let said = crate::testing::FLAC.replace(
        r#""sample_rate": "44100"}"#,
        r#""sample_rate": "44100", "duration": "200.0"}"#,
    );
    let wanted = [crate::analysis::Kind::Integrity];
    let cut = Fake {
        held: vec![("Song.flac".into(), 50)],
        wavs: vec![("Song.flac".into(), wav_of(50.0))],
        ..Fake::default()
    }
    .probe("Song.flac", &said);
    let facts = gather(&cut, &source, &dir.path().join("cut"), &wanted).unwrap();
    assert_eq!(
        facts.damage(),
        Some(crate::integrity::Damage::Cut {
            said: 200.0,
            ends: 50.0
        })
    );
    assert_eq!((facts.duration, facts.length()), (Some(200.0), Some(50.0)));
    assert!(facts.audio.unwrap().quality.is_some());
    let whole = Fake {
        wavs: vec![("Song.flac".into(), wav_of(200.0))],
        ..Fake::default()
    }
    .probe("Song.flac", &said);
    let facts = gather(&whole, &source, &dir.path().join("whole"), &wanted).unwrap();
    assert_eq!((facts.damage(), facts.length()), (None, Some(200.0)));
}

/// An MP3 whose ffprobe says it plays `said` seconds, its first bytes
/// `head`, decoding to `decoded` seconds of audio.
fn mp3_facts(head: Vec<u8>, said: f64, decoded: f64) -> Facts {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.mp3");
    std::fs::write(&source.path, head).unwrap();
    let json = format!(
        r#"{{"streams": [{{"index": 0, "codec_type": "audio", "codec_name": "mp3",
            "sample_rate": "44100", "channels": 2, "duration": "{said}"}}],
            "format": {{"format_name": "mp3", "duration": "{said}"}}}}"#
    );
    let fake = Fake {
        wavs: vec![("Song.mp3".into(), wav_of(decoded))],
        ..Fake::default()
    }
    .probe("Song.mp3", &json);
    let wanted = [crate::analysis::Kind::Integrity];
    gather(&fake, &source, &dir.path().join("scratch"), &wanted).unwrap()
}

#[test]
fn an_mp3_whose_length_ffmpeg_guessed_is_whole_and_plays_its_packets() {
    let bare = mp3_facts(crate::probe::mp3_head(true, true, true, None), 62.0, 40.0);
    assert_eq!(bare.damage(), None, "62 s is a guess from its bitrate");
    assert_eq!(bare.integrity().unwrap().said, None);
    assert_eq!((bare.duration, bare.length()), (None, Some(40.0)));
    let xing = Some((b"Xing", 36, 0x0f, 7656));
    let cut = mp3_facts(crate::probe::mp3_head(true, true, true, xing), 200.0, 50.0);
    assert_eq!(
        cut.damage(),
        Some(crate::integrity::Damage::Cut {
            said: 200.0,
            ends: 50.0
        }),
        "its Xing header counts the frames it should hold"
    );
    assert_eq!(cut.length(), Some(50.0));
}

#[test]
fn an_adts_stream_states_no_length_to_be_cut_short_of() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.aac");
    let json = r#"{"streams": [{"index": 0, "codec_type": "audio", "codec_name": "aac",
        "sample_rate": "44100", "channels": 2, "duration": "123.0"}],
        "format": {"format_name": "aac", "duration": "123.0"}}"#;
    let fake = Fake {
        wavs: vec![("Song.aac".into(), wav_of(44.0))],
        ..Fake::default()
    }
    .probe("Song.aac", json);
    let wanted = [crate::analysis::Kind::Integrity];
    let facts = gather(&fake, &source, &dir.path().join("scratch"), &wanted).unwrap();
    assert_eq!((facts.damage(), facts.length()), (None, Some(44.0)));
}

#[test]
fn a_subtitle_running_past_the_audio_cuts_nothing_off() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "chan/Song [aaaaaaaaaaa].mkv");
    let probe = crate::testing::ORIGINAL
        .replace(
            r#""sample_rate": "48000"}"#,
            r#""sample_rate": "48000", "tags": {"DURATION": "00:00:02.000000000"}}"#,
        )
        .replace(r#""duration": "200.0""#, r#""duration": "4.0""#);
    let fake = Fake::default().probe("Song", &probe);
    let wanted = [crate::analysis::Kind::Integrity];
    let facts = gather(&fake, &source, &dir.path().join("scratch"), &wanted).unwrap();
    assert!(facts.integrity().is_some());
    assert_eq!((facts.damage(), facts.length()), (None, Some(2.0)));
    assert_eq!(facts.duration, Some(4.0), "the file's, its subtitle's too");
}

#[test]
fn holes_in_its_packets_are_missing_audio() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "A/Song.flac");
    let mut list = "#tb 0: 1/1000\n".to_string();
    for n in (0..500).filter(|n| !(150..325).contains(n)) {
        let pts = n * 20;
        let _ = writeln!(list, "0, {pts}, {pts}, 20, 300, 0x0");
    }
    let fake = Fake {
        timelines: vec![("Song.flac".into(), list)],
        wavs: vec![("Song.flac".into(), wav_of(6.5))],
        ..Fake::default()
    }
    .probe("Song.flac", crate::testing::FLAC);
    let wanted = [crate::analysis::Kind::Integrity];
    let facts = gather(&fake, &source, &dir.path().join("scratch"), &wanted).unwrap();
    let Some(crate::integrity::Damage::Missing { seconds, holes }) = facts.damage() else {
        panic!("{:?}", facts.integrity());
    };
    assert!((seconds - 3.5).abs() < 0.01 && (holes - 3.5).abs() < 0.01);
}

/// `seconds` of quiet tone as an analysis decodes it.
fn wav_of(seconds: f64) -> Vec<u8> {
    crate::testing::wav(
        8000,
        1,
        None,
        &crate::testing::sine(8000, 1, 1000.0, 0.1, seconds),
    )
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
    assert_eq!(unkept(&image), []);
    std::fs::write(
        dir.path().join("Lantern Hours.cue"),
        "FILE \"Lantern Hours.flac\" WAVE\n  TRACK 01 AUDIO\n    FLAGS DCP PRE\n",
    )
    .unwrap();
    assert_eq!(unkept(&image), [Unkept::PreEmphasis, Unkept::CueSheet]);
}

#[test]
fn an_analysis_whose_run_fails_starts_over_alone() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let wanted = [analysis::Kind::Loudness];
    let whole = gather(&Fake::default(), &source, &dir.path().join("a"), &wanted).unwrap();
    let failing = Fake {
        failing: vec!["wav".into()],
        ..Fake::default()
    };
    let facts = gather(&failing, &source, &dir.path().join("b"), &wanted).unwrap();
    let analysis = |f: &Facts| f.audio.as_ref().unwrap().analysis.clone();
    assert_eq!(analysis(&facts), analysis(&whole));
    assert!(facts.print.is_some(), "the other outputs ran alone");
}

#[test]
fn an_analysis_caught_up_takes_one_run_and_reads_the_opus_header() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let fake = Fake::default();
    let facts = gather(&fake, &source, &dir.path().join("a"), &[]).unwrap();
    let audio = facts.audio.unwrap();
    assert!(audio.analysis.loudness.is_none() && !fake.ran(ffmpeg::PIPE));
    let before = fake.calls().len();
    let a = analyze(
        &fake,
        &source,
        &audio,
        &[analysis::Kind::Loudness],
        &dir.path().join("b"),
    )
    .unwrap();
    assert_eq!(fake.calls().len(), before + 1);
    assert_eq!(a.loudness().map(|m| m.opus_gain), Some(0));
}

#[test]
fn a_picture_is_looked_at_when_measured_and_an_old_one_caught_up_alone() {
    let dir = tempfile::tempdir().unwrap();
    let source = located(dir.path(), "c/Song [aaaaaaaaaaa].mkv");
    let fake = Fake::default();
    let mut facts = gather(&fake, &source, &dir.path().join("scratch"), &[]).unwrap();
    assert!(facts.covers[0].look.is_some());
    assert!(facts.pictured());
    let look = facts.covers[0].look;
    facts.covers[0].look = None;
    facts.picture_method.clear();
    assert!(!facts.pictured());
    let fake = Fake::default();
    let looks = super::look(&fake, &source, &dir.path().join("scratch")).unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 3, "{calls:#?}");
    assert!(
        !fake.ran("s16le") && !fake.ran("wav"),
        "nothing but the pictures decoded"
    );
    facts.take_looks(&looks);
    assert!(facts.pictured());
    assert_eq!(facts.covers[0].look, look);
}

/// Audio cut off at 11 kHz, mono in two channels, clipped, past full
/// scale, of 16 bits and silent, each three segments at 44.1 kHz, and
/// whether it has headroom.
fn quality_audio() -> Vec<(Vec<quality::Segment>, bool)> {
    use quality::Segment;

    let noise = |n: usize, seed: u64| -> Vec<f32> {
        crate::testing::words(n, seed)
            .into_iter()
            .map(|w| (f64::from(w) / f64::from(u32::MAX) - 0.5) as f32)
            .collect()
    };
    let tones = |seed: u64| -> Vec<f32> {
        let hz = crate::testing::words(40, seed);
        (0..44_100)
            .map(|n| {
                let t = f64::from(n) / 44_100.0;
                let sum: f64 = hz
                    .iter()
                    .map(|w| (std::f64::consts::TAU * f64::from(200 + w % 10_800) * t).sin())
                    .sum();
                (sum / 40.0) as f32
            })
            .collect()
    };
    let stereo = |l: &[f32], r: &[f32]| -> Segment {
        Segment::new(2, l.iter().zip(r).flat_map(|(l, r)| [*l, *r]).collect())
    };
    let thrice = |s: Segment| vec![s.clone(), s.clone(), s];
    let (left, right) = (tones(1), tones(2));
    let full: Vec<f32> = left
        .iter()
        .zip(noise(left.len(), 3))
        .map(|(t, n)| t + n)
        .collect();
    let half: Vec<f32> = full.iter().map(|x| x / 2.0).collect();
    let clipped: Vec<f32> = noise(2 * 16_384, 4)
        .iter()
        .map(|x| (x * 3.0).clamp(-1.0, 1.0))
        .collect();
    let past: Vec<f32> = noise(2 * 16_384, 5).iter().map(|x| x * 2.5).collect();
    let sixteen: Vec<f32> = noise(2 * 16_384, 6)
        .iter()
        .map(|x| (x * 32_768.0).round() / 32_768.0)
        .collect();
    // A tone at every FFT bin to 16 kHz, and 25 dB down from there to
    // 20 kHz, as Opus fills past an MP3's lowpass.
    let phases = crate::testing::words(1858, 7);
    let filled: Vec<f32> = (0..16_384)
        .map(|n| {
            let t = f64::from(n) / 44_100.0;
            let sum: f64 = phases
                .iter()
                .enumerate()
                .map(|(k, phase)| {
                    let hz = (k + 1) as f64 * 44_100.0 / 4096.0;
                    let gain = if hz <= 16_000.0 { 1.0 } else { 0.056 };
                    let phase = f64::from(*phase) / f64::from(u32::MAX);
                    gain * (std::f64::consts::TAU * (hz * t + phase)).sin()
                })
                .sum();
            (sum / 200.0) as f32
        })
        .collect();
    vec![
        (thrice(stereo(&filled, &filled)), false),
        (thrice(stereo(&left, &right)), false),
        (thrice(stereo(&full, &half)), false),
        (thrice(Segment::new(2, clipped)), false),
        (thrice(Segment::new(2, past)), true),
        (thrice(Segment::new(2, sixteen)), false),
        (thrice(Segment::new(1, vec![0.0; 16_384])), false),
    ]
}

/// Gray pictures detailed, upscaled, boxed in bars, blocky and flat.
fn quality_pictures() -> Vec<(Vec<u8>, u32, u32)> {
    let gray = |n: usize, seed: u64| -> Vec<u8> {
        crate::testing::words(n, seed)
            .into_iter()
            .map(|w| (w >> 24) as u8)
            .collect()
    };
    let detailed = gray(256 * 256, 7);
    let small = gray(96 * 96, 8);
    let upscaled: Vec<u8> = (0..384 * 384)
        .map(|i| {
            let at = |x: usize, y: usize| u32::from(small[y.min(95) * 96 + x.min(95)]);
            let (x, y) = (i % 384 / 4, i / 384 / 4);
            let (fx, fy) = ((i % 384 % 4) as u32, (i / 384 % 4) as u32);
            let mixed = (4 - fx) * (4 - fy) * at(x, y)
                + fx * (4 - fy) * at(x + 1, y)
                + (4 - fx) * fy * at(x, y + 1)
                + fx * fy * at(x + 1, y + 1);
            (mixed / 16) as u8
        })
        .collect();
    let boxed: Vec<u8> = (0..320 * 180)
        .map(|i| {
            let (x, y) = (i % 320, i / 320);
            if (70..250).contains(&x) {
                detailed[y * 256 + x - 70]
            } else {
                0
            }
        })
        .collect();
    let dark_edged: Vec<u8> = boxed
        .iter()
        .enumerate()
        .map(|(i, &g)| if (90..230).contains(&(i % 320)) { g } else { 0 })
        .collect();
    let blocky: Vec<u8> = (0..256 * 256)
        .map(|i| {
            let (x, y) = (i % 256, i / 256);
            detailed[y / 8 * 8 * 256 + x / 8 * 8].wrapping_add(((x % 8 + y % 8) / 4) as u8)
        })
        .collect();
    vec![
        (detailed, 256, 256),
        (upscaled, 384, 384),
        (boxed, 320, 180),
        (dark_edged, 320, 180),
        (blocky, 256, 256),
        (vec![90; 64 * 64], 64, 64),
    ]
}

/// What the quality measures make of [`quality_audio`] and
/// [`quality_pictures`], of a graymap transparent in places, and of one
/// past what [`quality::MOST_PIXELS`] allows, with [`quality::METHOD`].
const QUALITY_WRITTEN: (&str, &str) = (
    "quality/11",
    "87163984360d9ace671d595e98b066e89b1ca51642f5464b83f7530125bcb892",
);

#[test]
fn the_quality_method_rises_with_what_it_makes() {
    let mut digest = crate::testing::Digest::default();
    for (segments, headroom) in quality_audio() {
        digest.debug(&quality::audio(&segments, 44_100, headroom));
    }
    let pictures = quality_pictures();
    for (pixels, w, h) in &pictures {
        digest.debug(&quality::image(pixels, *w, *h));
    }
    let dir = tempfile::tempdir().unwrap();
    let header = "P7\nWIDTH 64\nHEIGHT 64\nDEPTH 2\nMAXVAL 255\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n";
    let mut transparent = header.as_bytes().to_vec();
    transparent.extend(pictures[0].0[..64 * 64].iter().zip(0..).flat_map(|(g, i)| {
        let alpha = if (i / 64) % 16 < 8 {
            255
        } else {
            (i % 64 * 4) as u8
        };
        [*g, alpha]
    }));
    let pam = dir.path().join("transparent.pam");
    std::fs::write(&pam, &transparent).unwrap();
    let past = dir.path().join("past.pam");
    std::fs::write(&past, &transparent).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&past)
        .unwrap()
        .set_len(2 * 8192 * 8192 + 1024)
        .unwrap();
    for path in [pam, past] {
        digest.debug(&measure_image(&path).0);
    }
    crate::testing::check(QUALITY_WRITTEN, quality::METHOD, &digest.hex());
}

#[test]
fn facts_of_files_holding_the_same_bytes_move_to_their_new_revision() {
    let mut facts = Facts::unreadable("10:20".into());
    facts.content = Some("aa;bb".into());
    facts.audio = Some(AudioFacts {
        digest: Some("0123456789abcdef0123456789abcdef".into()),
        ..AudioFacts::default()
    });
    let moved = facts.moved_to("11:30", Some("aa;bb")).unwrap();
    assert_eq!(moved.rev, "11:30");
    assert_eq!(
        moved.audio_rev(),
        facts.audio_rev(),
        "its plans name it alike"
    );
    assert!(
        facts.moved_to("11:30", Some("aa;cc")).is_none(),
        "other bytes"
    );
    assert!(facts.moved_to("11:30", None).is_none(), "unread");

    let mut named = facts.clone();
    named.audio.as_mut().unwrap().digest = None;
    assert!(
        named.moved_to("11:30", Some("aa;bb")).is_none(),
        "a part its plans name by the revision"
    );
    let mut earlier = facts.clone();
    earlier.content = None;
    assert!(earlier.moved_to("11:30", None).is_none(), "no digest kept");
}

#[test]
fn lyrics_know_whether_they_sing_in_latin_letters_alone() {
    let latin = |text: &str| lyrics_of(LyricsAt::File, text).latin;
    assert_eq!(latin("[00:01.00]yoru no fune ga\n"), Some(true));
    assert_eq!(latin("[00:01.00]夜の船が\n"), Some(false));
    assert_eq!(latin("[00:01.00]Night ferry, 夜の船\n"), Some(false));
}

#[test]
fn a_facts_row_an_earlier_muman_wrote_reads_and_writes_back_as_it_was() {
    let row = r#"{"rev":"r","method":"m","duration":200.0,"audio":null,"covers":[],"lyrics":[],"tags":{"title":{"values":["Lantern Weather"],"structured":true}},"tags_method":"tags/12","release":false}"#;
    let facts: Facts = serde_json::from_str(row).unwrap();
    let title = &facts.tags[&crate::tags::Field::Title];
    assert_eq!(title.also, []);
    assert_eq!(facts.artists, []);
    let written = serde_json::to_string(&facts).unwrap();
    assert!(!written.contains("\"also\""), "{written}");
    assert!(
        !facts.tags_hold(Kind::Media),
        "read again for what it lacks"
    );
}
