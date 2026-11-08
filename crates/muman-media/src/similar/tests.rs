use super::*;

fn same(a: &str, b: &str) -> bool {
    fold(a) == fold(b)
}

fn distance(a: &str, b: &str) -> Option<f64> {
    compare(&Name::new(a), &Name::new(b)).map(|c| c.distance)
}

fn conflicts(a: &str, b: &str) -> Vec<&'static str> {
    compare(&Name::new(a), &Name::new(b)).unwrap().conflicts
}

#[test]
fn latin_folds_case_width_marks_and_letters_without_a_base() {
    assert!(same("Straße", "STRASSE"));
    assert!(same("Ｍａｒｌｏ", "marlo"));
    assert!(same("Café Ünïcode", "cafe unicode"));
    assert!(same("Mưa Đêm", "mua dem"));
    assert!(same("Işık", "ISIK"));
    assert_eq!(fold("İnce"), "ince");
    assert!(same("Ølstad Æble", "olstad aeble"));
    assert!(same("Rain & Snow", "rain and snow"));
    assert!(same("The Glass Orchards", "Glass Orchards, The"));
    assert!(same("a\u{200D}b\u{FE0F}", "ab"));
    assert_eq!(fold("  Marlo -- Venn!! "), "marlo venn");
}

#[test]
fn cyrillic_and_greek_keep_their_letters_apart() {
    assert!(same("Ёлка", "елка"));
    assert!(!same("Мой", "Мои"));
    assert!(same("Ωδή", "ωδη"));
    assert!(same("ΛΟΓΟΣ", "λογος"));
}

#[test]
fn arabic_persian_and_hebrew_fold_as_their_analyzers_do() {
    assert!(same("أَمَل", "امل"));
    assert!(same("مـطـر", "مطر"));
    assert!(same("رحلة", "رحله"));
    assert!(same("ٱلقمر", "القمر"));
    assert!(same("یار", "يار"));
    assert!(same("کتاب", "كتاب"));
    assert!(same("שָׁלוֹם", "שלום"));
    assert!(same("ך", "כ"));
}

#[test]
fn indic_and_southeast_asian_marks_are_letters_kept() {
    assert!(!same("कुल", "कल"));
    assert!(!same("தமிழ்", "தமிழ"));
    assert!(!same("ที่", "ท"));
    assert!(!same("ខ្មែ", "ខមែ"));
    assert!(!same("が", "か"));
    assert_eq!(tokens("हिन्दी गाना"), ["हिन्दी", "गाना"]);
}

#[test]
fn chinese_and_japanese_fold_their_forms_alike() {
    assert!(same("東京雨夜", "东京雨夜"));
    assert!(same("ｶﾀｶﾅ", "カタカナ"));
    assert!(same("カタカナ", "かたかな"));
    for line in include_str!("han.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let pair: Vec<char> = line.chars().collect();
        assert_eq!(pair.len(), 2, "{line}");
        assert_ne!(pair[0], pair[1]);
        assert!(!HAN.contains_key(&pair[1]), "{line} folds again");
    }
}

#[test]
fn every_decimal_digit_is_its_ascii_digit() {
    assert_eq!(fold("٣ ३ ๓ ３ ໓"), "3 3 3 3 3");
    assert_eq!(fold("𝟗𝟘𝟙"), "901");
}

#[test]
fn words_are_found_in_scripts_written_without_spaces() {
    let thai = tokens("ฝนตกหนักมาก");
    assert!(thai.len() > 1, "{thai:?}");
    assert!(thai.iter().all(|w| !is_mark(w.chars().next().unwrap())));
    let japanese = tokens("東京の夜");
    assert!(japanese.contains(&"東京".to_string()) || japanese.contains(&"东京".to_string()));
    assert_eq!(tokens("Marlo, Venn!"), ["marlo", "venn"]);
}

