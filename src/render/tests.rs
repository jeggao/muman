use crate::settings::LyricsPlacement;
use lofty::flac::FlacFile;
use lofty::id3::v2::Id3v2Tag;
use lofty::mp4::Mp4File;
use lofty::mpeg::MpegFile;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::tag::Accessor;

use super::*;
use crate::codec::Codec;
use crate::library::{Ledger, Owned};
use crate::quality::Rect;
use crate::resolve::{AudioRef, LyricsFile, render_version};
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
        folder: None,
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
            made: Some(COVER_MAKING),
            out: None,
        }),
        lyrics: Some(LyricsRef {
            key: yt("aaaaaaaaaaa"),
            rev: "r".into(),
            at: LyricsAt::Stream { index: 2 },
            shift_ms: 0,
            stretch_ppm: 0,
            placement: LyricsPlacement::Sidecar,
            files: None,
            inside: None,
            end_ms: None,
            language: None,
            clean_off: Vec::new(),
            cleaning: crate::clean::lyrics::EDITION,
        }),
        tags: vec![
            ("TITLE".into(), vec!["Song".into()]),
            ("ARTIST".into(), vec!["A, B".into()]),
            ("ALBUM".into(), vec!["Record".into()]),
        ],
        loudness: None,
        tagging: crate::resolve::TAGGING,
        chapters: Vec::new(),
    }
}

fn go(fake: &Fake, f: &Fixture, plan: &Plan) -> Result<Rendered> {
    let folder = Path::new("A/Record");
    // As a sync claims the files a build before left at the song's paths.
    let claim: Vec<Owned> = fs::read_dir(f.dir.path().join("lib").join(folder))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| Ledger::taken(&folder.join(e.file_name())))
        .collect();
    go_claiming(fake, f, plan, &claim)
}

fn go_claiming(fake: &Fake, f: &Fixture, plan: &Plan, claim: &[Owned]) -> Result<Rendered> {
    render(
        fake,
        &Job {
            plan,
            stem: Path::new("A/Record/Song"),
            library: &f.dir.path().join("lib"),
            sources: &f.sources,
            scratch: &f.dir.path().join("scratch"),
            claim,
            stale: &[],
        },
    )
}

