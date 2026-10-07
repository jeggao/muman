use super::*;
use crate::settings::{Settings, read};

fn doc(text: &str) -> DocumentMut {
    text.parse().unwrap()
}

fn renames() -> impl Iterator<Item = &'static Rename> {
    HISTORY
        .iter()
        .flat_map(|e| e.steps)
        .filter_map(|s| match s {
            Step::Rename(r) => Some(r),
            Step::Default(_) => None,
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
        let path: Vec<&str> = r.table.split('.').collect();
        let table = path
            .iter()
            .try_fold(new.as_item(), |item, key| item.get(key))
            .and_then(Item::as_table)
            .unwrap_or_else(|| panic!("no [{}] in new.toml", r.table));
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
    assert_eq!(renamed.len(), renames().count(), "{renamed:?}");
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

/// A third edition that raised `opus_bitrate` to today's default.
const RAISED: Editions = Editions {
    current: 3,
    history: &[
        HISTORY[0],
        Edition {
            number: 3,
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
    assert_eq!(edition_of(&d), 3);
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
    assert_eq!(edition_of(&d), 3);
    assert_eq!(read(&d).unwrap().audio.opus_kbps, 170);
    let chosen = doc("version = 1\nedition = 3\n[audio]\nopus_bitrate = \"128 kb/s\"\n");
    assert_eq!(stale(&chosen, &RAISED), [], "set after edition 3");
    assert!(read(&doc("edition = 0\n")).is_err());
    assert!(read(&doc("edition = \"one\"\n")).is_err());
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
