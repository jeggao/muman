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

const UPLOAD: &str = "[[song]]\nsources = [\"youtube:uuuuuuuuuuu\"]\n";

#[test]
fn an_upload_looks_up_its_release_and_a_release_its_upload() {
    let (_d, m, s) = setup(UPLOAD, &[("youtube:uuuuuuuuuuu", false)]);
    assert_eq!(
        finds(&m, &s, false),
        [(yt("uuuuuuuuuuu"), Provider::YouTubeMusic)]
    );
    let (_d, m, s) = setup(UPLOAD, &[("youtube:uuuuuuuuuuu", true)]);
    assert_eq!(
        finds(&m, &s, false),
        [(yt("uuuuuuuuuuu"), Provider::YouTube)]
    );
    let (_d, m, s) = setup(
        "[[song]]\nsources = [\"youtube:uuuuuuuuuuu\", \"youtube:rrrrrrrrrrr\"]\n",
        &[
            ("youtube:uuuuuuuuuuu", false),
            ("youtube:rrrrrrrrrrr", true),
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
    let (_d, m, mut s) = setup(UPLOAD, &[("youtube:uuuuuuuuuuu", false)]);
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
        &[("youtube:uuuuuuuuuuu", false)],
    );
    assert_eq!(finds(&m, &s, false), []);
}
