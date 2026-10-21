//! aria2c handed the command lines and input files muman builds, against
//! a swarm of this process's own: a tracker served on a loopback port and
//! a second aria2c seeding a torrent made here. An option aria2c renames
//! or drops, or no longer takes in an input file, or a file it numbers
//! otherwise than muman selects it by, fails here and not on a user's
//! first fetch. Ignored by default, since it
//! needs aria2c; CI runs it on every platform with
//! `cargo test --test torrent -- --ignored`. Nothing leaves the machine:
//! DHT, peer exchange and local discovery are off, and the one tracker
//! names the one seeder.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use muman::aria2::{Failure, Places, Wanted, fetch, fetch_all, metainfo_of, take_up};
use muman::runner::System;
use muman::settings::Torrent;
use muman::source::SourceKey;
use muman::torrent::{Magnet, Metainfo, selected};
use sha1::{Digest, Sha1};

const SONG: &[u8] = include_bytes!("../testdata/silence.flac");
const PIECE: usize = 16 * 1024;

fn text(s: &str) -> Vec<u8> {
    [format!("{}:", s.len()).into_bytes(), s.as_bytes().to_vec()].concat()
}

/// A torrent of `files`, each a path and its bytes, padding files among
/// them as BEP 47 marks them, announced at `tracker`: its metainfo.
fn metainfo(name: &str, files: &[(&[&str], Vec<u8>, bool)], tracker: &str) -> Vec<u8> {
    let all: Vec<u8> = files.iter().flat_map(|(_, b, _)| b.clone()).collect();
    let pieces: Vec<u8> = all
        .chunks(PIECE)
        .flat_map(|c| Sha1::digest(c).to_vec())
        .collect();
    let mut list = b"l".to_vec();
    for (path, bytes, pad) in files {
        list.extend(b"d");
        if *pad {
            list.extend(text("attr"));
            list.extend(text("p"));
        }
        list.extend(text("length"));
        list.extend(format!("i{}e", bytes.len()).bytes());
        list.extend(text("path"));
        list.extend(b"l");
        for part in *path {
            list.extend(text(part));
        }
        list.extend(b"ee");
    }
    list.extend(b"e");
    let mut info = b"d".to_vec();
    info.extend(text("files"));
    info.extend(list);
    info.extend(text("name"));
    info.extend(text(name));
    info.extend(text("piece length"));
    info.extend(format!("i{PIECE}e").bytes());
    info.extend(text("pieces"));
    info.extend(format!("{}:", pieces.len()).bytes());
    info.extend(pieces);
    info.extend(b"e");
    [
        b"d".to_vec(),
        text("announce"),
        text(tracker),
        text("info"),
        info,
        b"e".to_vec(),
    ]
    .concat()
}

/// A port nothing listens on now.
fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// An HTTP tracker on a loopback port that names the one peer at `peer`
/// to every announce, until the process ends; its announce address.
fn tracker(peer: u16) -> String {
    tracker_heard(peer).0
}

