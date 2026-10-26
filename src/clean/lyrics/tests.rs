use sha2::{Digest as _, Sha256};

use super::*;

fn song() -> Context<'static> {
    Context {
        title: Some("Paper Comets"),
        artists: vec!["Marlo Venn", "Ada Quill"],
        subtitle: false,
    }
}

fn subtitle() -> Context<'static> {
    Context {
        subtitle: true,
        ..song()
    }
}

fn run(text: &str) -> String {
    clean(text, &song(), &[]).text
}

fn only(rule: &str, text: &str, ctx: &Context<'_>) -> Cleaned {
    let off: Vec<String> = names().filter(|n| *n != rule).map(str::to_string).collect();
    clean(text, ctx, &off)
}

#[test]
fn spacing_trims_lines_and_drops_what_cannot_be_seen() {
    assert_eq!(
        run(
            "\n[00:01.00] Paper comets  over\u{a0}the bay \n[00:03.00] \n\n[00:04.00]\u{200b}\u{200b} Lantern\u{200b} \u{feff}\n"
        ),
        "[00:01.00]Paper comets over the bay\n[00:03.00]\n[00:04.00]Lantern\n"
    );
    for kept in [
        "[00:01.00]灯台の夢\u{3000}雨の街\n",
        "[00:01.00]Two of us 👩\u{200d}🚀\n",
        "[00:01.00]שיר\u{200f} אחד\n",
    ] {
        assert_eq!(run(kept), kept);
    }
    assert_eq!(
        run("\n\nfirst line\n\n\nsecond verse\n\n"),
        "first line\n\nsecond verse\n",
        "untimed stanzas stay apart by one blank line"
    );
}

#[test]
fn markup_and_subtitle_escapes_are_no_text() {
    assert_eq!(
        run("[00:01.00]<b>Lantern</b>\\hweather &amp; rain\n[00:02.00]{\\an8}<i>Quay</i>\n"),
        "[00:01.00]Lantern weather & rain\n[00:02.00]Quay\n"
    );
    assert_eq!(
        run("[00:01.00]<00:01.00>rain <00:01.50>falls a < b {c}\n"),
        "[00:01.00]<00:01.00>rain <00:01.50>falls a < b {c}\n"
    );
}

#[test]
fn a_look_alike_letter_takes_its_word_s_script() {
    assert_eq!(
        run("[00:01.00]Turn your h\u{435}ad and walk\n[00:02.00]\u{41d}ey, it's me\n"),
        "[00:01.00]Turn your head and walk\n[00:02.00]Hey, it's me\n"
    );
    assert_eq!(
        run("Пр\u{438}вeт, песня\n"),
        "Привет, песня\n",
        "a Latin e in a Cyrillic word finds the Cyrillic е the lyrics hold"
    );
    for kept in ["極Chu の 重音TETO\n", "Ivan Ивaн\n", "Πaper\n"] {
        assert_eq!(only("lyrics.tidy.homoglyphs", kept, &song()).text, kept);
    }
}

#[test]
fn cues_go_timed_or_not() {
    assert_eq!(
        run("[Chorus]\nThe lamps along the quay\n[Repeat: x4]\n\n[Fades]\n"),
        "The lamps along the quay\n"
    );
    assert_eq!(
        run(
            "[00:01.00](gentle piano solo)\n[00:02.00]*GuitarSolo*\n[00:03.00](Spoken) Maybe the lamps\n[00:04.00](Down by the river)\n[00:05.00](IRISH DANCE BREAK)\n"
        ),
        "[00:03.00]Maybe the lamps\n[00:04.00](Down by the river)\n"
    );
}

