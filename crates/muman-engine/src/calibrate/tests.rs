use super::*;

const ONE: &str = "00000000-0000-0000-0000-000000000001";
const TWO: &str = "00000000-0000-0000-0000-000000000002";
const THREE: &str = "00000000-0000-0000-0000-000000000003";

fn record(id: &str) -> SourceKey {
    SourceKey::parse(&format!("musicbrainz:{id}")).unwrap()
}

fn tagged(tags: &[(Field, &str)]) -> Facts {
    let mut f = Facts::unreadable("r".into());
    for (field, value) in tags {
        let offer = crate::tags::Offer {
            values: vec![(*value).to_string()],
            structured: true,
            also: Vec::new(),
        };
        f.tags.insert(*field, offer);
    }
    f
}

fn found(from: &SourceKey, record: &SourceKey, candidates: &[&str]) -> Looked {
    Looked {
        from: from.clone(),
        find: "acoustid".into(),
        method: String::new(),
        at: 0,
        outcome: Outcome::Found(record.clone()),
        unchanged: 0,
        candidates: (candidates.iter())
            .map(|id| crate::acoustid::Candidate {
                id: crate::record::Mbid::parse(id).unwrap(),
                title: String::new(),
                artists: Vec::new(),
                length_ms: None,
                score: 900,
                sources: 1,
                releases: Vec::new(),
            })
            .collect(),
    }
}

/// An upload playing 200 s, titled and credited as its channel wrote.
fn upload(title: &str, artist: &str) -> Facts {
    let mut f = Facts::unreadable("rev".into());
    f.duration = Some(200.0);
    f.audio = Some(crate::facts::AudioFacts {
        index: 1,
        codec: "opus".into(),
        channels: 2,
        sample_rate: 48_000,
        ..crate::facts::AudioFacts::default()
    });
    for (field, value) in [(Field::Title, title), (Field::Artist, artist)] {
        let offer = crate::tags::Offer {
            values: vec![value.to_string()],
            structured: false,
            also: Vec::new(),
        };
        f.tags.insert(field, offer);
    }
    f
}

#[test]
fn the_library_s_records_and_look_alikes_are_weighed_as_vetting_weighs_them() {
    let (a, b) = (
        SourceKey::youtube("aaaaaaaaaaa"),
        SourceKey::youtube("bbbbbbbbbbb"),
    );
    let (night, glass) = (record(ONE), record(TWO));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("songs.toml"),
        format!(
            "version = 1\n[[song]]\nsources = [\"{a}\", \"{night}\"]\n\
             [[song]]\nsources = [\"{b}\", \"{glass}\"]\n"
        ),
    )
    .unwrap();
    let manifest = Manifest::load(dir.path()).unwrap();
    let mut state = State::default();
    state.facts.extend([
        (a.clone(), upload("Night Ferry", "Marlo Venn")),
        (b.clone(), upload("Glass Orchard", "Marlo Venn")),
        (
            night.clone(),
            tagged(&[
                (Field::Title, "Night Ferry"),
                (Field::Artist, "Marlo Venn"),
                (Field::MusicBrainzTrackId, ONE),
            ]),
        ),
        (
            glass.clone(),
            tagged(&[
                (Field::Title, "Glass Orchard"),
                (Field::Artist, "Marlo Venn"),
                (Field::MusicBrainzTrackId, TWO),
            ]),
        ),
    ]);
    state.lookups.push(found(&a, &night, &[]));
    let c = calibrate(&manifest, &manifest.settings, &state, dir.path()).unwrap();
    assert_eq!((c.same, c.other), (1, 2), "{c:?}");
    assert_eq!(c.how[PRINT.how], 1);
    assert_eq!(c.how[CREDIT_ALIKE.how], 2);
    let title = c.by["title"];
    assert_eq!(title.same.total(), 0, "a lookup ranks by title");
    assert_eq!(title.other.disagreed, 2, "{title:?}");
    assert!(
        !c.by.contains_key("credits"),
        "a lookup ranks by credits, and the look-alikes were chosen by one"
    );
}

#[test]
fn evidence_that_tells_the_labels_apart_is_weighed_by_its_log_odds() {
    let tally = Tally {
        same: Counts {
            agreed: 90,
            disagreed: 2,
            silent: 8,
        },
        other: Counts {
            agreed: 5,
            disagreed: 80,
            silent: 15,
        },
    };
    let (agrees, disagrees) = tally.suggested().unwrap();
    assert!(agrees >= 7, "{agrees}");
    assert!(disagrees <= -8, "{disagrees}");
    let few = Tally {
        other: Counts {
            disagreed: MIN_LABELLED - 1,
            ..Counts::default()
        },
        ..tally
    };
    assert_eq!(few.suggested(), None, "too few of another");
}

