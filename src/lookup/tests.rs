use super::*;

const DAY: u64 = 24 * 3600;
const NOW: u64 = 100 * DAY;

fn yt(id: &str) -> SourceKey {
    SourceKey::youtube(id)
}

/// A song list of `body` and a state where each key given is measured,
/// a release or not.
fn setup(body: &str, measured: &[(&str, bool)]) -> (tempfile::TempDir, Manifest, State) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("songs.toml"),
        format!("version = 1\n{body}"),
    )
    .unwrap();
    let manifest = Manifest::load(dir.path()).unwrap();
    let mut state = State::default();
    for (key, release) in measured {
        let mut f = Facts::unreadable("1".into());
        f.release = *release;
        state.facts.insert(SourceKey::parse(key).unwrap(), f);
    }
    (dir, manifest, state)
}

fn finds(manifest: &Manifest, state: &State, force: bool) -> Vec<(SourceKey, Provider)> {
    due(manifest, state, &Vec::new(), NOW, force)
        .into_iter()
        .map(|d| (d.from, d.find))
        .collect()
}

const UPLOAD: &str = "[[song]]\nsources = [\"youtube.com:uuuuuuuuuuu\"]\n";

#[test]
fn an_upload_looks_up_its_release_and_a_release_its_upload() {
    let (_d, m, s) = setup(UPLOAD, &[("youtube.com:uuuuuuuuuuu", false)]);
    assert_eq!(
        finds(&m, &s, false),
        [(yt("uuuuuuuuuuu"), Provider::YouTubeMusic)]
    );
    let (_d, m, s) = setup(UPLOAD, &[("youtube.com:uuuuuuuuuuu", true)]);
    assert_eq!(
        finds(&m, &s, false),
        [(yt("uuuuuuuuuuu"), Provider::YouTube)]
    );
    let (_d, m, s) = setup(
        "[[song]]\nsources = [\"youtube.com:uuuuuuuuuuu\", \"youtube.com:rrrrrrrrrrr\"]\n",
        &[
            ("youtube.com:uuuuuuuuuuu", false),
            ("youtube.com:rrrrrrrrrrr", true),
        ],
    );
    assert!(
        finds(&m, &s, false).is_empty(),
        "a song with both looks neither up"
    );
}

#[test]
fn a_file_of_your_own_looks_nothing_up_on_youtube() {
    let (_d, m, s) = setup(
        "[[song]]\nsources = [\"manual:a.flac\"]\n",
        &[("manual:a.flac", false)],
    );
    assert_eq!(finds(&m, &s, false), []);
}

#[test]
fn a_lookup_is_made_again_only_when_due() {
    let (_d, m, mut s) = setup(UPLOAD, &[("youtube.com:uuuuuuuuuuu", false)]);
    let looked = |at: u64, outcome: Outcome| Looked {
        from: yt("uuuuuuuuuuu"),
        find: "youtube-music".into(),
        method: method(Provider::YouTubeMusic).into(),
        at,
        outcome,
    };
    s.lookups = vec![looked(NOW - DAY, Outcome::Nothing)];
    assert!(finds(&m, &s, false).is_empty(), "found nothing a day ago");
    assert_eq!(finds(&m, &s, true).len(), 1, "unless forced");
    s.lookups = vec![looked(NOW - 15 * DAY, Outcome::Nothing)];
    assert_eq!(
        finds(&m, &s, false).len(),
        1,
        "two weeks on, it is asked again"
    );
    s.lookups = vec![looked(0, Outcome::Declined)];
    assert_eq!(finds(&m, &s, false), []);
    s.lookups = vec![Looked {
        method: "youtube-music/0".into(),
        ..looked(NOW, Outcome::Nothing)
    }];
    assert_eq!(
        finds(&m, &s, false).len(),
        1,
        "made another way, it is made again"
    );
    s.lookups = vec![looked(
        NOW - 2 * 3600 + 60,
        Outcome::Failed {
            count: 2,
            error: "x".into(),
        },
    )];
    assert!(
        finds(&m, &s, false).is_empty(),
        "a second failure waits two hours"
    );
}