#[test]
fn credits_at_either_end_go_in_every_shape() {
    let lrc = "[00:01.00]Lyricist / Songwriter / Producer: Marlo Venn\n\
               [00:02.00]Composed and Written by: Ada Quill\n\
               [00:03.00]Translation (English) : someone_x\n\
               [00:04.00]Producer/Composition/Lyrics/Arrangement by\n\
               [00:05.00]by Marlo Venn\n\
               [00:06.00]Voice  Rin Hollow\n\
               [00:07.00]M V： 青い森\n\
               [00:08.00]The lamps along the quay\n\
               [00:09.00]Music by the water plays all night\n\
               [00:10.00]Last of the lamps\n\
               [00:11.00](Subtitles: tobin_ash)\n\
               [00:12.00](Lyrics posted by Ada Quill)\n\
               [00:13.00]詞：ほしのきり\n\
               [00:14.00]write to lamps@example.org\n";
    assert_eq!(
        run(lrc),
        "[00:08.00]The lamps along the quay\n\
         [00:09.00]Music by the water plays all night\n\
         [00:10.00]Last of the lamps\n"
    );
    for sung in [
        "Words Don't Come Easy\nto me\n",
        "Mix: the night and the morning light\n",
        "I'm @ home tonight\n",
    ] {
        assert_eq!(run(sung), sung);
    }
}

#[test]
fn the_title_restated_goes_but_a_title_sung_stays() {
    assert_eq!(
        run(
            "[00:01.00]\"Paper Comets\"\n[00:02.00]Art: @someone\n[00:03.00]The lamps along the quay\n"
        ),
        "[00:03.00]The lamps along the quay\n"
    );
    assert_eq!(
        run("[00:01.00]PAPER COMETS\n[00:02.00]Vocal: Rin Hollow\n[00:03.00]The lamps\n"),
        "[00:03.00]The lamps\n"
    );
    assert_eq!(
        run(
            "[00:01.00]\"Lanterns Out\"\n[00:02.00]Marlo Venn feat. Ada Quill\n[00:03.00]The lamps\n"
        ),
        "[00:03.00]The lamps\n",
        "a quoted heading in another language goes with the artist beside it"
    );
    assert_eq!(
        run("[00:01.00]Lantern Film Soundtrack 14 - Paper Comets by Marlo Venn\n"),
        ""
    );
    let sung = "[00:01.00]Paper comets\n[00:02.00]over the bay\n[00:03.00]Paper comets\n\
                [00:04.00]Lyrics posted by Ada Quill\n";
    assert_eq!(
        run(sung),
        "[00:01.00]Paper comets\n[00:02.00]over the bay\n[00:03.00]Paper comets\n"
    );
    assert_eq!(
        clean(
            "[00:01.00]\"Paper Comets\"\n[00:02.00]The lamps\n",
            &Context::default(),
            &[]
        )
        .text,
        "[00:01.00]\"Paper Comets\"\n[00:02.00]The lamps\n",
        "knowing no title, a quoted line is sung"
    );
}

#[test]
fn headers_name_only_the_song_in_timed_lyrics_and_nothing_in_untimed() {
    assert_eq!(run("[length:03:00]\nThe lamps\n"), "The lamps\n");
    assert_eq!(
        run("[ar:Marlo Venn]\n[by:someone]\n[re:a tool]\n[length:03:00]\n[00:01.00]The lamps\n"),
        "[ar:Marlo Venn]\n[length:03:00]\n[00:01.00]The lamps\n"
    );
}

#[test]
fn lines_are_put_in_order_and_frames_of_one_line_are_one() {
    assert_eq!(
        run("[ar:x]\n[00:02.00]second\n[00:01.00]first\n"),
        "[ar:x]\n[00:01.00]first\n[00:02.00]second\n"
    );
    assert_eq!(
        run(
            "[00:01.00]Lantern\n[00:01.03]Lantern\n[00:01.06]Lantern\n[00:01.06]Lantern\n[00:02.00]Quay\n"
        ),
        "[00:01.00]Lantern\n[00:02.00]Quay\n"
    );
    let stutter = "[00:01.00]go\n[00:01.40]go\n[00:01.80]go\n[00:03.00]home\n[00:04.00]home\n";
    assert_eq!(run(stutter), stutter, "a line sung again is two lines");
    let two = "[00:01.00]There once was a lamp\n[00:01.00]that lit the quay\n";
    assert_eq!(run(two), two, "a subtitle of two lines");
}

