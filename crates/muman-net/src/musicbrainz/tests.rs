use std::sync::Mutex;

use super::*;
use crate::http::TransportError;
use crate::lrclib::testing::Server;
use crate::record::{Alias, CreditedArtist};

const REC: &str = "00000000-0000-0000-0000-00000000000";

/// The recording ID `REC` ends with `n`.
fn mbid(n: &str) -> Mbid {
    Mbid::parse(&format!("{REC}{n}")).unwrap()
}
const REL: &str = "00000000-0000-0000-0000-0000000000a";

fn query() -> Query {
    Query {
        title: "Lantern Weather".into(),
        artist: "Paper Comets, Ada Quill".into(),
        album: None,
        seconds: 354.0,
        mono: None,
    }
}

fn client<'a>(server: &'a (dyn HttpTransport + Sync), throttle: &'a Throttle) -> Client<'a> {
    Client {
        base: "http://mb.test/",
        transport: server,
        throttle,
        releases: Box::leak(Box::default()),
    }
}

/// One recording of `title`, `length` ms long, on the releases given.
fn recording(n: u32, title: &str, length: u64, releases: &str) -> String {
    format!(
        r#"{{"id": "{REC}{n}", "score": 100, "title": "{title}", "length": {length},
            "artist-credit": [{{"name": "Paper Comets", "joinphrase": " & ",
                "artist": {{"id": "{REC}a", "name": "Paper Comets"}}}}, {{"name": "Ada Quill"}}],
            "isrcs": [{{"id": "XX0000000001"}}],
            "releases": [{releases}]}}"#
    )
}

/// A release of type `primary`, with `secondary` types, as a search lists it.
fn release(n: u32, title: &str, primary: &str, secondary: &str, date: &str) -> String {
    format!(
        r#"{{"id": "{REL}{n}", "title": "{title}", "status": "Official", "date": "{date}",
            "artist-credit": [{{"name": "Paper Comets"}}],
            "country": "XW",
            "release-group": {{"id": "{REL}{n}", "primary-type": "{primary}", "secondary-types": [{secondary}]}},
            "media": [{{"position": 2, "track-offset": 4, "track-count": 11,
                "track": [{{"id": "{REL}{n}", "number": "5"}}]}}]}}"#
    )
}

fn search(recordings: &[String]) -> String {
    format!(
        r#"{{"count": 9, "recordings": [{}]}}"#,
        recordings.join(",")
    )
}