/// [`tracker`], each announcing peer's port sent on the receiver as it
/// announces.
fn tracker_heard(peer: u16) -> (String, mpsc::Receiver<u16>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (heard, hear) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            if let Some(port) = request
                .split_once("&port=")
                .and_then(|(_, rest)| rest.split('&').next()?.parse().ok())
            {
                let _ = heard.send(port);
            }
            let mut body = b"d8:intervali5e5:peers6:".to_vec();
            body.extend([127, 0, 0, 1]);
            body.extend(peer.to_be_bytes());
            body.extend(b"e");
            let _ = write!(
                stream,
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    (format!("http://127.0.0.1:{port}/announce"), hear)
}

/// No DHT, peer exchange or local discovery: the tracker alone.
const ALONE: [&str; 3] = [
    "--enable-dht=false",
    "--enable-peer-exchange=false",
    "--bt-enable-lpd=false",
];

/// An aria2c seeding the torrent `metainfo` from `dir` on `port`.
struct Seeder(Child);

impl Seeder {
    fn start(metainfo: &Path, dir: &Path, port: u16) -> Self {
        let child = Command::new(std::env::var_os("MUMAN_ARIA2C").unwrap_or("aria2c".into()))
            .args(ALONE)
            .args([
                "--no-conf=true",
                "--check-integrity=true",
                "--seed-ratio=0.0",
                &format!("--listen-port={port}"),
                &format!("--dir={}", dir.display()),
            ])
            .arg(metainfo)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("aria2c spawns");
        Self(child)
    }
}

impl Drop for Seeder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "needs aria2c"]
fn aria2c_fetches_the_files_muman_selects_from_a_swarm() {
    let runner = System::default();
    runner
        .require("aria2c")
        .expect("found, at or above its floor");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let seed_port = free_port();
    let announce = tracker(seed_port);
    let name = "Lantern Weather";
    let notes = b"Marlo Venn, recorded at home".repeat(1000);
    let pad = vec![0; PIECE - notes.len() % PIECE];
    let files: Vec<(&[&str], Vec<u8>, bool)> = vec![
        (&["notes.nfo"], notes, false),
        (&[".pad", "pad"], pad, true),
        (&["CD1", "01 Paper Comets.flac"], SONG.to_vec(), false),
    ];
    let bytes = metainfo(name, &files, &announce);
    let meta = Metainfo::read(&bytes).unwrap();

    let seed = root.join("seed");
    for (path, content, _) in &files {
        let at = path.iter().fold(seed.join(name), |p, part| p.join(part));
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, content).unwrap();
    }
    let seed_meta = root.join("seed.torrent");
    std::fs::write(&seed_meta, &bytes).unwrap();
    let _seeder = Seeder::start(&seed_meta, &seed, seed_port);

    let (store, partial) = (root.join("store"), root.join("partial"));
    let options = Torrent {
        args: ALONE
            .iter()
            .map(ToString::to_string)
            .chain([format!("--listen-port={}", free_port())])
            .collect(),
        stall: Duration::from_secs(60),
        ..Torrent::default()
    };
    let places = Places {
        store: &store,
        partial: &partial,
        options: &options,
    };
    let magnet = Magnet::parse(&format!(
        "magnet:?xt=urn:btih:{}&tr={}",
        meta.hash(),
        announce
    ))
    .unwrap();
    let mut out = Vec::new();
    let asked = metainfo_of(&runner, &mut out, &places, &magnet).unwrap_or_else(|e| {
        panic!("{e:#}\n{}", String::from_utf8_lossy(&out));
    });
    assert_eq!(Metainfo::read(&asked).unwrap().hash(), meta.hash());

    muman::aria2::keep_metainfo(&store, meta.hash(), &bytes).unwrap();
    let wanted = selected(&meta);
    assert_eq!(wanted.len(), 1, "the song alone");
    let fetched = fetch(&runner, &mut out, &places, &meta, &wanted).unwrap();
    assert_eq!(fetched.failure, None, "{}", String::from_utf8_lossy(&out));
    let [SourceKey::Torrent(key)] = fetched.arrived.as_slice() else {
        panic!("{:?}", fetched.arrived);
    };
    let song = muman::store::torrent_path(&store, key);
    assert_eq!(std::fs::read(&song).unwrap(), SONG, "the song, whole");
    assert!(
        !store.join(meta.hash()).join("notes.nfo").exists(),
        "a file not selected stays on its peers"
    );
}

#[test]
#[ignore = "needs aria2c"]
fn a_download_already_made_is_taken_up_with_no_peer() {
    let runner = System::default();
    runner
        .require("aria2c")
        .expect("found, at or above its floor");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let nobody = format!("http://127.0.0.1:{}/announce", free_port());
    let name = "The Glass Orchards";
    let files: Vec<(&[&str], Vec<u8>, bool)> = vec![
        (&["scan.png"], vec![7; 5000], false),
        (&["Rooms of Salt.flac"], SONG.to_vec(), false),
        (&["log.txt"], vec![9; 3000], false),
    ];
    let bytes = metainfo(name, &files, &nobody);
    let meta = Metainfo::read(&bytes).unwrap();
    let downloads = root.join("downloads");
    for (path, content, _) in &files {
        let at = path
            .iter()
            .fold(downloads.join(name), |p, part| p.join(part));
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, content).unwrap();
    }
    let (store, partial) = (root.join("store"), root.join("partial"));
    let options = Torrent {
        args: ALONE
            .iter()
            .map(ToString::to_string)
            .chain([format!("--listen-port={}", free_port())])
            .collect(),
        stall: Duration::from_secs(60),
        ..Torrent::default()
    };
    let places = Places {
        store: &store,
        partial: &partial,
        options: &options,
    };
    muman::aria2::keep_metainfo(&store, meta.hash(), &bytes).unwrap();
    let wanted = selected(&meta);
    let copied = take_up(&places, &meta, &wanted, &downloads).unwrap();
    assert_eq!(copied, 3, "the song and the two files its pieces share");
    let mut out = Vec::new();
    let fetched = fetch(&runner, &mut out, &places, &meta, &wanted).unwrap();
    assert_eq!(fetched.failure, None, "{}", String::from_utf8_lossy(&out));
    let [SourceKey::Torrent(key)] = fetched.arrived.as_slice() else {
        panic!("{:?}", fetched.arrived);
    };
    let song = muman::store::torrent_path(&store, key);
    assert_eq!(std::fs::read(&song).unwrap(), SONG, "the song, whole");
}

