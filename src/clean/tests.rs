use super::*;

fn offer(values: &[&str], structured: bool) -> Offer {
    Offer {
        values: values.iter().map(|v| (*v).to_string()).collect(),
        structured,
    }
}

fn run(
    field: Field,
    values: &[&str],
    structured: bool,
    credited: &[&str],
    settings: &Settings,
) -> Cleaned {
    let names = names(credited.iter().copied());
    let albums = Albums::default();
    let offers = Offers::new();
    let ctx = Context {
        names: &names,
        albums: &albums,
        offers: &offers,
    };
    clean(field, &offer(values, structured), &ctx, settings)
}

/// An upload's title, cleaned with every rule, its song crediting
/// `credited`.
fn upload(title: &str, credited: &[&str]) -> String {
    run(
        Field::Title,
        &[title],
        false,
        credited,
        &Settings::default(),
    )
    .values
    .join("|")
}

/// A field kept for it, cleaned with every rule.
fn kept(field: Field, value: &str) -> Vec<String> {
    run(field, &[value], true, &[], &Settings::default()).values
}

fn bare(s: &str) -> String {
    normalize(without_credits(s))
}

/// Uploads beside the release YouTube Music matched them to: each
/// upload's title, the names its song's sources credit, the title
/// expected, and the release's title.
const PAIRS: &[(&str, &[&str], &str, &str)] = &[
    (
        "月曜日OLの逃避行 / 夜鳴ナギ・灰谷ユノ・星野キリ",
        &["Fe7rin", "Yonaki Nagi", "Yuno Haitani", "KIRI HOSHINO"],
        "月曜日OLの逃避行",
        "月曜日OLの逃避行 (feat. 夜鳴ナギ, 灰谷ユノ & 星野キリ)",
    ),
    (
        "相対的隣人賛歌 / 星野キリ・夜鳴ナギ♀",
        &["Fe7rin", "KIRI HOSHINO", "Yonaki Nagi"],
        "相対的隣人賛歌",
        "相対的隣人賛歌 (feat. 星野キリ & 夜鳴ナギ)",
    ),
    (
        "楽観的週末讃歌 / 星野キリ",
        &["Fe7rin"],
        "楽観的週末讃歌",
        "楽観的週末讃歌",
    ),
    (
        "Kiri Hoshino - Kiri territory",
        &["MorrowK", "Kiri Hoshino"],
        "Kiri territory",
        "Kiri Territory",
    ),
    (
        "キリモミ / 星野キリSV",
        &["Kashiwa Basalt"],
        "キリモミ",
        "キリモミ",
    ),
    (
        "ふわりらりん / くもりび feat.灯台ルル（Fuwa-rarin / Kumoribi feat.Ruru）",
        &["くもりび/ kumoribi", "Kumoribi"],
        "ふわりらりん",
        "ふわりらりん",
    ),
    (
        "[MV] PBW - 'ルララリ' (RURARARI) feat. 星野キリ",
        &["PaperBoatsWander", "MOROMIKA", "Kiri Hoshino"],
        "ルララリ (RURARARI)",
        "ルララリ (feat. Kiri Hoshino)",
    ),
    (
        "【Official MV】 NO LANTERN ft. 星野キリ・白波コロ (Kiri Hoshino・Shiranami Koro)",
        &["QLuneP"],
        "NO LANTERN",
        "NO LANTERN",
    ),
    (
        "MOTH ERA / Kurogane Tsumu",
        &["oKelvra"],
        "MOTH ERA",
        "MOTH ERA",
    ),
    (
        "Silverfish feat. Shiranami Koro",
        &["Quarry Hollow", "Shiranami Koro"],
        "Silverfish",
        "Silverfish",
    ),
    (
        "\"My Martian Cousin\" feat. Shiranami Koro & Shiranami Koro",
        &["Quarry Hollow", "Shiranami Koro"],
        "My Martian Cousin",
        "My Martian Cousin",
    ),
    (
        "[MV] PBW - 'Mis-Step MMO' 白波コロ & 星野キリ",
        &["PaperBoatsWander"],
        "Mis-Step MMO",
        "Mis-Step MMO",
    ),
    (
        "[MV] PBW - '星間郵便' (Starmail) 白波コロ & 星野キリ",
        &["PaperBoatsWander", "MOROMIKA"],
        "星間郵便 (Starmail)",
        "星間郵便",
    ),
    (
        "[MV] PBW - 'I'm Wandering Comet' feat. 星野キリ(Kiri Hoshino)",
        &["PaperBoatsWander", "Tama Vell", "MOROMIKA", "Kiri Hoshino"],
        "I'm Wandering Comet",
        "I'm Wandering Comet (feat. Kiri Hoshino)",
    ),
    (
        "[MV] PBW - '散歩中毒' (Strollaholic) feat. 白波コロ",
        &["PaperBoatsWander", "MOROMIKA", "Shiranami Koro"],
        "散歩中毒 (Strollaholic)",
        "散歩中毒 (feat. Shiranami Koro)",
    ),
    (
        "TOOLATE! / Shiranami Koro",
        &["oKelvra", "Shiranami Koro"],
        "TOOLATE!",
        "TOOLATE!",
    ),
    (
        "Waltz Wonderful (feat. Haitani Yuno)",
        &["Marlo Venn / MarloV", "Marlo Venn"],
        "Waltz Wonderful",
        "Waltz Wonderful",
    ),
    (
        "Gleaming Garden / Marlo Venn feat. Shiranami Koro",
        &[
            "Shiranami Koro",
            "Marlo Venn / MarloV",
            "tollamo",
            "白波コロ",
        ],
        "Gleaming Garden",
        "Gleaming Garden (feat. Shiranami Koro)",
    ),
    (
        "DIG FOR GLORY (feat. Kiri Hoshino)",
        &["Marlo Venn / MarloV", "Lumenhart"],
        "DIG FOR GLORY",
        "DIG FOR GLORY",
    ),
    (
        "I Wish That I Could Float (feat. MOMI SV)",
        &["Marlo Venn / MarloV"],
        "I Wish That I Could Float",
        "I Wish That I Could Float",
    ),
    (
        "Cobalt Corners (feat. Kiri Hoshino)",
        &["Marlo Venn / MarloV"],
        "Cobalt Corners",
        "Cobalt Corners",
    ),
    (
        "PIGEONHEAD (w/ OK Pebble) feat. Kiri Hoshino",
        &["Marlo Venn / MarloV", "OK Pebble", "wren tallow"],
        "PIGEONHEAD",
        "PIGEONHEAD (feat. OK Pebble)",
    ),
    ("TILES 🎵", &["plain trapezoid"], "TILES", "TILES"),
    (
        "QUIT DAWDLING",
        &["plain trapezoid"],
        "QUIT DAWDLING",
        "QUIT DAWDLING!",
    ),
    (
        "holy pancake",
        &["plain trapezoid"],
        "holy pancake",
        "Holy Pancake",
    ),
    (
        "jolly squidmas!",
        &["plain trapezoid"],
        "jolly squidmas!",
        "jolly squidmas",
    ),
    (
        "behold! sorcerer newt!",
        &["plain trapezoid"],
        "behold! sorcerer newt!",
        "behold! sorcerer newt!",
    ),
    ("goat", &["plain trapezoid"], "goat", "goat"),
    (
        "squid koro",
        &["plain trapezoid"],
        "squid koro",
        "squid koro",
    ),
    ("zorb", &["plain trapezoid"], "zorb", "zorb"),
    (
        "squid trio",
        &["plain trapezoid"],
        "squid trio",
        "squid trio",
    ),
    (
        "time to go home!",
        &["plain trapezoid"],
        "time to go home!",
        "time to go home!",
    ),
    (
        "this is a snail",
        &["plain trapezoid"],
        "this is a snail",
        "this is a snail",
    ),
    (
        "blip blorp",
        &["plain trapezoid"],
        "blip blorp",
        "blip blorp",
    ),
    ("honk", &["plain trapezoid"], "honk", "honk"),
    ("purr", &["plain trapezoid"], "purr", "purr"),
    (
        "ロンリーランナー／星野キリSV",
        &["カラメロ", "karamero"],
        "ロンリーランナー",
        "ロンリーランナー",
    ),
];