#[test]
fn credits_packaging_and_order_cost_nothing_and_decorations_little() {
    assert_eq!(distance("Lantern Weather", "lantern weather!"), Some(0.0));
    assert_eq!(
        distance("Lantern Weather (feat. Kiri Hoshino)", "Lantern Weather"),
        Some(0.0)
    );
    assert_eq!(
        distance("Lantern Weather (Official Video)", "Lantern Weather"),
        Some(0.0)
    );
    assert!(distance("Weather Lantern", "Lantern Weather").unwrap() < 0.04);
    assert!(distance("Lantern Wether", "Lantern Weather").unwrap() < 0.1);
    assert!(distance("Copper Moth", "Lantern Weather").unwrap() > 0.5);
    let remaster = distance("Lantern Weather (Remastered 2011)", "Lantern Weather").unwrap();
    assert!(remaster > 0.04 && remaster < 0.25, "{remaster}");
    assert_eq!(
        conflicts("Lantern Weather (Remastered 2011)", "Lantern Weather"),
        [] as [&str; 0]
    );
}

#[test]
fn version_words_name_conflicts_in_any_language() {
    assert_eq!(
        conflicts("Lantern Weather (Live)", "Lantern Weather"),
        ["live version"]
    );
    assert_eq!(
        conflicts("Lantern Weather - Live at the Pier", "Lantern Weather"),
        ["live version"]
    );
    assert_eq!(
        conflicts("Lantern Weather (en vivo)", "Lantern Weather"),
        ["live version"]
    );
    assert_eq!(
        conflicts("雨の街 (ライブ ver.)", "雨の街"),
        ["live version"]
    );
    assert_eq!(
        conflicts("雨の街【オフボーカル】", "雨の街"),
        ["instrumental"]
    );
    assert_eq!(conflicts("바람 (Inst.)", "바람"), ["instrumental"]);
    assert_eq!(conflicts("夜行 (現場)", "夜行"), ["live version"]);
    assert_eq!(conflicts("でもね", "でもね"), [] as [&str; 0]);
    assert_eq!(conflicts("夜行 (デモ)", "夜行"), ["demo"]);
    assert_eq!(
        conflicts("Lantern Weather (Live)", "Lantern Weather (Live)"),
        Vec::<&str>::new()
    );
    assert_eq!(
        conflicts("Copper Moth, Part 1", "Copper Moth, Part 2"),
        ["another number"]
    );
    for entry in VERSIONS
        .iter()
        .flat_map(|(_, e)| e.iter())
        .chain(&SOFT_VERSIONS)
    {
        assert_eq!(fold(entry), *entry);
    }
}

#[test]
fn hangul_is_measured_in_jamo() {
    let near = distance("바람", "바램").unwrap();
    assert!(near > 0.0 && near < 0.25, "{near}");
}

#[test]
fn names_in_scripts_no_rule_bridges_are_left_uncompared() {
    assert_eq!(distance("東京の雨", "Tokyo no Ame"), None);
    assert_eq!(distance("바람", "Baram"), None);
    let romanized = distance("Марло Венн", "Marlo Venn").unwrap();
    assert!(romanized < 0.1, "{romanized}");
    assert_eq!(distance("1999", "1999"), Some(0.0));
    assert_eq!(distance("!!!", "Marlo"), None);
}

#[test]
fn a_file_name_is_read_every_way_it_may_be_meant() {
    let g = guesses("03 - Marlo Venn - Lantern Weather");
    assert_eq!(
        g[0],
        Guess {
            track: Some(3),
            artist: Some("Marlo Venn".into()),
            title: "Lantern Weather".into()
        }
    );
    assert!(
        g.iter()
            .any(|g| g.title == "Marlo Venn" && g.track == Some(3))
    );
    let g = guesses("Marlo Venn – Lantern Weather");
    assert_eq!(g[0].artist.as_deref(), Some("Marlo Venn"));
    assert_eq!(g[1].artist.as_deref(), Some("Lantern Weather"));
    assert_eq!(guesses("07 Lantern Weather")[0].track, Some(7));
    assert_eq!(
        guesses("Lantern Weather by Marlo Venn")[0]
            .artist
            .as_deref(),
        Some("Marlo Venn")
    );
    assert_eq!(
        guesses("Lantern_Weather").last().unwrap().title,
        "Lantern Weather"
    );
    assert_eq!(
        guesses("٠٣ - Marlo Venn - Lantern Weather")[0].track,
        Some(3)
    );
}

