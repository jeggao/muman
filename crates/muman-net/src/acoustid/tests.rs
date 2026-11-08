use super::*;
use crate::lrclib::testing::Server;

const REC: &str = "00000000-0000-0000-0000-00000000000";

fn query() -> Query {
    Query {
        print: Print::new(
            (0..1500_u32)
                .map(|n| n.wrapping_mul(2_654_435_761))
                .collect(),
        )
        .unwrap(),
        seconds: 241.4,
        title: Some("Lantern Weather".into()),
        artist: Some("Paper Comets, Ada Quill".into()),
        album: None,
        mono: None,
    }
}

fn client<'a>(server: &'a (dyn HttpTransport + Sync), throttle: &'a Throttle) -> Client<'a> {
    Client {
        base: "http://acoustid.test/",
        key: "k3y",
        transport: server,
        throttle,
    }
}

/// A recording `seconds` long with `sources` prints, on one release
/// group of the secondary types given.
fn recording(n: u32, title: &str, seconds: f64, sources: u32, secondary: &str) -> String {
    format!(
        r#"{{"id": "{REC}{n}", "title": "{title}", "duration": {seconds}, "sources": {sources},
            "artists": [{{"id": "{REC}a", "name": "Paper Comets"}}],
            "releasegroups": [{{"id": "{REC}b", "type": "Album", "secondarytypes": [{secondary}]}}]}}"#
    )
}

