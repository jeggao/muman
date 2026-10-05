use super::*;
use crate::codec::Codec;
use crate::settings::Library;

static NAMING: std::sync::LazyLock<Naming> = std::sync::LazyLock::new(|| {
    Naming::new(&Library::default(), std::path::Path::new("lib")).unwrap()
});
static AUDIO: std::sync::LazyLock<Audio> = std::sync::LazyLock::new(Audio::default);
static QUALITY: std::sync::LazyLock<Quality> = std::sync::LazyLock::new(Quality::default);
use crate::facts::{AudioFacts, CoverFacts, LyricsFacts};
use crate::lyrics::{Language, Timing};
use crate::quality::{AudioQuality, ImageQuality};
use crate::tags::Offers;

fn yt(id: &str) -> SourceKey {
    SourceKey::youtube(id)
}

fn manual(path: &str) -> SourceKey {
    SourceKey::Manual(path.into())
}

/// Facts of a source with audio of `codec` cut at `khz`, stereo, clean.
fn audio(codec: &str, khz: f64) -> Facts {
    let mut f = Facts::unreadable(format!("rev-{codec}-{khz}"));
    f.duration = Some(200.0);
    f.audio = Some(AudioFacts {
        index: 1,
        codec: codec.into(),
        channels: 2,
        quality: Some(AudioQuality {
            bandwidth_hz: khz * 1000.0,
            incoherence: 0.2,
            clipping: 0.0,
        }),
        bytes: None,
    });
    f
}

fn image(width: u32, height: u32, content: Rect, effective: u32) -> ImageQuality {
    ImageQuality {
        width,
        height,
        content,
        effective,
        blockiness: 0.05,
    }
}

fn full(w: u32, h: u32) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    }
}

fn cover(f: &mut Facts, at: CoverAt, q: ImageQuality) {
    f.covers.push(CoverFacts {
        at,
        mimetype: "image/jpeg".into(),
        quality: Some(q),
    });
}

fn lyrics(f: &mut Facts, at: LyricsAt, code: &str, lines: usize) {
    f.lyrics.push(LyricsFacts {
        at,
        language: Language::Codes(vec![code.into()]),
        timing: (lines > 0).then_some(Timing {
            lines,
            first_ms: 10_000,
            last_ms: 180_000,
        }),
        stated_ms: None,
    });
}

fn tag(f: &mut Facts, field: Field, values: &[&str], structured: bool) {
    f.tags.insert(
        field,
        Offer {
            values: values.iter().map(|v| (*v).to_string()).collect(),
            structured,
        },
    );
}

/// Whole milliseconds of a test's durations, which are whole seconds.
#[allow(clippy::cast_possible_truncation)]
fn ms(seconds: Option<f64>) -> i64 {
    seconds.map_or(0, |d| d.round() as i64 * 1000)
}

fn aligned(
    facts: &BTreeMap<SourceKey, Facts>,
    a: &SourceKey,
    b: &SourceKey,
    offset_ms: i64,
    score: f64,
) -> Aligned {
    Aligned {
        a: a.clone(),
        b: b.clone(),
        method: crate::align::METHOD.into(),
        revs: (facts[a].rev.clone(), facts[b].rev.clone()),
        offset_ms,
        stretch_ppm: 0,
        score,
        coverage: 1.0,
        a_ms: ms(facts[a].duration),
        b_ms: ms(facts[b].duration),
        a_extra_ms: (ms(facts[a].duration) - ms(facts[b].duration)).max(0),
    }
}

/// Both directions of a comparison where `a` plays `offset_ms` later.
fn both(
    facts: &BTreeMap<SourceKey, Facts>,
    a: &SourceKey,
    b: &SourceKey,
    offset_ms: i64,
    score: f64,
) -> [Aligned; 2] {
    [
        aligned(facts, a, b, offset_ms, score),
        aligned(facts, b, a, -offset_ms, score),
    ]
}

#[test]
fn lyrics_from_a_copy_that_runs_slow_are_shortened_by_its_drift() {
    let (song, facts, mut alignments) = release_and_video();
    let video = yt("vvvvvvvvvvv");
    for a in alignments.iter_mut().filter(|a| a.a == video) {
        a.stretch_ppm = 1000;
    }
    let lyrics = run(&song, &facts, &alignments).plan.lyrics.unwrap();
    assert_eq!(lyrics.key, video);
    assert_eq!(lyrics.stretch_ppm, 1000);
}