#[test]
fn shingles_and_containment() {
    let words = tokens("the glass orchards bloom at night");
    let a = shingles(&words, 3);
    assert_eq!(a.len(), 4);
    let part = shingles(&words[..4], 3);
    assert!((containment(&a, &part) - 1.0).abs() < 1e-9);
    assert!(containment(&a, &shingles(&tokens("copper moths in rain"), 3)) < 1e-9);
    assert_eq!(shingles(&tokens("rain"), 3).len(), 1);
    assert!(shingles(&[], 3).is_empty());
}

#[test]
fn assign_finds_the_least_sum_and_breaks_ties_alike() {
    assert_eq!(
        assign(&[vec![4, 1, 3], vec![2, 0, 5], vec![3, 2, 2]]),
        [1, 0, 2]
    );
    assert_eq!(assign(&[vec![9, 1, 9, 9], vec![1, 9, 9, 9]]), [1, 0]);
    assert_eq!(assign(&[vec![0, 0], vec![0, 0]]), [0, 1]);
    assert_eq!(assign(&[]), [] as [usize; 0]);
}

#[test]
fn version_words_are_whole_loanwords_in_japanese() {
    assert_eq!(conflicts("夜行 (なんでもない)", "夜行"), [] as [&str; 0]);
    assert_eq!(conflicts("夜行 (ドライブ)", "夜行"), [] as [&str; 0]);
    assert_eq!(conflicts("夜行 〜雨でも〜", "夜行"), [] as [&str; 0]);
    assert_eq!(conflicts("夜行 (デモ)", "夜行"), ["demo"]);
    assert_eq!(conflicts("夜行 (ライブ)", "夜行"), ["live version"]);
}

#[test]
fn nested_brackets_close_where_they_match() {
    let n = Name::new("Lantern ((Live) Demo)");
    assert_eq!(n.core, "lantern");
    assert_eq!(n.versions().0, ["demo", "live version"].into());
}

#[test]
fn fold_is_the_same_folded_again_and_letters_without_a_base_fold_with_marks() {
    assert!(same("Ǿlstad", "Ølstad"));
    assert!(same("ǽble", "æble"));
    for s in [
        "a३\u{94d}",
        "Marlo the the",
        "The The Glass",
        "ǽ\u{301}x",
        "Ёлка й",
    ] {
        let f = fold(s);
        assert_eq!(fold(&f), f, "{s}");
    }
}

#[test]
fn a_word_said_again_is_not_said_once_and_kanji_and_kana_are_not_compared() {
    assert!(distance("Moth", "Moth Moth Moth").unwrap() > 0.25);
    assert!(distance("Lantern Weather", "Lantern Lantern Weather Weather").unwrap() > 0.1);
    assert_eq!(distance("Weather Lantern", "Lantern Weather"), Some(0.0));
    assert_eq!(distance("东京", "とうきょう"), None);
    assert!(distance("東京の雨", "東京のあめ").is_some());
    assert_eq!(shingles(&tokens("rain"), 0).len(), 1);
}

#[test]
fn names_match_exactly_before_one_holds_the_other() {
    assert_eq!(
        names_match("Lantern Weather", "lantern weather!"),
        Some(Exactness::Exact)
    );
    assert_eq!(
        names_match("Ｌａｎｔｅｒｎ　Ｗｅａｔｈｅｒ", "Lantern Weather"),
        Some(Exactness::Exact),
        "full-width forms are the same letters"
    );
    assert_eq!(
        names_match("Lantern Weather (Live)", "Lantern Weather"),
        Some(Exactness::Holds)
    );
    assert_eq!(names_match("Second Tune", "Lantern Weather"), None);
    assert_eq!(names_match("", "Lantern Weather"), None);
}