#[test]
fn the_official_album_of_the_fitting_recording_is_taken() {
    let body = search(&[
        recording(
            1,
            "Lantern Weather",
            300_000,
            &release(1, "Far Off", "Album", "", "1999"),
        ),
        recording(
            2,
            "Lantern Weather",
            355_000,
            &[
                release(2, "Best of the Comets", "Album", r#""Compilation""#, "2001"),
                release(3, "Lantern Weather", "Single", "", "2003-01-02"),
                release(4, "Rooms of Salt", "Album", "", "2003-05-06"),
                release(5, "Rooms of Salt (Deluxe)", "Album", "", "2010"),
            ]
            .join(","),
        ),
    ]);
    let server = Server::default().answer("/ws/2/recording?", &body);
    let throttle = Throttle::none();
    let record = client(&server, &throttle).find(&query()).unwrap().unwrap();
    assert_eq!(record.id.as_str(), format!("{REC}2"));
    assert_eq!(record.artists, ["Paper Comets", "Ada Quill"]);
    assert_eq!(
        record.artist_ids,
        [format!("{REC}a")],
        "an artist without one is skipped"
    );
    assert_eq!(record.isrcs, ["XX0000000001"]);
    let release = record.release.unwrap();
    assert_eq!(release.title, "Rooms of Salt");
    assert_eq!(release.date.as_deref(), Some("2003-05-06"));
    assert_eq!((release.track, release.disc), (Some(5), Some(2)));
    assert_eq!(release.tracks, Some(11));
    assert_eq!(release.country.as_deref(), Some("XW"));
    assert_eq!(release.group_id, Some(format!("{REL}4")));
    assert_eq!(release.track_id, Some(format!("{REL}4")));
    let asked = server.asked.lock().unwrap();
    assert_eq!(
        asked[0],
        "http://mb.test/ws/2/recording?query=recording%3A%22Lantern%20Weather%22%20AND%20\
         artist%3A%22Paper%20Comets%22%20AND%20dur%3A%5B351000%20TO%20357000%5D&limit=100&fmt=json"
    );
}

#[test]
fn the_recording_on_most_releases_is_the_original_whatever_its_comment() {
    let album = |n| release(n, "Rooms of Salt", "Album", "", "2005");
    let comment = |r: String, c: &str| {
        r.replace(
            r#""score""#,
            &format!(r#""disambiguation": "{c}", "score""#),
        )
    };
    let mix = comment(
        recording(
            1,
            "Lantern Weather",
            355_000,
            &[album(1), album(6)].join(","),
        ),
        "2005 surround mix",
    );
    let reissue = recording(2, "Lantern Weather", 354_000, &album(2));
    let original = comment(
        recording(
            3,
            "Lantern Weather",
            356_000,
            &[
                album(3),
                release(4, "Rooms of Salt", "Album", "", "1991"),
                album(5),
            ]
            .join(","),
        ),
        "original mix",
    );
    let server = Server::default().answer("/ws/2/recording?", &search(&[mix, reissue, original]));
    let throttle = Throttle::none();
    let record = client(&server, &throttle).find(&query()).unwrap().unwrap();
    assert_eq!(record.id.as_str(), format!("{REC}3"));
    assert_eq!(record.release.unwrap().date.as_deref(), Some("1991"));
}

#[test]
fn the_song_s_own_album_ranks_first() {
    let body = search(&[recording(
        1,
        "Lantern Weather",
        354_000,
        &[
            release(1, "Rooms of Salt", "Album", "", "2003"),
            release(2, "Best of the Comets", "Album", r#""Compilation""#, "2001"),
        ]
        .join(","),
    )]);
    let server = Server::default().answer("/ws/2/recording?", &body);
    let throttle = Throttle::none();
    let q = Query {
        album: Some("Best of the Comets".into()),
        ..query()
    };
    let record = client(&server, &throttle).find(&q).unwrap().unwrap();
    assert_eq!(record.release.unwrap().title, "Best of the Comets");
}

#[test]
fn nothing_fits_another_song_another_length_or_a_video() {
    let video = recording(3, "Lantern Weather", 354_000, "")
        .replace(r#""score""#, r#""video": true, "score""#);
    let body = search(&[
        recording(1, "Second Tune", 354_000, ""),
        recording(2, "Lantern Weather", 362_000, ""),
        video,
    ]);
    let server = Server::default().answer("/ws/2/recording?", &body);
    let throttle = Throttle::none();
    assert_eq!(client(&server, &throttle).find(&query()).unwrap(), None);
    let none = Server::default();
    assert_eq!(client(&none, &throttle).find(&query()).unwrap(), None);
}

#[test]
fn a_recording_on_no_release_offers_its_own_names() {
    let body = search(&[recording(1, "Lantern Weather", 353_000, "")]);
    let server = Server::default().answer("/ws/2/recording?", &body);
    let throttle = Throttle::none();
    let record = client(&server, &throttle).find(&query()).unwrap().unwrap();
    assert_eq!(record.title, "Lantern Weather");
    assert_eq!(record.release, None);
}

#[test]
fn a_record_fetched_again_keeps_the_album_it_was_kept_on() {
    let body = recording(
        8,
        "Lantern Weather",
        355_000,
        &[
            release(1, "Rooms of Salt", "Album", "", "2003"),
            release(2, "Best of the Comets", "Album", "", "1999"),
        ]
        .join(","),
    );
    let server = Server::default().answer(&format!("/ws/2/recording/{REC}8?inc="), &body);
    let throttle = Throttle::none();
    let again = |album| {
        client(&server, &throttle)
            .by_id(&mbid("8"), album)
            .unwrap()
            .unwrap()
            .release
            .unwrap()
            .title
    };
    assert_eq!(again(None), "Best of the Comets", "the earliest, unasked");
    assert_eq!(again(Some("Rooms of Salt")), "Rooms of Salt");
}

#[test]
fn a_lookup_by_id_reads_the_track_a_lookup_lists() {
    let body = format!(
        r#"{{"id": "{REC}7", "title": "Lantern Weather", "length": null, "video": false,
            "isrcs": ["XX0000000002"],
            "artist-credit": [{{"name": "Paper Comets"}}],
            "relations": [{{"type": "performance", "work": {{"languages": ["jpn", "eng"]}}}},
                          {{"type": "performance", "work": {{"languages": ["eng"]}}}},
                          {{"type": "producer", "artist": {{"name": "Odile Marsh"}}}}],
            "releases": [{{"id": "{REL}7", "title": "Rooms of Salt", "status": null,
                "release-group": {{"primary-type": "Album", "secondary-types": []}},
                "media": [{{"position": 1, "track-offset": 2, "tracks": [{{"position": 3, "number": "A3"}}]}}]}}]}}"#
    );
    let server = Server::default().answer(&format!("/ws/2/recording/{REC}7?inc="), &body);
    let throttle = Throttle::none();
    let record = client(&server, &throttle)
        .by_id(&mbid("7"), None)
        .unwrap()
        .unwrap();
    assert_eq!(record.isrcs, ["XX0000000002"]);
    assert_eq!(record.languages, ["jpn", "eng"], "each work's, each once");
    let release = record.release.unwrap();
    assert_eq!(
        (release.track, release.disc, release.date),
        (Some(3), Some(1), None)
    );
    assert_eq!(
        client(&Server::default(), &throttle)
            .by_id(&mbid("f"), None)
            .unwrap(),
        None
    );
}

/// Refuses the first `refusals` requests with a 503, then answers an
/// empty search; records each request's headers.
#[derive(Debug, Default)]
struct Busy {
    refusals: usize,
    asked: Mutex<Vec<Vec<(String, String)>>>,
}

impl HttpTransport for Busy {
    fn get(
        &self,
        _: &str,
        headers: &[(&str, &str)],
        _: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        let mut asked = self.asked.lock().unwrap();
        asked.push(
            headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        );
        if asked.len() <= self.refusals {
            return Err(TransportError::Status {
                code: 503,
                body: "Your requests are exceeding the allowable rate limit.".into(),
                retry_after: None,
            });
        }
        Ok(br#"{"recordings": []}"#.to_vec())
    }
}

#[test]
fn a_refusal_for_going_too_fast_is_asked_again_three_times() {
    let throttle = Throttle::none();
    let busy = Busy {
        refusals: 3,
        ..Busy::default()
    };
    assert_eq!(client(&busy, &throttle).find(&query()).unwrap(), None);
    let asked = busy.asked.lock().unwrap();
    assert_eq!(asked.len(), 4);
    assert!(
        asked[0]
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.starts_with("muman/"))
    );
    assert!(
        asked[0]
            .iter()
            .any(|(k, v)| k == "Accept" && v == "application/json")
    );

    let busier = Busy {
        refusals: 4,
        ..Busy::default()
    };
    let e = client(&busier, &throttle).find(&query()).unwrap_err();
    assert!(e.downcast_ref::<crate::http::Refusing>().is_some(), "{e:#}");
}

#[test]
fn a_recording_titled_exactly_wins_over_one_holding_the_title() {
    let body = search(&[
        recording(
            1,
            "Lantern Weather Again",
            355_000,
            &release(1, "Rooms of Salt", "Album", "", "2003"),
        ),
        recording(
            2,
            "Lantern Weather",
            355_000,
            &release(2, "Lantern Weather", "Single", "", "2004"),
        ),
    ]);
    let server = Server::default().answer("/ws/2/recording?", &body);
    let throttle = Throttle::none();
    let record = client(&server, &throttle).find(&query()).unwrap().unwrap();
    assert_eq!(record.title, "Lantern Weather");
}

#[test]
fn an_mbid_has_its_shape() {
    assert!(is_mbid("0a1b2c3d-0000-4fff-8000-00000000000f"));
    assert!(!is_mbid("0a1b2c3d00004fff800000000000000f"));
    assert!(!is_mbid("0a1b2c3d-0000-4fff-8000-00000000000g"));
}

#[test]
fn a_record_is_kept_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let record = Record {
        id: mbid("1"),
        title: "Lantern Weather".into(),
        artists: vec!["Paper Comets".into()],
        credit: String::new(),
        artist_ids: Vec::new(),
        comment: String::new(),
        isrcs: Vec::new(),
        length_ms: Some(354_000),
        release: None,
        editions: Vec::new(),
        original_date: Some("2001-05-03".into()),
        languages: vec!["eng".into()],
        credited_artists: vec![CreditedArtist {
            id: format!("{REC}a"),
            name: "Paper Comets".into(),
            sort_name: "Comets, Paper".into(),
            aliases: vec![Alias {
                name: "ペーパー・コメッツ".into(),
                sort_name: String::new(),
                locale: Some("ja".into()),
                primary: true,
                kind: Some("Artist name".into()),
                ended: false,
            }],
            genres: Vec::new(),
        }],
        genres: vec![Genre {
            name: "dream pop".into(),
            count: 2,
        }],
    };
    let path = keep(dir.path(), &record).unwrap();
    assert_eq!(path, path_of(dir.path(), &record.id));
    assert_eq!(read(&path).unwrap(), record);
}

#[test]
fn a_release_s_label_and_barcode_are_looked_up_once_an_album() {
    let body = search(&[recording(
        1,
        "Lantern Weather",
        355_000,
        &release(1, "Rooms of Salt", "Album", r#""Compilation""#, "2003"),
    )
    .replace(
        r#""score": 100"#,
        r#""score": 100, "first-release-date": "2001-05-03""#,
    )
    .replace(
        r#""track-count": 11"#,
        r#""track-count": 11, "format": "CD""#,
    )]);
    let looked = r#"{"id": "x", "barcode": "0012345678905", "asin": "B000000001",
        "release-group": {"first-release-date": "1999-02"},
        "text-representation": {"language": "eng", "script": "Latn"},
        "label-info": [{"catalog-number": "HLR-0042", "label": {"name": "Harbor Lights Records"}},
                       {"catalog-number": "HLR-0042", "label": {"name": "Harbor Lights Records"}}]}"#;
    let works = r#"{"id": "x", "relations": [{"work": {"languages": ["kor"]}}]}"#;
    let server = Server::default()
        .answer("/ws/2/recording?", &body)
        .answer("/ws/2/recording/", works)
        .answer("/ws/2/release/", looked);
    let throttle = Throttle::none();
    let c = client(&server, &throttle);
    let record = c.find(&query()).unwrap().unwrap();
    assert_eq!(record.languages, ["kor"], "a search lists no works");
    assert_eq!(record.original_date.as_deref(), Some("2001-05-03"));
    let r = record.release.unwrap();
    assert_eq!(r.status.as_deref(), Some("Official"));
    assert_eq!(r.types, ["Album", "Compilation"]);
    assert_eq!(r.media.as_deref(), Some("CD"));
    assert_eq!(r.details.labels, ["Harbor Lights Records"]);
    assert_eq!(r.details.catalog_numbers, ["HLR-0042"]);
    assert_eq!(r.details.barcode.as_deref(), Some("0012345678905"));
    assert_eq!(r.details.script.as_deref(), Some("Latn"));
    assert_eq!(r.details.original_date.as_deref(), Some("1999-02"));
    c.find(&query()).unwrap().unwrap();
    let asked = server.asked.lock().unwrap();
    assert_eq!(
        asked
            .iter()
            .filter(|u| u.contains("/ws/2/release/"))
            .count(),
        1,
        "{asked:?}"
    );
    drop(asked);

    let failing = Server::default()
        .answer("/ws/2/recording?", &body)
        .answer("/ws/2/release/", "<html>busy</html>");
    let c = client(&failing, &throttle);
    assert!(
        c.find(&query()).is_err(),
        "a release that did not answer is asked again, not kept without its details"
    );
}

/// `release` moved into the release group `group`.
fn in_group(release: &str, n: u32, group: &str) -> String {
    release.replace(
        &format!(r#""release-group": {{"id": "{REL}{n}""#),
        &format!(r#""release-group": {{"id": "{group}""#),
    )
}

#[test]
fn a_record_keeps_its_credit_its_comment_and_its_album_s_other_editions() {
    let group = format!("{REL}0");
    let body = search(&[recording(
        1,
        "Lantern Weather",
        354_000,
        &[
            in_group(&release(1, "Rooms of Salt", "Album", "", "1991"), 1, &group),
            in_group(&release(2, "Rooms of Salt", "Album", "", "2003"), 2, &group),
            in_group(&release(2, "Rooms of Salt", "Album", "", "2003"), 2, &group),
            release(3, "Best of the Comets", "Album", r#""Compilation""#, "2001"),
        ]
        .join(","),
    )
    .replace(r#""score""#, r#""disambiguation": "mono mix", "score""#)]);
    let server = Server::default().answer("/ws/2/recording?", &body);
    let throttle = Throttle::none();
    let record = client(&server, &throttle).find(&query()).unwrap().unwrap();
    assert_eq!(record.credit, "Paper Comets & Ada Quill");
    assert_eq!(record.comment, "mono mix");
    assert_eq!(record.release.unwrap().id, format!("{REL}1"));
    let editions: Vec<&str> = record.editions.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        editions,
        [format!("{REL}2")],
        "each once, of its group alone"
    );
}

#[test]
fn a_comment_naming_the_other_mix_than_the_song_s_ranks_after_its_own() {
    let album = |n| release(n, "Rooms of Salt", "Album", "", "1991");
    let comment = |r: String, c: &str| {
        r.replace(
            r#""score""#,
            &format!(r#""disambiguation": "{c}", "score""#),
        )
    };
    let stereo = comment(
        recording(
            1,
            "Lantern Weather",
            354_000,
            &[album(1), album(2)].join(","),
        ),
        "original stereo album mix",
    );
    let mono = comment(
        recording(2, "Lantern Weather", 354_000, &album(3)),
        "original mono album mix",
    );
    let body = search(&[stereo, mono]);
    let found = |mono: Option<bool>| {
        let server = Server::default().answer("/ws/2/recording?", &body);
        let throttle = Throttle::none();
        let q = Query { mono, ..query() };
        let record = client(&server, &throttle).find(&q).unwrap().unwrap();
        record.id.as_str().to_string()
    };
    assert_eq!(found(Some(true)), format!("{REC}2"));
    assert_eq!(found(Some(false)), format!("{REC}1"));
    assert_eq!(
        found(None),
        format!("{REC}1"),
        "unknown, the most releases win"
    );
}

#[test]
fn a_comment_names_a_mix_by_the_word() {
    assert!(other_mix("original mono album mix", Some(false)));
    assert!(other_mix("Stereo", Some(true)));
    assert!(!other_mix("original mono album mix", Some(true)));
    assert!(!other_mix("mono and stereo", Some(true)));
    assert!(!other_mix("monologue", Some(false)));
    assert!(!other_mix("stereo mix", None));
}

/// Paper Comets, credited with the aliases and genres a lookup asking for
/// them lists, in no order.
fn credited_with_aliases() -> String {
    format!(
        r#"[{{"name": "Paper Comets", "joinphrase": "",
            "artist": {{"id": "{REC}a", "name": "Paper Comets", "sort-name": "Comets, Paper",
                "aliases": [
                    {{"name": "Paper Commets", "sort-name": "Paper Commets", "locale": null,
                      "primary": null, "type": "Search hint", "ended": false}},
                    {{"name": "ペーパー・コメッツ", "sort-name": "ペーパー・コメッツ", "locale": "ja",
                      "primary": true, "type": "Artist name", "ended": false}},
                    {{"name": "Comets of Paper", "sort-name": "Comets of Paper", "locale": null,
                      "primary": null, "type": null, "ended": true}}],
                "genres": [{{"name": "dream pop", "id": "{REC}c", "count": 1, "disambiguation": ""}},
                           {{"name": "shoegaze", "id": "{REC}d", "count": 3, "disambiguation": ""}}]}}}}]"#
    )
}

#[test]
fn a_lookup_by_id_asks_for_aliases_and_genres_and_keeps_them() {
    let body = format!(
        r#"{{"id": "{REC}7", "title": "Lantern Weather", "length": 354000,
            "artist-credit": {},
            "genres": [{{"name": "indie rock", "count": 2}}, {{"name": "ambient", "count": 2}}],
            "releases": [{}]}}"#,
        credited_with_aliases(),
        release(1, "Rooms of Salt", "Album", "", "2003")
    );
    let looked = format!(
        r#"{{"id": "{REL}1", "artist-credit": {},
            "genres": [{{"name": "noise pop", "count": 1}}],
            "release-group": {{"first-release-date": "2003", "genres": [{{"name": "shoegaze", "count": 4}}]}}}}"#,
        credited_with_aliases()
    );
    let server = Server::default()
        .answer(&format!("/ws/2/recording/{REC}7?"), &body)
        .answer("/ws/2/release/", &looked);
    let throttle = Throttle::none();
    let record = client(&server, &throttle)
        .by_id(&mbid("7"), None)
        .unwrap()
        .unwrap();
    let asked = server.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2, "no request added: {asked:?}");
    assert!(
        asked[0].contains("?inc=work-rels+artist-credits+aliases+genres+releases+"),
        "{}",
        asked[0]
    );
    assert!(
        asked[1].ends_with("?inc=labels+release-groups+artist-credits+aliases+genres&fmt=json"),
        "{}",
        asked[1]
    );
    let genre = |name: &str, count: i32| Genre {
        name: name.into(),
        count,
    };
    assert_eq!(
        record.genres,
        [genre("ambient", 2), genre("indie rock", 2)],
        "the most voted first, then by name"
    );
    let artist = &record.credited_artists[0];
    assert_eq!(
        (
            artist.id.as_str(),
            artist.name.as_str(),
            artist.sort_name.as_str()
        ),
        (format!("{REC}a").as_str(), "Paper Comets", "Comets, Paper")
    );
    let aliases: Vec<&str> = artist.aliases.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        aliases,
        ["ペーパー・コメッツ", "Comets of Paper", "Paper Commets"],
        "primary first, then of a locale, then any but a search hint"
    );
    let primary = &artist.aliases[0];
    assert_eq!(
        (
            primary.locale.as_deref(),
            primary.primary,
            primary.kind.as_deref()
        ),
        (Some("ja"), true, Some("Artist name"))
    );
    assert!(artist.aliases[1].ended);
    assert_eq!(artist.genres, [genre("shoegaze", 3), genre("dream pop", 1)]);
    let details = record.release.unwrap().details;
    assert_eq!(details.credited_artists, record.credited_artists);
    assert_eq!(details.genres, [genre("noise pop", 1)]);
    assert_eq!(details.group_genres, [genre("shoegaze", 4)]);
}