/// The files a render wrote beside its song.
fn names(r: &Rendered) -> Vec<PathBuf> {
    r.beside.iter().map(|(p, _)| p.clone()).collect()
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
    assert_eq!(names(&r), [PathBuf::from("A/Record/Song.lrc")]);
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
fn lyrics_written_alone_are_those_a_build_writes_and_go_when_the_plan_has_none() {
    let f = fixture();
    let p = plan(Format::Copy { codec: Codec::Opus });
    let built = go(&Fake::default(), &f, &p).unwrap();
    let lib = f.dir.path().join("lib");
    let lrc = lib.join("A/Record/Song.lrc");
    let whole = fs::read(&lrc).unwrap();
    let claim = [Ledger::taken(Path::new("A/Record/Song.lrc"))];
    let scratch = f.dir.path().join("scratch-beside");
    let job = |plan| Job {
        plan,
        stem: Path::new("A/Record/Song"),
        library: &lib,
        sources: &f.sources,
        scratch: &scratch,
        claim: &claim,
        stale: &[],
    };
    let fake = Fake::default();
    let alone = beside(&fake, &job(&p)).unwrap();
    assert_eq!(alone.files, built.beside);
    assert_eq!(fs::read(&lrc).unwrap(), whole, "as a build writes them");
    assert!(!fake.ran("copy"), "the audio left alone");
    assert!(fake.ran("0:2"), "the subtitle stream read again");
    assert_eq!(leftover_parts(&lib.join("A/Record")), Vec::<PathBuf>::new());

    let mut embedded = p.clone();
    embedded.lyrics.as_mut().unwrap().placement = LyricsPlacement::Embedded;
    let stale = [PathBuf::from("A/Record/Song.lrc")];
    let none = beside(
        &Fake::default(),
        &Job {
            stale: &stale,
            ..job(&embedded)
        },
    )
    .unwrap();
    assert_eq!(none.files, []);
    assert!(!lrc.exists(), "the last build's beside the song gone");
}

#[test]
fn lyrics_go_inside_the_song_in_the_format_the_plan_names() {
    let f = fixture();
    let mut p = plan(Format::Copy { codec: Codec::Opus });
    let lyrics = p.lyrics.as_mut().unwrap();
    lyrics.placement = LyricsPlacement::Embedded;
    lyrics.inside = Some(LyricsFile {
        format: crate::settings::LyricsFormat::Txt,
        ..LyricsFile::default()
    });
    let r = go(&Fake::default(), &f, &p).unwrap();
    assert_eq!(names(&r), [] as [PathBuf; 0]);
    let mut file = fs::File::open(f.dir.path().join("lib").join(&r.audio)).unwrap();
    let opus = OpusFile::read_from(&mut file, ParseOptions::new()).unwrap();
    assert_eq!(opus.vorbis_comments().get("LYRICS"), Some("line\nmore\n"));
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
        made: None,
        out: None,
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
    assert!(
        fake.ran("accurate_rnd+bitexact") && fake.ran("-dct"),
        "the same bytes on every processor"
    );
}

fn place(
    file: Option<&str>,
    format: ImageFormat,
    keep: bool,
    max_side: Option<u32>,
    quality: Option<u8>,
) -> CoverPlace {
    CoverPlace {
        file: file.map(str::to_string),
        format,
        keep,
        max_side,
        quality,
    }
}

#[test]
fn a_cover_goes_to_each_place_its_plan_names_each_picture_made_once() {
    let f = fixture();
    let fake = Fake::default();
    let mut p = plan(Format::Copy { codec: Codec::Opus });
    p.cover.as_mut().unwrap().out = Some(vec![
        CoverPlace::default(),
        place(Some(""), ImageFormat::Png, true, None, None),
        place(
            Some(".small"),
            ImageFormat::Webp,
            false,
            Some(300),
            Some(80),
        ),
        place(Some(".tiny"), ImageFormat::Webp, false, Some(300), Some(80)),
        place(Some(".j"), ImageFormat::Jpeg, false, None, Some(80)),
    ]);
    let r = go(&fake, &f, &p).unwrap();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(
        names(&r),
        [
            "A/Record/Song.lrc",
            "A/Record/Song.png",
            "A/Record/Song.small.webp",
            "A/Record/Song.tiny.webp",
            "A/Record/Song.j.jpg",
        ]
        .map(PathBuf::from)
    );
    let lib = f.dir.path().join("lib/A/Record");
    assert_eq!(
        fs::read(lib.join("Song.png")).unwrap(),
        crate::testing::PIXEL,
        "the attachment's own bytes"
    );
    assert_eq!(
        fs::read(lib.join("Song.tiny.webp")).unwrap(),
        crate::testing::PIXEL_WEBP
    );
    assert_eq!(
        fs::read(lib.join("Song.j.jpg")).unwrap(),
        crate::testing::PIXEL_JPEG
    );
    let args: Vec<String> = fake.calls().concat();
    let count = |a: &str| args.iter().filter(|x| *x == a).count();
    assert_eq!(count("libwebp"), 1, "places alike share one picture");
    assert!(fake.ran("scale=w='min(300,iw)':h='min(300,ih)':force_original_aspect_ratio=decrease"));
    assert_eq!(count("80"), 1, "WebP at its quality");
    assert!(fake.ran(&jpeg_scale(80).to_string()) && fake.ran("mjpeg"));
    assert_eq!(count("png"), 0, "a PNG kept as its own bytes");
    assert_eq!((jpeg_scale(100), jpeg_scale(1)), (2, 31));
    assert_eq!(leftover_parts(&lib), Vec::<PathBuf>::new());

    let mut file = fs::File::open(f.dir.path().join("lib").join(&r.audio)).unwrap();
    let opus = OpusFile::read_from(&mut file, ParseOptions::new()).unwrap();
    assert_eq!(opus.vorbis_comments().pictures().len(), 1);
    p.cover.as_mut().unwrap().out = Some(vec![place(Some(""), ImageFormat::Png, true, None, None)]);
    let r = go(&Fake::default(), &f, &p).unwrap();
    let mut file = fs::File::open(f.dir.path().join("lib").join(&r.audio)).unwrap();
    let opus = OpusFile::read_from(&mut file, ParseOptions::new()).unwrap();
    assert!(opus.vorbis_comments().pictures().is_empty(), "a file alone");
}

#[test]
fn a_cover_file_written_alone_is_the_one_a_build_writes() {
    let f = fixture();
    let mut p = plan(Format::Copy { codec: Codec::Opus });
    p.lyrics = None;
    p.cover = Some(CoverRef {
        key: yt("bbbbbbbbbbb"),
        rev: "r".into(),
        at: CoverAt::Picture { index: 3 },
        mimetype: "image/png".into(),
        crop: None,
        made: Some(COVER_MAKING),
        out: Some(vec![
            CoverPlace::default(),
            place(Some(""), ImageFormat::Png, true, None, None),
            place(Some(".j"), ImageFormat::Jpeg, false, Some(600), Some(80)),
        ]),
    });
    let built = go(&Fake::default(), &f, &p).unwrap();
    let lib = f.dir.path().join("lib");
    let claim = [
        Ledger::taken(Path::new("A/Record/Song.png")),
        Ledger::taken(Path::new("A/Record/Song.j.jpg")),
    ];
    let fake = Fake::default();
    let alone = beside(
        &fake,
        &Job {
            plan: &p,
            stem: Path::new("A/Record/Song"),
            library: &lib,
            sources: &f.sources,
            scratch: &f.dir.path().join("scratch-beside"),
            claim: &claim,
            stale: &[],
        },
    )
    .unwrap();
    assert!(alone.problems.is_empty(), "{:?}", alone.problems);
    assert_eq!(alone.files, built.beside);
    assert_eq!(fake.calls().len(), 1, "one run for the pictures");
    let args = &fake.calls()[0];
    assert!(
        !args
            .iter()
            .any(|a| a == "-c:a" || a.ends_with("Song.opus.part")),
        "the audio left alone: {args:?}"
    );
    assert_eq!(
        args.iter().filter(|a| *a == "1:3" || *a == "0:3").count(),
        2,
        "the picture stream copied once and made a JPEG once: {args:?}"
    );
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

/// A FLAC of `padding` bytes of padding after an empty stream info.
fn padded_flac(padding: u32) -> Vec<u8> {
    let mut bytes = b"fLaC".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 34]);
    bytes.extend_from_slice(&[0; 34]);
    bytes.push(FLAC_PADDING | 0x80);
    bytes.extend_from_slice(&padding.to_be_bytes()[1..]);
    bytes.resize(bytes.len() + padding as usize, 0);
    bytes
}

