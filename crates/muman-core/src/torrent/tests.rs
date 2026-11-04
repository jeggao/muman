use sha1::Sha1;
use sha2::Sha256;

use super::*;

/// A bencoded value to build a torrent from, written as a test reads.
enum B {
    I(i64),
    S(Vec<u8>),
    L(Vec<B>),
    D(Vec<(&'static str, B)>),
}

fn s(text: &str) -> B {
    B::S(text.as_bytes().to_vec())
}

fn path(parts: &[&str]) -> B {
    B::L(parts.iter().map(|p| s(p)).collect())
}

fn encode(value: &B) -> Vec<u8> {
    match value {
        B::I(n) => format!("i{n}e").into_bytes(),
        B::S(bytes) => [format!("{}:", bytes.len()).into_bytes(), bytes.clone()].concat(),
        B::L(items) => [
            b"l".to_vec(),
            items.iter().flat_map(encode).collect(),
            b"e".to_vec(),
        ]
        .concat(),
        B::D(entries) => {
            let mut sorted: Vec<&(&str, B)> = entries.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            let body: Vec<u8> = sorted
                .iter()
                .flat_map(|(k, v)| [encode(&s(k)), encode(v)].concat())
                .collect();
            [b"d".to_vec(), body, b"e".to_vec()].concat()
        }
    }
}

/// A torrent of `info`, announced at `trackers`, and its info's bytes.
fn torrent(info: B, trackers: &[&str]) -> (Vec<u8>, Vec<u8>) {
    let raw = encode(&info);
    let mut top = vec![("info", info)];
    if let Some(first) = trackers.first() {
        top.push(("announce", s(first)));
        top.push((
            "announce-list",
            B::L(trackers.iter().map(|t| B::L(vec![s(t)])).collect()),
        ));
    }
    (encode(&B::D(top)), raw)
}

fn album_v1() -> B {
    B::D(vec![
        ("name", s("Marlo Venn - Lantern Weather")),
        ("piece length", B::I(16384)),
        ("pieces", B::S(vec![0; 20])),
        (
            "files",
            B::L(vec![
                B::D(vec![
                    ("length", B::I(300)),
                    ("path", path(&["01 Paper Comets.flac"])),
                ]),
                B::D(vec![
                    ("length", B::I(12)),
                    ("path", path(&[".pad", "12"])),
                    ("attr", s("p")),
                ]),
                B::D(vec![
                    ("length", B::I(200)),
                    ("path", path(&["Scans", "front.jpg"])),
                    ("path.utf-8", path(&["Scans", "Front.jpg"])),
                ]),
            ]),
        ),
    ])
}

#[test]
fn a_v1_torrent_is_named_by_the_sha1_of_its_info() {
    let (bytes, raw) = torrent(album_v1(), &["udp://tracker.example:1337/announce"]);
    let meta = Metainfo::read(&bytes).unwrap();
    assert_eq!(meta.v1.as_deref(), Some(hex(&Sha1::digest(&raw)).as_str()));
    assert_eq!(meta.v2, None);
    assert_eq!(meta.hash(), meta.v1.as_deref().unwrap());
    assert!(meta.fetchable());
    assert_eq!(meta.name, "Marlo Venn - Lantern Weather");
    assert!(!meta.private);
    assert_eq!(meta.trackers, ["udp://tracker.example:1337/announce"]);
    let files: Vec<(PathBuf, u64, usize)> = meta
        .files
        .iter()
        .map(|f| (f.path.clone(), f.length, f.index))
        .collect();
    assert_eq!(
        files,
        [
            (PathBuf::from("01 Paper Comets.flac"), 300, 1),
            (["Scans", "Front.jpg"].iter().collect(), 200, 3),
        ],
        "the padding file is no file of it, but keeps its place in the count"
    );
    assert_eq!(
        meta.key(&meta.files[0]).to_string(),
        format!("torrent:{}/01 Paper Comets.flac", meta.hash())
    );
}

#[test]
fn a_file_shares_its_end_pieces_with_its_neighbours_alone() {
    let file =
        |name: &str, length: i64| B::D(vec![("length", B::I(length)), ("path", path(&[name]))]);
    let info = B::D(vec![
        ("name", s("Lantern Weather")),
        ("piece length", B::I(100)),
        ("pieces", B::S(vec![0; 5 * 20])),
        (
            "files",
            B::L(vec![
                file("a.nfo", 150),
                file("b.flac", 200),
                file("c.jpg", 50),
                file("d.log", 10),
            ]),
        ),
    ]);
    let (bytes, _) = torrent(info, &[]);
    let meta = Metainfo::read(&bytes).unwrap();
    assert_eq!(meta.piece_length, 100);
    let offsets: Vec<u64> = meta.files.iter().map(|f| f.offset).collect();
    assert_eq!(offsets, [0, 150, 350, 400]);
    let song = [&meta.files[1]];
    let names: Vec<String> = sharing(&meta, &song)
        .iter()
        .map(|f| f.path.display().to_string())
        .collect();
    assert_eq!(names, ["a.nfo", "c.jpg"], "d.log starts a piece of its own");
}

#[test]
fn a_torrent_of_one_file_names_it_by_its_name() {
    let info = B::D(vec![
        ("name", s("Rooms of Salt.flac")),
        ("length", B::I(42)),
        ("piece length", B::I(16384)),
        ("pieces", B::S(vec![0; 20])),
        ("private", B::I(1)),
    ]);
    let (bytes, _) = torrent(info, &["https://tracker.example/a/announce"]);
    let meta = Metainfo::read(&bytes).unwrap();
    assert_eq!(meta.files.len(), 1);
    assert_eq!(meta.files[0].path, Path::new("Rooms of Salt.flac"));
    assert!(meta.private);
}

fn tree_file(length: i64, root: u8) -> B {
    B::D(vec![(
        "",
        B::D(vec![
            ("length", B::I(length)),
            ("pieces root", B::S(vec![root; 32])),
        ]),
    )])
}

#[test]
fn a_hybrid_is_named_by_its_v1_hash_and_keeps_its_v2_hash_and_roots() {
    let B::D(mut info) = album_v1() else {
        unreachable!()
    };
    info.push(("meta version", B::I(2)));
    info.push((
        "file tree",
        B::D(vec![
            ("01 Paper Comets.flac", tree_file(300, 0xab)),
            ("Scans", B::D(vec![("Front.jpg", tree_file(200, 0xcd))])),
        ]),
    ));
    let (bytes, raw) = torrent(B::D(info), &[]);
    let meta = Metainfo::read(&bytes).unwrap();
    assert_eq!(meta.v1.as_deref(), Some(hex(&Sha1::digest(&raw)).as_str()));
    assert_eq!(
        meta.v2.as_deref(),
        Some(hex(&Sha256::digest(&raw)).as_str())
    );
    assert_eq!(meta.hash().len(), V1_HEX);
    assert_eq!(meta.files[0].root, Some("ab".repeat(32)));
    assert_eq!(meta.files[1].root, Some("cd".repeat(32)));
}

#[test]
fn a_v2_torrent_is_named_by_its_v2_hash_and_no_v1_fetcher_takes_it() {
    let info = B::D(vec![
        ("name", s("The Glass Orchards")),
        ("meta version", B::I(2)),
        ("piece length", B::I(16384)),
        (
            "file tree",
            B::D(vec![
                ("b.flac", tree_file(5, 1)),
                ("a.flac", tree_file(7, 2)),
            ]),
        ),
    ]);
    let (bytes, raw) = torrent(info, &[]);
    let meta = Metainfo::read(&bytes).unwrap();
    assert_eq!(meta.v1, None);
    assert_eq!(meta.hash(), hex(&Sha256::digest(&raw)));
    assert!(!meta.fetchable());
    let names: Vec<&Path> = meta.files.iter().map(|f| f.path.as_path()).collect();
    assert_eq!(names, [Path::new("a.flac"), Path::new("b.flac")]);
    assert_eq!(meta.files[1].index, 2);
}

#[test]
fn a_path_leaving_the_torrent_refuses_it() {
    for parts in [
        &["..", "x.flac"][..],
        &["a/b.flac"],
        &["a\\b.flac"],
        &[""],
        &["."],
        &[".. ", "x.flac"],
        &["..."],
        &["C:", "x.flac"],
        &["c:x.flac"],
        &["Album\u{1b}]0;Lantern\u{7}", "x.flac"],
        &["x\u{9b}2J.flac"],
        &["x\0.flac"],
    ] {
        let info = B::D(vec![
            ("name", s("Album")),
            ("piece length", B::I(16384)),
            ("pieces", B::S(vec![0; 20])),
            (
                "files",
                B::L(vec![B::D(vec![("length", B::I(1)), ("path", path(parts))])]),
            ),
        ]);
        let (bytes, _) = torrent(info, &[]);
        assert!(Metainfo::read(&bytes).is_err(), "{parts:?}");
    }
}

#[test]
fn what_is_no_torrent_is_refused_without_panicking() {
    let deep = "l".repeat(10_000) + &"e".repeat(10_000);
    for bad in [
        &b""[..],
        b"de",
        b"d4:infoi1ee",
        b"d4:info",
        b"5:abc",
        b"i12",
        b"d4:infodee",
        deep.as_bytes(),
    ] {
        assert!(
            Metainfo::read(bad).is_err(),
            "{}",
            String::from_utf8_lossy(bad)
        );
    }
}

#[test]
fn a_magnet_link_names_its_torrent_by_hex_or_base32() {
    let hex = "0123456789abcdef0123456789abcdef01234567";
    let link = format!(
        "magnet:?xt=urn:btih:{}&dn=Lantern%20Weather&tr=udp%3A%2F%2Ftracker.example%3A1337",
        hex.to_uppercase()
    );
    let magnet = Magnet::parse(&link).unwrap();
    assert_eq!(magnet.hash(), hex);
    assert_eq!(magnet.name.as_deref(), Some("Lantern Weather"));
    assert_eq!(magnet.trackers, ["udp://tracker.example:1337"]);
    assert_eq!(Magnet::parse(&magnet.link()).unwrap(), magnet);
    let base32 = Magnet::parse("magnet:?xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH").unwrap();
    assert_eq!(base32.hash(), hex);
}

#[test]
fn a_hybrid_magnet_link_carries_both_hashes() {
    let v1 = "1".repeat(40);
    let v2 = "2".repeat(64);
    let magnet = Magnet::parse(&format!("magnet:?xt=urn:btih:{v1}&xt=urn:btmh:1220{v2}")).unwrap();
    assert_eq!(magnet.v1.as_deref(), Some(v1.as_str()));
    assert_eq!(magnet.v2.as_deref(), Some(v2.as_str()));
    assert_eq!(magnet.hash(), v1);
    let v2_only = Magnet::parse(&format!("magnet:?xt=urn:btmh:1220{v2}")).unwrap();
    assert_eq!(v2_only.hash(), v2);
    for bad in [
        "https://example.org/x",
        "magnet:?dn=nothing",
        "magnet:?xt=urn:btih:abc",
        "magnet:?xt=urn:btmh:1114abcd",
    ] {
        assert!(Magnet::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_file_is_kept_at_a_path_every_system_takes() {
    let kept = |p: &str| stored(&crate::relpath::from_portable(p));
    assert_eq!(
        kept("CD1/01 Paper Comets.flac"),
        Path::new("CD1").join("01 Paper Comets.flac")
    );
    assert_eq!(
        kept("Who? What: Where*.flac"),
        Path::new("Who？ What： Where＊.flac")
    );
    assert_eq!(kept(".hidden.flac"), Path::new("．hidden.flac"));
    assert_eq!(
        kept("Demos. /a.flac"),
        Path::new("Demos．\u{3000}").join("a.flac")
    );
    assert_eq!(kept("con.flac"), Path::new("con_.flac"));
}

#[test]
fn of_two_files_kept_at_one_place_the_first_is_taken() {
    let meta = Metainfo::read(&crate::testing::metainfo(
        "Marlo Venn - Paper Comets",
        &[
            "01 Lantern?.mp3",
            "01 Lantern？.mp3",
            "02 Wren.flac",
            "02 WREN.flac",
            "03 Ash.mp3",
        ],
    ))
    .unwrap();
    let taken: Vec<&Path> = selected(&meta).iter().map(|f| f.path.as_path()).collect();
    assert_eq!(
        taken,
        ["01 Lantern?.mp3", "02 Wren.flac", "03 Ash.mp3"].map(Path::new)
    );
    assert_eq!(
        shadowed(&meta),
        [
            (Path::new("01 Lantern？.mp3"), Path::new("01 Lantern?.mp3")),
            (Path::new("02 WREN.flac"), Path::new("02 Wren.flac")),
        ]
    );
}

#[test]
fn a_file_checks_when_every_piece_it_spans_does() {
    let pieces: Vec<u8> = [&b"aaaabb"[..], b"bbcc"]
        .iter()
        .flat_map(|p| Sha1::digest(p).to_vec())
        .collect();
    let info = B::D(vec![
        ("name", s("Marlo Venn - Lantern Weather")),
        ("piece length", B::I(6)),
        ("pieces", B::S(pieces)),
        (
            "files",
            B::L(vec![
                B::D(vec![("length", B::I(4)), ("path", path(&["a.flac"]))]),
                B::D(vec![("length", B::I(4)), ("path", path(&["b.flac"]))]),
                B::D(vec![("length", B::I(2)), ("path", path(&["c.flac"]))]),
            ]),
        ),
    ]);
    let meta = Metainfo::read(&torrent(info, &[]).0).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let at = |f: &File| dir.path().join(f.index.to_string());
    let write = |n: usize, bytes: &[u8]| std::fs::write(dir.path().join(n.to_string()), bytes);
    let checked = || {
        meta.files
            .iter()
            .map(|f| checks(&meta, f, at))
            .collect::<Vec<_>>()
    };
    write(1, b"aaaa").unwrap();
    assert_eq!(
        checked(),
        [false, false, false],
        "b.flac's start is missing"
    );
    write(2, b"bbbb").unwrap();
    write(3, b"cX").unwrap();
    assert_eq!(checked(), [true, false, false]);
    write(3, b"cc").unwrap();
    assert_eq!(checked(), [true, true, true]);
}

#[test]
fn a_part_too_long_for_a_name_is_shortened_by_its_hash_and_keeps_its_extension() {
    let long = |c: &str| format!("{} Paper Comets.flac", c.repeat(120));
    let (a, b) = (
        stored(Path::new(&long("?"))),
        stored(Path::new(&long("夜"))),
    );
    for at in [&a, &b] {
        let name = at.to_str().unwrap();
        assert!(
            name.len() <= crate::store::PART_BYTES,
            "{} bytes: {name}",
            name.len()
        );
        assert_eq!(at.extension(), Some(std::ffi::OsStr::new("flac")), "{name}");
    }
    assert_ne!(a, b);
    assert_ne!(
        stored(Path::new(&long("？"))),
        a,
        "a lookalike's hash is its own"
    );
    assert_eq!(
        stored(Path::new("01 Paper Comets.flac")),
        Path::new("01 Paper Comets.flac")
    );
}

/// A v1 torrent of `files`, each a name and a length, in pieces of
/// `piece`, holding `hashes` piece hashes.
fn sized(files: &[(&str, i64)], piece: i64, hashes: usize) -> Vec<u8> {
    let list = files
        .iter()
        .map(|(name, length)| B::D(vec![("length", B::I(*length)), ("path", path(&[name]))]))
        .collect();
    let info = B::D(vec![
        ("name", s("The Glass Orchards")),
        ("piece length", B::I(piece)),
        ("pieces", B::S(vec![0; 20 * hashes])),
        ("files", B::L(list)),
    ]);
    torrent(info, &[]).0
}

/// A v2 torrent of files of `lengths`, in pieces of `piece`.
fn sized_v2(lengths: &[i64], piece: i64) -> Vec<u8> {
    let tree = lengths
        .iter()
        .zip(["a.flac", "b.flac", "c.flac"])
        .map(|(length, name)| (name, tree_file(*length, 1)))
        .collect();
    let info = B::D(vec![
        ("name", s("The Glass Orchards")),
        ("meta version", B::I(2)),
        ("piece length", B::I(piece)),
        ("file tree", B::D(tree)),
    ]);
    torrent(info, &[]).0
}

#[test]
fn a_piece_or_a_torrent_too_long_to_be_real_refuses_it_unread() {
    let huge = 1 << 42;
    assert!(Metainfo::read(&sized(&[("a.flac", huge)], huge, 1)).is_err());
    let most = i64::MAX;
    let three = [("a.flac", most), ("b.flac", most), ("c.flac", most)];
    assert!(Metainfo::read(&sized(&three, 16384, 1)).is_err());
    assert!(Metainfo::read(&sized(&[("a.flac", 1 << 48)], 1 << 28, 1)).is_err());
    for piece in [0, -1, 1 << 30] {
        assert!(
            Metainfo::read(&sized(&[("a.flac", 10)], piece, 1)).is_err(),
            "{piece}"
        );
    }
    assert!(Metainfo::read(&sized_v2(&[most, most, most], 16384)).is_err());
    assert!(Metainfo::read(&sized_v2(&[most, most], most)).is_err());
    assert!(
        Metainfo::read(&sized_v2(&[10], 16383)).is_err(),
        "below 16 KiB"
    );
    assert!(
        Metainfo::read(&sized_v2(&[10], 3 << 14)).is_err(),
        "no power of two"
    );
    assert!(Metainfo::read(&sized_v2(&[10, 10], 1 << 15)).is_ok());
}

#[test]
fn a_v1_torrent_holds_a_hash_for_each_piece_its_files_span() {
    let files = [("a.flac", 150), ("b.flac", 100)];
    assert!(Metainfo::read(&sized(&files, 100, 3)).is_ok());
    for hashes in [0, 2, 4] {
        assert!(
            Metainfo::read(&sized(&files, 100, hashes)).is_err(),
            "{hashes}"
        );
    }
}

#[test]
fn a_name_with_a_colon_past_its_start_is_a_plain_name() {
    let meta = Metainfo::read(&crate::testing::metainfo(
        "Marlo Venn: Lantern Weather",
        &["Act 1: Paper Comets.flac", "1:2.flac", "...x.flac"],
    ))
    .unwrap();
    assert_eq!(meta.name, "Marlo Venn: Lantern Weather");
    assert_eq!(meta.files.len(), 3);
}

#[test]
fn a_piece_longer_than_a_read_is_checked_a_chunk_at_a_time() {
    let piece = CHECK_CHUNK * 3;
    let song: Vec<u8> = (0..CHECK_CHUNK * 2 + 5).map(|i| (i % 251) as u8).collect();
    let pad = piece - song.len();
    let tail = b"tail".to_vec();
    let whole: Vec<u8> = [song.clone(), vec![0; pad], tail.clone(), vec![0; 6]].concat();
    let pieces: Vec<u8> = whole
        .chunks(piece)
        .flat_map(|p| Sha1::digest(p).to_vec())
        .collect();
    let entry = |name: &str, length: usize, padding: bool| {
        let mut e = vec![
            ("length", B::I(i64::try_from(length).unwrap())),
            ("path", path(&[name])),
        ];
        if padding {
            e.push(("attr", s("p")));
        }
        B::D(e)
    };
    let info = B::D(vec![
        ("name", s("Marlo Venn - Lantern Weather")),
        ("piece length", B::I(i64::try_from(piece).unwrap())),
        ("pieces", B::S(pieces)),
        (
            "files",
            B::L(vec![
                entry("a.flac", song.len(), false),
                entry("pad0", pad, true),
                entry("b.flac", tail.len(), false),
                entry("pad1", 6, true),
            ]),
        ),
    ]);
    let meta = Metainfo::read(&torrent(info, &[]).0).unwrap();
    assert_eq!(meta.total, whole.len() as u64);
    let dir = tempfile::tempdir().unwrap();
    let at = |f: &File| dir.path().join(f.index.to_string());
    std::fs::write(dir.path().join("1"), &song).unwrap();
    std::fs::write(dir.path().join("3"), &tail).unwrap();
    assert!(checks(&meta, &meta.files[0], at));
    assert!(
        checks(&meta, &meta.files[1], at),
        "its piece ends in padding"
    );
    let mut damaged = song;
    damaged[CHECK_CHUNK + 7] ^= 1;
    std::fs::write(dir.path().join("1"), &damaged).unwrap();
    assert!(!checks(&meta, &meta.files[0], at));
}
