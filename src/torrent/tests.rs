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
    for parts in [&["..", "x.flac"][..], &["a/b.flac"], &[""], &["."]] {
        let info = B::D(vec![
            ("name", s("Album")),
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
