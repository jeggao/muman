use super::*;
use crate::codec::Codec;

fn opus() -> Plan {
    crate::testing::every_part(Format::Copy { codec: Codec::Opus })
}

/// What each change to `base` writes again, by the change's name.
fn redone(base: &Plan) -> Vec<(&'static str, Redo)> {
    crate::testing::mutations()
        .into_iter()
        .map(|(name, change)| {
            let mut plan = base.clone();
            change(&mut plan);
            (name, parts(&plan).redo_from(&parts(base)))
        })
        .collect()
}

const B: Redo = Redo {
    build: true,
    retag: false,
    beside: false,
};
const T: Redo = Redo {
    build: false,
    retag: true,
    beside: false,
};
const F: Redo = Redo {
    build: false,
    retag: false,
    beside: true,
};
const NONE: Redo = Redo {
    build: false,
    retag: false,
    beside: false,
};

const fn and(a: Redo, b: Redo) -> Redo {
    Redo {
        build: a.build || b.build,
        retag: a.retag || b.retag,
        beside: a.beside || b.beside,
    }
}

#[test]
fn each_change_lands_in_the_part_that_writes_it() {
    let both = and(B, F);
    let lyric = and(T, F);
    let expected: Vec<(&str, Redo)> = vec![
        ("version", B),
        ("format", B),
        ("audio.key", B),
        ("audio.rev", B),
        ("audio.index", B),
        ("cover", both),
        ("cover.rev", both),
        ("cover.at", both),
        ("cover.mimetype", both),
        ("cover.crop", both),
        ("cover.made", both),
        // The file beside's place alone.
        ("cover.out", F),
        // The lyrics inside are the tags', and so a retag's.
        ("lyrics", lyric),
        ("lyrics.rev", lyric),
        ("lyrics.at", lyric),
        ("lyrics.shift_ms", lyric),
        ("lyrics.stretch_ppm", lyric),
        // The files beside are named, so only the lyrics inside go.
        ("lyrics.placement", T),
        ("lyrics.files", F),
        ("lyrics.inside", T),
        // A file of cues' end, which the lyrics inside never read.
        ("lyrics.end_ms", F),
        ("lyrics.language", T),
        ("lyrics.clean_off", lyric),
        ("lyrics.cleaning", lyric),
        ("lyrics.writing", lyric),
        // The title, which the lyrics' cleaning reads.
        ("tags", lyric),
        ("tags.order", T),
        ("loudness", T),
        ("loudness.track", T),
        ("loudness.album", T),
        // A copy holds no gain in its samples.
        ("loudness.apply", NONE),
        ("loudness.opus_gain", T),
        ("loudness.bits", T),
        ("tagging", T),
        // Opus holds chapters in its tags.
        ("chapters", T),
        ("folder", F),
    ];
    assert_eq!(redone(&opus()), expected);
}

#[test]
fn an_mp4_s_chapters_are_built_and_another_s_tagged() {
    let chapters = |codec| {
        redone(&crate::testing::every_part(Format::Encode {
            codec,
            kbps: Some(256),
            adapt: None,
            mix: None,
        }))
        .into_iter()
        .find(|(name, _)| *name == "chapters")
        .unwrap()
        .1
    };
    assert_eq!(chapters(Codec::Aac), B);
    assert_eq!(chapters(Codec::Mp3), T);
}

#[test]
fn a_plan_s_parts_are_equal_where_the_plan_is() {
    for format in [
        Format::Copy { codec: Codec::Opus },
        Format::Copy { codec: Codec::Flac },
        Format::Encode {
            codec: Codec::Aac,
            kbps: Some(256),
            adapt: None,
            mix: None,
        },
    ] {
        let base = crate::testing::every_part(format);
        for (name, change) in crate::testing::mutations() {
            let mut plan = base.clone();
            change(&mut plan);
            assert_eq!(
                plan == base,
                parts(&plan) == parts(&base),
                "{format:?}: {name}"
            );
        }
    }
}

