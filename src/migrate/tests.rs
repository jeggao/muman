use super::*;
use crate::settings::{Settings, To, read};

fn doc(text: &str) -> DocumentMut {
    text.parse().unwrap()
}

fn renames() -> impl Iterator<Item = &'static Rename> {
    HISTORY
        .iter()
        .flat_map(|e| e.steps)
        .filter_map(|s| match s {
            Step::Rename(r) => Some(r),
            Step::Move(_) | Step::MoveTable(_) | Step::Reshape(_) | Step::Default(_) => None,
        })
}

/// A song list of the first edition setting every setting since renamed,
/// each to its default then, with comments on two.
const OLD: &str = "version = 1

[audio]
# Mine.
opus_kbps = 160 # kept
opus_surround_kbps = 256
vorbis_kbps = 192
aac_kbps = 256
mp3_kbps = 320
min_kbps = 64

[quality.purity]
weight = 10000
step_ms = 2000

[quality.bandwidth]
step_hz = 500

[ytdlp]
partial_days = 14

[history]
runs = 3
max_mib = 2048

[providers.lrclib]
recheck_days = 7

[[song]]
sources = [\"manual:a.flac\"]
lyrics_offset_ms = -120
";

#[test]
fn the_editions_count_up_to_the_current_one() {
    let numbers: Vec<i64> = HISTORY.iter().map(|e| e.number).collect();
    let expected: Vec<i64> = (FIRST + 1..=EDITION).collect();
    assert_eq!(numbers, expected);
    assert_eq!(EDITIONS.current, EDITION);
    let new = defaults();
    assert_eq!(edition_of(&crate::manifest::NEW.parse().unwrap()), EDITION);
    for r in renames().filter(|r| !r.table.contains('*') && r.table != "song") {
        let table = now_named(r.table, FIRST, &EDITIONS);
        let path: Vec<&str> = table.split('.').collect();
        let table = path
            .iter()
            .try_fold(new.as_item(), |item, key| item.get(key))
            .and_then(Item::as_table)
            .unwrap_or_else(|| panic!("no [{table}] in new.toml"));
        assert!(!table.contains_key(r.old), "new.toml names `{}`", r.old);
        let commented = crate::manifest::NEW.contains(&format!("# {} = ", r.new));
        assert!(
            table.contains_key(r.new) || commented,
            "new.toml lacks `{}`",
            r.new
        );
    }
}

#[test]
fn every_old_name_reads_as_its_new_one_with_its_comments() {
    let mut d = doc(OLD);
    let renamed = rename_all(&mut d, &EDITIONS).unwrap();
    let moved = [
        "[quality.purity] is [audio.pick.purity] now",
        "[quality.bandwidth] is [audio.pick.bandwidth] now",
    ];
    assert_eq!(
        renamed.len(),
        renames().count() + moved.len(),
        "{renamed:?}"
    );
    let last: Vec<String> = renamed[renames().count()..]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(last, moved);
    let text = d.to_string();
    assert!(
        text.contains("# Mine.\nopus_bitrate = \"160 kb/s\" # kept\n"),
        "{text}"
    );
    for (want, before) in [
        ("step = \"2 s\"", "weight = 10000\n"),
        ("keep_partial = \"14 days\"", "[ytdlp]\n"),
        ("max_size = \"2 GiB\"", "runs = 3\n"),
        ("recheck = \"7 days\"", "[providers.lrclib]\n"),
        (
            "lyrics_offset = \"-120 ms\"",
            "sources = [\"manual:a.flac\"]\n",
        ),
    ] {
        assert!(
            text.contains(&format!("{before}{want}\n")),
            "{want}: {text}"
        );
    }
    assert_eq!(
        renamed[0].to_string(),
        "[audio] `opus_kbps = 160` is `opus_bitrate = \"160 kb/s\"` now"
    );
    let defaults = Settings {
        audio: crate::settings::Audio {
            min_kbps: Some(64),
            ..crate::settings::Audio::default()
        },
        ..Settings::default()
    };
    assert_eq!(
        read(&doc(OLD)).unwrap(),
        defaults,
        "read old, it is renamed"
    );
    assert_eq!(read(&d).unwrap(), defaults);
    assert!(
        rename_all(&mut d, &EDITIONS).unwrap().is_empty(),
        "once is all"
    );
    assert_eq!(d.to_string(), text);
}

#[test]
fn a_setting_named_both_ways_or_given_an_old_value_it_never_took_is_refused() {
    let both = "[history]\nmax_mib = 10\nmax_size = \"1 GiB\"\n";
    let e = rename_all(&mut doc(both), &EDITIONS).unwrap_err();
    assert!(format!("{e:#}").contains("keep `max_size`"), "{e:#}");
    let e = rename_all(&mut doc("[ytdlp]\npartial_days = \"soon\"\n"), &EDITIONS).unwrap_err();
    assert!(
        format!("{e:#}").contains("set `keep_partial` instead"),
        "{e:#}"
    );
}

/// A fourth edition that raised `opus_bitrate` to today's default.
const RAISED: Editions = Editions {
    current: 4,
    history: &[
        HISTORY[0],
        HISTORY[1],
        Edition {
            number: 4,
            steps: &[Step::Default(Change {
                key: "audio.opus_bitrate",
                was: "\"128 kb/s\"",
            })],
        },
    ],
};

#[test]
fn a_setting_at_its_editions_old_default_is_stale_until_updated() {
    let mut d = doc("version = 1\nedition = 2\n[audio]\nopus_bitrate = \"128kbps\" # as written\n");
    let found = stale(&d, &RAISED);
    assert_eq!(
        found,
        [Stale {
            key: "audio.opus_bitrate".into(),
            was: "\"128 kb/s\"".into(),
            now: "\"160 kb/s\"".into(),
        }],
        "an old default spelled otherwise is that default"
    );
    assert_eq!(stale_warnings(&found).len(), 2);
    crate::settings::fill(&mut d, &RAISED);
    assert_eq!(edition_of(&d), 2, "a stale setting holds the edition");
    assert_eq!(update(&mut d, &RAISED), found);
    assert!(
        d.to_string()
            .contains("opus_bitrate = \"160 kb/s\" # as written"),
        "{d}"
    );
    assert_eq!(edition_of(&d), 4);
    assert_eq!(stale(&d, &RAISED), []);
}

#[test]
fn an_old_default_under_an_old_name_is_renamed_then_offered() {
    let mut d = doc("version = 1\n[audio]\nopus_kbps = 128\n");
    assert_eq!(
        stale(&d, &RAISED),
        [],
        "read as it is, the old name is no setting"
    );
    rename_all(&mut d, &RAISED).unwrap();
    assert_eq!(stale(&d, &RAISED).len(), 1);
    let mut d = doc("version = 1\n[audio]\nopus_kbps = 128\n");
    assert_eq!(update(&mut d, &RAISED).len(), 1, "update renames first");
}

#[test]
fn a_setting_changed_by_hand_stays_and_the_edition_rises() {
    let mut d = doc("version = 1\n[audio]\nopus_bitrate = \"170 kb/s\"\n");
    assert_eq!(edition_of(&d), FIRST);
    assert_eq!(stale(&d, &RAISED), []);
    assert!(crate::settings::fill(&mut d, &RAISED));
    assert_eq!(edition_of(&d), 4);
    assert_eq!(read(&d).unwrap().audio.opus_kbps, 170);
    let chosen = doc("version = 1\nedition = 4\n[audio]\nopus_bitrate = \"128 kb/s\"\n");
    assert_eq!(stale(&chosen, &RAISED), [], "set after edition 4");
    assert!(read(&doc("edition = 0\n")).is_err());
    assert!(read(&doc("edition = \"one\"\n")).is_err());
}

#[test]
fn the_third_editions_old_defaults_move_and_a_choice_stays() {
    let mut d = doc("version = 1
edition = 2
[audio]
codecs = [\"opus\", \"flac\"] # as written
[quality.stereo]
weight = 5
[ytdlp]
format = \"bestaudio\"
");
    rename_all(&mut d, &EDITIONS).unwrap();
    let keys: Vec<String> = stale(&d, &EDITIONS).into_iter().map(|s| s.key).collect();
    assert_eq!(keys, ["audio.codecs", "audio.pick.stereo.weight"]);
    update(&mut d, &EDITIONS);
    let read = read(&d).unwrap();
    assert_eq!(read.audio.codecs, Settings::default().audio.codecs);
    assert_eq!(read.audio.pick.stereo.weight, 5000);
    assert_eq!(read.ytdlp.format, "bestaudio", "set by hand");
    assert!(d.to_string().contains("\"flac\"] # as written"), "{d}");
    assert_eq!(edition_of(&d), EDITION);
}

#[test]
fn the_fourth_edition_leaves_comment_streams_out_of_the_subtitles() {
    let mut d = doc("version = 1\nedition = 3\n[ytdlp]\nsub_langs = \"all,-live_chat\"\n");
    let keys: Vec<String> = stale(&d, &EDITIONS).into_iter().map(|s| s.key).collect();
    assert_eq!(keys, ["ytdlp.sub_langs"]);
    update(&mut d, &EDITIONS);
    assert_eq!(
        read(&d).unwrap().ytdlp.sub_langs,
        "all,-live_chat,-comments,-danmaku"
    );
    let chosen = doc("version = 1\nedition = 3\n[ytdlp]\nsub_langs = \"en.*\"\n");
    assert_eq!(stale(&chosen, &EDITIONS), []);
}

#[test]
fn a_file_of_a_later_edition_is_refused_with_its_edition_named() {
    let e = read(&doc("edition = 99\n[audio]\nopus_mode = \"fast\"\n")).unwrap_err();
    let text = format!("{e:#}");
    assert!(
        text.contains("edition 99, written by a later muman"),
        "{text}"
    );
    assert!(text.contains("opus_mode"), "{text}");
    assert!(
        read(&doc("edition = 99\n")).is_ok(),
        "nothing it does not know"
    );
}

#[test]
fn the_lyrics_languages_move_to_their_own_table_with_their_comments() {
    let mut d = doc("[defaults]\n# Mine.\nlyrics = [\"ja\", \"en\"]\n\n[[song]]\nsources = []\n");
    let moved = rename_all(&mut d, &EDITIONS).unwrap();
    assert_eq!(
        moved[0].to_string(),
        "[defaults] `lyrics = [\"ja\", \"en\"]` is `[lyrics] read = [\"ja\", \"en\"]` now"
    );
    let text = d.to_string();
    assert!(!text.contains("[defaults]"), "{text}");
    assert!(
        text.contains("# Mine.\nread = [\"ja\", \"en\"]\n"),
        "{text}"
    );
    let read: Vec<String> = crate::settings::read(&doc("[defaults]\nlyrics = [\"ja\"]\n"))
        .unwrap()
        .lyrics
        .read
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(read, ["ja"], "read old, it is moved");
    assert!(
        rename_all(&mut d, &EDITIONS).unwrap().is_empty(),
        "once is all"
    );

    let both = "[defaults]\nlyrics = [\"ja\"]\n[lyrics]\nread = [\"en\"]\n";
    let e = rename_all(&mut doc(both), &EDITIONS).unwrap_err();
    assert!(format!("{e:#}").contains("keep [lyrics] `read`"), "{e:#}");
}

#[test]
fn a_table_moves_whole_with_its_comments_and_leaves_nothing_behind() {
    let mut d = doc("edition = 5
# Mine.
[quality.stereo]
weight = 0 # kept

[clean.tidy]
spacing = false

[clean.lyrics.tidy]
markup = false
");
    let moved = rename_all(&mut d, &EDITIONS).unwrap();
    let said: Vec<String> = moved.iter().map(ToString::to_string).collect();
    assert_eq!(
        said,
        [
            "[clean.lyrics] is [lyrics.clean] now",
            "[clean] is [tags.clean] now",
            "[quality.stereo] is [audio.pick.stereo] now",
        ]
    );
    let text = d.to_string();
    assert!(
        text.contains("# Mine.\n[audio.pick.stereo]\nweight = 0 # kept\n"),
        "{text}"
    );
    assert!(
        text.contains("[tags.clean.tidy]\nspacing = false\n"),
        "{text}"
    );
    assert!(
        text.contains("[lyrics.clean.tidy]\nmarkup = false\n"),
        "{text}"
    );
    assert!(
        !text.contains("[quality") && !text.contains("[clean"),
        "{text}"
    );
    assert!(
        rename_all(&mut d, &EDITIONS).unwrap().is_empty(),
        "once is all"
    );
    assert_eq!(d.to_string(), text);

    let both = "[quality.stereo]\nweight = 0\n[audio.pick.stereo]\nweight = 1\n";
    let e = rename_all(&mut doc(both), &EDITIONS).unwrap_err();
    assert!(
        format!("{e:#}").contains("keep [audio.pick.stereo]"),
        "{e:#}"
    );
    let e = rename_all(&mut doc("quality = { stereo = 1 }\n"), &EDITIONS).unwrap_err();
    assert!(format!("{e:#}").contains("must be a table"), "{e:#}");
}

#[test]
fn where_lyrics_went_reads_as_the_outputs_writing_them_there() {
    for (placement, to) in [
        ("sidecar", vec![To::File]),
        ("embedded", vec![To::Tag]),
        ("both", vec![To::File, To::Tag]),
    ] {
        let mut d = doc(&format!(
            "edition = 6\n[library]\n# Mine.\nlyrics = \"{placement}\"\n[lyrics]\nread = [\"en\"]\n"
        ));
        let moved = rename_all(&mut d, &EDITIONS).unwrap();
        let said: Vec<String> = moved.iter().map(ToString::to_string).collect();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with(&format!(
                "[library] `lyrics = \"{placement}\"` is `[[lyrics.out]] to = "
            )),
            "{said:?}"
        );
        let text = d.to_string();
        assert!(!text.contains("# Mine."), "{text}");
        assert!(
            text.contains("# Where the lyrics a song takes are written"),
            "{text}"
        );
        let read = read(&d).unwrap();
        let wrote: Vec<To> = read.lyrics.out.iter().map(|o| o.to).collect();
        assert_eq!(wrote, to, "{text}");
        assert!(
            rename_all(&mut d, &EDITIONS).unwrap().is_empty(),
            "once is all"
        );
    }
    let both = "[library]\nlyrics = \"both\"\n[lyrics]\nout = []\n";
    let e = rename_all(&mut doc(both), &EDITIONS).unwrap_err();
    assert!(format!("{e:#}").contains("keep [lyrics] `out`"), "{e:#}");
    let e = rename_all(&mut doc("[library]\nlyrics = \"aside\"\n"), &EDITIONS).unwrap_err();
    assert!(
        format!("{e:#}").contains("set [lyrics] `out` instead"),
        "{e:#}"
    );
    let read = read(&doc("[library]\nlyrics = \"embedded\"\n")).unwrap();
    assert_eq!(read.lyrics.out.len(), 1, "read old, it is reshaped");
}