#[test]
fn uploads_clean_to_their_release_s_name() {
    assert_eq!(PAIRS.len(), 37);
    for (title, credited, expected, release) in PAIRS {
        let cleaned = upload(title, credited);
        assert_eq!(cleaned, *expected, "{title}");
        // Romanization is kept, as the user writes it; releases leave it out.
        if !expected.contains(" (") {
            assert_eq!(normalize(&cleaned), bare(release), "{title}");
        }
    }
}

#[test]
fn uploads_without_a_release_lose_their_packaging() {
    let channel = ["Marlo Venn / MarloV"];
    for (title, credited, expected) in [
        (
            "BRACE UP - marlo venn x quiet ledger feat. MOMI & tilda marsh",
            channel.as_slice(),
            "BRACE UP",
        ),
        ("\"Blue-12\" | Kiri Hoshino", &["Quarry Hollow"], "Blue-12"),
        (
            "【Shiranami Koro V4 English】Fennick's in the Lighthouse【Cover】",
            &["OddlyKettle"],
            "Fennick's in the Lighthouse",
        ),
        (
            "ぺん -「うっかり機能」HOKKORITA 【HD.256k.Kara】",
            &["loopwick"],
            "うっかり機能",
        ),
        (
            "Shiranami Koro ~ Double Dummy - Full Song (English Subtitles v2) [LanternP - REUPLOAD w/subtitles added]",
            &["Kanzashi9"],
            "Shiranami Koro ~ Double Dummy",
        ),
        (
            "\"Kazoo\" feat. Shiranami Koro",
            &["Quarry Hollow"],
            "Kazoo",
        ),
        (
            "My Beloved Pet Snail feat. Shiranami Koro (The Ideal Pet)",
            &["MinnoEX", "Quarry Hollow"],
            "My Beloved Pet Snail (The Ideal Pet)",
        ),
        (
            "Mondays kinda drag ft. Kiri Hoshino",
            &["VELA"],
            "Mondays kinda drag",
        ),
        (
            "there's sleet in my socks! ❆",
            &["plain trapezoid"],
            "there's sleet in my socks!",
        ),
        (
            "【オリジナル楽曲】爆走!! ネコ神セレナーデ☆ / こもれびね（7さい）【OZONIC（くるみ&K.volt）】",
            &["こもれびね"],
            "爆走!! ネコ神セレナーデ☆",
        ),
        (
            "NO ENCORES / MARLO VENN + LOWTIDE",
            &["Marlo Venn / MarloV", "Q-BATT"],
            "NO ENCORES",
        ),
        (
            "[MV] PBW - LIVE AMA DINER (Streaming Festival 12.0)",
            &["PaperBoatsWander", "MOROMIKA"],
            "LIVE AMA DINER (Streaming Festival 12.0)",
        ),
        (
            "Just One More Puzzle / Marlo Venn feat. Kiri Hoshino",
            &["jomp gaming"],
            "Just One More Puzzle",
        ),
    ] {
        assert_eq!(upload(title, credited), expected, "{title}");
    }
}