fn why<'r>(r: &'r Resolved, key: &str) -> &'r TagWhy {
    r.why.tags.iter().find(|t| t.key == key).unwrap()
}

fn song(sources: &[SourceKey]) -> Song {
    Song {
        sources: sources.to_vec(),
        ..Song::default()
    }
}

fn run(song: &Song, facts: &BTreeMap<SourceKey, Facts>, alignments: &[Aligned]) -> Resolved {
    run_with(song, facts, alignments, &AUDIO, &QUALITY)
}

fn run_with(
    song: &Song,
    facts: &BTreeMap<SourceKey, Facts>,
    alignments: &[Aligned],
    audio: &Audio,
    quality: &Quality,
) -> Resolved {
    resolve(&Input {
        song,
        album: None,
        facts,
        alignments,
        lyrics: &["en".to_string()],
        clean: &Settings::default(),
        albums: &Albums::default(),
        naming: &NAMING,
        audio,
        quality,
        placement: LyricsPlacement::Sidecar,
    })
    .unwrap()
}

/// A release and the music video of the same recording: the video runs
/// 20 s longer, carries English subtitles and a 16:9 thumbnail; the
/// release has square art and release tags.
fn release_and_video() -> (Song, BTreeMap<SourceKey, Facts>, Vec<Aligned>) {
    let (release, video) = (yt("rrrrrrrrrrr"), yt("vvvvvvvvvvv"));
    let mut r = audio("opus", 20.0);
    r.rev = "r".into();
    cover(
        &mut r,
        CoverAt::Picture { index: 3 },
        image(1400, 1400, full(1400, 1400), 1400),
    );
    tag(&mut r, Field::Title, &["Song (feat. Wren)"], true);
    tag(&mut r, Field::Artist, &["Hoshi7ne", "Hollis Wren"], true);
    tag(&mut r, Field::Album, &["Record"], true);
    tag(&mut r, Field::Date, &["2026-05-27"], true);
    let mut v = audio("opus", 20.0);
    v.rev = "v".into();
    v.duration = Some(220.0);
    cover(
        &mut v,
        CoverAt::Attachment { ordinal: 1 },
        image(1280, 720, full(1280, 720), 714),
    );
    lyrics(&mut v, LyricsAt::Stream { index: 2 }, "en", 30);
    tag(
        &mut v,
        Field::Title,
        &["Song / Wren (Official Video)"],
        false,
    );
    tag(&mut v, Field::Artist, &["Hoshi7ne"], false);
    tag(&mut v, Field::Date, &["2026-05-20"], false);
    let facts = BTreeMap::from([(release.clone(), r), (video.clone(), v)]);
    let alignments = both(&facts, &video, &release, 20_000, 0.99).to_vec();
    (song(&[video, release]), facts, alignments)
}

#[test]
fn the_release_wins_audio_cover_and_tags_and_the_video_gives_its_lyrics() {
    let (song, facts, alignments) = release_and_video();
    let r = run(&song, &facts, &alignments);
    assert_eq!(r.plan.audio.key, yt("rrrrrrrrrrr"), "{:?}", r.why);
    assert_eq!(r.plan.format, Format::Copy { codec: Codec::Opus });
    let cover = r.plan.cover.unwrap();
    assert_eq!(cover.key, yt("rrrrrrrrrrr"));
    assert_eq!(cover.crop, None);
    let lyrics = r.plan.lyrics.unwrap();
    assert_eq!(lyrics.key, yt("vvvvvvvvvvv"));
    assert_eq!(lyrics.shift_ms, 20_000, "the video plays 20 s later");
    assert_eq!(lyrics.stretch_ppm, 0);
    let get = |k: &str| {
        r.plan
            .tags
            .iter()
            .find(|(t, _)| t == k)
            .map(|(_, v)| v.join("|"))
    };
    assert_eq!(get("TITLE").as_deref(), Some("Song (feat. Wren)"));
    assert_eq!(get("ARTIST").as_deref(), Some("Hoshi7ne, Hollis Wren"));
    assert_eq!(get("ALBUM").as_deref(), Some("Record"));
    assert_eq!(get("ALBUMARTIST").as_deref(), Some("Hoshi7ne"));
    assert_eq!(get("DATE").as_deref(), Some("2026-05-27"));
    assert_eq!(r.stem, PathBuf::from("Hoshi7ne/Record/Song (feat. Wren)"));
}