#[test]
fn names_agree_alike_disagree_apart_and_say_nothing_between_or_across_scripts() {
    let agree = |a: &str, b: &str| super::agree(&Name::new(a), &Name::new(b));
    assert_eq!(agree("Harbor Lights", "Harbour Lights"), Some(true));
    assert_eq!(
        agree("Lantern Weather", "Lantern Weather (Radio Edit)"),
        Some(true)
    );
    assert_eq!(agree("Night Ferry", "Paper Comets"), Some(false));
    assert_eq!(
        agree("Lanterns Over Harbor", "Lanterns Over Harbor Wall at Dusk"),
        Some(true),
        "a title cut short"
    );
    assert_eq!(
        agree("Night Ferry", "Night Ferry Wreck at Dawn"),
        Some(false),
        "two words are no cut"
    );
    assert_eq!(
        agree("Glass Orchard Nights", "Glass Orchard Nights, Volume 2"),
        Some(false),
        "another of a series"
    );
    assert_eq!(
        agree("Lantern Weather", "Lantern Weather [Live at the Pier]"),
        Some(true),
        "its version is the caller's to weigh"
    );
    assert_eq!(
        versions_in("Lantern Weather [Live at the Pier]"),
        BTreeSet::from(["live version"])
    );
    assert_eq!(
        versions_in("Live Forever"),
        BTreeSet::from(["live version"])
    );
    assert_eq!(
        agree(
            "Idiot Wind (take 1, 9/16/74)",
            "Idiot Wind (Alternate Take 5)"
        ),
        Some(true),
        "a take or a date tells releases apart, not songs"
    );
    assert_eq!(agree("Song 2", "Song"), Some(false), "another number");
    assert_eq!(agree("Song One", "Song Four"), None, "neither");
    assert_eq!(
        agree("夜の船", "Night Ferry"),
        None,
        "no rule compares them"
    );
}

#[test]
fn credits_agree_on_a_whole_name_never_on_one_holding_another() {
    assert_eq!(
        credits_agree(&["Marlo Venn & Kiri Hoshino"], &["The Marlo Venn"]),
        Some(true)
    );
    assert_eq!(
        credits_agree(&["Kiri Hoshino feat. MARLO VENN"], &["Marlo Venn"]),
        Some(true)
    );
    assert_eq!(
        credits_agree(&["Marlo Venn and the Glass Orchards"], &["Glass Orchards"]),
        Some(true)
    );
    assert_eq!(
        credits_agree(&["Marlo Venn Tribute Band"], &["Marlo Venn"]),
        Some(false)
    );
    assert_eq!(
        credits_agree(&["Marlo’s Garden"], &["Marlos Garden"]),
        Some(true)
    );
    assert_eq!(credits_agree(&["マルロ"], &["Marlo Venn"]), None);
    assert_eq!(credits_agree(&[], &["Marlo Venn"]), None);
}

#[test]
fn titles_are_read_as_files_write_them_and_a_version_counts_where_the_record_names_it() {
    let title = |record: &str, file: &str| titles_agree(&[record], &[vec![file]], false);
    assert_eq!(title("Night Ferry", "Marlo Venn / Night Ferry"), Some(true));
    assert_eq!(title("Night Ferry", "1st Set: Night Ferry"), Some(true));
    assert_eq!(
        title(
            "Night Ferry",
            "Night Ferry [Live at the Pier, Harbor Town - May 1..."
        ),
        Some(true),
        "cut off inside its bracket"
    );
    assert_eq!(
        title("Night Ferry", "Night Ferry (Live)"),
        Some(true),
        "a record keeps live out of its title"
    );
    assert_eq!(
        title("Night Ferry (live at the Pier)", "Night Ferry"),
        Some(false)
    );
    assert_eq!(
        titles_agree(
            &["Night Ferry Suite: II. Dusk"],
            &[vec!["Night Ferry Suite", "II. Dusk"]],
            false
        ),
        Some(true),
        "a work and its movement"
    );
    assert_ne!(
        titles_agree(
            &["Night Ferry Suite: II. Dusk"],
            &[vec!["Harbor Suite: II. Dusk"]],
            false
        ),
        Some(true),
        "never by the movement alone"
    );
    assert_eq!(
        title("NIGHT FERRY <Kiri Version>", "Night Ferry (Kiri)"),
        Some(true)
    );
    assert_eq!(
        title("Night Ferry: Soliloquy", "Soliloquy (Night Ferry)"),
        Some(true)
    );
    assert_eq!(title("Dusk / Dawn", "Dusk - Dawn"), Some(true), "a medley");
    assert_eq!(title("夜の船", "Night Ferry"), None);
    assert_eq!(title("Rooms of Salt", "Rooms of Salt CD1"), Some(true));
    assert_eq!(
        without_disc("Rooms of Salt - Disc 2"),
        Some("Rooms of Salt")
    );
    assert_eq!(without_disc("Rooms of Salt 2"), None);
}
