//! Plans holding every part, and the change of each field of one, for
//! the tests of how plans are compared, keyed and rendered: a test that
//! goes through [`mutations`] sees a field added to a plan without a
//! mutation of its own fail. Compiled for tests alone, or with the
//! `testing` feature.

/// A plan in `format` with every part a plan can hold filled: a cropped
/// cover going to the tag and a file, lyrics moved, stretched and going
/// inside the song and beside it, gains, chapters and a folder cover.
#[must_use]
pub fn every_part(format: crate::resolve::Format) -> crate::resolve::Plan {
    use crate::facts::CoverAt;
    use crate::loudness::{Apply, Gains, Level};
    use crate::resolve::{AudioRef, CoverPlace, CoverRef, ImageFormat, Plan, render_version};
    use crate::source::SourceKey;

    let key = SourceKey::youtube("aaaaaaaaaaa");
    let cover = CoverRef {
        key: key.clone(),
        rev: "r1".into(),
        at: CoverAt::Attachment { ordinal: 1 },
        mimetype: "image/webp".into(),
        crop: Some(crate::quality::Rect {
            x: 2,
            y: 4,
            width: 60,
            height: 56,
        }),
        made: Some(crate::render::COVER_MAKING),
        out: Some(vec![
            CoverPlace::default(),
            CoverPlace {
                file: Some(".front".into()),
                format: ImageFormat::Jpeg,
                keep: false,
                max_side: Some(600),
                quality: Some(90),
            },
        ]),
    };
    Plan {
        version: render_version(format.codec()),
        format,
        audio: AudioRef {
            key: key.clone(),
            rev: "r1".into(),
            index: 1,
        },
        cover: Some(cover.clone()),
        lyrics: Some(every_lyrics()),
        tags: vec![
            ("TITLE".into(), vec!["Lantern Weather".into()]),
            (
                "ARTIST".into(),
                vec!["Marlo Venn".into(), "Kiri Hoshino".into()],
            ),
            ("ALBUM".into(), vec!["The Glass Orchards".into()]),
            ("TRACKNUMBER".into(), vec!["5".into()]),
        ],
        loudness: Some(Gains {
            track: Level {
                gain: -612,
                peak: Some(912_000),
            },
            album: Some(Level {
                gain: -580,
                peak: None,
            }),
            apply: Apply::Tags,
            opus_gain: 0,
            bits: 16,
        }),
        tagging: crate::resolve::TAGGING,
        chapters: vec![
            crate::chapters::Chapter {
                start_ms: 0,
                end_ms: 60_000,
                title: "Harbor".into(),
            },
            crate::chapters::Chapter {
                start_ms: 60_000,
                end_ms: 221_000,
                title: String::new(),
            },
        ],
        folder: Some(CoverRef {
            out: Some(vec![CoverPlace {
                file: Some("folder".into()),
                format: ImageFormat::Png,
                keep: true,
                max_side: None,
                quality: None,
            }]),
            ..cover
        }),
    }
}

/// Lyrics [`every_part`] holds: moved, stretched, cleaned with a rule off,
/// and written inside the song and to two files beside it.
fn every_lyrics() -> crate::resolve::LyricsRef {
    use crate::facts::LyricsAt;
    use crate::resolve::{LyricsFile, LyricsRef};
    use crate::settings::{LyricsFormat, LyricsPlacement};
    use crate::source::SourceKey;

    LyricsRef {
        key: SourceKey::Manual("Lantern Weather.lrc".into()),
        rev: "r2".into(),
        at: LyricsAt::File,
        shift_ms: 250,
        stretch_ppm: 1200,
        placement: LyricsPlacement::Both,
        files: Some(vec![
            LyricsFile::default(),
            LyricsFile {
                suffix: ".en".into(),
                format: LyricsFormat::Vtt,
                words: false,
            },
        ]),
        inside: Some(LyricsFile {
            suffix: String::new(),
            format: LyricsFormat::Txt,
            words: false,
        }),
        end_ms: Some(221_000),
        language: Some("eng".into()),
        clean_off: vec!["lyrics.packaging.credits".into()],
        cleaning: crate::clean::lyrics::EDITION,
        writing: crate::lyrics::WRITING,
    }
}