#[test]
fn a_search_s_record_gains_aliases_and_genres_from_the_lookups_it_makes() {
    let body = search(&[recording(
        1,
        "Lantern Weather",
        355_000,
        &release(1, "Rooms of Salt", "Album", "", "2003"),
    )]);
    let looked = format!(
        r#"{{"id": "{REC}1", "artist-credit": {},
            "genres": [{{"name": "dream pop", "count": 5}}],
            "relations": [{{"work": {{"languages": ["eng"]}}}}]}}"#,
        credited_with_aliases()
    );
    let server = Server::default()
        .answer("/ws/2/recording?", &body)
        .answer("/ws/2/recording/", &looked)
        .answer("/ws/2/release/", r#"{"id": "x"}"#);
    let throttle = Throttle::none();
    let record = client(&server, &throttle).find(&query()).unwrap().unwrap();
    let asked = server.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 3, "no request added: {asked:?}");
    assert!(
        asked[2].ends_with("?inc=work-rels+artist-credits+aliases+genres&fmt=json"),
        "{}",
        asked[2]
    );
    assert_eq!(record.languages, ["eng"]);
    assert_eq!(record.genres[0].name, "dream pop");
    assert_eq!(record.credited_artists[0].aliases.len(), 3);
    let details = record.release.unwrap().details;
    assert!(
        details.credited_artists.is_empty() && details.group_genres.is_empty(),
        "a release answered without them still reads"
    );
}

#[test]
fn a_record_kept_before_aliases_and_genres_reads_without_them() {
    let json = format!(
        r#"{{"id": "{REC}1", "title": "Lantern Weather", "artists": ["Paper Comets"],
            "release": {{"id": "{REL}1", "title": "Rooms of Salt", "artists": ["Paper Comets"],
                "labels": ["Harbor Lights Records"]}}}}"#
    );
    let record: Record = serde_json::from_str(&json).unwrap();
    assert!(record.credited_artists.is_empty() && record.genres.is_empty());
    let release = record.release.unwrap();
    assert_eq!(release.details.labels, ["Harbor Lights Records"]);
    assert_eq!(release.details.genres, []);
}