#[test]
fn the_ranking_follows_measures_not_the_list_s_order() {
    let (mut song, facts, alignments) = release_and_video();
    song.sources.reverse();
    assert_eq!(
        run(&song, &facts, &alignments).plan.audio.key,
        yt("rrrrrrrrrrr")
    );
}

#[test]
fn wider_bandwidth_wins_between_clean_recordings_and_a_transcode_cannot_fake_it() {
    let (opus, real, fake) = (yt("ooooooooooo"), manual("cd.flac"), manual("fake.flac"));
    let facts = BTreeMap::from([
        (opus.clone(), audio("opus", 20.0)),
        (real.clone(), audio("flac", 22.0)),
        (fake.clone(), audio("flac", 16.0)),
    ]);
    let mut alignments = Vec::new();
    for (a, b) in [(&opus, &real), (&opus, &fake), (&real, &fake)] {
        alignments.extend(both(&facts, a, b, 0, 0.99));
    }
    let r = run(
        &song(&[opus.clone(), fake.clone(), real.clone()]),
        &facts,
        &alignments,
    );
    assert_eq!(r.plan.audio.key, real);
    assert_eq!(r.plan.format, Format::Copy { codec: Codec::Flac });
    let r = run(&song(&[fake, opus.clone()]), &facts, &alignments);
    assert_eq!(r.plan.audio.key, opus, "a 16 kHz FLAC is a lossy transcode");
}

#[test]
fn real_stereo_beats_mono_in_two_channels() {
    let (mono, stereo) = (yt("mmmmmmmmmmm"), yt("sssssssssss"));
    let mut m = audio("opus", 20.0);
    m.audio
        .as_mut()
        .unwrap()
        .quality
        .as_mut()
        .unwrap()
        .incoherence = 0.0;
    let facts = BTreeMap::from([(mono.clone(), m), (stereo.clone(), audio("aac", 20.0))]);
    let alignments = both(&facts, &mono, &stereo, 0, 0.99).to_vec();
    let r = run(&song(&[mono, stereo.clone()]), &facts, &alignments);
    assert_eq!(r.plan.audio.key, stereo);
    assert_eq!(
        r.plan.format,
        Format::Encode {
            codec: Codec::Opus,
            kbps: Some(160),
        }
    );
}

#[test]
fn a_pin_overrides_the_measures() {
    let (mut song, facts, alignments) = release_and_video();
    song.audio = Some(yt("vvvvvvvvvvv"));
    song.lyrics = Some(LyricsPin::None);
    let r = run(&song, &facts, &alignments);
    assert_eq!(r.plan.audio.key, yt("vvvvvvvvvvv"));
    assert_eq!(r.why.audio, "pinned");
    assert!(r.plan.lyrics.is_none());
}

#[test]
fn square_detail_beats_pixel_count_and_borders_are_cropped() {
    let (a, b, c) = (yt("aaaaaaaaaaa"), yt("bbbbbbbbbbb"), yt("ccccccccccc"));
    let mut fa = audio("opus", 20.0);
    fa.rev = "a".into();
    // Upscaled: 3000 px square holding 700 px of detail.
    cover(
        &mut fa,
        CoverAt::Picture { index: 2 },
        image(3000, 3000, full(3000, 3000), 700),
    );
    let mut fb = audio("opus", 20.0);
    fb.rev = "b".into();
    cover(
        &mut fb,
        CoverAt::Picture { index: 2 },
        image(1000, 1000, full(1000, 1000), 1000),
    );
    let mut fc = audio("opus", 20.0);
    fc.rev = "c".into();
    // A 1920×1080 frame with 1080 px square art pillarboxed in it.
    let inner = Rect {
        x: 420,
        y: 0,
        width: 1080,
        height: 1080,
    };
    cover(
        &mut fc,
        CoverAt::Attachment { ordinal: 0 },
        image(1920, 1080, inner, 1080),
    );
    let facts = BTreeMap::from([(a.clone(), fa), (b.clone(), fb), (c.clone(), fc)]);
    let r = run(&song(&[a.clone(), b.clone()]), &facts, &[]);
    assert_eq!(r.plan.cover.unwrap().key, b, "detail, not pixels");
    let r = run(&song(&[a, b, c.clone()]), &facts, &[]);
    let cover = r.plan.cover.unwrap();
    assert_eq!(cover.key, c);
    assert_eq!(cover.crop, Some(inner));
}

