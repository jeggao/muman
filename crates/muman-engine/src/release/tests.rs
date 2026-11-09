use super::*;
use crate::codec::Codec;
use crate::resolve::{AudioRef, Format, Plan, TAGGING, TagWhy, Why, render_version};

const ALBUM: &str = "00000000-0000-0000-0000-0000000000a1";
const BEST_OF: &str = "00000000-0000-0000-0000-0000000000a2";
const VIDEO: &str = "00000000-0000-0000-0000-0000000000a3";

fn file(n: usize) -> SourceKey {
    SourceKey::parse(&format!(
        "torrent:{}/Harbor Nights/0{n}.flac",
        "a".repeat(40)
    ))
    .unwrap()
}

fn record(n: usize) -> SourceKey {
    SourceKey::parse(&format!(
        "musicbrainz:00000000-0000-0000-0000-00000000000{n}"
    ))
    .unwrap()
}

/// The recording of song `n`, on each of `on`: a release and the track.
fn recording(n: usize, on: &[(&str, &str, &[&str], u32)]) -> Candidate {
    Candidate {
        id: crate::musicbrainz::Mbid::parse(&format!("00000000-0000-0000-0000-0000000000{n}0"))
            .unwrap(),
        title: format!("Song {n}"),
        artists: vec!["Marlo Venn".into()],
        length_ms: Some(200_000),
        score: 950,
        sources: 4,
        releases: on
            .iter()
            .map(|(id, title, types, track)| CandidateRelease {
                id: (*id).to_string(),
                group_id: Some(format!("{id}-group")),
                title: (*title).to_string(),
                date: Some(if *id == ALBUM { "2001" } else { "2009" }.into()),
                country: Some("XW".into()),
                types: types.iter().map(|t| (*t).to_string()).collect(),
                disc: Some(1),
                discs: Some(1),
                track: Some(*track),
                tracks: Some(3),
            })
            .collect(),
    }
}

fn offer(value: &str) -> Offer {
    Offer {
        values: vec![value.to_string()],
        structured: true,
    }
}

/// Songs of one folder, each a file numbered as its place and a record on
/// `on[n]`, the release it is on now.
struct Library {
    songs: Vec<Song>,
    facts: BTreeMap<SourceKey, Facts>,
    planned: Vec<(usize, Resolved)>,
    candidates: BTreeMap<SourceKey, Vec<Candidate>>,
}

fn library(on: &[&str], candidates: Vec<Vec<Candidate>>) -> Library {
    let mut lib = Library {
        songs: Vec::new(),
        facts: BTreeMap::new(),
        planned: Vec::new(),
        candidates: BTreeMap::new(),
    };
    for (n, (release, found)) in on.iter().zip(candidates).enumerate() {
        let mut f = Facts::unreadable(format!("f{n}"));
        f.duration = Some(200.0);
        f.tags.insert(Field::Album, offer("Harbor Nights"));
        f.tags.insert(Field::Track, offer(&(n + 1).to_string()));
        f.audio = Some(crate::facts::AudioFacts::default());
        lib.facts.insert(file(n), f);
        let mut r = Facts::unreadable(format!("r{n}"));
        let id = format!("00000000-0000-0000-0000-0000000000{n}0");
        r.tags.insert(Field::MusicBrainzTrackId, offer(&id));
        lib.facts.insert(record(n), r);
        lib.candidates.insert(record(n), found);
        lib.songs.push(Song {
            sources: vec![file(n), record(n)],
            ..Song::default()
        });
        let plan = Plan {
            folder: None,
            version: render_version(Codec::Flac),
            format: Format::Copy { codec: Codec::Flac },
            audio: AudioRef {
                key: file(n),
                rev: format!("f{n}"),
                index: 0,
            },
            cover: None,
            lyrics: None,
            tags: vec![(
                Field::MusicBrainzAlbumId.vorbis().into(),
                vec![(*release).to_string()],
            )],
            loudness: None,
            tagging: TAGGING,
            chapters: Vec::new(),
        };
        let why = Why {
            tags: vec![TagWhy {
                key: Field::MusicBrainzAlbumId.vorbis().into(),
                from: record(n).to_string(),
                cleaned: Vec::new(),
            }],
            ..Why::default()
        };
        lib.planned.push((
            n,
            Resolved {
                plan,
                stem: format!("song {n}").into(),
                why,
            },
        ));
    }
    lib
}

fn choose_in(lib: &Library, settings: &Release) -> BTreeMap<usize, Pick> {
    let none: &[Candidate] = &[];
    choose(
        &lib.planned,
        &lib.songs,
        &lib.facts,
        &|_, key| lib.candidates.get(key).map_or(none, Vec::as_slice),
        settings,
    )
}

fn value(pick: &Pick, field: Field) -> Option<&str> {
    pick.offers.get(&field)?.values.first().map(String::as_str)
}