fn answer(results: &[(f64, &[String])]) -> String {
    let results: Vec<String> = results
        .iter()
        .map(|(score, recs)| {
            format!(
                r#"{{"id": "{REC}c", "score": {score}, "recordings": [{}]}}"#,
                recs.join(",")
            )
        })
        .collect();
    format!(r#"{{"status": "ok", "results": [{}]}}"#, results.join(","))
}

fn find(body: &str, q: &Query) -> (Option<String>, Vec<String>) {
    let server = Server::default().answer("/v2/lookup?", body);
    let throttle = Throttle::none();
    let found = client(&server, &throttle).find(q).unwrap();
    (
        found.first().map(|c| c.id.to_string()),
        server.asked.into_inner().unwrap(),
    )
}

#[test]
fn the_request_carries_the_key_the_length_and_the_print() {
    let q = query();
    let (_, asked) = find(&answer(&[]), &q);
    let url = &asked[0];
    assert!(url.starts_with("http://acoustid.test/v2/lookup?client=k3y&"));
    assert!(url.contains("&duration=241&"));
    assert!(url.contains("&meta=recordings+releasegroups+releases+tracks+sources&"));
    assert!(url.ends_with(&format!("&fingerprint={}", q.print.encoded())));
}

#[test]
fn the_named_recording_with_the_most_sources_on_a_plain_release_wins() {
    let body = answer(&[
        (
            0.97,
            &[
                recording(1, "Lantern Weather", 241.0, 3, ""),
                recording(2, "Lantern Weather", 242.0, 40, r#""Compilation""#),
                recording(3, "Lantern Weather", 241.0, 40, ""),
                recording(4, "Some Other Name", 241.0, 90, ""),
            ],
        ),
        (0.4, &[recording(5, "Lantern Weather", 241.0, 500, "")]),
    ]);
    assert_eq!(find(&body, &query()).0, Some(format!("{REC}3")));
}

#[test]
fn a_song_without_names_takes_the_most_sourced_recording() {
    let body = answer(&[(
        0.9,
        &[
            recording(1, "Lantern Weather", 241.0, 3, ""),
            recording(4, "Some Other Name", 241.0, 90, ""),
        ],
    )]);
    let q = Query {
        title: None,
        artist: None,
        ..query()
    };
    assert_eq!(find(&body, &q).0, Some(format!("{REC}4")));
}

#[test]
fn a_low_score_or_another_length_is_no_match() {
    let body = answer(&[
        (0.3, &[recording(1, "Lantern Weather", 241.0, 9, "")]),
        (0.9, &[recording(2, "Lantern Weather", 200.0, 9, "")]),
    ]);
    assert_eq!(find(&body, &query()).0, None);
    assert_eq!(find(&answer(&[]), &query()).0, None);
}

#[test]
fn a_recording_of_another_title_by_another_artist_never_fits() {
    let by = |n: u32, title: &str, artist: &str| {
        recording(n, title, 241.0, 90, "").replace("Paper Comets", artist)
    };
    let body = answer(&[(
        0.97,
        &[
            by(1, "Night Ferry", "Kiri Hoshino"),
            by(2, "Lantern Weather", "Paper Comets Tribute Band"),
        ],
    )]);
    let server = Server::default().answer("/v2/lookup?", &body);
    let throttle = Throttle::none();
    let found = client(&server, &throttle).find(&query()).unwrap();
    assert_eq!(
        found.iter().map(|c| c.id.to_string()).collect::<Vec<_>>(),
        [format!("{REC}2")],
        "a cover act whose title agrees still fits"
    );
    let body = answer(&[(0.97, &[by(1, "Night Ferry", "Kiri Hoshino")])]);
    let q = Query {
        artist: None,
        ..query()
    };
    assert_eq!(
        find(&body, &q).0,
        Some(format!("{REC}1")),
        "no artist to hold it to"
    );
}

/// Answers every GET with AcoustID's 400 for a key it does not know.
struct Refuses;

impl HttpTransport for Refuses {
    fn get(&self, _: &str, _: &[(&str, &str)], _: Duration) -> Result<Vec<u8>, TransportError> {
        Err(TransportError::Status {
            code: 400,
            body: r#"{"error": {"code": 4, "message": "invalid API key"}, "status": "error"}"#
                .into(),
            retry_after: None,
        })
    }
}

#[test]
fn an_error_is_told_by_its_message() {
    let throttle = Throttle::none();
    let e = client(&Refuses, &throttle).find(&query()).unwrap_err();
    assert_eq!(e.to_string(), "AcoustID: invalid API key");
}

/// Answers every GET as its function says, given the address asked.
struct Answers(fn(&str) -> Result<Vec<u8>, TransportError>);

impl HttpTransport for Answers {
    fn get(&self, url: &str, _: &[(&str, &str)], _: Duration) -> Result<Vec<u8>, TransportError> {
        (self.0)(url)
    }
}

#[test]
fn no_error_names_the_key_or_the_print() {
    let answers = [
        Answers(|_| Ok(vec![0xff, 0xfe, b'{'])),
        Answers(|url| Err(TransportError::Transport(format!("cut off reading {url}")))),
        Answers(|url| Err(TransportError::Unreachable(format!("no route to {url}")))),
        Answers(|url| {
            Err(TransportError::Status {
                code: 502,
                body: format!("<p>bad gateway for {url}</p>"),
                retry_after: None,
            })
        }),
    ];
    let said: Vec<String> = answers
        .iter()
        .map(|answer| {
            let throttle = Throttle::none();
            format!(
                "{:#}",
                client(answer, &throttle).find(&query()).unwrap_err()
            )
        })
        .collect();
    for e in &said {
        assert!(!e.contains("k3y") && !e.contains("fingerprint"), "{e}");
    }
    assert!(
        said[0].starts_with("http://acoustid.test/v2/lookup: the body is not UTF-8"),
        "{}",
        said[0]
    );
}

/// `recording` stating no length.
fn untimed(n: u32, sources: u32) -> String {
    recording(n, "Lantern Weather", 0.0, sources, "").replace(r#""duration": 0, "#, "")
}

#[test]
fn a_length_stated_a_few_seconds_off_wins_over_none_stated() {
    let body = answer(&[(
        0.98,
        &[
            untimed(1, 13),
            recording(2, "Lantern Weather", 247.0, 280, ""),
            recording(3, "Lantern Weather", 248.0, 1200, ""),
        ],
    )]);
    let (found, _) = find(&body, &query());
    assert_eq!(found, Some(format!("{REC}3")));
    let throttle = Throttle::none();
    let server = Server::default().answer("/v2/lookup?", &body);
    let all: Vec<String> = client(&server, &throttle)
        .find(&query())
        .unwrap()
        .iter()
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(
        all,
        [format!("{REC}3"), format!("{REC}2"), format!("{REC}1")],
        "every one that fits, best first"
    );
}

#[test]
fn a_recording_stating_no_length_that_one_print_names_is_no_match() {
    let (found, _) = find(&answer(&[(1.0, &[untimed(1, 1)])]), &query());
    assert_eq!(found, None);
    let (found, _) = find(&answer(&[(1.0, &[untimed(1, 2)])]), &query());
    assert_eq!(found, Some(format!("{REC}1")));
}

/// A release of `title` dated `date`, as AcoustID writes a date, holding
/// the recording on the second of its two discs as track 5 of 11.
fn released(id: &str, title: &str, date: &str) -> String {
    format!(
        r#"{{"id": "{id}", "title": "{title}", "country": "XW", "date": {date},
            "medium_count": 2, "track_count": 20,
            "mediums": [{{"position": 2, "track_count": 11, "format": "CD",
                "tracks": [{{"id": "{REC}e", "position": 5, "title": "Lantern Weather"}}]}}]}}"#
    )
}

/// A release group `n` of the secondary types given, on `releases`.
fn grouped(n: u32, secondary: &str, releases: &[String]) -> String {
    format!(
        r#"{{"id": "{REC}{n}", "type": "Album", "title": "Rooms of Salt",
            "secondarytypes": [{secondary}], "releases": [{}]}}"#,
        releases.join(",")
    )
}