#[test]
fn a_subtitle_typing_a_line_out_is_that_line() {
    let typed = "[00:08.00]I don't know\n[00:09.50]I don't know what it is\n\
                 [00:11.00]I don't know what it is like\n[00:13.00]to feel alone\n";
    assert_eq!(
        clean(typed, &subtitle(), &[]).text,
        "[00:08.00]I don't know what it is like\n[00:13.00]to feel alone\n"
    );
    assert_eq!(run(typed), typed, "lines that grow in a file are sung");
    let again = "[00:04.00]Lanterns?\n[00:05.00]Lanterns? Not tonight\n";
    assert_eq!(
        clean(again, &subtitle(), &[]).text,
        again,
        "a line sung again longer is two"
    );
}

#[test]
fn a_rule_switched_off_does_nothing_and_each_rule_says_what_it_changed() {
    let text = "[00:01.00] Lantern \n[00:02.00][Chorus]\n";
    let c = clean(text, &song(), &[]);
    assert_eq!(c.text, "[00:01.00]Lantern\n");
    assert_eq!(c.rules, ["lyrics.tidy.spacing", "lyrics.packaging.cues"]);
    let c = clean(text, &song(), &["lyrics.tidy.spacing".to_string()]);
    assert_eq!(c.text, "[00:01.00] Lantern \n");
    let none: Vec<String> = names().map(str::to_string).collect();
    assert_eq!(clean("a\r\nb", &song(), &none).text, "a\nb\n");
}

#[test]
fn every_rule_is_a_switch_of_a_tier() {
    let tiers: Vec<&str> = super::super::TIERS.iter().map(|(t, _)| *t).collect();
    assert_eq!(switches().count(), names().count());
    for (tier, key) in switches() {
        assert!(tiers.contains(&tier));
        let mut s = Settings::default();
        assert!(s.set(&format!("lyrics.{tier}.{key}"), false));
        assert_eq!(off(&s), [format!("lyrics.{tier}.{key}")]);
    }
}

/// Lyrics every rule changes something in.
const SAMPLES: [&str; 6] = [
    "\n[00:01.00] <b>Paper comets</b>\\h \n[00:00.50]\"Paper Comets\"\n[00:00.60]Art: @someone\n",
    "[length:03:00]\n[Chorus]\nTurn your h\u{435}ad\n\n\nLyrics posted by Ada Quill\n",
    "[by:someone]\n[00:01.00]Lantern\n[00:01.03]Lantern\n[00:03.00](Spoken) Quay &amp; lamps\n[00:02.00]Pier\n",
    "[00:08.00]I don't\n[00:09.00]I don't know\n[00:09.50]I don't know what it is\n[00:11.00]to feel\n",
    "[00:01.00]Marlo Venn feat. Ada Quill\n[00:02.00]Voice  Rin Hollow\n[00:03.00]The lamps\n",
    "[00:01.00]灯台の夢\u{3000}雨\u{200b}\n[00:02.00]*GuitarSolo*\n[00:03.00]詞：ほしのきり\n",
];

/// What the rules write of [`SAMPLES`], with [`EDITION`]. A change to any
/// rule's output changes it: raise [`EDITION`], so every song with lyrics
/// is written again, then record the new digest here.
const WRITTEN: (u32, &str) = (
    2,
    "23dab8721db48cfcfc3b467ab1a0032192859d90abaeeb746e96813a633b9d74",
);

#[test]
fn the_edition_rises_with_what_the_rules_write() {
    let mut hash = Sha256::new();
    let mut ran = BTreeSet::new();
    for text in SAMPLES {
        let c = clean(text, &subtitle(), &[]);
        hash.update(c.text.as_bytes());
        hash.update([0]);
        ran.extend(c.rules);
    }
    assert_eq!(ran, names().collect(), "every rule changes a sample");
    let digest = crate::facts::hex(&hash.finalize());
    assert_eq!(
        (EDITION, digest.as_str()),
        WRITTEN,
        "the lyric rules write otherwise: raise clean::lyrics::EDITION and record the digest"
    );
}