/// A change to one part of a plan, by the part's name.
pub type Mutation = (&'static str, fn(&mut crate::resolve::Plan));

/// A change to each part of a plan [`every_part`] fills.
#[must_use]
pub fn mutations() -> Vec<Mutation> {
    use crate::resolve::Plan;

    fn cover(p: &mut Plan) -> &mut crate::resolve::CoverRef {
        p.cover.as_mut().unwrap()
    }
    fn lyrics(p: &mut Plan) -> &mut crate::resolve::LyricsRef {
        p.lyrics.as_mut().unwrap()
    }
    fn gains(p: &mut Plan) -> &mut crate::loudness::Gains {
        p.loudness.as_mut().unwrap()
    }
    vec![
        ("version", |p| p.version += 1),
        ("format", |p| {
            p.format = crate::resolve::Format::Copy {
                codec: crate::codec::Codec::WavPack,
            };
        }),
        ("audio.key", |p| {
            p.audio.key = crate::source::SourceKey::youtube("bbbbbbbbbbb");
        }),
        ("audio.rev", |p| p.audio.rev = "r9".into()),
        ("audio.index", |p| p.audio.index = 2),
        ("cover", |p| p.cover = None),
        ("cover.rev", |p| cover(p).rev = "r9".into()),
        ("cover.at", |p| {
            cover(p).at = crate::facts::CoverAt::Picture { index: 3 }
        }),
        ("cover.mimetype", |p| cover(p).mimetype = "image/png".into()),
        ("cover.crop", |p| cover(p).crop = None),
        ("cover.made", |p| cover(p).made = None),
        ("cover.out", |p| {
            cover(p).out.as_mut().unwrap()[1].max_side = Some(1200);
        }),
        ("lyrics", |p| p.lyrics = None),
        ("lyrics.rev", |p| lyrics(p).rev = "r9".into()),
        ("lyrics.at", |p| {
            lyrics(p).at = crate::facts::LyricsAt::Sidecar;
        }),
        ("lyrics.shift_ms", |p| lyrics(p).shift_ms = -250),
        ("lyrics.stretch_ppm", |p| lyrics(p).stretch_ppm = 0),
        ("lyrics.placement", |p| {
            lyrics(p).placement = crate::settings::LyricsPlacement::Sidecar;
        }),
        ("lyrics.files", |p| {
            lyrics(p).files.as_mut().unwrap()[1].words = true;
        }),
        ("lyrics.inside", |p| lyrics(p).inside = None),
        ("lyrics.end_ms", |p| lyrics(p).end_ms = Some(220_000)),
        ("lyrics.language", |p| {
            lyrics(p).language = Some("jpn".into());
        }),
        ("lyrics.clean_off", |p| lyrics(p).clean_off.clear()),
        ("lyrics.cleaning", |p| lyrics(p).cleaning += 1),
        ("lyrics.writing", |p| lyrics(p).writing += 1),
        ("tags", |p| p.tags[0].1 = vec!["Paper Comets".into()]),
        ("tags.order", |p| p.tags.swap(0, 1)),
        ("loudness", |p| p.loudness = None),
        ("loudness.track", |p| gains(p).track.gain = -700),
        ("loudness.album", |p| gains(p).album = None),
        ("loudness.apply", |p| {
            gains(p).apply = crate::loudness::Apply::Volume(-612);
        }),
        ("loudness.opus_gain", |p| gains(p).opus_gain = 256),
        ("loudness.bits", |p| gains(p).bits = 24),
        ("tagging", |p| p.tagging += 1),
        ("chapters", |p| p.chapters[0].title = "Lantern".into()),
        ("folder", |p| p.folder = None),
    ]
}
