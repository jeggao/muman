use sha2::{Digest as _, Sha256};

use super::*;

fn run(text: &str) -> String {
    clean(text, &Context::default(), &[]).text
}

#[test]
fn markup_cues_and_credits_at_either_end_go() {
    assert_eq!(
        run(
            "[ar:x]\n[00:01.00]Lyrics: Marlo Venn\n[00:02.00]<i>The lamps</i>\n\
             [00:03.00][Chorus]\n[00:04.00]Music by Ada Quill burns low\n\
             [00:05.00]Last of the lamps\n[00:06.00]Mix: Tobin Ash\n"
        ),
        "[ar:x]\n[00:02.00]The lamps\n[00:04.00]Music by Ada Quill burns low\n\
         [00:05.00]Last of the lamps\n"
    );
}

#[test]
fn a_rule_switched_off_does_nothing_and_each_rule_says_what_it_changed() {
    let text = "[00:01.00]<b>Lantern</b>\n[00:02.00][Chorus]\n";
    let c = clean(text, &Context::default(), &[]);
    assert_eq!(c.text, "[00:01.00]Lantern\n");
    assert_eq!(c.rules, ["lyrics.tidy.markup", "lyrics.packaging.cues"]);
    let c = clean(
        text,
        &Context::default(),
        &["lyrics.packaging.cues".to_string()],
    );
    assert_eq!(c.text, "[00:01.00]Lantern\n[00:02.00][Chorus]\n");
    let none: Vec<String> = names().map(str::to_string).collect();
    assert_eq!(clean("a\r\nb", &Context::default(), &none).text, "a\nb\n");
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
const SAMPLES: [&str; 2] = [
    "[00:01.00]Lyrics: Marlo Venn\n[00:02.00]<i>Paper comets</i>\n[00:03.00]♪\n",
    "[ar:x]\n[00:01.00]{\\an8}Lantern\n[00:02.00](Guitar solo)\n[00:03.00]Quay\n",
];

/// What the rules write of [`SAMPLES`], with [`EDITION`]. A change to any
/// rule's output changes it: raise [`EDITION`], so every song with lyrics
/// is written again, then record the new digest here.
const WRITTEN: (u32, &str) = (
    1,
    "0085d4e846b9648ed75f0a1e0f0e83c3467f1781cdca840b7ff35edc46e190e1",
);

#[test]
fn the_edition_rises_with_what_the_rules_write() {
    let mut hash = Sha256::new();
    let mut ran = BTreeSet::new();
    for text in SAMPLES {
        let c = clean(text, &Context::default(), &[]);
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