#[test]
fn an_upload_s_dash_reads_by_who_is_credited_else_by_convention() {
    assert_eq!(upload("Song - Artist", &["Artist"]), "Song");
    assert_eq!(upload("Artist - Song", &[]), "Song");
    assert_eq!(upload("Venn & Hoshino - The Lantern", &[]), "The Lantern");
    assert_eq!(
        upload("Mr. Grey Tide - Paper Lantern Orchestra", &["PLO"]),
        "Mr. Grey Tide"
    );
    assert_eq!(
        upload("Song - Acoustic Version", &[]),
        "Song - Acoustic Version"
    );
    assert_eq!(
        upload("Artist - Song", &["Artist", "Song"]),
        "Artist - Song"
    );
    assert_eq!(
        upload("The \"Real\" Paper Moth", &[]),
        "The \"Real\" Paper Moth"
    );
    assert_eq!(upload("Defeat feat", &[]), "Defeat feat");
    assert_eq!(upload("feat. Kiri", &[]), "feat. Kiri");
}

#[test]
fn a_store_s_packaging_goes_and_another_recording_stays() {
    for (raw, expected) in [
        (
            "Like a Paper Lantern (Album Version)",
            "Like a Paper Lantern",
        ),
        (
            "Down by the Quarry Lookin' (Explicit) (Album Version)",
            "Down by the Quarry Lookin'",
        ),
        (
            "Lamplighter Girl  (Live at Fennick Hall, UK - August 1974)",
            "Lamplighter Girl (Live at Fennick Hall, UK - August 1974)",
        ),
        (
            "Ostrel Harbor Blues (Enhanced 1972 West Hollow Tapes)  ((1972 West Hollow Tapes Remastered Enhanced))",
            "Ostrel Harbor Blues (Enhanced 1972 West Hollow Tapes)",
        ),
        (
            "Song For Wren  ((Original 1972 Odell Ferrin album Version Remastered)) ((Original 1972 Odell Ferrin album Version Remastered))",
            "Song For Wren (Original 1972 Odell Ferrin album Version Remastered)",
        ),
        ("There Goes The Tide - 2019 Remaster", "There Goes The Tide"),
        ("Quill! (Remastered 2009)", "Quill!"),
        (
            "It'S All Quiet Now, Paper Moth  (Live)",
            "It's All Quiet Now, Paper Moth (Live)",
        ),
        (
            "Keepers of Rain (Live) (Live Version)",
            "Keepers of Rain (Live)",
        ),
        ("Song\u{a0}Title\u{200b}", "Song Title"),
    ] {
        assert_eq!(kept(Field::Title, raw), [expected], "{raw}");
    }
    for keep in [
        "To Odelie (Live)",
        "This Field Is Your Field (Live Version)",
        "Don't Fold Twice, It's All Paper (Demo)",
        "Don't Let Me Drift (2021 Mix)",
        "I'll Keep It in the Drawer (Studio Outtake - 1973)",
        "Fennick Road / Lantern Keeper (Reprise)",
        "Clean Slate Kid (Paper Harbor Alternate Take)",
        "Possibly 9th Avenue (Single Version)",
        "Refraction (Big Band Version)",
        "Tin Tin Song / Northern Kestrel (1972 West Hollow Tapes Original Version)",
        "Kapitel 3.4 - Logbooks, Vol. 1",
        "Fenn the Lamplighter (The Mighty Fenn)",
        "Live",
        "Clean",
        "O'Fennick's Song",
        "DON'T WAVE",
        "Driftin’ Lanterns",
        "#7 & 41",
    ] {
        assert_eq!(kept(Field::Title, keep), [keep], "{keep}");
    }
}

