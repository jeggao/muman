use std::sync::Mutex;

use super::*;
use crate::http::TransportError;
use crate::lrclib::testing::Server;

const REC: &str = "00000000-0000-0000-0000-00000000000";
const REL: &str = "00000000-0000-0000-0000-0000000000a";

fn query() -> Query {
    Query {
        title: "Lantern Weather".into(),
        artist: "Paper Comets, Ada Quill".into(),
        album: None,
        seconds: 354.0,
    }
}

fn client<'a>(server: &'a (dyn HttpTransport + Sync), throttle: &'a Throttle) -> Client<'a> {
    Client {
        base: "http://mb.test/",
        transport: server,
        throttle,
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
    assert_eq!(record.id, format!("{REC}2"));
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
    assert_eq!(record.id, format!("{REC}3"));
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
            .by_id(&format!("{REC}8"), album)
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
            "releases": [{{"id": "{REL}7", "title": "Rooms of Salt", "status": null,
                "release-group": {{"primary-type": "Album", "secondary-types": []}},
                "media": [{{"position": 1, "track-offset": 2, "tracks": [{{"position": 3, "number": "A3"}}]}}]}}]}}"#
    );
    let server = Server::default().answer(&format!("/ws/2/recording/{REC}7?inc="), &body);
    let throttle = Throttle::none();
    let record = client(&server, &throttle)
        .by_id(&format!("{REC}7"), None)
        .unwrap()
        .unwrap();
    assert_eq!(record.isrcs, ["XX0000000002"]);
    let release = record.release.unwrap();
    assert_eq!(
        (release.track, release.disc, release.date),
        (Some(3), Some(1), None)
    );
    assert_eq!(
        client(&Server::default(), &throttle)
            .by_id("x", None)
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
        id: format!("{REC}1"),
        title: "Lantern Weather".into(),
        artists: vec!["Paper Comets".into()],
        artist_ids: Vec::new(),
        isrcs: Vec::new(),
        length_ms: Some(354_000),
        release: None,
    };
    let path = keep(dir.path(), &record).unwrap();
    assert_eq!(path, path_of(dir.path(), &record.id));
    assert_eq!(read(&path).unwrap(), record);
}