#[test]
fn lyrics_need_the_same_recording_and_prefer_timed_and_preferred() {
    let (track, other, live) = (yt("ttttttttttt"), yt("ooooooooooo"), yt("lllllllllll"));
    let mut track_facts = audio("opus", 20.0);
    track_facts.rev = "t".into();
    lyrics(&mut track_facts, LyricsAt::Stream { index: 2 }, "en", 0);
    let mut o = audio("opus", 20.0);
    o.rev = "o".into();
    lyrics(&mut o, LyricsAt::Stream { index: 3 }, "fr", 40);
    lyrics(&mut o, LyricsAt::Stream { index: 4 }, "en", 40);
    let mut l = audio("opus", 16.0);
    l.rev = "l".into();
    lyrics(&mut l, LyricsAt::Stream { index: 2 }, "en", 60);
    let facts = BTreeMap::from([
        (track.clone(), track_facts),
        (other.clone(), o),
        (live.clone(), l),
    ]);
    let mut alignments = both(&facts, &other, &track, 920, 0.95).to_vec();
    alignments.extend(both(&facts, &live, &track, 0, 0.3));
    let mut s = song(&[track.clone(), other.clone(), live.clone()]);
    s.lyrics_offset_ms = 100;
    let r = run(&s, &facts, &alignments);
    assert_eq!(r.plan.audio.key, track);
    let lyrics = r.plan.lyrics.unwrap();
    assert_eq!(
        (lyrics.key, lyrics.at),
        (other, LyricsAt::Stream { index: 4 }),
        "{:?}",
        r.why
    );
    assert_eq!(lyrics.shift_ms, 820, "the offset, less the hand correction");
}

#[test]
fn an_upload_s_title_and_channel_make_a_single() {
    let up = yt("uuuuuuuuuuu");
    let mut u = audio("opus", 20.0);
    tag(&mut u, Field::Title, &["Song / Wren"], false);
    tag(&mut u, Field::Artist, &["Hoshi7ne"], false);
    let facts = BTreeMap::from([(up.clone(), u)]);
    let r = run(&song(&[up]), &facts, &[]);
    let get = |k: &str| {
        r.plan
            .tags
            .iter()
            .find(|(t, _)| t == k)
            .map(|(_, v)| v.join("|"))
    };
    assert_eq!(get("ALBUM").as_deref(), Some("Song"));
    assert_eq!(get("ALBUMARTIST").as_deref(), Some("Hoshi7ne"));
    assert_eq!(r.stem, PathBuf::from("Hoshi7ne/Song/Song"));
    assert_eq!(why(&r, "TITLE").cleaned, ["guesswork.title_artist"]);
}

#[test]
fn a_switched_off_rule_leaves_the_offer_as_it_came() {
    let up = yt("uuuuuuuuuuu");
    let mut u = audio("opus", 20.0);
    tag(&mut u, Field::Title, &["Song / Wren"], false);
    tag(&mut u, Field::Artist, &["Hoshi7ne"], false);
    let facts = BTreeMap::from([(up.clone(), u)]);
    let mut clean = Settings::default();
    clean.set("guesswork.title_artist", false);
    let r = resolve(&Input {
        song: &song(&[up]),
        album: None,
        facts: &facts,
        alignments: &[],
        lyrics: &[],
        clean: &clean,
        albums: &Albums::default(),
        naming: &NAMING,
        audio: &AUDIO,
        quality: &QUALITY,
        placement: LyricsPlacement::Sidecar,
    })
    .unwrap();
    assert_eq!(r.stem, PathBuf::from("Hoshi7ne/Song ⧸ Wren/Song ⧸ Wren"));
    assert_eq!(why(&r, "TITLE").cleaned, Vec::<&str>::new());
}

