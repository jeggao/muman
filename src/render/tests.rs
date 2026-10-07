use crate::settings::LyricsPlacement;
use lofty::flac::FlacFile;
use lofty::id3::v2::Id3v2Tag;
use lofty::mp4::Mp4File;
use lofty::mpeg::MpegFile;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::tag::Accessor;

use super::*;
use crate::codec::Codec;
use crate::quality::Rect;
use crate::resolve::{AudioRef, render_version};
use crate::store::Kind;
use crate::testing::Fake;

struct Fixture {
    dir: tempfile::TempDir,
    sources: BTreeMap<SourceKey, Located>,
}

fn yt(id: &str) -> SourceKey {
    SourceKey::youtube(id)
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut sources = BTreeMap::new();
    for id in ["aaaaaaaaaaa", "bbbbbbbbbbb"] {
        let path = dir.path().join(format!("store/c/Song [{id}].mkv"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"mkv").unwrap();
        sources.insert(
            yt(id),
            Located {
                key: yt(id),
                path,
                kind: Kind::Media,
                lyrics: None,
                covers: Vec::new(),
            },
        );
    }
    Fixture { dir, sources }
}

fn plan(format: Format) -> Plan {
    Plan {
        version: render_version(format.codec()),
        format,
        audio: AudioRef {
            key: yt("aaaaaaaaaaa"),
            rev: "r".into(),
            index: 1,
        },
        cover: Some(CoverRef {
            key: yt("aaaaaaaaaaa"),
            rev: "r".into(),
            at: CoverAt::Attachment { ordinal: 1 },
            mimetype: "image/webp".into(),
            crop: None,
        }),
        lyrics: Some(LyricsRef {
            key: yt("aaaaaaaaaaa"),
            rev: "r".into(),
            at: LyricsAt::Stream { index: 2 },
            shift_ms: 0,
            stretch_ppm: 0,
            placement: LyricsPlacement::Sidecar,
        }),
        tags: vec![
            ("TITLE".into(), vec!["Song".into()]),
            ("ARTIST".into(), vec!["A, B".into()]),
            ("ALBUM".into(), vec!["Record".into()]),
        ],
    }
}

fn go(fake: &Fake, f: &Fixture, plan: &Plan) -> Result<Rendered> {
    render(
        fake,
        &Job {
            plan,
            stem: Path::new("A/Record/Song"),
            library: &f.dir.path().join("lib"),
            sources: &f.sources,
            scratch: &f.dir.path().join("scratch"),
        },
    )
}

fn leftover_parts(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "part"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn an_opus_song_is_copied_tagged_covered_and_given_lyrics_in_one_run() {
    let f = fixture();
    let fake = Fake::default();
    let r = go(&fake, &f, &plan(Format::Copy { codec: Codec::Opus })).unwrap();
    assert_eq!(r.audio, PathBuf::from("A/Record/Song.opus"));
    assert_eq!(r.lyrics, Some(PathBuf::from("A/Record/Song.lrc")));
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(
        fake.calls().len(),
        2,
        "the WebP dump, then every output at once"
    );
    assert!(fake.ran("copy") && !fake.ran("libopus"));
    assert!(fake.ran("0:2"), "the subtitle stream is the lyrics");

    let lib = f.dir.path().join("lib");
    let mut file = fs::File::open(lib.join(&r.audio)).unwrap();
    let opus = OpusFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let c = opus.vorbis_comments();
    assert_eq!(c.get("TITLE"), Some("Song"));
    assert_eq!(c.get("ARTIST"), Some("A, B"));
    assert_eq!(c.pictures().len(), 1);
    assert_eq!(c.pictures()[0].0.pic_type(), PictureType::CoverFront);
    assert_eq!(c.pictures()[0].0.mime_type(), Some(&MimeType::Png));
    assert_eq!(
        fs::read_to_string(lib.join("A/Record/Song.lrc")).unwrap(),
        "[00:01.00]line\n[00:30.00]more\n"
    );
    assert_eq!(leftover_parts(&lib.join("A/Record")), Vec::<PathBuf>::new());
}

#[test]
fn a_jpeg_picture_stream_is_copied_and_a_bordered_one_cropped_to_png() {
    let f = fixture();
    let fake = Fake::default();
    let mut p = plan(Format::Copy { codec: Codec::Opus });
    p.cover = Some(CoverRef {
        key: yt("bbbbbbbbbbb"),
        rev: "r".into(),
        at: CoverAt::Picture { index: 3 },
        mimetype: "image/jpeg".into(),
        crop: None,
    });
    let r = go(&fake, &f, &p).unwrap();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(fake.calls().len(), 1, "no dump");
    assert!(fake.ran("1:3"), "the second input's picture");
    assert!(!fake.ran("png"));

    let fake = Fake::default();
    p.cover.as_mut().unwrap().crop = Some(Rect {
        x: 420,
        y: 0,
        width: 1080,
        height: 1080,
    });
    go(&fake, &f, &p).unwrap();
    assert!(fake.ran("crop=1080:1080:420:0") && fake.ran("png"));
}

#[test]
fn a_flac_song_keeps_its_tags_and_cover_as_flac() {
    let f = fixture();
    let fake = Fake::default();
    let r = go(&fake, &f, &plan(Format::Copy { codec: Codec::Flac })).unwrap();
    assert_eq!(r.audio, PathBuf::from("A/Record/Song.flac"));
    let mut file = fs::File::open(f.dir.path().join("lib").join(&r.audio)).unwrap();
    let flac = FlacFile::read_from(&mut file, ParseOptions::new()).unwrap();
    assert_eq!(flac.vorbis_comments().unwrap().get("ALBUM"), Some("Record"));
    assert_eq!(flac.pictures().len(), 1);
}

#[test]
fn another_lossy_codec_is_encoded_to_opus() {
    let f = fixture();
    let fake = Fake::default();
    go(
        &fake,
        &f,
        &plan(Format::Encode {
            codec: Codec::Opus,
            kbps: Some(160),
            adapt: None,
            mix: None,
        }),
    )
    .unwrap();
    assert!(fake.ran("libopus") && fake.ran("160k"));
}

/// The plan's tags with one no format names, and lyrics embedded.
fn tagged(format: Format) -> Plan {
    let mut p = plan(format);
    p.tags.push(("TRACKNUMBER".into(), vec!["3".into()]));
    p.tags.push(("TRACKTOTAL".into(), vec!["12".into()]));
    p.tags
        .push(("GENRE".into(), vec!["House".into(), "Disco".into()]));
    p.tags.push(("FAVOURITE".into(), vec!["Bright".into()]));
    p.lyrics.as_mut().unwrap().placement = LyricsPlacement::Embedded;
    p
}

#[test]
fn an_mp3_song_is_tagged_with_id3v2() {
    let f = fixture();
    let fake = Fake::default();
    let r = go(
        &fake,
        &f,
        &tagged(Format::Encode {
            codec: Codec::Mp3,
            kbps: Some(320),
            adapt: None,
            mix: None,
        }),
    )
    .unwrap();
    assert!(fake.ran("libmp3lame") && fake.ran("320k"));
    assert_eq!(r.audio, PathBuf::from("A/Record/Song.mp3"));
    let mut file = fs::File::open(f.dir.path().join("lib").join(&r.audio)).unwrap();
    let mp3 = MpegFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let id3: &Id3v2Tag = mp3.id3v2().unwrap();
    assert_eq!(id3.title().as_deref(), Some("Song"));
    assert_eq!(id3.artist().as_deref(), Some("A, B"));
    assert_eq!(id3.track(), Some(3));
    assert_eq!(id3.track_total(), Some(12));
    assert_eq!(id3.genre().as_deref(), Some("House / Disco"));
    assert_eq!(id3.get_user_text("FAVOURITE"), Some("Bright"));
    let generic = lofty::tag::Tag::from(id3.clone());
    assert_eq!(
        generic.get_string(lofty::tag::ItemKey::UnsyncLyrics),
        Some("[00:01.00]line\n[00:30.00]more\n")
    );
    assert_eq!(generic.pictures().len(), 1);
}

#[test]
fn an_m4a_song_is_tagged_with_atoms() {
    let f = fixture();
    let fake = Fake::default();
    let r = go(&fake, &f, &tagged(Format::Copy { codec: Codec::Aac })).unwrap();
    assert!(fake.ran("copy") && fake.ran("ipod"));
    assert_eq!(r.audio, PathBuf::from("A/Record/Song.m4a"));
    let mut file = fs::File::open(f.dir.path().join("lib").join(&r.audio)).unwrap();
    let m4a = Mp4File::read_from(&mut file, ParseOptions::new()).unwrap();
    let ilst = m4a.ilst().unwrap();
    assert_eq!(ilst.title().as_deref(), Some("Song"));
    assert_eq!(ilst.track(), Some(3));
    assert_eq!(ilst.track_total(), Some(12));
    let mood = ilst
        .get(&lofty::mp4::AtomIdent::Freeform {
            mean: "com.apple.iTunes".into(),
            name: "FAVOURITE".into(),
        })
        .unwrap();
    assert!(
        mood.data()
            .any(|d| matches!(d, lofty::mp4::AtomData::UTF8(s) if s == "Bright"))
    );
    assert_eq!(ilst.pictures().unwrap().count(), 1);
}

#[test]
fn a_vorbis_song_keeps_vorbis_comments_in_ogg() {
    let f = fixture();
    let fake = Fake::default();
    let r = go(
        &fake,
        &f,
        &tagged(Format::Encode {
            codec: Codec::Vorbis,
            kbps: Some(192),
            adapt: None,
            mix: None,
        }),
    )
    .unwrap();
    assert!(fake.ran("libvorbis") && fake.ran("192k"));
    assert_eq!(r.audio, PathBuf::from("A/Record/Song.ogg"));
    let mut file = fs::File::open(f.dir.path().join("lib").join(&r.audio)).unwrap();
    let ogg = VorbisFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let c = ogg.vorbis_comments();
    assert_eq!(c.get("FAVOURITE"), Some("Bright"));
    assert_eq!(c.get_all("GENRE").collect::<Vec<_>>(), ["House", "Disco"]);
    assert_eq!(c.pictures().len(), 1);
}

#[test]
fn failed_lyrics_cost_only_themselves_and_clear_the_last_build_s() {
    let f = fixture();
    let stale = f.dir.path().join("lib/A/Record/Song.lrc");
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    fs::write(&stale, "old").unwrap();
    let fake = Fake {
        failing: vec!["lrc".into()],
        ..Fake::default()
    };
    let r = go(&fake, &f, &plan(Format::Copy { codec: Codec::Opus })).unwrap();
    assert_eq!(r.lyrics, None);
    assert!(
        r.problems.iter().any(|p| p.starts_with("no lyrics")),
        "{:?}",
        r.problems
    );
    assert!(!stale.exists());
    assert!(f.dir.path().join("lib/A/Record/Song.opus").exists());
}

#[test]
fn failed_audio_fails_the_song_and_leaves_nothing_behind() {
    let f = fixture();
    let fake = Fake {
        failing: vec!["opus".into()],
        ..Fake::default()
    };
    assert!(go(&fake, &f, &plan(Format::Copy { codec: Codec::Opus })).is_err());
    let dir = f.dir.path().join("lib/A/Record");
    assert_eq!(leftover_parts(&dir), Vec::<PathBuf>::new());
    assert!(!dir.join("Song.opus").exists());
}

#[test]
fn a_lyrics_file_is_cleaned_and_moved_without_ffmpeg() {
    let mut f = fixture();
    let lrc = f.dir.path().join("store/x.lrc");
    fs::write(&lrc, "[00:01.00]♪\n[00:03.00]sung\n").unwrap();
    let key = SourceKey::Manual("x.lrc".into());
    f.sources.insert(
        key.clone(),
        Located {
            key: key.clone(),
            path: lrc,
            kind: Kind::Lyrics,
            lyrics: None,
            covers: Vec::new(),
        },
    );
    let mut p = plan(Format::Copy { codec: Codec::Opus });
    p.cover = None;
    p.lyrics = Some(LyricsRef {
        key,
        rev: "r".into(),
        at: LyricsAt::File,
        shift_ms: 1000,
        stretch_ppm: 0,
        placement: LyricsPlacement::Sidecar,
    });
    let fake = Fake::default();
    go(&fake, &f, &p).unwrap();
    assert!(!fake.ran("lrc"));
    assert_eq!(
        fs::read_to_string(f.dir.path().join("lib/A/Record/Song.lrc")).unwrap(),
        "[00:02.00]sung\n"
    );
}

#[test]
fn a_retag_writes_the_new_tags_and_keeps_the_cover_and_lyrics_in_every_format() {
    use lofty::file::TaggedFileExt;
    let formats = [
        Format::Copy { codec: Codec::Opus },
        Format::Copy { codec: Codec::Flac },
        Format::Encode {
            codec: Codec::Vorbis,
            kbps: Some(192),
            adapt: None,
            mix: None,
        },
        Format::Encode {
            codec: Codec::Mp3,
            kbps: Some(320),
            adapt: None,
            mix: None,
        },
        Format::Encode {
            codec: Codec::Aac,
            kbps: Some(256),
            adapt: None,
            mix: None,
        },
    ];
    for format in formats {
        let f = fixture();
        let mut p = plan(format);
        if let Some(l) = p.lyrics.as_mut() {
            l.placement = LyricsPlacement::Embedded;
        }
        let fake = Fake::default();
        let first = go(&fake, &f, &p).unwrap();
        p.tags[0] = ("TITLE".into(), vec!["Paper Comets".into()]);
        let library = f.dir.path().join("lib");
        let calls = fake.calls().len();
        let again = retag(&library, &first.audio, first.lyrics.as_deref(), &p).unwrap();
        assert_eq!(fake.calls().len(), calls, "{format:?}: nothing is run");
        assert_eq!(again.audio, first.audio);
        let file = lofty::read_from_path(library.join(&again.audio)).unwrap();
        let tag = file.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("Paper Comets"), "{format:?}");
        assert_eq!(tag.artist().as_deref(), Some("A, B"), "{format:?}");
        assert_eq!(tag.pictures().len(), 1, "{format:?}: the cover stays");
        let lyrics = tag
            .get_string(ItemKey::Lyrics)
            .or_else(|| tag.get_string(ItemKey::UnsyncLyrics));
        assert!(
            lyrics.is_some_and(|l| l.contains("line")),
            "{format:?}: the lyrics stay"
        );
        assert!(leftover_parts(&library.join("A/Record")).is_empty());
    }
}

#[test]
fn a_song_is_the_same_bytes_every_build_and_a_retag_what_a_build_writes() {
    for format in [
        Format::Copy { codec: Codec::Opus },
        Format::Copy { codec: Codec::Flac },
        Format::Encode {
            codec: Codec::Vorbis,
            kbps: Some(192),
            adapt: None,
            mix: None,
        },
        Format::Encode {
            codec: Codec::Mp3,
            kbps: Some(320),
            adapt: None,
            mix: None,
        },
        Format::Encode {
            codec: Codec::Aac,
            kbps: Some(256),
            adapt: None,
            mix: None,
        },
    ] {
        let mut p = plan(format);
        p.tags.extend([
            ("GENRE".into(), vec!["Folk".into()]),
            ("DATE".into(), vec!["1916".into()]),
            ("TRACKNUMBER".into(), vec!["3".into()]),
            ("TRACKTOTAL".into(), vec!["12".into()]),
            ("ALBUMARTIST".into(), vec!["Marlo Venn".into()]),
            (
                "MUSICBRAINZ_TRACKID".into(),
                vec!["00000000-0000-0000-0000-000000000001".into()],
            ),
            ("CUSTOMKEY".into(), vec!["x".into()]),
        ]);
        let built = |p: &Plan| {
            let f = fixture();
            let r = go(&Fake::default(), &f, p).unwrap();
            (
                fs::read(f.dir.path().join("lib").join(&r.audio)).unwrap(),
                f,
                r,
            )
        };
        let (first, f, r) = built(&p);
        for _ in 0..8 {
            assert!(
                built(&p).0 == first,
                "{format:?}: the same song, other bytes"
            );
        }
        let mut q = p.clone();
        q.tags[0] = (
            "TITLE".into(),
            vec!["Paper Comets, a title longer than the last".into()],
        );
        let library = f.dir.path().join("lib");
        retag(&library, &r.audio, r.lyrics.as_deref(), &q).unwrap();
        let retagged = fs::read(library.join(&r.audio)).unwrap();
        assert!(
            retagged == built(&q).0,
            "{format:?}: a retag writes what a build would"
        );
    }
}

#[test]
fn opus_holds_gains_as_r128_and_no_peaks() {
    let tag = |k: &str, v: &str| (k.to_string(), vec![v.to_string()]);
    let tags = [
        tag("TITLE", "Lantern"),
        tag("REPLAYGAIN_TRACK_GAIN", "-6.12 dB"),
        tag("REPLAYGAIN_TRACK_PEAK", "0.900000"),
        tag("REPLAYGAIN_ALBUM_GAIN", "1.00 dB"),
    ];
    assert_eq!(
        opus_gains(&tags),
        [
            tag("TITLE", "Lantern"),
            tag("R128_TRACK_GAIN", "-2847"),
            tag("R128_ALBUM_GAIN", "-1024"),
        ]
    );
}