#[test]
fn songs_of_one_folder_are_put_on_the_release_that_holds_them_all() {
    let on_both = |n: usize| {
        recording(
            n,
            &[
                (
                    ALBUM,
                    "Harbor Nights",
                    &["Album"],
                    u32::try_from(n).unwrap() + 1,
                ),
                (
                    BEST_OF,
                    "The Best of Marlo Venn",
                    &["Album", "Compilation"],
                    9,
                ),
            ],
        )
    };
    let lib = library(
        &[ALBUM, ALBUM, BEST_OF],
        vec![vec![on_both(0)], vec![on_both(1)], vec![on_both(2)]],
    );
    let picks = choose_in(&lib, &Release::default());
    assert_eq!(
        picks.keys().copied().collect::<Vec<_>>(),
        [2],
        "the one elsewhere"
    );
    let pick = &picks[&2];
    assert_eq!(pick.key, record(2));
    assert_eq!(value(pick, Field::Album), Some("Harbor Nights"));
    assert_eq!(value(pick, Field::MusicBrainzAlbumId), Some(ALBUM));
    assert_eq!(value(pick, Field::Track), Some("3"));
    assert_eq!(value(pick, Field::Date), Some("2001"));
    assert_eq!(
        value(pick, Field::MusicBrainzTrackId),
        None,
        "its record's recording"
    );
    assert_eq!(
        pick.why,
        "on \"Harbor Nights\", the release 3 of its album's 3 songs are on"
    );
}

#[test]
fn a_release_holding_too_few_of_a_folder_s_songs_is_not_its_album_s() {
    let lib = library(
        &[ALBUM, BEST_OF, VIDEO],
        vec![
            vec![recording(0, &[(ALBUM, "Harbor Nights", &["Album"], 1)])],
            vec![recording(
                1,
                &[(BEST_OF, "Best Of", &["Album", "Compilation"], 4)],
            )],
            vec![recording(
                2,
                &[(VIDEO, "Harbor Nights Live", &["Album", "Live"], 3)],
            )],
        ],
    );
    assert!(choose_in(&lib, &Release::default()).is_empty());
}

#[test]
fn a_kind_avoided_is_taken_only_where_nothing_else_would_be() {
    let two = |n: usize| {
        recording(
            n,
            &[
                (
                    BEST_OF,
                    "Harbor Nights",
                    &["Album", "Compilation"],
                    1 + u32::try_from(n).unwrap(),
                ),
                (
                    VIDEO,
                    "Harbor Nights",
                    &["Album", "Live"],
                    1 + u32::try_from(n).unwrap(),
                ),
            ],
        )
    };
    let lib = library(&[BEST_OF, BEST_OF], vec![vec![two(0)], vec![two(1)]]);
    let avoid = Release {
        avoid: vec!["compilation".into()],
        ..Release::default()
    };
    let picks = choose_in(&lib, &avoid);
    assert_eq!(picks.len(), 2);
    assert!(
        picks
            .values()
            .all(|p| value(p, Field::MusicBrainzAlbumId) == Some(VIDEO))
    );
    assert!(
        choose_in(&lib, &Release::default()).is_empty(),
        "both on it already"
    );
}

#[test]
fn two_songs_never_share_a_track() {
    let lib = library(
        &[BEST_OF, BEST_OF],
        vec![
            vec![recording(0, &[(ALBUM, "Harbor Nights", &["Album"], 1)])],
            vec![
                recording(1, &[(ALBUM, "Harbor Nights", &["Album"], 1)]),
                recording(2, &[(ALBUM, "Harbor Nights", &["Album"], 2)]),
            ],
        ],
    );
    let picks = choose_in(&lib, &Release::default());
    assert_eq!(value(&picks[&0], Field::Track), Some("1"));
    assert_eq!(value(&picks[&1], Field::Track), Some("2"));
    assert_eq!(
        value(&picks[&1], Field::MusicBrainzTrackId),
        Some("00000000-0000-0000-0000-000000000020"),
        "another recording of its print"
    );
}

#[test]
fn original_puts_each_song_on_its_recording_s_first_album() {
    let lib = library(
        &[BEST_OF],
        vec![vec![recording(
            0,
            &[
                (BEST_OF, "Best Of", &["Album", "Compilation"], 4),
                (ALBUM, "Harbor Nights", &["Album"], 1),
            ],
        )]],
    );
    let original = Release {
        prefer: ReleasePrefer::Original,
        ..Release::default()
    };
    let picks = choose_in(&lib, &original);
    assert_eq!(value(&picks[&0], Field::MusicBrainzAlbumId), Some(ALBUM));
    assert!(
        picks[&0]
            .why
            .ends_with("the first album its recording came out on")
    );
}

#[test]
fn least_cost_places_every_row_it_can_once() {
    let costs = vec![
        vec![Some(4), Some(1), None],
        vec![Some(2), Some(0), Some(5)],
        vec![None, None, None],
    ];
    assert_eq!(least_cost(&costs), [Some(1), Some(0), None]);
    assert_eq!(least_cost(&[vec![None, Some(3)]]), [Some(1)]);
    assert_eq!(least_cost(&[]), Vec::<Option<usize>>::new());
}

#[test]
fn a_song_pinned_to_a_release_goes_on_it_and_counts_for_no_album() {
    let on_both = |n: usize| {
        let track = u32::try_from(n).unwrap() + 1;
        recording(
            n,
            &[
                (ALBUM, "Harbor Nights", &["Album"], track),
                (
                    BEST_OF,
                    "The Best of Marlo Venn",
                    &["Album", "Compilation"],
                    9,
                ),
            ],
        )
    };
    let mut lib = library(
        &[ALBUM, ALBUM, ALBUM],
        vec![vec![on_both(0)], vec![on_both(1)], vec![on_both(2)]],
    );
    lib.songs[2].release = Some(BEST_OF.into());
    let picks = choose_in(&lib, &Release::default());
    assert_eq!(picks.keys().copied().collect::<Vec<_>>(), [2]);
    assert_eq!(value(&picks[&2], Field::MusicBrainzAlbumId), Some(BEST_OF));
    assert!(picks[&2].why.ends_with("the release it is pinned to"));
}