#[test]
fn a_hand_set_tag_is_never_cleaned() {
    let key = manual("a.flac");
    let mut f = audio("flac", 22.0);
    tag(&mut f, Field::Title, &["Song (Album Version)"], true);
    tag(&mut f, Field::Artist, &["Chan"], true);
    let facts = BTreeMap::from([(key.clone(), f)]);
    let plain = run(&song(std::slice::from_ref(&key)), &facts, &[]);
    assert_eq!(plain.stem, PathBuf::from("Chan/Song/Song"));
    assert_eq!(why(&plain, "TITLE").cleaned, ["packaging.album_version"]);
    let mut s = song(&[key]);
    s.tags = vec![("title".into(), vec!["Mine  (Album Version)".into()])];
    let r = run(&s, &facts, &[]);
    assert_eq!(
        r.stem,
        PathBuf::from("Chan/Mine  (Album Version)/Mine  (Album Version)")
    );
}

#[test]
fn an_album_s_disc_in_its_name_is_its_disc_number() {
    let key = manual("Night Ferries/Live/CD2/01.flac");
    let mut f = audio("flac", 22.0);
    tag(&mut f, Field::Title, &["Salt Road Motel"], true);
    tag(&mut f, Field::Artist, &["Night Ferries"], true);
    tag(&mut f, Field::Album, &["Night Ferries Live (CD2)"], true);
    tag(&mut f, Field::Track, &["1"], true);
    let facts = BTreeMap::from([(key.clone(), f)]);
    let r = run(&song(&[key]), &facts, &[]);
    assert_eq!(
        r.stem,
        PathBuf::from("Night Ferries/Night Ferries Live/2-01 Salt Road Motel")
    );
    assert_eq!(why(&r, "DISCNUMBER").cleaned, [clean::DISC_IN_ALBUM]);
}

#[test]
fn an_artist_named_for_another_album_of_its_owner_is_the_owner() {
    let key = manual("a.flac");
    let mut f = audio("flac", 22.0);
    tag(&mut f, Field::Title, &["Song"], true);
    tag(&mut f, Field::Artist, &["Tin Roof Psalms"], true);
    tag(&mut f, Field::Album, &["Glass Weather"], true);
    tag(&mut f, Field::AlbumArtist, &["Marlo Venn"], true);
    let mut other = Facts::unreadable("o".into());
    tag(&mut other, Field::Album, &["Tin Roof Psalms"], true);
    tag(&mut other, Field::AlbumArtist, &["Marlo Venn"], true);
    let albums = Albums::of([&f.tags, &other.tags]);
    let facts = BTreeMap::from([(key.clone(), f)]);
    let r = resolve(&Input {
        song: &song(&[key]),
        album: None,
        facts: &facts,
        alignments: &[],
        lyrics: &[],
        clean: &Settings::default(),
        albums: &albums,
        naming: &NAMING,
        audio: &AUDIO,
        quality: &QUALITY,
        placement: LyricsPlacement::Sidecar,
    })
    .unwrap();
    let artist = r.plan.tags.iter().find(|(k, _)| k == "ARTIST").unwrap();
    assert_eq!(artist.1, ["Marlo Venn"]);
}

#[test]
fn a_store_s_packaging_no_longer_splits_agreement() {
    let (a, b) = (manual("a.flac"), manual("b.flac"));
    let mut fa = audio("flac", 22.0);
    fa.rev = "a".into();
    tag(&mut fa, Field::Genre, &["Rock"], true);
    tag(&mut fa, Field::Title, &["Song (Album Version)"], true);
    let mut fb = audio("flac", 22.0);
    fb.rev = "b".into();
    tag(&mut fb, Field::Title, &["Song"], true);
    let facts = BTreeMap::from([(a.clone(), fa), (b.clone(), fb)]);
    let r = run(&song(&[a, b]), &facts, &[]);
    assert_eq!(
        r.plan.tags[0],
        ("TITLE".to_string(), vec!["Song".to_string()])
    );
}

#[test]
fn a_single_is_named_for_its_title_as_set_by_hand() {
    let up = yt("uuuuuuuuuuu");
    let mut u = audio("opus", 20.0);
    tag(
        &mut u,
        Field::Title,
        &["Song (Full Song) [REUPLOAD]"],
        false,
    );
    tag(&mut u, Field::Artist, &["Chan"], false);
    let facts = BTreeMap::from([(up.clone(), u)]);
    let mut s = song(&[up]);
    s.tags = vec![("title".into(), vec!["Song".into()])];
    let r = run(&s, &facts, &[]);
    assert_eq!(r.stem, PathBuf::from("Chan/Song/Song"));
}