#[test]
fn flac_tags_and_a_cover_go_into_the_padding_or_the_file_is_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let mut comments = VorbisComments::default();
    comments.set_vendor("Lavf".to_string());
    comments.push("TITLE".to_string(), "Lantern Weather".to_string());
    comments.push("EMPTY".to_string(), String::new());
    let picture = Picture::from_reader(&mut &crate::testing::PIXEL[..]).unwrap();
    comments.insert_picture(picture, None).unwrap();
    let read = |path: &Path| {
        let mut file = fs::File::open(path).unwrap();
        FlacFile::read_from(&mut file, ParseOptions::new().read_properties(false)).unwrap()
    };

    let roomy = dir.path().join("roomy.flac");
    fs::write(&roomy, padded_flac(4096)).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&roomy)
        .unwrap();
    assert!(write_flac_blocks(&mut file, &comments).unwrap());
    drop(file);
    assert_eq!(fs::metadata(&roomy).unwrap().len(), 4 + 4 + 34 + 4 + 4096);
    let flac = read(&roomy);
    let own = flac.vorbis_comments().unwrap();
    assert_eq!(own.vendor(), "Lavf");
    assert_eq!(own.get("TITLE"), Some("Lantern Weather"));
    assert_eq!(own.get("EMPTY"), None);
    assert_eq!(flac.pictures().len(), 1);
    let bytes = fs::read(&roomy).unwrap();
    assert_eq!(bytes.get(4), Some(&0), "stream info first, not last");
    assert!(
        bytes[..bytes.len() - 32]
            .windows(1)
            .rposition(|b| b[0] == (FLAC_PADDING | 0x80))
            .is_some(),
        "padding stays, and last"
    );

    let tight = dir.path().join("tight.flac");
    let before = padded_flac(8);
    fs::write(&tight, &before).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&tight)
        .unwrap();
    assert!(!write_flac_blocks(&mut file, &comments).unwrap());
    assert_eq!(fs::read(&tight).unwrap(), before, "left as it was");
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
fn an_opus_file_with_no_speakers_named_is_tagged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("song.opus");
    fs::write(&path, crate::ogg::discrete(crate::testing::SILENCE_OPUS)).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    assert!(
        OpusFile::read_from(&mut file, ParseOptions::new()).is_err(),
        "lofty refuses mapping family 255 for its properties"
    );
    let tags = vec![("TITLE".to_string(), vec!["Song".to_string()])];
    write_tags(
        &path,
        Format::Copy { codec: Codec::Opus },
        &tags,
        None,
        None,
        &[],
    )
    .unwrap();

    let read = crate::tags::read_file(&path).unwrap();
    assert_eq!(read.get("title"), Some(&vec!["Song".to_string()]));
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes[28 + 18], 255, "the header keeps its mapping family");
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
fn an_m4a_s_chapters_are_given_to_ffmpeg_and_no_other_metadata() {
    let f = fixture();
    let fake = Fake::default();
    let mut p = plan(Format::Copy { codec: Codec::Aac });
    p.chapters = vec![
        crate::chapters::Chapter {
            start_ms: 0,
            end_ms: 500,
            title: "Part One".into(),
        },
        crate::chapters::Chapter {
            start_ms: 500,
            end_ms: 1000,
            title: String::new(),
        },
    ];
    go(&fake, &f, &p).unwrap();
    let read = |fake: &Fake| {
        fake.calls()
            .iter()
            .flatten()
            .any(|a| a.ends_with("chapters.ffmeta"))
    };
    assert!(read(&fake) && fake.ran("-map_metadata:g"));
    let opus = Fake::default();
    go(
        &opus,
        &f,
        &Plan {
            chapters: p.chapters.clone(),
            ..plan(Format::Copy { codec: Codec::Opus })
        },
    )
    .unwrap();
    assert!(!read(&opus), "written as comments, not by ffmpeg");
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
fn an_m4a_song_holds_the_values_of_one_tag_in_one_atom_and_reads_back_whole() {
    let f = fixture();
    let fake = Fake::default();
    let mut p = tagged(Format::Copy { codec: Codec::Aac });
    p.tags[1].1 = vec!["Odile Marsh".into(), "Bram Feld".into()];
    p.tags
        .push(("MOOD".into(), vec!["sleepy".into(), "drowsy".into()]));
    p.tags
        .push(("SHADE".into(), vec!["amber".into(), "slate".into()]));
    let r = go(&fake, &f, &p).unwrap();
    let path = f.dir.path().join("lib").join(&r.audio);
    let mut file = fs::File::open(&path).unwrap();
    let m4a = Mp4File::read_from(&mut file, ParseOptions::new()).unwrap();
    let mut atoms: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for atom in m4a.ilst().unwrap() {
        let name = match atom.ident() {
            lofty::mp4::AtomIdent::Fourcc(f) => f.iter().map(|&b| char::from(b)).collect(),
            lofty::mp4::AtomIdent::Freeform { name, .. } => name.to_string(),
        };
        let values = atom
            .data()
            .filter_map(|d| match d {
                lofty::mp4::AtomData::UTF8(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        atoms.entry(name).or_default().push(values);
    }
    let one = |name: &str| atoms[name].clone();
    assert_eq!(one("©ART"), [["Odile Marsh", "Bram Feld"]]);
    assert_eq!(one("©gen"), [["House", "Disco"]]);
    assert_eq!(one("MOOD"), [["sleepy", "drowsy"]]);
    assert_eq!(one("SHADE"), [["amber", "slate"]]);
    let read = crate::tags::read_file(&path).unwrap();
    assert_eq!(read["artist"], ["Odile Marsh", "Bram Feld"]);
    assert_eq!(read["genre"], ["House", "Disco"]);
    assert_eq!(read["mood"], ["sleepy", "drowsy"]);
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
fn lyrics_not_claimed_are_neither_written_over_nor_removed() {
    let f = fixture();
    let mine = f.dir.path().join("lib/A/Record/Song.lrc");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, "mine").unwrap();
    let p = plan(Format::Copy { codec: Codec::Opus });
    let e = go_claiming(&Fake::default(), &f, &p, &[]).unwrap_err();
    assert!(format!("{e:#}").contains("not muman's"), "{e:#}");
    assert_eq!(fs::read_to_string(&mine).unwrap(), "mine");
    assert!(!f.dir.path().join("lib/A/Record/Song.opus").exists());
    assert_eq!(
        leftover_parts(&f.dir.path().join("lib/A/Record")),
        Vec::<PathBuf>::new()
    );
    let none = Plan {
        lyrics: None,
        ..plan(Format::Copy { codec: Codec::Opus })
    };
    go_claiming(&Fake::default(), &f, &none, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(&mine).unwrap(),
        "mine",
        "the song writes none and its last build left none there"
    );
}

#[cfg(unix)]
#[test]
fn a_link_left_at_a_staged_name_is_not_written_through() {
    let f = fixture();
    let folder = f.dir.path().join("lib/A/Record");
    fs::create_dir_all(&folder).unwrap();
    let elsewhere = f.dir.path().join("elsewhere");
    fs::write(&elsewhere, "not the library's").unwrap();
    for name in ["Song.opus.part", "Song.lrc.part"] {
        std::os::unix::fs::symlink(&elsewhere, folder.join(name)).unwrap();
    }
    let r = go(
        &Fake::default(),
        &f,
        &plan(Format::Copy { codec: Codec::Opus }),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "not the library's");
    assert_eq!(names(&r).len(), 1);
    for file in std::iter::once(&r.audio).chain(&names(&r)) {
        let meta = fs::symlink_metadata(f.dir.path().join("lib").join(file)).unwrap();
        assert!(meta.file_type().is_file(), "{}", file.display());
    }
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
    assert_eq!(names(&r), [] as [PathBuf; 0]);
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
        files: None,
        inside: None,
        end_ms: None,
        language: None,
        clean_off: Vec::new(),
        cleaning: crate::clean::lyrics::EDITION,
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
        Format::Copy {
            codec: Codec::WavPack,
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
        let again = retag(
            &library,
            &first.audio,
            &names(&first),
            &p,
            &[Ledger::taken(&first.audio)],
        )
        .unwrap();
        assert_eq!(fake.calls().len(), calls, "{format:?}: nothing is run");
        assert_eq!(again.audio, first.audio);
        let file = lofty::read_from_path(library.join(&again.audio)).unwrap();
        let tag = file.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("Paper Comets"), "{format:?}");
        assert_eq!(tag.artist().as_deref(), Some("A, B"), "{format:?}");
        assert!(
            again.problems.is_empty(),
            "{format:?}: {:?}",
            again.problems
        );
        let pictures = match format.codec() {
            Codec::WavPack => usize::from(ape_cover(&library.join(&again.audio)).is_some()),
            _ => tag.pictures().len(),
        };
        assert_eq!(pictures, 1, "{format:?}: the cover stays");
        let lyrics = tag
            .get_string(ItemKey::Lyrics)
            .or_else(|| tag.get_string(ItemKey::UnsyncLyrics));
        assert!(
            lyrics.is_some_and(|l| l.contains("line")),
            "{format:?}: the lyrics stay"
        );
        assert_eq!(
            leftover_parts(&library.join("A/Record")),
            [] as [PathBuf; 0]
        );
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
        retag(
            &library,
            &r.audio,
            &names(&r),
            &q,
            &[Ledger::taken(&r.audio)],
        )
        .unwrap();
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

/// Tags that touch every way a field is written: several values, a place
/// and its total, a date, IDs, a gain, a role, a tag of no field and one
/// lofty would write under a frame of no text.
fn every_kind() -> Vec<(String, Vec<String>)> {
    [
        ("TITLE", vec!["Lantern Weather"]),
        ("ARTIST", vec!["Marlo Venn", "Kiri Hoshino"]),
        ("TRACKNUMBER", vec!["5"]),
        ("TRACKTOTAL", vec!["9"]),
        ("DISCNUMBER", vec!["2"]),
        ("DATE", vec!["2019-07"]),
        ("ISRC", vec!["XXZZ12400001", "XXZZ12400002"]),
        (
            "MUSICBRAINZ_TRACKID",
            vec!["00000000-0000-0000-0000-000000000001"],
        ),
        (
            "MUSICBRAINZ_ALBUMID",
            vec!["00000000-0000-0000-0000-000000000002"],
        ),
        ("REPLAYGAIN_TRACK_GAIN", vec!["-6.12 dB"]),
        ("PRODUCER", vec!["Ada Quill", "Bram Feld"]),
        ("COMPOSER", vec!["Odile Marsh"]),
        ("WORK", vec!["Harbor Suite"]),
        ("LANGUAGE", vec!["jpn"]),
        ("LYRICS", vec!["[00:01.00]the lamps go out\n"]),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.into_iter().map(str::to_string).collect()))
    .collect()
}

fn written(fixture: &[u8], name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, fixture).unwrap();
    (dir, path)
}

fn cover() -> Picture {
    Picture::unchecked(crate::testing::PIXEL.to_vec())
        .pic_type(PictureType::CoverFront)
        .mime_type(MimeType::Png)
        .build()
}

#[test]
fn an_mp3_holds_each_field_in_the_frame_picard_writes() {
    use lofty::id3::v2::{FrameId, Id3v2Version};

    let (_dir, path) = written(crate::testing::SILENCE_MP3, "song.mp3");
    let codec = Format::Copy { codec: Codec::Mp3 };
    write_tags(&path, codec, &every_kind(), Some("fra"), Some(cover()), &[]).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    let mp3 = MpegFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let id3 = mp3.id3v2().unwrap();
    assert_eq!(id3.original_version(), Id3v2Version::V4);
    let text = |id: &str| id3.get_text(&FrameId::new(id.to_string()).unwrap());
    assert_eq!(text("TPE1"), Some("Marlo Venn\0Kiri Hoshino"));
    assert_eq!(text("TRCK"), Some("5/9"));
    assert_eq!(text("TPOS"), Some("2"));
    assert_eq!(text("TSRC"), Some("XXZZ12400001\0XXZZ12400002"));
    assert_eq!(text("TCOM"), Some("Odile Marsh"));
    assert_eq!(id3.get_user_text("WORK"), Some("Harbor Suite"));
    assert_eq!(id3.get_user_text("REPLAYGAIN_TRACK_GAIN"), Some("-6.12 dB"));
    assert_eq!(
        id3.get_user_text("MusicBrainz Album Id"),
        Some("00000000-0000-0000-0000-000000000002")
    );
    let frames: Vec<&Frame<'_>> = id3.into_iter().collect();
    assert!(frames.iter().any(|f| matches!(f, Frame::Timestamp(t)
        if f.id_str() == "TDRC" && t.timestamp.to_string() == "2019-07")));
    assert!(
        frames
            .iter()
            .any(|f| matches!(f, Frame::UniqueFileIdentifier(u)
        if u.identifier.as_ref() == b"00000000-0000-0000-0000-000000000001"))
    );
    assert!(frames.iter().any(|f| matches!(f, Frame::KeyValue(k)
        if f.id_str() == "TIPL" && k.key_value_pairs.len() == 2)));
    assert!(
        frames
            .iter()
            .any(|f| matches!(f, Frame::UnsynchronizedText(u)
        if &u.language == b"fra")),
        "the lyrics' language, not the song's"
    );
    assert_eq!(frames.iter().filter(|f| f.id_str() == "APIC").count(), 1);

    let read = crate::tags::read_file(&path).unwrap();
    assert_eq!(read["artist"], ["Marlo Venn", "Kiri Hoshino"]);
    assert_eq!(read["isrc"], ["XXZZ12400001", "XXZZ12400002"]);
    assert_eq!(read["replaygain_track_gain"], ["-6.12 dB"]);
    assert_eq!(
        read["musicbrainz_trackid"],
        ["00000000-0000-0000-0000-000000000001"]
    );
}

#[test]
fn an_m4a_holds_each_field_in_the_atom_picard_writes() {
    use lofty::mp4::{AtomData, AtomIdent};

    let (_dir, path) = written(crate::testing::SILENCE_M4A, "song.m4a");
    let codec = Format::Copy { codec: Codec::Aac };
    write_tags(&path, codec, &every_kind(), None, Some(cover()), &[]).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    let m4a = Mp4File::read_from(&mut file, ParseOptions::new()).unwrap();
    let ilst = m4a.ilst().unwrap();
    assert_eq!((ilst.track(), ilst.track_total()), (Some(5), Some(9)));
    assert_eq!(ilst.disk(), Some(2));
    let free = |name: &str| -> Vec<String> {
        ilst.get(&AtomIdent::Freeform {
            mean: "com.apple.iTunes".into(),
            name: name.to_string().into(),
        })
        .map(|a| {
            a.data()
                .filter_map(|d| match d {
                    AtomData::UTF8(s) => Some(s.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
    };
    assert_eq!(
        free("REPLAYGAIN_TRACK_GAIN"),
        ["-6.12 dB"],
        "Picard's capitals"
    );
    assert_eq!(free("ISRC"), ["XXZZ12400001", "XXZZ12400002"]);
    assert_eq!(
        free("MusicBrainz Track Id"),
        ["00000000-0000-0000-0000-000000000001"]
    );
    let wrt = ilst.get(&AtomIdent::Fourcc(*b"\xa9wrt")).unwrap();
    assert!(
        wrt.data()
            .any(|d| matches!(d, AtomData::UTF8(s) if s == "Odile Marsh"))
    );
    assert!(ilst.get(&AtomIdent::Fourcc(*b"\xa9lyr")).is_some());
    assert_eq!(ilst.pictures().unwrap().count(), 1);

    let read = crate::tags::read_file(&path).unwrap();
    assert_eq!(read["tracknumber"], ["5"]);
    assert_eq!(read["tracktotal"], ["9"]);
    assert_eq!(read["replaygain_track_gain"], ["-6.12 dB"]);
}

#[test]
fn a_wavpack_file_holds_each_field_in_the_item_picard_writes() {
    let (_dir, path) = written(crate::testing::SILENCE_WV, "song.wv");
    let codec = Format::Copy {
        codec: Codec::WavPack,
    };
    write_tags(&path, codec, &every_kind(), None, Some(cover()), &[]).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    let wv = lofty::wavpack::WavPackFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let ape = wv.ape().unwrap();
    let text = |key: &str| match ape.get(key).map(lofty::ape::ApeItem::value) {
        Some(ItemValue::Text(t)) => Some(t.clone()),
        _ => None,
    };
    assert_eq!(text("Track").as_deref(), Some("5/9"));
    assert_eq!(text("Disc").as_deref(), Some("2"));
    assert_eq!(text("Year").as_deref(), Some("2019-07"));
    assert_eq!(text("ISRC").as_deref(), Some("XXZZ12400001\0XXZZ12400002"));
    assert_eq!(text("Artist").as_deref(), Some("Marlo Venn\0Kiri Hoshino"));
    assert!(text("TRACKTOTAL").is_none(), "the total is in Track");
    match ape.get("Cover Art (Front)").map(lofty::ape::ApeItem::value) {
        Some(ItemValue::Binary(b)) => {
            assert!(
                b.starts_with(b"cover.png\0"),
                "named so readers tell its format"
            );
        }
        other => panic!("no cover: {other:?}"),
    }

    let read = crate::tags::read_file(&path).unwrap();
    assert_eq!(read["tracknumber"], ["5/9"]);
    assert_eq!(read["artist"], ["Marlo Venn", "Kiri Hoshino"]);
}

#[test]
fn a_total_with_no_place_is_kept_where_a_place_cannot_hold_it() {
    let tags = vec![("TRACKTOTAL".to_string(), vec!["9".to_string()])];
    let (_dir, path) = written(crate::testing::SILENCE_MP3, "song.mp3");
    write_tags(
        &path,
        Format::Copy { codec: Codec::Mp3 },
        &tags,
        None,
        None,
        &[],
    )
    .unwrap();
    let mut file = fs::File::open(&path).unwrap();
    let mp3 = MpegFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let id3 = mp3.id3v2().unwrap();
    assert_eq!(id3.get_user_text("TRACKTOTAL"), Some("9"));
    assert!(
        id3.get_text(&lofty::id3::v2::FrameId::new("TRCK").unwrap())
            .is_none()
    );
}

#[test]
fn a_cover_or_a_tag_the_file_does_not_hold_as_written_is_told() {
    let (_dir, path) = written(crate::testing::SILENCE_WV, "song.wv");
    let tags = vec![("TITLE".to_string(), vec!["Lantern Weather".to_string()])];
    let format = Format::Copy {
        codec: Codec::WavPack,
    };
    write_tags(&path, format, &tags, None, Some(cover()), &[]).unwrap();
    assert_eq!(verify(&path, &tags, true), Vec::<String>::new());
    write_tags(&path, format, &tags, None, None, &[]).unwrap();
    assert_eq!(verify(&path, &tags, true), ["cover not held as written"]);
    let more = vec![
        ("TITLE".to_string(), vec!["Lantern Weather".to_string()]),
        ("ALBUM".to_string(), vec!["Glass Orchards".to_string()]),
    ];
    assert_eq!(
        verify(&path, &more, false),
        ["tags not held as written: ALBUM"]
    );
}

#[test]
fn a_cover_too_large_for_any_container_is_made_smaller() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.png");
    let mut bytes = crate::testing::PIXEL.to_vec();
    bytes.resize(FLAC_BLOCK + 1, 0);
    fs::write(&path, &bytes).unwrap();
    let huge = read_picture(&path, ImageFormat::Png).unwrap();
    let fake = Fake::default();
    let fitted = fit_cover(&fake, huge, FLAC_BLOCK, &path, dir.path()).unwrap();
    assert!(fitted.data().len() <= FLAC_BLOCK);
    assert!(fake.ran("mjpeg"));
}

#[test]
fn a_cover_is_held_to_what_each_container_leaves_it() {
    let lyrics = vec![("LYRICS".to_string(), vec!["x".repeat(1 << 20)])];
    assert_eq!(cover_room(Container::Flac, &lyrics), FLAC_BLOCK);
    let ogg = cover_room(Container::Ogg, &[]);
    assert!(
        (ogg + 1024).div_ceil(3) * 4 < 1 << 24,
        "its base64 is read by lofty"
    );
    assert!(cover_room(Container::WavPack, &lyrics) < FLAC_BLOCK - (1 << 20));
}

/// A value of `spec`'s kind as a song list sets it; none for a measure.
fn sample(spec: &crate::tags::Spec) -> Option<&'static str> {
    use crate::tags::Kind as K;
    Some(match spec.kind {
        K::Text => "Lantern Weather",
        K::Count => "12",
        K::Place => "3",
        K::Date => "2001-05-03",
        K::Isrc => "XXZZ12400001",
        K::Mbid => "00000000-0000-4000-8000-0000000000aa",
        K::Decimal => "127.5",
        K::Flag => "1",
        K::MusicalKey => "Abm",
        K::Barcode => "0012345678905",
        K::Language => "eng",
        K::Gain | K::Peak => return None,
    })
}

#[test]
fn every_field_is_read_back_from_every_container_as_written() {
    use crate::testing::{
        SILENCE_FLAC, SILENCE_M4A, SILENCE_MP3, SILENCE_OPUS, SILENCE_VORBIS, SILENCE_WV,
    };

    let tags: Vec<(String, Vec<String>)> = crate::tags::TABLE
        .iter()
        .filter_map(|spec| Some((spec.vorbis.to_string(), vec![sample(spec)?.to_string()])))
        .collect();
    for (fixture, name, codec) in [
        (SILENCE_FLAC, "song.flac", Codec::Flac),
        (SILENCE_OPUS, "song.opus", Codec::Opus),
        (SILENCE_VORBIS, "song.ogg", Codec::Vorbis),
        (SILENCE_MP3, "song.mp3", Codec::Mp3),
        (SILENCE_M4A, "song.m4a", Codec::Aac),
        (SILENCE_WV, "song.wv", Codec::WavPack),
    ] {
        let (_dir, path) = written(fixture, name);
        write_tags(&path, Format::Copy { codec }, &tags, None, None, &[]).unwrap();
        assert_eq!(verify(&path, &tags, false), [] as [String; 0], "{name}");
    }
}

#[test]
fn chapters_are_written_in_each_container_s_own_form() {
    use crate::chapters::Chapter;
    use lofty::id3::v2::Frame;

    let chapters = [
        Chapter {
            start_ms: 0,
            end_ms: 400,
            title: "Part One".into(),
        },
        Chapter {
            start_ms: 400,
            end_ms: 900,
            title: "Part Two".into(),
        },
    ];
    let title = vec![("TITLE".to_string(), vec!["Lantern Weather".to_string()])];
    let (_d1, flac) = written(crate::testing::SILENCE_FLAC, "song.flac");
    write_tags(
        &flac,
        Format::Copy { codec: Codec::Flac },
        &title,
        None,
        None,
        &chapters,
    )
    .unwrap();
    let mut file = fs::File::open(&flac).unwrap();
    let read = FlacFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let comments = read.vorbis_comments().unwrap();
    assert_eq!(comments.get("CHAPTER001"), Some("00:00:00.400"));
    assert_eq!(comments.get("CHAPTER001NAME"), Some("Part Two"));

    let (_d2, mp3) = written(crate::testing::SILENCE_MP3, "song.mp3");
    write_tags(
        &mp3,
        Format::Copy { codec: Codec::Mp3 },
        &title,
        None,
        None,
        &chapters,
    )
    .unwrap();
    let mut file = fs::File::open(&mp3).unwrap();
    let id3 = MpegFile::read_from(&mut file, ParseOptions::new())
        .unwrap()
        .id3v2()
        .unwrap()
        .clone();
    let frames: Vec<Frame<'_>> = id3.into_iter().collect();
    let toc = frames.iter().find_map(|f| match f {
        Frame::TableOfContents(t) => Some(t.entries.len()),
        _ => None,
    });
    assert_eq!(toc, Some(2));
    let times: Vec<std::ops::Range<u32>> = frames
        .iter()
        .filter_map(|f| match f {
            Frame::Chapter(c) => Some(c.times.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(times, [0..400, 400..900]);
    assert_eq!(verify(&mp3, &title, false), [] as [String; 0]);
}

#[test]
fn a_flac_tagged_after_none_names_the_vendor_a_build_does() {
    let (_dir, path) = written(crate::testing::SILENCE_FLAC, "song.flac");
    let flac = Format::Copy { codec: Codec::Flac };
    write_tags(&path, flac, &[], None, None, &[]).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    let bare = FlacFile::read_from(&mut file, ParseOptions::new()).unwrap();
    assert!(bare.vorbis_comments().is_none(), "no tags, no block");
    let title = vec![("TITLE".to_string(), vec!["Lantern Weather".to_string()])];
    write_tags(&path, flac, &title, None, None, &[]).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    let read = FlacFile::read_from(&mut file, ParseOptions::new()).unwrap();
    assert_eq!(read.vorbis_comments().unwrap().vendor(), FFMPEG_VENDOR);
}

#[test]
fn an_id3v2_3_artist_is_read_whole_and_a_genre_number_by_its_name() {
    use lofty::config::WriteOptions;
    use lofty::id3::v2::{FrameId, TextInformationFrame};
    use lofty::tag::TagExt;

    let (_dir, path) = written(crate::testing::SILENCE_MP3, "song.mp3");
    let mut tag = Id3v2Tag::new();
    for (id, value) in [("TPE1", "Ash/Ember"), ("TCON", "17\0Night Static")] {
        tag.insert(Frame::Text(TextInformationFrame::new(
            FrameId::new(id).unwrap(),
            lofty::TextEncoding::UTF16,
            value.to_string(),
        )));
    }
    tag.save_to_path(&path, WriteOptions::new().use_id3v23(true))
        .unwrap();
    let read = crate::tags::read_file(&path).unwrap();
    assert_eq!(read["artist"], ["Ash/Ember"], "Picard reads it as one");
    assert_eq!(read["genre"], ["Rock", "Night Static"]);
}

fn extended() -> Vec<(String, Vec<String>)> {
    [
        ("TITLE", vec!["Lantern Weather"]),
        ("BPM", vec!["127.5"]),
        ("KEY", vec!["Abm"]),
        ("COMPILATION", vec!["1"]),
        ("LABEL", vec!["Harbor Lights Records"]),
        ("BARCODE", vec!["0012345678905"]),
        ("ARTISTSORT", vec!["Venn, Marlo"]),
        ("ORIGINALDATE", vec!["2001-05-03"]),
        ("GROUPING", vec!["Night Songs"]),
        ("COMMENT", vec!["take two"]),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.into_iter().map(str::to_string).collect()))
    .collect()
}

#[test]
fn extended_tags_are_written_as_each_container_types_them_and_read_back() {
    use lofty::id3::v2::FrameId;
    use lofty::mp4::{AtomData, AtomIdent};

    let (_d1, mp3) = written(crate::testing::SILENCE_MP3, "song.mp3");
    write_tags(
        &mp3,
        Format::Copy { codec: Codec::Mp3 },
        &extended(),
        None,
        None,
        &[],
    )
    .unwrap();
    let mut file = fs::File::open(&mp3).unwrap();
    let id3 = MpegFile::read_from(&mut file, ParseOptions::new())
        .unwrap()
        .id3v2()
        .unwrap()
        .clone();
    let text = |id: &str| {
        id3.get_text(&FrameId::new(id.to_string()).unwrap())
            .map(str::to_string)
    };
    assert_eq!(
        text("TBPM").as_deref(),
        Some("128"),
        "TBPM holds a whole number"
    );
    assert_eq!(text("TKEY").as_deref(), Some("Abm"));
    assert_eq!(text("TCMP").as_deref(), Some("1"));
    assert_eq!(text("TPUB").as_deref(), Some("Harbor Lights Records"));
    assert_eq!(text("TSOP").as_deref(), Some("Venn, Marlo"));
    assert_eq!(text("TIT1").as_deref(), Some("Night Songs"));
    assert!((&id3).into_iter().any(|f| matches!(f, Frame::Comment(c)
        if &c.language == b"eng" && c.content == "take two")));
    assert_eq!(id3.get_user_text("BARCODE"), Some("0012345678905"));
    assert_eq!(verify(&mp3, &extended(), false), [] as [String; 0]);

    let (_d2, m4a) = written(crate::testing::SILENCE_M4A, "song.m4a");
    write_tags(
        &m4a,
        Format::Copy { codec: Codec::Aac },
        &extended(),
        None,
        None,
        &[],
    )
    .unwrap();
    let mut file = fs::File::open(&m4a).unwrap();
    let m = Mp4File::read_from(&mut file, ParseOptions::new()).unwrap();
    let ilst = m.ilst().unwrap();
    let tmpo = ilst.get(&AtomIdent::Fourcc(*b"tmpo")).unwrap();
    assert!(
        tmpo.data().any(
            |d| matches!(d, AtomData::Unknown { data, .. } if data == &[0, 128])
                || matches!(d, AtomData::SignedInteger(128))
        ),
        "tmpo is a 16-bit integer"
    );
    let cpil = ilst.get(&AtomIdent::Fourcc(*b"cpil")).unwrap();
    assert!(cpil.data().any(|d| matches!(d, AtomData::Bool(true))
        || matches!(d, AtomData::Unknown { data, .. } if data == &[1])));
    assert_eq!(verify(&m4a, &extended(), false), [] as [String; 0]);

    let (_d3, opus) = written(crate::testing::SILENCE_OPUS, "song.opus");
    write_tags(
        &opus,
        Format::Copy { codec: Codec::Opus },
        &extended(),
        None,
        None,
        &[],
    )
    .unwrap();
    let mut file = fs::File::open(&opus).unwrap();
    let ogg = OpusFile::read_from(&mut file, ParseOptions::new()).unwrap();
    let c = ogg.vorbis_comments();
    assert_eq!(c.get("KEY"), Some("Abm"));
    assert_eq!(c.get("INITIALKEY"), Some("Abm"), "as DJ tools name it");
    assert_eq!(c.get("BPM"), Some("127.5"));

    for path in [&mp3, &m4a, &opus] {
        let offers = crate::tags::from_container(&crate::tags::read_file(path).unwrap());
        assert_eq!(
            offers[&Field::InitialKey].values,
            ["Abm"],
            "{}",
            path.display()
        );
        assert_eq!(
            offers[&Field::Compilation].values,
            ["1"],
            "{}",
            path.display()
        );
        assert_eq!(offers[&Field::Label].values, ["Harbor Lights Records"]);
        assert_eq!(offers[&Field::OriginalDate].values, ["2001-05-03"]);
        assert!(offers.contains_key(&Field::Bpm), "{}", path.display());
    }
}

#[test]
fn a_key_tkey_cannot_hold_is_kept_as_its_vorbis_name() {
    use lofty::id3::v2::FrameId;

    let tags = vec![("KEY".to_string(), vec!["8A".to_string()])];
    let (_dir, mp3) = written(crate::testing::SILENCE_MP3, "song.mp3");
    write_tags(
        &mp3,
        Format::Copy { codec: Codec::Mp3 },
        &tags,
        None,
        None,
        &[],
    )
    .unwrap();
    let mut file = fs::File::open(&mp3).unwrap();
    let id3 = MpegFile::read_from(&mut file, ParseOptions::new())
        .unwrap()
        .id3v2()
        .unwrap()
        .clone();
    assert!(id3.get_text(&FrameId::new("TKEY").unwrap()).is_none());
    assert_eq!(id3.get_user_text("INITIALKEY"), Some("8A"));
    assert_eq!(verify(&mp3, &tags, false), [] as [String; 0]);
}