#[test]
fn albums_lose_a_disc_and_keep_their_edition() {
    assert_eq!(
        kept(Field::Album, "Marlo Venn Live (CD1)"),
        ["Marlo Venn Live"]
    );
    assert_eq!(kept(Field::Album, "Live - Disc 2 of 3"), ["Live"]);
    assert_eq!(disc_in("Paper Tides Of Ostrel (CD2)"), Some((22, 2)));
    assert_eq!(disc_in("Disco Lighthouse"), None);
    assert_eq!(
        disc_in("Discotheque Moons (Disc 1)").map(|(_, n)| n),
        Some(1)
    );
    for keep in [
        "MOTH ERA (DELUXE EDITION)",
        "The Glass Orchards 1971 – 1975 (2023 Edition)",
        "Disco Nimbus (Remastered)",
    ] {
        assert_eq!(kept(Field::Album, keep), [keep], "{keep}");
    }
    assert_eq!(
        kept(
            Field::Album,
            "The Ostrel Archive Volumes 1-3    (Rare And Unreleased)  1971-1984"
        ),
        ["The Ostrel Archive Volumes 1-3 (Rare And Unreleased) 1971-1984"]
    );
}

#[test]
fn joined_names_and_genres_split_where_a_list_is_certain() {
    assert_eq!(kept(Field::Genre, "Pop, Rock"), ["Pop", "Rock"]);
    assert_eq!(kept(Field::Genre, "Rock & Roll"), ["Rock & Roll"]);
    assert_eq!(
        kept(Field::Artist, "The Glass Orchards;Odell Ferrin"),
        ["The Glass Orchards", "Odell Ferrin"]
    );
    for keep in [
        "Fennick, Ostrel, Wren & Tamm",
        "QR/ZX",
        "Pella VXV",
        "Venn / Hoshino",
    ] {
        assert_eq!(kept(Field::Artist, keep), [keep], "{keep}");
    }
    assert_eq!(kept(Field::Artist, "Marlo Venn / MarloV"), ["Marlo Venn"]);
}

