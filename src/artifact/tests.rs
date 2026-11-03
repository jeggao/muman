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
        ("lyrics", and(both, T)),
        ("lyrics.rev", both),
        ("lyrics.at", both),
        ("lyrics.shift_ms", both),
        ("lyrics.stretch_ppm", both),
        // The files beside are named, so only the lyrics inside go.
        ("lyrics.placement", B),
        ("lyrics.files", F),
        ("lyrics.inside", B),
        // A file of cues' end, which the lyrics inside never read.
        ("lyrics.end_ms", F),
        ("lyrics.language", T),
        ("lyrics.clean_off", both),
        ("lyrics.cleaning", both),
        ("lyrics.writing", both),
        // The title, which the lyrics' cleaning reads.
        ("tags", and(both, T)),
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