/// The projections a sync decided by before these parts, as they were,
/// to check the parts against: `media_equal` without the lyrics inside,
/// which a retag now writes.
mod before {
    use std::path::{Path, PathBuf};

    use crate::resolve::{CoverRef, LyricsRef, Plan};
    use crate::settings::LyricsPlacement;

    type Names<'a> = Option<(Option<&'a str>, Vec<&'a str>)>;

    pub(super) fn media_equal(a: &Plan, b: &Plan) -> bool {
        a.version == b.version
            && a.format == b.format
            && a.chain() == b.chain()
            && (a.opus_header().is_none() || b.opus_header().is_some())
            && a.audio == b.audio
            && cover_inside(a) == cover_inside(b)
            && a.chapters == b.chapters
    }

    pub(super) fn beside_equal(a: &Plan, b: &Plan) -> bool {
        let beside = |p| lyrics_held(p).filter(|(l, _)| !l.files().is_empty());
        let covers = |p: &Plan| p.cover.as_ref().and_then(|c| c.only(|p| p.file.is_some()));
        beside(a) == beside(b) && covers(a) == covers(b) && a.folder == b.folder
    }

    fn cover_inside(plan: &Plan) -> Option<CoverRef> {
        plan.cover.as_ref()?.only(|p| p.file.is_none())
    }

    fn lyrics_held(plan: &Plan) -> Option<(LyricsRef, Names<'_>)> {
        let lyrics = plan.lyrics.as_ref()?;
        let held = LyricsRef {
            placement: LyricsPlacement::Both,
            files: Some(lyrics.files()),
            ..lyrics.lines()
        };
        Some((held, lyrics_names(plan)))
    }

    pub(super) fn planned(audio: &Path, plan: &Plan) -> Vec<PathBuf> {
        let stem = audio.with_extension("");
        let lyrics = plan
            .lyrics
            .iter()
            .flat_map(LyricsRef::files)
            .map(|f| crate::outputs::lyrics_file(&stem, &f));
        let covers = plan
            .cover
            .iter()
            .flat_map(CoverRef::files)
            .filter_map(|p| crate::outputs::cover_file(&stem, &p));
        let folder = plan
            .folder
            .iter()
            .flat_map(CoverRef::files)
            .filter_map(|p| crate::outputs::folder_file(&stem, &p));
        lyrics.chain(covers).chain(folder).collect()
    }

    pub(super) fn besides_audio(plan: &Plan) -> (u64, Vec<u64>) {
        let tags: usize = plan
            .tags
            .iter()
            .flat_map(|(k, values)| values.iter().map(move |v| k.len() + v.len() + 8))
            .sum();
        let cover = plan.cover.as_ref();
        let inside = cover.and_then(CoverRef::inside).map_or(0, |_| 200 * 1024);
        let lyrics = plan.lyrics.as_ref();
        let embedded = lyrics.filter(|l| l.tag()).map_or(0, |_| 3 * 1024);
        let files = lyrics.map_or(0, |l| l.files().len());
        let pictures = cover.map_or(0, |c| c.files().len());
        let folder = plan.folder.as_ref().map_or(0, |f| f.files().len());
        let mut beside = vec![3 * 1024; files];
        beside.extend(vec![200 * 1024; pictures + folder]);
        (tags as u64 + inside + embedded + 4 * 1024, beside)
    }

    fn lyrics_names(plan: &Plan) -> Names<'_> {
        let lyrics = plan.lyrics.as_ref()?;
        let reads = crate::clean::lyrics::NAMED
            .iter()
            .any(|rule| !lyrics.clean_off.iter().any(|off| off == rule));
        let values = |name: &'static str| {
            plan.tags
                .iter()
                .filter(move |(k, _)| k == name)
                .flat_map(|(_, v)| v.iter().map(String::as_str))
        };
        reads.then(|| {
            (
                values("TITLE").next(),
                values("ARTIST").chain(values("ARTISTS")).collect(),
            )
        })
    }
}