#[test]
fn a_record_is_labelled_by_the_ids_its_song_s_files_carry_first() {
    let file = SourceKey::Manual("a.flac".into());
    let (same, other) = (record(ONE), record(TWO));
    let mut facts = BTreeMap::from([
        (file.clone(), tagged(&[(Field::MusicBrainzTrackId, ONE)])),
        (same.clone(), tagged(&[(Field::MusicBrainzTrackId, ONE)])),
        (other.clone(), tagged(&[(Field::MusicBrainzTrackId, TWO)])),
    ]);
    let song = Song {
        sources: vec![file.clone(), same.clone(), other.clone()],
        ..Song::default()
    };
    let lookups = BTreeMap::new();
    let by = |key: &SourceKey, facts: &BTreeMap<SourceKey, Facts>| {
        label(&song, key, facts, &lookups).map(|l| (l.same, l.how, l.chosen_by.len()))
    };
    assert_eq!(by(&same, &facts), Some((true, BY_ID, 0)));
    assert_eq!(by(&other, &facts), Some((false, BY_ID, 0)));
    facts.remove(&file);
    assert_eq!(by(&other, &facts), None, "files naming no ID");
}

#[test]
fn a_record_the_song_s_own_print_found_is_counted_for_what_finding_it_never_weighed() {
    let file = SourceKey::Manual("a.flac".into());
    let found_record = record(ONE);
    let facts = BTreeMap::from([(
        found_record.clone(),
        tagged(&[(Field::MusicBrainzTrackId, ONE)]),
    )]);
    let song = Song {
        sources: vec![file.clone(), found_record.clone()],
        ..Song::default()
    };
    let ranked = found(&file, &found_record, &[ONE, TWO]);
    let lookups = BTreeMap::from([(&file, &ranked)]);
    assert_eq!(label(&song, &found_record, &facts, &lookups), Some(PRINT));
    let alone = found(&file, &found_record, &[ONE]);
    let lookups = BTreeMap::from([(&file, &alone)]);
    assert_eq!(
        label(&song, &found_record, &facts, &lookups),
        Some(PRINT_ALONE)
    );
    let mut c = Calibration::default();
    let sides = [
        ("title", Some(true)),
        ("credits", Some(true)),
        ("length", Some(true)),
        ("release", None),
        ("unit", Some(false)),
    ];
    c.count(PRINT, sides);
    assert_eq!(
        c.by.keys().copied().collect::<Vec<_>>(),
        ["release", "unit"],
        "title, credits and length chose it"
    );
    assert_eq!(c.by["unit"].same.disagreed, 1);
    assert_eq!((c.same, c.how[PRINT.how]), (1, 1));
}

#[test]
fn another_song_s_record_sharing_a_title_or_a_credit_is_another_unless_it_names_the_same() {
    let file = SourceKey::Manual("a.flac".into());
    let (mine, by_title, by_credit, again) = (
        record(ONE),
        record(TWO),
        record(THREE),
        record("00000000-0000-0000-0000-000000000004"),
    );
    let facts = BTreeMap::from([
        (
            file.clone(),
            tagged(&[(Field::Title, "Night Ferry"), (Field::Artist, "Marlo Venn")]),
        ),
        (
            mine.clone(),
            tagged(&[
                (Field::Title, "Night Ferry"),
                (Field::Artist, "Marlo Venn"),
                (Field::MusicBrainzTrackId, ONE),
            ]),
        ),
        (
            by_title.clone(),
            tagged(&[
                (Field::Title, "Night Ferry"),
                (Field::Artist, "Kiri Hoshino"),
                (Field::MusicBrainzTrackId, TWO),
            ]),
        ),
        (
            by_credit.clone(),
            tagged(&[
                (Field::Title, "Glass Orchard"),
                (Field::Artist, "Marlo Venn & Kiri Hoshino"),
                (Field::MusicBrainzTrackId, THREE),
            ]),
        ),
        (
            again.clone(),
            tagged(&[
                (Field::Title, "Night Ferry"),
                (Field::MusicBrainzTrackId, ONE),
            ]),
        ),
    ]);
    let songs = [
        Song {
            sources: vec![file.clone(), mine.clone()],
            ..Song::default()
        },
        Song {
            sources: vec![by_title.clone()],
            ..Song::default()
        },
        Song {
            sources: vec![by_credit.clone()],
            ..Song::default()
        },
        Song {
            sources: vec![again.clone()],
            ..Song::default()
        },
    ];
    let alike = Lookalikes::of(&songs, &facts);
    let chosen = alike.of_song(0, &songs[0], &facts, &BTreeMap::new());
    assert_eq!(
        chosen,
        [(&by_title, TITLE_ALIKE), (&by_credit, CREDIT_ALIKE)],
        "the same recording listed again is no look-alike"
    );
}
