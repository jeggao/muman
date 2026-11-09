use super::*;
use crate::codec::Codec;
use crate::record::Alias;
use crate::resolve::{AudioRef, Format, Plan, TAGGING, TagWhy, Why, render_version};

const VENN: &str = "00000000-0000-0000-0000-0000000000b1";
const OTHER: &str = "00000000-0000-0000-0000-0000000000b2";
const RECORD: &str = "musicbrainz:00000000-0000-0000-0000-000000000001";

/// Song `n` of `tags`, each a field, its value and where it came from.
fn song(n: usize, tags: &[(Field, &str, &str)]) -> (usize, Resolved) {
    let (tags, why) = tags
        .iter()
        .map(|(f, v, from)| {
            let why = TagWhy {
                key: f.vorbis().to_string(),
                from: (*from).to_string(),
                cleaned: Vec::new(),
            };
            ((f.vorbis().to_string(), vec![(*v).to_string()]), why)
        })
        .unzip();
    let plan = Plan {
        folder: None,
        version: render_version(Codec::Opus),
        format: Format::Copy { codec: Codec::Opus },
        audio: AudioRef {
            key: SourceKey::youtube("vid00000001"),
            rev: "r".into(),
            index: 0,
        },
        cover: None,
        lyrics: None,
        tags,
        loudness: None,
        tagging: TAGGING,
        chapters: Vec::new(),
    };
    let why = Why {
        tags: why,
        ..Why::default()
    };
    (
        n,
        Resolved {
            plan,
            stem: format!("song {n}").into(),
            why,
        },
    )
}

/// A record crediting artist `id`, known by `name` and `aliases`, each
/// alias a name, its locale and whether it is primary.
fn knowing(id: &str, name: &str, aliases: &[(&str, &str, bool)]) -> BTreeMap<SourceKey, Facts> {
    let mut f = Facts::unreadable("r".into());
    f.artists = vec![CreditedArtist {
        id: id.into(),
        name: name.into(),
        sort_name: String::new(),
        aliases: aliases
            .iter()
            .map(|(name, locale, primary)| Alias {
                name: (*name).into(),
                sort_name: String::new(),
                locale: Some((*locale).into()),
                primary: *primary,
                kind: Some("Artist name".into()),
                ended: false,
            })
            .collect(),
        genres: Vec::new(),
    }];
    BTreeMap::from([(SourceKey::parse(RECORD).unwrap(), f)])
}

fn names(spelled: &BTreeMap<usize, Vec<Agreed>>) -> Vec<(usize, Field, &str)> {
    spelled
        .iter()
        .flat_map(|(n, all)| all.iter().map(move |a| (*n, a.field, a.values[0].as_str())))
        .collect()
}

#[test]
fn every_spelling_of_one_artist_is_written_as_its_own_name() {
    let planned = vec![
        song(
            0,
            &[
                (Field::Artist, "Marlo Venn", RECORD),
                (Field::MusicBrainzArtistId, VENN, RECORD),
            ],
        ),
        song(1, &[(Field::Artist, "MARLO VENN", "manual:a.flac")]),
        song(
            2,
            &[(Field::AlbumArtist, "The Marlo Venn", "manual:b.flac")],
        ),
    ];
    let facts = knowing(VENN, "Marlo Venn", &[]);
    let spelled = spell(&planned, &facts, Spelling::Original);
    assert_eq!(
        names(&spelled),
        [
            (1, Field::Artist, "Marlo Venn"),
            (2, Field::AlbumArtist, "Marlo Venn")
        ]
    );
    assert_eq!(
        spelled[&1][0].from,
        format!("musicbrainz artist {VENN}'s own name")
    );
}

#[test]
fn an_artist_is_written_in_its_own_script_or_in_latin_letters_as_asked() {
    let planned = vec![
        song(0, &[(Field::Artist, "Marlo Venn", "manual:a.flac")]),
        song(
            1,
            &[
                (Field::Artist, "マルロ・ヴェン", RECORD),
                (Field::MusicBrainzArtistId, VENN, RECORD),
            ],
        ),
    ];
    let facts = knowing(
        VENN,
        "マルロ・ヴェン",
        &[("Marlo Venn", "en", true), ("Marlo Ven", "de", false)],
    );
    let original = spell(&planned, &facts, Spelling::Original);
    assert_eq!(names(&original), [(0, Field::Artist, "マルロ・ヴェン")]);
    let latin = spell(&planned, &facts, Spelling::Latin);
    assert_eq!(names(&latin), [(1, Field::Artist, "Marlo Venn")]);
}