#[test]
fn a_provider_turned_off_is_not_looked_up() {
    let (_d, m, s) = setup(
        &format!("[providers.youtube-music]\nenabled = false\n{UPLOAD}"),
        &[("youtube.com:uuuuuuuuuuu", false)],
    );
    assert_eq!(finds(&m, &s, false), []);
}

const OWN: &str = "[[song]]\nsources = [\"manual:a.flac\"]\n";

/// A song list of one file of the user's own, measured `printed` or not,
/// and its plan, which writes `tags`.
fn own(printed: bool, tags: &[(Field, &str)]) -> (tempfile::TempDir, Manifest, State, Planned) {
    let (dir, manifest, mut state) = setup(OWN, &[("manual:a.flac", false)]);
    let key = SourceKey::parse("manual:a.flac").unwrap();
    let facts = state.facts.get_mut(&key).unwrap();
    facts.duration = Some(240.0);
    facts.print = printed.then(|| crate::fingerprint::Print::new(vec![7; 100]).unwrap());
    let plan = crate::resolve::Plan {
        version: 1,
        format: crate::resolve::Format::Copy {
            codec: crate::codec::Codec::Flac,
        },
        audio: crate::resolve::AudioRef {
            key,
            rev: "1".into(),
            index: 0,
        },
        cover: None,
        lyrics: None,
        tags: tags
            .iter()
            .map(|(f, v)| (f.vorbis().to_string(), vec![(*v).to_string()]))
            .collect(),
        loudness: None,
    };
    let resolved = Resolved {
        plan,
        stem: std::path::PathBuf::new(),
        why: crate::resolve::Why::default(),
    };
    (dir, manifest, state, vec![(0, resolved)])
}

fn found(manifest: &Manifest, state: &State, planned: &Planned) -> Vec<Provider> {
    due(manifest, state, planned, NOW, false)
        .into_iter()
        .map(|d| d.find)
        .collect()
}

const NAMED: [(Field, &str); 2] = [
    (Field::Title, "Lantern Weather"),
    (Field::Artist, "Paper Comets"),
];

#[test]
fn a_file_without_names_is_looked_up_by_its_print() {
    let (_d, m, s, p) = own(true, &[]);
    assert_eq!(found(&m, &s, &p), [Provider::AcoustId]);
    let (_d, m, s, p) = own(false, &[]);
    assert_eq!(found(&m, &s, &p), [], "no print, no names: nothing to ask");
}

#[test]
fn musicbrainz_waits_for_acoustid() {
    let (_d, m, mut s, p) = own(true, &NAMED);
    assert_eq!(found(&m, &s, &p), [Provider::Lrclib, Provider::AcoustId]);
    s.lookups = vec![Looked {
        from: SourceKey::parse("manual:a.flac").unwrap(),
        find: "acoustid".into(),
        method: method(Provider::AcoustId).into(),
        at: NOW,
        outcome: Outcome::Nothing,
    }];
    assert_eq!(
        found(&m, &s, &p),
        [Provider::Lrclib, Provider::MusicBrainz],
        "AcoustID found nothing: search by name"
    );
    let (_d, m, s, p) = own(false, &NAMED);
    assert_eq!(found(&m, &s, &p), [Provider::Lrclib, Provider::MusicBrainz]);
}

#[test]
fn a_song_with_a_musicbrainz_record_asks_neither() {
    let (_d, mut m, s, p) = own(true, &NAMED);
    m.songs[0]
        .sources
        .push(SourceKey::parse("musicbrainz:00000000-0000-0000-0000-000000000001").unwrap());
    assert_eq!(found(&m, &s, &p), [Provider::Lrclib]);
}

fn due_of(n: u8, find: Provider) -> Due {
    Due {
        song: usize::from(n),
        from: yt(&format!("vid0000000{n}")),
        find,
    }
}

