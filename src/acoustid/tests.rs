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
        found.map(|id| id.to_string()),
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
    assert!(url.contains("meta=recordings+releasegroups+sources"));
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