#[test]
fn a_channel_is_its_owner_s_name() {
    let channel = |v: &str| run(Field::Artist, &[v], false, &[], &Settings::default()).values;
    assert_eq!(channel("くもりび/ kumoribi"), ["くもりび"]);
    assert_eq!(channel("Marlo Venn / MarloV"), ["Marlo Venn"]);
    assert_eq!(channel("MarloVennVEVO"), ["MarloVenn"]);
    assert_eq!(channel("Some Band Official"), ["Some Band"]);
    assert_eq!(channel("Official"), ["Official"]);
    assert_eq!(channel("QR/ZX"), ["QR/ZX"]);
}

#[test]
fn an_album_named_as_the_artist_gives_way_to_its_owner() {
    let release = |album: &str| {
        Offers::from([
            (Field::Album, offer(&[album], true)),
            (Field::AlbumArtist, offer(&["Odell Ferrin"], true)),
        ])
    };
    let library = [release("Route 9 Revisited"), release("Rolling Fog")];
    let albums = Albums::of(&library);
    let mine = release("Rolling Fog");
    let ctx = Context {
        names: &[],
        albums: &albums,
        offers: &mine,
    };
    let artist = |v: &str| {
        clean(
            Field::Artist,
            &offer(&[v], true),
            &ctx,
            &Settings::default(),
        )
    };
    let c = artist("Route 9 Revisited");
    assert_eq!(c.values, ["Odell Ferrin"]);
    assert_eq!(c.rules, ["structure.album_as_artist"]);
    assert_eq!(artist("Pellucid Fox").values, ["Pellucid Fox"]);
}

#[test]
fn each_rule_names_itself_and_stops_when_switched_off() {
    let all = run(
        Field::Title,
        &["Song  (Album Version)"],
        true,
        &[],
        &Settings::default(),
    );
    assert_eq!(all.values, ["Song"]);
    assert_eq!(all.rules, ["tidy.spacing", "packaging.album_version"]);

    let mut settings = Settings::default();
    assert!(settings.set("packaging.album_version", false));
    assert!(!settings.set("packaging.nonsense", false));
    let some = run(
        Field::Title,
        &["Song  (Album Version)"],
        true,
        &[],
        &settings,
    );
    assert_eq!(some.values, ["Song (Album Version)"]);
    assert_eq!(some.rules, ["tidy.spacing"]);

    let mut settings = Settings::default();
    settings.set("guesswork.title_artist", false);
    settings.set("guesswork.credits", false);
    assert_eq!(
        run(
            Field::Title,
            &["Song / Kiri feat. Koro"],
            false,
            &[],
            &settings
        )
        .values,
        ["Song / Kiri feat. Koro"]
    );
}

#[test]
fn with_every_rule_off_nothing_changes() {
    for (title, credited, _, _) in PAIRS {
        let c = run(Field::Title, &[title], false, credited, &Settings::none());
        assert_eq!(c.values, [*title]);
        assert!(c.rules.is_empty());
    }
    let c = run(Field::Genre, &["Pop, Rock"], true, &[], &Settings::none());
    assert_eq!(c.values, ["Pop, Rock"]);
}

#[test]
fn a_rule_never_empties_a_value() {
    assert_eq!(upload("【MV】", &[]), "【MV】");
    assert_eq!(upload("(Official Video)", &[]), "(Official Video)");
    assert_eq!(kept(Field::Title, "(Album Version)"), ["(Album Version)"]);
}

#[test]
fn every_switch_belongs_to_a_tier() {
    let switches: Vec<_> = switches().collect();
    assert_eq!(switches.len(), RULES.len());
    assert!(
        switches
            .iter()
            .all(|(tier, _)| TIERS.iter().any(|(t, _)| t == tier))
    );
}

#[test]
fn a_featuring_credit_is_no_part_of_the_name() {
    assert_eq!(without_credits("ルララリ (feat. Kiri Hoshino)"), "ルララリ");
    assert_eq!(without_credits("Song [ft. A]"), "Song");
    assert_eq!(without_credits("(feat. A)"), "(feat. A)");
    assert_eq!(without_credits("Defeated"), "Defeated");
}

#[test]
fn packaging_words_never_outweigh_another_recording() {
    assert!(is_packaging("Official Music Video"));
    assert!(is_packaging("M/V"));
    assert!(is_packaging("Full Ver."));
    assert!(!is_packaging("Live Video"));
    assert!(!is_packaging("feat. Kiri"));
    assert!(!is_packaging("Cover"));
}