#[test]
fn a_name_two_artists_share_and_a_credit_of_several_are_left_alone() {
    let planned = vec![
        song(
            0,
            &[
                (Field::Artist, "Lanterns", RECORD),
                (Field::MusicBrainzArtistId, VENN, RECORD),
            ],
        ),
        song(
            1,
            &[
                (Field::Artist, "Lanterns", RECORD),
                (Field::MusicBrainzArtistId, OTHER, RECORD),
            ],
        ),
        song(2, &[(Field::Artist, "LANTERNS", "manual:a.flac")]),
        song(
            3,
            &[(Field::Artist, "Lanterns & Kiri Hoshino", "manual:b.flac")],
        ),
        song(4, &[(Field::Artist, "lanterns", "song.tags")]),
        song(5, &[(Field::Artist, "Glass Orchard", "manual:c.flac")]),
    ];
    let facts = knowing(VENN, "The Lanterns", &[]);
    assert_eq!(
        names(&spell(&planned, &facts, Spelling::Original)),
        [(0, Field::Artist, "The Lanterns")]
    );
}

#[test]
fn an_artist_known_by_its_id_alone_is_spelled_as_most_records_credit_it() {
    let planned: Vec<_> = ["Marlo Venn", "Marlo Venn", "MARLO VENN"]
        .iter()
        .enumerate()
        .map(|(n, name)| {
            song(
                n,
                &[
                    (Field::Artist, name, RECORD),
                    (Field::MusicBrainzArtistId, VENN, RECORD),
                ],
            )
        })
        .collect();
    let spelled = spell(&planned, &BTreeMap::new(), Spelling::Original);
    assert_eq!(names(&spelled), [(2, Field::Artist, "Marlo Venn")]);
    assert_eq!(
        spelled[&2][0].from,
        format!("as 2 of 3 records crediting musicbrainz artist {VENN} spell it")
    );
}

#[test]
fn records_spelling_an_artist_in_its_own_script_win_over_more_in_latin_letters() {
    let credit = |n: usize, name: &str| {
        song(
            n,
            &[
                (Field::Artist, name, RECORD),
                (Field::MusicBrainzArtistId, VENN, RECORD),
            ],
        )
    };
    let planned = vec![
        credit(0, "MARLO VENN"),
        credit(1, "MARLO VENN"),
        credit(2, "マルロ・ヴェン"),
    ];
    let spelled = spell(&planned, &BTreeMap::new(), Spelling::Original);
    assert_eq!(
        names(&spelled),
        [
            (0, Field::Artist, "マルロ・ヴェン"),
            (1, Field::Artist, "マルロ・ヴェン")
        ]
    );
    let tied = vec![credit(0, "The Lanterns"), credit(1, "Lanterns")];
    assert!(
        spell(&tied, &BTreeMap::new(), Spelling::Original).is_empty(),
        "a tie"
    );
}

#[test]
fn names_spelled_more_than_one_way_with_no_id_are_suggested() {
    let planned = vec![
        song(0, &[(Field::Artist, "Glass Orchard", "manual:a.flac")]),
        song(1, &[(Field::Artist, "Glass Orchard", "manual:b.flac")]),
        song(2, &[(Field::Artist, "GLASS ORCHARD", "manual:c.flac")]),
        song(3, &[(Field::Artist, "Paper Comets", "manual:d.flac")]),
    ];
    assert_eq!(
        suggestions(&planned, &BTreeMap::new()),
        [vec![
            ("Glass Orchard".to_string(), 2),
            ("GLASS ORCHARD".to_string(), 1)
        ]]
    );
}

#[test]
fn a_search_hint_neither_spells_an_artist_nor_joins_a_name_to_it() {
    let planned = vec![
        song(
            0,
            &[
                (Field::Artist, "マルロ・ヴェン", RECORD),
                (Field::Artists, "マルロ・ヴェン", RECORD),
                (Field::MusicBrainzArtistId, VENN, RECORD),
            ],
        ),
        song(1, &[(Field::Artist, "Venn", "manual:a.flac")]),
    ];
    let mut facts = knowing(
        VENN,
        "マルロ・ヴェン",
        &[("Marlo Venn", "ja", false), ("Aaron Legal", "en", false)],
    );
    let artist = &mut facts.values_mut().next().unwrap().artists[0];
    artist.aliases[1].kind = Some("Legal name".into());
    artist.aliases.push(Alias {
        name: "Venn".into(),
        sort_name: String::new(),
        locale: None,
        primary: false,
        kind: Some("Search hint".into()),
        ended: false,
    });
    let latin = spell(&planned, &facts, Spelling::Latin);
    assert_eq!(
        names(&latin),
        [
            (0, Field::Artist, "Marlo Venn"),
            (0, Field::Artists, "Marlo Venn")
        ],
        "an artist name before a legal name, its ARTISTS with it, and no hint"
    );
}

#[test]
fn an_album_artist_taken_from_an_artist_set_by_hand_is_left_alone() {
    let planned = vec![song(
        0,
        &[
            (Field::Artist, "MARLO VENN", "song.tags"),
            (
                Field::AlbumArtist,
                "MARLO VENN",
                "the first artist of song.tags",
            ),
            (Field::MusicBrainzArtistId, VENN, RECORD),
        ],
    )];
    let facts = knowing(VENN, "Marlo Venn", &[]);
    assert!(spell(&planned, &facts, Spelling::Original).is_empty());
}