#[test]
fn an_album_artist_taken_from_the_artist_follows_a_hand_set_one() {
    let up = yt("uuuuuuuuuuu");
    let mut u = audio("opus", 20.0);
    tag(&mut u, Field::Title, &["Song"], true);
    tag(&mut u, Field::Artist, &["Chan"], true);
    let facts = BTreeMap::from([(up.clone(), u)]);
    let mut s = song(&[up]);
    s.tags = vec![(
        "artist".into(),
        vec!["Marlo Venn".into(), "The Glass Orchards".into()],
    )];
    let r = run(&s, &facts, &[]);
    assert_eq!(r.stem, PathBuf::from("Marlo Venn/Song/Song"));
    assert_eq!(why(&r, "ALBUMARTIST").from, "the first artist of song.tags");
}

#[test]
fn hand_set_tags_and_the_album_win_last() {
    let (mut s, facts, alignments) = release_and_video();
    let album = Album {
        source: SourceKey::playlist("OLAK5uy_x"),
        tracks: Some(12),
        tags: vec![("album_artist".into(), vec!["Various".into()])],
    };
    s.track = Some(3);
    s.tags = vec![
        ("title".into(), vec!["Mine".into()]),
        ("artist".into(), vec!["Someone".into()]),
        ("comment".into(), vec!["x".into()]),
    ];
    let r = resolve(&Input {
        song: &s,
        album: Some(&album),
        facts: &facts,
        alignments: &alignments,
        lyrics: &["en".to_string()],
        clean: &Settings::default(),
        albums: &Albums::default(),
        naming: &NAMING,
        audio: &AUDIO,
        quality: &QUALITY,
        placement: LyricsPlacement::Sidecar,
    })
    .unwrap();
    let get = |k: &str| {
        r.plan
            .tags
            .iter()
            .find(|(t, _)| t == k)
            .map(|(_, v)| v.join("|"))
    };
    assert_eq!(get("TITLE").as_deref(), Some("Mine"));
    assert_eq!(get("ALBUMARTIST").as_deref(), Some("Various"));
    assert_eq!(get("TRACKNUMBER").as_deref(), Some("3"));
    assert_eq!(get("TRACKTOTAL").as_deref(), Some("12"));
    assert_eq!(get("COMMENT").as_deref(), Some("x"));
    assert_eq!(r.stem, PathBuf::from("Various/Record/03 Mine"));
    assert_eq!(why(&r, "TITLE").from, "song.tags");
    assert_eq!(why(&r, "TITLE").cleaned, Vec::<&str>::new());
}

#[test]
fn an_album_never_splits_across_sources() {
    let (a, b) = (manual("a.flac"), yt("bbbbbbbbbbb"));
    let mut fa = audio("flac", 22.0);
    fa.rev = "a".into();
    tag(&mut fa, Field::Album, &["Record"], true);
    tag(&mut fa, Field::Track, &["2"], true);
    let mut fb = audio("opus", 20.0);
    fb.rev = "b".into();
    tag(&mut fb, Field::Album, &["Record (Deluxe)"], true);
    tag(&mut fb, Field::Track, &["7"], true);
    tag(&mut fb, Field::Date, &["2020-01-01"], true);
    let facts = BTreeMap::from([(a.clone(), fa), (b.clone(), fb)]);
    let r = run(&song(&[a, b]), &facts, &[]);
    let get = |k: &str| {
        r.plan
            .tags
            .iter()
            .find(|(t, _)| t == k)
            .map(|(_, v)| v.join("|"))
    };
    assert_eq!(get("ALBUM").as_deref(), Some("Record"));
    assert_eq!(get("TRACKNUMBER").as_deref(), Some("2"));
    assert_eq!(
        get("DATE").as_deref(),
        Some("2020-01-01"),
        "a field the release lacks is still filled"
    );
}

