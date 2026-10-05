use crate::settings::LyricsPlacement;
use lofty::flac::FlacFile;
use lofty::ogg::OpusFile;

use super::*;
use crate::quality::Rect;
use crate::resolve::{AudioRef, RENDER_VERSION};
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
        version: RENDER_VERSION,
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
    let r = go(&fake, &f, &plan(Format::OpusCopy)).unwrap();
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
    let mut p = plan(Format::OpusCopy);
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
    let r = go(&fake, &f, &plan(Format::FlacCopy)).unwrap();
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
        &plan(Format::OpusEncode {
            channels: 2,
            kbps: 160,
        }),
    )
    .unwrap();
    assert!(fake.ran("libopus") && fake.ran("160k"));
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
    let r = go(&fake, &f, &plan(Format::OpusCopy)).unwrap();
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
    assert!(go(&fake, &f, &plan(Format::OpusCopy)).is_err());
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
    let mut p = plan(Format::OpusCopy);
    p.cover = None;
    p.lyrics = Some(LyricsRef {
        key,
        rev: "r".into(),
        at: LyricsAt::File,
        shift_ms: 1000,
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