#[test]
fn the_parts_decide_as_the_projections_they_replace_but_where_named() {
    let mut differ = Vec::new();
    for format in [
        Format::Copy { codec: Codec::Opus },
        Format::Copy { codec: Codec::Flac },
        Format::Encode {
            codec: Codec::Aac,
            kbps: Some(256),
            adapt: None,
            mix: None,
        },
    ] {
        let base = crate::testing::every_part(format);
        for (name, change) in crate::testing::mutations() {
            let mut plan = base.clone();
            change(&mut plan);
            let now = redo(&base, &plan);
            let build = !before::media_equal(&base, &plan);
            let beside = !before::beside_equal(&base, &plan);
            if (now.build, now.beside) != (build, beside) {
                differ.push((format.codec(), name, now.build, now.beside));
            }
        }
    }
    let expected = vec![
        // The form of the lyrics inside, which no file beside reads.
        (Codec::Opus, "lyrics.inside", false, false),
        // Chapters, which Opus and FLAC hold in their tags.
        (Codec::Opus, "chapters", false, false),
        (Codec::Flac, "lyrics.inside", false, false),
        (Codec::Flac, "chapters", false, false),
        (Codec::Aac, "lyrics.inside", false, false),
    ];
    assert_eq!(differ, expected);
}

#[test]
fn the_lyrics_are_cleaned_by_the_names_their_part_holds() {
    let base = opus();
    let made = parts(&base);
    let named = made.lyric_text().unwrap().context();
    assert_eq!(named.title, Some("Lantern Weather"));
    assert_eq!(named.artists, ["Marlo Venn", "Kiri Hoshino"]);
    assert!(!named.subtitle);
    let mut plan = base.clone();
    let lyrics = plan.lyrics.as_mut().unwrap();
    lyrics.clean_off = crate::clean::lyrics::NAMED.map(String::from).to_vec();
    lyrics.at = LyricsAt::Stream { index: 2 };
    let made = parts(&plan);
    let unnamed = made.lyric_text().unwrap().context();
    assert_eq!(unnamed.title, None);
    assert_eq!(unnamed.artists, Vec::<&str>::new());
    assert!(unnamed.subtitle);
}

#[test]
fn a_cover_s_parts_name_its_places_inside_then_beside_and_the_folder_s_apart() {
    let plan = opus();
    let made = parts(&plan);
    let places = |covers: Vec<&CoverPart<'_>>| {
        covers
            .into_iter()
            .map(|c| c.place.file.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        places(made.covers().collect()),
        [None, Some(".front".to_string())]
    );
    assert_eq!(
        places(made.covers_beside().collect()),
        [Some(".front".to_string())]
    );
    assert_eq!(
        places(made.folder_covers().collect()),
        [Some("folder".to_string())]
    );
}

#[test]
fn the_parts_name_and_guess_the_files_beside_as_the_plan_did() {
    let audio = std::path::Path::new("Marlo Venn/The Glass Orchards/05 Lantern Weather.opus");
    for format in [
        Format::Copy { codec: Codec::Opus },
        Format::Copy { codec: Codec::Flac },
    ] {
        let base = crate::testing::every_part(format);
        for (name, change) in crate::testing::mutations() {
            let mut plan = base.clone();
            change(&mut plan);
            let made = parts(&plan);
            assert_eq!(
                made.files(audio),
                before::planned(audio, &plan),
                "{format:?}: {name}"
            );
            assert_eq!(
                made.guess(),
                before::besides_audio(&plan),
                "{format:?}: {name}"
            );
        }
    }
    let plan = opus();
    let made = parts(&plan);
    let folder = made.folder_files(audio);
    assert_eq!(
        folder,
        [std::path::PathBuf::from(
            "Marlo Venn/The Glass Orchards/folder.png"
        )]
    );
    assert!(made.files(audio).ends_with(&folder));
}