#[test]
fn release_ids_come_with_the_album_and_recording_ids_on_their_own() {
    let (a, mb) = (
        manual("a.flac"),
        SourceKey::parse("musicbrainz:00000000-0000-0000-0000-000000000001").unwrap(),
    );
    let mut fa = audio("flac", 22.0);
    tag(&mut fa, Field::Album, &["Record"], true);
    tag(&mut fa, Field::Isrc, &["XX0000000001"], true);
    let mut fmb = Facts::unreadable("mb".into());
    tag(&mut fmb, Field::Album, &["Record (Deluxe)"], true);
    tag(
        &mut fmb,
        Field::MusicBrainzAlbumId,
        &["00000000-0000-0000-0000-00000000000a"],
        true,
    );
    tag(
        &mut fmb,
        Field::MusicBrainzTrackId,
        &["00000000-0000-0000-0000-000000000001"],
        true,
    );
    let facts = BTreeMap::from([(a.clone(), fa), (mb.clone(), fmb)]);
    let r = run(&song(&[a, mb.clone()]), &facts, &[]);
    let get = |k: &str| {
        r.plan
            .tags
            .iter()
            .find(|(t, _)| t == k)
            .map(|(_, v)| v.join("|"))
    };
    assert_eq!(get("ALBUM").as_deref(), Some("Record"));
    assert_eq!(
        get("MUSICBRAINZ_ALBUMID"),
        None,
        "another release's ID never joins the album"
    );
    assert_eq!(why(&r, "MUSICBRAINZ_TRACKID").from, mb.to_string());
    assert_eq!(get("ISRC").as_deref(), Some("XX0000000001"));
}

#[test]
fn formats_follow_the_winning_codec() {
    let opus = |kbps| Format::Encode {
        codec: Codec::Opus,
        kbps: Some(kbps),
    };
    let flac = Format::Encode {
        codec: Codec::Flac,
        kbps: None,
    };
    for (codec, channels, format) in [
        ("alac", 2, flac),
        ("pcm_s16le", 2, flac),
        ("mp3", 2, opus(160)),
        ("aac", 6, opus(256)),
    ] {
        let key = manual("x");
        let mut f = audio(codec, 20.0);
        f.audio.as_mut().unwrap().channels = channels;
        let facts = BTreeMap::from([(key.clone(), f)]);
        assert_eq!(
            run(&song(&[key]), &facts, &[]).plan.format,
            format,
            "{codec}"
        );
    }
}

#[test]
fn a_listed_codec_is_copied_and_the_rest_encoded_to_the_targets() {
    let settings = Audio {
        codecs: vec![Codec::Aac, Codec::Mp3],
        lossy: Codec::Mp3,
        lossless: Codec::Alac,
        ..Audio::default()
    };
    for (codec, format) in [
        ("aac", Format::Copy { codec: Codec::Aac }),
        ("mp3", Format::Copy { codec: Codec::Mp3 }),
        ("alac", Format::Copy { codec: Codec::Alac }),
        (
            "opus",
            Format::Encode {
                codec: Codec::Mp3,
                kbps: Some(320),
            },
        ),
        (
            "flac",
            Format::Encode {
                codec: Codec::Alac,
                kbps: None,
            },
        ),
    ] {
        let key = manual("x");
        let facts = BTreeMap::from([(key.clone(), audio(codec, 20.0))]);
        let r = run_with(&song(&[key]), &facts, &[], &settings, &QUALITY);
        assert_eq!(r.plan.format, format, "{codec}");
    }
    let lossy = Audio {
        lossless: Codec::Aac,
        ..settings
    };
    let key = manual("x");
    let facts = BTreeMap::from([(key.clone(), audio("pcm_s24le", 20.0))]);
    assert_eq!(
        run_with(&song(&[key]), &facts, &[], &lossy, &QUALITY)
            .plan
            .format,
        Format::Encode {
            codec: Codec::Aac,
            kbps: Some(256),
        },
        "lossless audio encodes to a lossy target at its bitrate"
    );
}