#[test]
fn a_run_makes_one_lookup_a_source_a_provider_and_asks_no_refusing_one() {
    let mut run = ThisRun::default();
    let all = || {
        vec![
            due_of(1, Provider::Lrclib),
            due_of(2, Provider::Lrclib),
            due_of(1, Provider::MusicBrainz),
        ]
    };
    assert_eq!(run.admit(all(), |_| 0).len(), 3);
    assert!(run.admit(all(), |_| 0).is_empty(), "each made already");
    let mut run = ThisRun::default();
    run.refused(Provider::Lrclib);
    let left = run.admit(all(), |_| 0);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].find, Provider::MusicBrainz);
}

#[test]
fn per_run_counts_the_whole_run() {
    let mut run = ThisRun::default();
    let first = run.admit(vec![due_of(1, Provider::Lrclib)], |_| 2);
    assert_eq!(first.len(), 1);
    let second = run.admit(
        vec![due_of(2, Provider::Lrclib), due_of(3, Provider::Lrclib)],
        |_| 2,
    );
    assert_eq!(second.len(), 1, "two in the run, not two a round");
    assert_eq!(run.waiting.get(&Provider::Lrclib), Some(&1));
}

/// The rows of the table in this module's docs of when a lookup is made
/// again: the docs are what each check below holds the code to.
fn table() -> Vec<(String, String)> {
    let source = include_str!("../lookup.rs");
    source
        .lines()
        .map_while(|l| l.strip_prefix("//!"))
        .filter_map(|l| {
            l.trim()
                .strip_prefix('|')?
                .strip_suffix('|')
                .map(str::to_string)
        })
        .filter(|row| !row.starts_with("---") && !row.contains("Recorded"))
        .map(|row| {
            let (a, b) = row.split_once('|').unwrap();
            (a.trim().to_string(), b.trim().to_string())
        })
        .collect()
}

#[test]
fn the_docs_say_when_a_lookup_is_made_again_as_it_is() {
    let rows = table();
    let said: Vec<(&str, &str)> = rows.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(
        said,
        [
            ("Nothing", "At once"),
            (
                "Made another way",
                "At once, unless it found a source the song lists"
            ),
            (
                "Found nothing, or an instrumental",
                "After the provider's `recheck`"
            ),
            (
                "Failed",
                "After an hour, doubling with each failure, up to a week"
            ),
            ("Found a source, or declined", "Never"),
        ],
        "a row changed: change the check below with it"
    );
    let week = 7 * DAY;
    let recheck = std::time::Duration::from_secs(30 * DAY);
    let at = |outcome: Outcome, age: u64| {
        let mut l = Looked::now(yt("vid00000001"), Provider::Lrclib, outcome);
        l.at = NOW - age;
        l
    };
    let due = |l: &Looked, listed: bool| l.due(NOW, recheck, &|_| listed);
    let (_dir, m, s) = setup(UPLOAD, &[("youtube.com:uuuuuuuuuuu", false)]);
    assert_eq!(s.lookups, []);
    assert!(!finds(&m, &s, false).is_empty(), "nothing recorded");
    let found = Outcome::Found(SourceKey::parse("lrclib:7").unwrap());
    let mut other = at(found.clone(), 0);
    other.method = "an older way".into();
    assert!(!due(&other, true) && due(&other, false), "made another way");
    for outcome in [Outcome::Nothing, Outcome::Instrumental] {
        assert!(!due(&at(outcome.clone(), 30 * DAY - 1), false));
        assert!(due(&at(outcome, 30 * DAY), false), "after recheck");
    }
    let failed = |count| Outcome::Failed {
        count,
        error: String::new(),
    };
    assert!(!due(&at(failed(1), 3599), false) && due(&at(failed(1), 3600), false));
    assert!(!due(&at(failed(2), 7199), false) && due(&at(failed(2), 7200), false));
    assert!(due(&at(failed(30), week), false), "up to a week");
    for outcome in [found, Outcome::Declined] {
        assert!(!due(&at(outcome, 99 * DAY), false), "never");
    }
}