#[test]
#[ignore = "needs aria2c"]
fn a_damaged_copy_has_its_bad_pieces_fetched_from_the_swarm() {
    let runner = System::default();
    runner
        .require("aria2c")
        .expect("found, at or above its floor");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let seed_port = free_port();
    let announce = tracker(seed_port);
    let name = "Rooms of Salt";
    let files: Vec<(&[&str], Vec<u8>, bool)> =
        vec![(&["Rooms of Salt.flac"], SONG.to_vec(), false)];
    let bytes = metainfo(name, &files, &announce);
    let meta = Metainfo::read(&bytes).unwrap();
    let seed = root.join("seed");
    std::fs::create_dir_all(seed.join(name)).unwrap();
    std::fs::write(seed.join(name).join("Rooms of Salt.flac"), SONG).unwrap();
    let seed_meta = root.join("seed.torrent");
    std::fs::write(&seed_meta, &bytes).unwrap();
    let _seeder = Seeder::start(&seed_meta, &seed, seed_port);

    let mut damaged = SONG.to_vec();
    let middle = damaged.len() / 2;
    damaged[middle..middle + 64].fill(0xAA);
    let downloads = root.join("downloads");
    std::fs::create_dir_all(downloads.join(name)).unwrap();
    std::fs::write(downloads.join(name).join("Rooms of Salt.flac"), &damaged).unwrap();

    let (store, partial) = (root.join("store"), root.join("partial"));
    let options = Torrent {
        args: ALONE
            .iter()
            .map(ToString::to_string)
            .chain([format!("--listen-port={}", free_port())])
            .collect(),
        stall: Duration::from_secs(60),
        ..Torrent::default()
    };
    let places = Places {
        store: &store,
        partial: &partial,
        options: &options,
    };
    muman::aria2::keep_metainfo(&store, meta.hash(), &bytes).unwrap();
    let wanted = selected(&meta);
    assert_eq!(take_up(&places, &meta, &wanted, &downloads).unwrap(), 1);
    let mut out = Vec::new();
    let fetched = fetch(&runner, &mut out, &places, &meta, &wanted).unwrap();
    assert_eq!(fetched.failure, None, "{}", String::from_utf8_lossy(&out));
    let [SourceKey::Torrent(key)] = fetched.arrived.as_slice() else {
        panic!("{:?}", fetched.arrived);
    };
    let song = muman::store::torrent_path(&store, key);
    assert_eq!(
        std::fs::read(&song).unwrap(),
        SONG,
        "the bad piece fetched whole"
    );
}

#[test]
#[ignore = "needs aria2c"]
fn torrents_fetched_in_one_run_end_each_on_their_own() {
    let runner = System::default();
    runner
        .require("aria2c")
        .expect("found, at or above its floor");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let seed_port = free_port();
    let (announce, heard) = tracker_heard(seed_port);
    let nobody = format!("http://127.0.0.1:{}/announce", free_port());
    let seeded = "Lantern Weather";
    let files: Vec<(&[&str], Vec<u8>, bool)> = vec![(&["Paper Comets.flac"], SONG.to_vec(), false)];
    let bytes = metainfo(seeded, &files, &announce);
    let meta = Metainfo::read(&bytes).unwrap();
    let seed = root.join("seed");
    std::fs::create_dir_all(seed.join(seeded)).unwrap();
    std::fs::write(seed.join(seeded).join("Paper Comets.flac"), SONG).unwrap();
    let seed_meta = root.join("seed.torrent");
    std::fs::write(&seed_meta, &bytes).unwrap();
    let _seeder = Seeder::start(&seed_meta, &seed, seed_port);
    // The seeder announces once it has checked its files and can send
    // them; fetched before then, the seeded torrent could stall too.
    while heard
        .recv_timeout(Duration::from_secs(60))
        .expect("the seeder announces")
        != seed_port
    {}
    let lost_bytes = metainfo(
        "The Glass Orchards",
        &[(&["Rooms of Salt.flac"], vec![5; 3000], false)],
        &nobody,
    );
    let lost = Metainfo::read(&lost_bytes).unwrap();

    // A home outside ASCII, as aria2c reads it from its input file.
    let home = root.join("Hjärta ♪");
    let (store, partial) = (home.join("store"), home.join("partial"));
    let options = Torrent {
        args: ALONE
            .iter()
            .map(ToString::to_string)
            .chain([format!("--listen-port={}", free_port())])
            .collect(),
        stall: Duration::from_secs(10),
        ..Torrent::default()
    };
    let places = Places {
        store: &store,
        partial: &partial,
        options: &options,
    };
    muman::aria2::keep_metainfo(&store, meta.hash(), &bytes).unwrap();
    muman::aria2::keep_metainfo(&store, lost.hash(), &lost_bytes).unwrap();
    let wanted = [
        Wanted {
            meta: &lost,
            files: selected(&lost),
        },
        Wanted {
            meta: &meta,
            files: selected(&meta),
        },
    ];
    let mut out = Vec::new();
    let fetched = fetch_all(&runner, &mut out, &places, &wanted).unwrap();
    let said = String::from_utf8_lossy(&out);
    assert!(
        matches!(fetched[0].failure, Some(Failure::Stalled(_))),
        "{:?}\n{said}",
        fetched[0]
    );
    assert_eq!(fetched[1].failure, None, "{said}");
    let [SourceKey::Torrent(key)] = fetched[1].arrived.as_slice() else {
        panic!("{:?}", fetched[1].arrived);
    };
    let song = muman::store::torrent_path(&store, key);
    assert_eq!(std::fs::read(&song).unwrap(), SONG, "the song, whole");
}