#[test]
fn plans_stored_before_other_codecs_read_as_today() {
    for (stored, format) in [
        (r#""OpusCopy""#, Format::Copy { codec: Codec::Opus }),
        (r#""FlacCopy""#, Format::Copy { codec: Codec::Flac }),
        (
            r#"{"OpusEncode":{"channels":2,"kbps":160}}"#,
            Format::Encode {
                codec: Codec::Opus,
                kbps: Some(160),
            },
        ),
        (
            r#""FlacEncode""#,
            Format::Encode {
                codec: Codec::Flac,
                kbps: None,
            },
        ),
    ] {
        assert_eq!(
            serde_json::from_str::<Format>(stored).unwrap(),
            format,
            "{stored}"
        );
    }
    let today = Format::Encode {
        codec: Codec::Mp3,
        kbps: Some(320),
    };
    let json = serde_json::to_string(&today).unwrap();
    assert_eq!(serde_json::from_str::<Format>(&json).unwrap(), today);
}

#[test]
fn a_measure_switched_off_or_outweighed_no_longer_decides() {
    let (narrow, wide) = (yt("nnnnnnnnnnn"), yt("wwwwwwwwwww"));
    let mut n = audio("opus", 16.0);
    n.rev = "n".into();
    let mut w = audio("opus", 20.0);
    w.rev = "w".into();
    w.audio
        .as_mut()
        .unwrap()
        .quality
        .as_mut()
        .unwrap()
        .incoherence = 0.0;
    let facts = BTreeMap::from([(narrow.clone(), n), (wide.clone(), w)]);
    let alignments = both(&facts, &narrow, &wide, 0, 0.99).to_vec();
    let songs = song(&[narrow.clone(), wide.clone()]);
    let pick = |quality: &Quality| {
        run_with(&songs, &facts, &alignments, &AUDIO, quality)
            .plan
            .audio
            .key
    };
    assert_eq!(pick(&QUALITY), wide, "8 steps of bandwidth outweigh mono");
    let mut off = Quality::default();
    off.bandwidth.enabled = false;
    assert_eq!(pick(&off), narrow, "without bandwidth, stereo decides");
    let mut heavy = Quality::default();
    heavy.stereo.weight = 100;
    assert_eq!(pick(&heavy), narrow, "stereo outweighs 8 steps");
}

#[test]
fn a_song_without_audio_on_disk_is_an_error() {
    let facts: BTreeMap<SourceKey, Facts> = BTreeMap::new();
    let s = song(&[yt("aaaaaaaaaaa")]);
    assert!(
        resolve(&Input {
            song: &s,
            album: None,
            facts: &facts,
            alignments: &[],
            lyrics: &[],
            clean: &Settings::default(),
            albums: &Albums::default(),
            naming: &NAMING,
            audio: &AUDIO,
            quality: &QUALITY,
            placement: LyricsPlacement::Sidecar,
        })
        .is_err()
    );
}

#[test]
fn every_ordered_pair_of_audible_sources_is_wanted() {
    let (song, facts, _) = release_and_video();
    assert_eq!(wanted_alignments(&song, &facts).len(), 2);
    let mut no_audio = Facts::unreadable("x".into());
    no_audio.tags = Offers::new();
    let mut facts = facts;
    facts.insert(manual("x.lrc"), no_audio);
    let mut song = song;
    song.sources.push(manual("x.lrc"));
    assert_eq!(wanted_alignments(&song, &facts).len(), 2);
}

/// Lyrics alone, as LRCLIB's are kept: timed, stating `stated_ms`.
fn stated_lyrics(stated_ms: i64) -> Facts {
    let mut f = Facts::unreadable("l".into());
    lyrics(&mut f, LyricsAt::File, "en", 30);
    f.lyrics[0].language = Language::Unstated;
    f.lyrics[0].stated_ms = Some(stated_ms);
    f
}

#[test]
fn stated_lyrics_yield_to_a_subtitle_of_the_same_recording_and_need_its_length() {
    let (mut s, mut facts, alignments) = release_and_video();
    let release_ms = ms(facts[&yt("rrrrrrrrrrr")].duration);
    let lrclib = SourceKey::parse("lrclib:7").unwrap();
    facts.insert(lrclib.clone(), stated_lyrics(release_ms + 500));
    s.sources.push(lrclib.clone());
    let r = run(&s, &facts, &alignments);
    assert_eq!(r.plan.lyrics.unwrap().key, yt("vvvvvvvvvvv"), "{:?}", r.why);

    facts.get_mut(&yt("vvvvvvvvvvv")).unwrap().lyrics.clear();
    let r = run(&s, &facts, &alignments);
    assert_eq!(r.plan.lyrics.unwrap().key, lrclib, "{:?}", r.why);

    facts.insert(lrclib.clone(), stated_lyrics(release_ms + 10_000));
    let r = run(&s, &facts, &alignments);
    assert!(
        r.plan.lyrics.is_none(),
        "timed to another length: {:?}",
        r.why
    );
}