#[test]
fn each_recording_that_fits_is_kept_with_the_releases_it_is_on() {
    let rec = format!(
        r#"{{"id": "{REC}1", "title": "Lantern Weather", "duration": 241.4, "sources": 12,
            "artists": [{{"id": "{REC}a", "name": "Paper Comets", "joinphrase": " & "}},
                        {{"id": "{REC}b", "name": "Ada Quill"}}],
            "releasegroups": [{}, {}]}}"#,
        grouped(
            7,
            r#""Compilation""#,
            &[released(
                &format!("{REC}4"),
                "Best of the Comets",
                r#"{"year": 1999}"#
            )]
        ),
        grouped(
            8,
            "",
            &[
                released(
                    &format!("{REC}5"),
                    "Rooms of Salt",
                    r#"{"year": 2003, "month": 5, "day": 6}"#
                ),
                released(
                    &format!("{REC}6"),
                    "Rooms of Salt",
                    r#"{"year": 2001, "month": 2}"#
                ),
            ]
        )
    );
    let plain = recording(2, "Lantern Weather", 242.0, 3, "");
    let body = answer(&[(0.97, &[rec, plain])]);
    let server = Server::default().answer("/v2/lookup?", &body);
    let throttle = Throttle::none();
    let found = client(&server, &throttle).find(&query()).unwrap();
    let first = &found[0];
    assert_eq!(first.id.to_string(), format!("{REC}1"));
    assert_eq!(first.title, "Lantern Weather");
    assert_eq!(first.artists, ["Paper Comets", "Ada Quill"]);
    assert_eq!(
        (first.length_ms, first.score, first.sources),
        (Some(241_400), 970, 12)
    );
    let ids: Vec<&str> = first.releases.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        [format!("{REC}6"), format!("{REC}5"), format!("{REC}4")],
        "an album before a compilation, then the earliest"
    );
    assert_eq!(
        first.releases[0],
        CandidateRelease {
            id: format!("{REC}6"),
            group_id: Some(format!("{REC}8")),
            title: "Rooms of Salt".into(),
            date: Some("2001-02".into()),
            country: Some("XW".into()),
            types: vec!["Album".into()],
            disc: Some(2),
            discs: Some(2),
            track: Some(5),
            tracks: Some(11),
        }
    );
    assert_eq!(first.releases[1].date.as_deref(), Some("2003-05-06"));
    assert_eq!(first.releases[2].types, ["Album", "Compilation"]);
    assert!(
        found[1].releases.is_empty(),
        "an answer listing no releases still reads"
    );

    let q = Query {
        album: Some("Best of the Comets".into()),
        ..query()
    };
    let server = Server::default().answer("/v2/lookup?", &body);
    let found = client(&server, &throttle).find(&q).unwrap();
    assert_eq!(
        found[0].releases[0].id,
        format!("{REC}4"),
        "the song's album first"
    );
}

#[test]
fn a_lookup_keeps_a_few_recordings_and_a_few_releases_of_each() {
    let releases: Vec<String> = (0..KEPT_RELEASES as u32 + 5)
        .map(|n| {
            released(
                &format!("00000000-0000-0000-0000-0000000001{n:02}"),
                "Rooms of Salt",
                &format!(r#"{{"year": {}}}"#, 2030 - n),
            )
        })
        .collect();
    let mut recordings: Vec<String> = (1..=KEPT as u32 + 1)
        .map(|n| recording(n, "Lantern Weather", 241.0, 9, ""))
        .collect();
    recordings[0] = recordings[0].replace(
        r#""releasegroups": [{"#,
        &format!(r#""releasegroups": [{}, {{"#, grouped(9, "", &releases)),
    );
    let server = Server::default().answer("/v2/lookup?", &answer(&[(0.9, &recordings)]));
    let throttle = Throttle::none();
    let found = client(&server, &throttle).find(&query()).unwrap();
    assert_eq!(found.len(), KEPT);
    let kept = &found[0].releases;
    assert_eq!(kept.len(), KEPT_RELEASES);
    assert_eq!(kept[0].date.as_deref(), Some("2001"), "the earliest kept");
}
