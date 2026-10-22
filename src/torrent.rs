//! `BitTorrent` metainfo and magnet links, read: the torrent a
//! `torrent:<hash>/<path>` key names and the files it holds.
//!
//! A torrent's hash is that of its bencoded `info` dictionary, exactly as
//! the file holds it, so the reader keeps the dictionary's bytes rather
//! than encoding it again: SHA-1 for a v1 torrent, SHA-256 for a v2 one
//! (BEP 52), both for a hybrid. A key names a torrent by its v1 hash
//! where it has one, since aria2 fetches v1 swarms alone, and by its v2
//! hash only where it has no other; a hybrid's v2 hash and each file's
//! `pieces root`, the hash of that file alone, are read too, so a fetcher
//! of v2 can take them up without the song list changing format.
//!
//! A file's path in a key is its path inside the torrent, each part in
//! NFC, without the torrent's own folder, `info.name`, which every file
//! of a torrent of several shares; a torrent of one file names it by
//! that. A part that would leave the torrent's folder, or holds a
//! separator, refuses the torrent: its keys would not read back as its
//! files. So does a part that is a plain name on one system and not on
//! another, the same on every system so a key reads alike on each: one
//! of dots and spaces alone, which Windows trims to nothing or to `..`,
//! and one starting with a letter and a colon, which Windows reads as a
//! drive, so a folder another client left is never read off it. A part
//! holding a control character refuses it too, rather than have the
//! character replaced: the key would no longer name the file a client
//! wrote, and printed as it is, an escape would drive the terminal. No
//! client writes one, and Windows keeps no file named with one. Padding
//! files (BEP 47) hold nothing and are no file of it.
//!
//! A torrent's numbers are read as a hostile file may write them. Its
//! piece length is at least a byte and at most libtorrent's largest,
//! 2^15 − 1 blocks of 16 KiB, and a v2 one a power of two from 16 KiB, as
//! BEP 52 has it; its files run to at most libtorrent's last offset, 256
//! TiB, every offset summed in checked arithmetic; and a v1 torrent holds
//! one hash for each piece its files span, as libtorrent asks. Within
//! these, no sum of offsets and lengths overflows, and a torrent that
//! breaks one is refused whole, as libtorrent refuses it. A piece is
//! checked by reading it a chunk at a time, never whole, so a long piece
//! costs time, not memory.
//!
//! Of a torrent's files, muman fetches its songs and what they bring
//! ([`selected`]): each `.lrc` and picture named as a song file is, and
//! the cover of a song's folder, by the manual folder's rules. Scans,
//! logs and anything else a torrent holds are left on its peers.
//!
//! Where a file is kept is a function of its key alone ([`stored`]):
//! each part with what Windows, FAT or a phone refuses replaced by its
//! full-width lookalike, as yt-dlp and the library's safe names do, the
//! same on every system so a home copies between them, and a part longer
//! than a name may be on any of them shortened, its hash after it. Two files of a
//! torrent can then be kept at one place, as `song?.mp3` beside
//! `song？.mp3`, or `Song.mp3` beside `song.mp3` on a system blind to
//! case; the first the torrent lists is taken and the other left on its
//! peers ([`shadowed`]), as one would be written over the other.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;

use crate::facts::hex;
use crate::source::{SourceKey, TorrentKey};

/// Hex digits of a v1 hash, SHA-1's 20 bytes.
pub const V1_HEX: usize = 40;

/// Hex digits of a v2 hash, SHA-256's 32 bytes.
pub const V2_HEX: usize = 64;

/// How deep a bencoded value may nest: a real torrent's file tree is a
/// few folders deep, and a hostile one must not exhaust the stack.
const MAX_DEPTH: usize = 64;

/// The longest piece read: libtorrent's, 2^15 - 1 blocks of 16 KiB.
const MAX_PIECE: u64 = ((1 << 15) - 1) * V2_MIN_PIECE;

/// The shortest piece of a v2 torrent, which BEP 52 has a power of two.
const V2_MIN_PIECE: u64 = 16 << 10;

/// The most bytes a torrent's files may run to, padding counted:
/// libtorrent's last file offset, 256 TiB.
const MAX_TOTAL: u64 = (1 << 48) - 1;

/// How much of a piece is read at once to check it.
const CHECK_CHUNK: usize = 1 << 20;

/// Whether `s` is a torrent's hash as a key writes it: v1 or v2, in
/// lower-case hex.
#[must_use]
pub fn is_hash(s: &str) -> bool {
    (s.len() == V1_HEX || s.len() == V2_HEX)
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A torrent as its metainfo describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metainfo {
    /// The SHA-1 of its `info`, where it is a v1 or hybrid torrent.
    pub v1: Option<String>,
    /// The SHA-256 of its `info`, where it is a v2 or hybrid torrent.
    pub v2: Option<String>,
    /// Its own name: the folder its files are in, or its one file.
    pub name: String,
    /// Its files in the torrent's order, which a fetch selects them by.
    pub files: Vec<File>,
    /// Whether it is private (BEP 27): reached by its trackers alone, whose
    /// addresses may hold a personal key.
    pub private: bool,
    pub trackers: Vec<String>,
    /// The bytes each piece holds, the last but one.
    pub piece_length: u64,
    /// The SHA-1 of each v1 piece, in order.
    pub pieces: Vec<[u8; 20]>,
    /// The bytes of all its files, padding counted.
    pub total: u64,
}

/// One file a torrent holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Its path inside the torrent, part by part in NFC.
    pub path: PathBuf,
    pub length: u64,
    /// The hash of its content alone, a v2 `pieces root` in hex.
    pub root: Option<String>,
    /// Its place among the files a fetch counts, from 1, as aria2's
    /// `--select-file` takes it.
    pub index: usize,
    /// Where its bytes start among the torrent's, padding counted.
    pub offset: u64,
}

impl Metainfo {
    /// Read a `.torrent` file's bytes.
    #[deny(clippy::arithmetic_side_effects)]
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let (top, rest) = parse(bytes, 0).context("not a torrent: its bencoding does not read")?;
        ensure!(rest.is_empty(), "not a torrent: bytes follow its metainfo");
        let Value::Dict(top, _) = top else {
            bail!("not a torrent: its metainfo is no dictionary");
        };
        let Some(Value::Dict(info, raw)) = get(&top, "info") else {
            bail!("not a torrent: it has no `info`");
        };
        let version = match get(info, "meta version") {
            Some(Value::Int(v)) => *v,
            _ => 1,
        };
        let v1_files = get(info, "pieces").is_some();
        ensure!(
            version == 1 || version == 2,
            "a torrent of BitTorrent version {version}, which muman does not read"
        );
        let v2 = (version == 2).then(|| hex(&Sha256::digest(raw)));
        let v1 = v1_files.then(|| hex(&Sha1::digest(raw)));
        ensure!(
            v1.is_some() || v2.is_some(),
            "not a torrent: its `info` has neither v1 pieces nor a v2 file tree"
        );
        let name = text(info, "name").context("a torrent with no `name`")?;
        let name = part(&name).with_context(|| format!("the torrent's name, {name:?}"))?;
        let roots = match get(info, "file tree") {
            Some(Value::Dict(tree, _)) if version == 2 => {
                let mut roots = Vec::new();
                tree_files(tree, &mut Vec::new(), &mut roots, 0)?;
                roots
            }
            _ => Vec::new(),
        };
        let piece_length = match get(info, "piece length") {
            Some(Value::Int(n)) => u64::try_from(*n)
                .ok()
                .filter(|n| (1..=MAX_PIECE).contains(n)),
            _ => None,
        }
        .context("not a torrent: its `piece length` is missing or out of bounds")?;
        ensure!(
            v2.is_none() || (piece_length.is_power_of_two() && piece_length >= V2_MIN_PIECE),
            "not a torrent: a v2 piece length of {piece_length}, no power of two from 16 KiB"
        );
        let (files, total) = if v1.is_some() {
            v1_list(info, &name, &roots)?
        } else {
            v2_list(roots, piece_length)?
        };
        let pieces = match get(info, "pieces") {
            Some(Value::Bytes(b)) => {
                let (pieces, rest) = b.as_chunks::<20>();
                let want = total.div_ceil(piece_length);
                ensure!(
                    rest.is_empty() && u64::try_from(pieces.len()).ok() == Some(want),
                    "not a torrent: it holds {} bytes of piece hashes for {want} pieces",
                    b.len()
                );
                pieces.to_vec()
            }
            Some(_) => bail!("not a torrent: its `pieces` is no string"),
            None => Vec::new(),
        };
        let private = matches!(get(info, "private"), Some(Value::Int(1)));
        let mut trackers = Vec::new();
        if let Some(Value::List(tiers)) = get(&top, "announce-list") {
            for tier in tiers {
                if let Value::List(urls) = tier {
                    trackers.extend(urls.iter().filter_map(Value::text));
                }
            }
        }
        if let Some(url) = get(&top, "announce").and_then(Value::text) {
            trackers.push(url);
        }
        dedup(&mut trackers);
        Ok(Self {
            v1,
            v2,
            name,
            files,
            private,
            trackers,
            piece_length,
            pieces,
            total,
        })
    }

    /// The hash its keys name it by: v1 where it has one.
    #[must_use]
    pub fn hash(&self) -> &str {
        self.v1
            .as_deref()
            .or(self.v2.as_deref())
            .unwrap_or_default()
    }

    /// The key of one of its files.
    #[must_use]
    pub fn key(&self, file: &File) -> SourceKey {
        SourceKey::Torrent(TorrentKey::new(self.hash(), &file.path))
    }

    /// Whether a fetcher of v1 swarms can fetch it.
    #[must_use]
    pub fn fetchable(&self) -> bool {
        self.v1.is_some()
    }
}

/// The files of `meta` muman fetches: its songs, and the lyrics and
/// pictures a song file brings, named as it is or its folder's cover;
/// of two kept at one place, the first alone ([`shadowed`]).
#[must_use]
pub fn selected(meta: &Metainfo) -> Vec<&File> {
    let mut places = std::collections::HashSet::new();
    wanted(meta)
        .into_iter()
        .filter(|f| places.insert(place_of(&f.path)))
        .collect()
}

/// Each file of `meta` muman would fetch but keeps no place for, as
/// another listed before it is kept at its path on some system, with
/// that other file's path.
#[must_use]
pub fn shadowed(meta: &Metainfo) -> Vec<(&Path, &Path)> {
    let mut places = std::collections::HashMap::new();
    let mut lost = Vec::new();
    for f in wanted(meta) {
        match places.entry(place_of(&f.path)) {
            std::collections::hash_map::Entry::Occupied(first) => {
                lost.push((f.path.as_path(), *first.get()));
            }
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(f.path.as_path());
            }
        }
    }
    lost
}

/// Where a file is kept as a system blind to case and composition sees
/// it: two lookalikes, or names apart only so, are one place there.
fn place_of(path: &Path) -> String {
    crate::relpath::folded(&stored(&crate::relpath::normalized(path)))
}

fn wanted(meta: &Metainfo) -> Vec<&File> {
    use crate::store::{FOLDER_COVERS, Kind, kind_of, named_as};
    let songs: Vec<&Path> = meta
        .files
        .iter()
        .filter(|f| kind_of(&f.path) == Some(Kind::Media))
        .map(|f| f.path.as_path())
        .collect();
    meta.files
        .iter()
        .filter(|f| match kind_of(&f.path) {
            Some(Kind::Media) => true,
            Some(Kind::Lyrics) => songs.iter().any(|s| named_as(&f.path, s)),
            Some(Kind::Image) => songs.iter().any(|s| {
                named_as(&f.path, s)
                    || (f.path.parent() == s.parent()
                        && f.path
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| {
                                FOLDER_COVERS.contains(&n.to_ascii_lowercase().as_str())
                            }))
            }),
            _ => false,
        })
        .collect()
}

/// Whether every v1 piece `file` of `meta` spans checks, its bytes read
/// from the files of `meta` at the paths `at` gives them: none missing or
/// short, and the hash of each the one the metainfo holds. Padding
/// between files reads as the zeros it holds.
#[must_use]
#[deny(clippy::arithmetic_side_effects)]
pub fn checks(meta: &Metainfo, file: &File, at: impl Fn(&File) -> PathBuf) -> bool {
    let piece = meta.piece_length;
    let Some((first, last)) = spanned(file, piece) else {
        return piece > 0;
    };
    (first..=last).all(|n| {
        let Some(want) = usize::try_from(n).ok().and_then(|n| meta.pieces.get(n)) else {
            return false;
        };
        let Some(start) = n.checked_mul(piece) else {
            return false;
        };
        let end = start.saturating_add(piece).min(meta.total);
        piece_hash(meta, start, end, &at).is_some_and(|h| h.as_slice() == want)
    })
}

/// The first and last piece `file` spans; none for an empty file, or a
/// piece of no length.
#[deny(clippy::arithmetic_side_effects)]
fn spanned(file: &File, piece: u64) -> Option<(u64, u64)> {
    let last_byte = file.offset.checked_add(file.length.checked_sub(1)?)?;
    Some((
        file.offset.checked_div(piece)?,
        last_byte.checked_div(piece)?,
    ))
}

/// The SHA-1 of the bytes from `start` to `end` of `meta`'s files, read
/// a chunk at a time; none where one of them is missing or short.
#[deny(clippy::arithmetic_side_effects)]
fn piece_hash(
    meta: &Metainfo,
    start: u64,
    end: u64,
    at: &impl Fn(&File) -> PathBuf,
) -> Option<sha1::digest::Output<Sha1>> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let size = end.checked_sub(start)?;
    let mut chunk = vec![0; usize::try_from(size).map_or(CHECK_CHUNK, |s| s.min(CHECK_CHUNK))];
    let mut hash = Sha1::new();
    let mut done = start;
    for f in meta.files.iter().filter(|f| f.length > 0) {
        let (from, to) = (
            start.max(f.offset),
            end.min(f.offset.checked_add(f.length)?),
        );
        if from >= to {
            continue;
        }
        zeros(&mut hash, &mut chunk, from.saturating_sub(done));
        let mut opened = std::fs::File::open(at(f)).ok()?;
        opened
            .seek(SeekFrom::Start(from.checked_sub(f.offset)?))
            .ok()?;
        let mut left = to.checked_sub(from)?;
        while left > 0 {
            let take = usize::try_from(left).map_or(chunk.len(), |n| n.min(chunk.len()));
            opened.read_exact(&mut chunk[..take]).ok()?;
            hash.update(&chunk[..take]);
            left = left.saturating_sub(u64::try_from(take).ok()?);
        }
        done = to;
    }
    zeros(&mut hash, &mut chunk, end.saturating_sub(done));
    Some(hash.finalize())
}

/// `n` zeros, as padding holds, hashed into `hash` through `chunk`.
#[deny(clippy::arithmetic_side_effects)]
fn zeros(hash: &mut Sha1, chunk: &mut [u8], mut n: u64) {
    chunk.fill(0);
    while n > 0 && !chunk.is_empty() {
        let take = usize::try_from(n).map_or(chunk.len(), |n| n.min(chunk.len()));
        hash.update(&chunk[..take]);
        n = n.saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
    }
}

/// The files of `meta` besides `files` that share a piece with one of
/// them: a piece is checked whole, so taking up a file already on disk
/// takes these up too.
#[must_use]
#[deny(clippy::arithmetic_side_effects)]
pub fn sharing<'m>(meta: &'m Metainfo, files: &[&File]) -> Vec<&'m File> {
    let piece = meta.piece_length.max(1);
    let pieces = |f: &File| {
        let last_byte = f.offset.saturating_add(f.length.saturating_sub(1));
        let piece_of = |byte: u64| byte.checked_div(piece).unwrap_or_default();
        piece_of(f.offset)..=piece_of(last_byte)
    };
    meta.files
        .iter()
        .filter(|f| f.length > 0 && !files.iter().any(|s| s.index == f.index))
        .filter(|f| {
            let mine = pieces(f);
            files.iter().any(|s| {
                let theirs = pieces(s);
                mine.start() <= theirs.end() && theirs.start() <= mine.end()
            })
        })
        .collect()
}

/// The files of a v1 or hybrid torrent's `files` or `length`, a hybrid's
/// with the `pieces root` its file tree gives each, and the bytes they
/// run to. Both name a file by its path below the torrent's folder, or a
/// torrent of one file by its name.
#[deny(clippy::arithmetic_side_effects)]
fn v1_list(
    info: &[(&[u8], Value<'_>)],
    name: &str,
    roots: &[(PathBuf, u64, Option<String>)],
) -> Result<(Vec<File>, u64)> {
    let root_of = |path: &Path| {
        roots
            .iter()
            .find(|(p, _, _)| p == path)
            .and_then(|(_, _, r)| r.clone())
    };
    let Some(Value::List(list)) = get(info, "files") else {
        let Some(Value::Int(length)) = get(info, "length") else {
            bail!("not a torrent: its `info` has neither `files` nor `length`");
        };
        let length = end_of(
            0,
            u64::try_from(*length).context("a file of negative length")?,
        )?;
        let file = File {
            root: root_of(Path::new(name)),
            path: PathBuf::from(name),
            length,
            index: 1,
            offset: 0,
        };
        return Ok((vec![file], length));
    };
    let mut files = Vec::new();
    let mut offset = 0;
    for (i, entry) in list.iter().enumerate() {
        let Value::Dict(entry, _) = entry else {
            bail!("not a torrent: a file entry is no dictionary");
        };
        let padding = text(entry, "attr").is_some_and(|a| a.contains('p'));
        let Some(Value::List(parts)) = get(entry, "path.utf-8").or_else(|| get(entry, "path"))
        else {
            bail!("not a torrent: a file with no `path`");
        };
        let Some(Value::Int(length)) = get(entry, "length") else {
            bail!("not a torrent: a file with no `length`");
        };
        let length = u64::try_from(*length).context("a file of negative length")?;
        let start = offset;
        offset = end_of(offset, length)?;
        if padding {
            continue;
        }
        let mut path = PathBuf::new();
        for p in parts {
            let p = p
                .text()
                .context("not a torrent: a path part is no string")?;
            path.push(part(&p).with_context(|| format!("a file's path in the torrent, {p:?}"))?);
        }
        ensure!(
            path.components().next().is_some(),
            "not a torrent: a file with an empty path"
        );
        files.push(File {
            root: root_of(&path),
            path,
            length,
            index: i.saturating_add(1),
            offset: start,
        });
    }
    Ok((files, offset))
}

/// The files of a v2 torrent's file tree, each starting a piece, and the
/// bytes they run to.
#[deny(clippy::arithmetic_side_effects)]
fn v2_list(roots: Vec<(PathBuf, u64, Option<String>)>, piece: u64) -> Result<(Vec<File>, u64)> {
    let mut files = Vec::new();
    let (mut offset, mut total) = (0, 0);
    for (i, (path, length, root)) in roots.into_iter().enumerate() {
        total = end_of(offset, length)?;
        files.push(File {
            path,
            length,
            root,
            index: i.saturating_add(1),
            offset,
        });
        let padded = length.div_ceil(piece).checked_mul(piece);
        offset = offset.saturating_add(padded.unwrap_or(u64::MAX));
    }
    Ok((files, total))
}

/// Where a file `length` long starting at `offset` ends, refused past
/// [`MAX_TOTAL`].
fn end_of(offset: u64, length: u64) -> Result<u64> {
    offset
        .checked_add(length)
        .filter(|end| *end <= MAX_TOTAL)
        .context("not a torrent: its files run past 256 TiB")
}

/// The files of a v2 file tree in its order, by path from the tree's
/// top, each with its length and `pieces root`.
fn tree_files(
    tree: &[(&[u8], Value<'_>)],
    at: &mut Vec<String>,
    out: &mut Vec<(PathBuf, u64, Option<String>)>,
    depth: usize,
) -> Result<()> {
    ensure!(
        depth < MAX_DEPTH,
        "not a torrent: its file tree nests too deep"
    );
    for (name, value) in tree {
        let Value::Dict(inner, _) = value else {
            bail!("not a torrent: its file tree holds what is no dictionary");
        };
        if name.is_empty() {
            let Some(Value::Int(length)) = get(inner, "length") else {
                bail!("not a torrent: a file in its tree with no `length`");
            };
            let root = match get(inner, "pieces root") {
                Some(Value::Bytes(b)) if b.len() == 32 => Some(hex(b)),
                _ => None,
            };
            let path: PathBuf = at.iter().collect();
            out.push((path, u64::try_from(*length)?, root));
            continue;
        }
        let name = String::from_utf8_lossy(name);
        let name = part(&name).with_context(|| format!("a path in the torrent, {name:?}"))?;
        at.push(name);
        tree_files(inner, at, out, depth + 1)?;
        at.pop();
    }
    Ok(())
}

/// One part of a path a torrent names, in NFC, refused where it is no
/// plain name of a file or folder on every system, as the module says.
fn part(text: &str) -> Result<String> {
    let nfc: String = text.nfc().collect();
    ensure!(
        !nfc.trim_end_matches(['.', ' ']).is_empty() && !nfc.contains(['/', '\\']),
        "is no plain name of a file"
    );
    ensure!(
        !nfc.chars().any(char::is_control),
        "holds a control character"
    );
    let mut start = nfc.chars();
    ensure!(
        !(start.next().is_some_and(|c| c.is_ascii_alphabetic()) && start.next() == Some(':')),
        "starts as a Windows drive does"
    );
    Ok(nfc)
}

fn dedup(list: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    list.retain(|t| seen.insert(t.clone()));
}

/// A magnet link's torrent: its hashes, name and trackers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Magnet {
    pub v1: Option<String>,
    pub v2: Option<String>,
    pub name: Option<String>,
    pub trackers: Vec<String>,
}

impl Magnet {
    /// Read a `magnet:?xt=urn:btih:…` link: a v1 hash in hex or base32, a
    /// v2 one as `urn:btmh:1220…`, or both for a hybrid.
    pub fn parse(link: &str) -> Result<Self> {
        let query = link
            .strip_prefix("magnet:?")
            .with_context(|| format!("`{link}` is no magnet link"))?;
        let mut magnet = Self {
            v1: None,
            v2: None,
            name: None,
            trackers: Vec::new(),
        };
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = percent_encoding::percent_decode_str(value)
                .decode_utf8_lossy()
                .into_owned();
            match key {
                "xt" => {
                    if let Some(h) = strip_prefix_ignore_case(&value, "urn:btih:") {
                        magnet.v1 = Some(v1_hash(h).with_context(|| {
                            format!("`{link}`: `{h}` is no v1 hash, in hex or base32")
                        })?);
                    } else if let Some(h) = strip_prefix_ignore_case(&value, "urn:btmh:") {
                        let h = h.to_ascii_lowercase();
                        let hash = h
                            .strip_prefix("1220")
                            .filter(|h| is_hash(h) && h.len() == V2_HEX);
                        magnet.v2 = Some(
                            hash.with_context(|| format!("`{link}`: `{h}` is no v2 hash"))?
                                .to_string(),
                        );
                    }
                }
                "dn" if !value.is_empty() => magnet.name = Some(value),
                "tr" if !value.is_empty() => magnet.trackers.push(value),
                _ => {}
            }
        }
        ensure!(
            magnet.v1.is_some() || magnet.v2.is_some(),
            "`{link}` names no torrent: it has no `xt=urn:btih:` or `xt=urn:btmh:`"
        );
        dedup(&mut magnet.trackers);
        Ok(magnet)
    }

    /// The hash its keys name it by: v1 where it has one.
    #[must_use]
    pub fn hash(&self) -> &str {
        self.v1
            .as_deref()
            .or(self.v2.as_deref())
            .unwrap_or_default()
    }

    /// The link again, to hand a fetcher: its hashes, name and trackers.
    #[must_use]
    pub fn link(&self) -> String {
        const KEPT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
            .remove(b'-')
            .remove(b'.')
            .remove(b'_')
            .remove(b'~');
        let encode = |s: &str| percent_encoding::utf8_percent_encode(s, KEPT).to_string();
        let mut parts = Vec::new();
        parts.extend(self.v1.iter().map(|h| format!("xt=urn:btih:{h}")));
        parts.extend(self.v2.iter().map(|h| format!("xt=urn:btmh:1220{h}")));
        parts.extend(self.name.iter().map(|n| format!("dn={}", encode(n))));
        parts.extend(self.trackers.iter().map(|t| format!("tr={}", encode(t))));
        format!("magnet:?{}", parts.join("&"))
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// A v1 hash as a magnet link writes it, 40 hex digits or 32 base32
/// letters, in lower-case hex.
fn v1_hash(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if lower.len() == V1_HEX && is_hash(&lower) {
        return Some(lower);
    }
    if text.len() != 32 {
        return None;
    }
    let mut bits: u64 = 0;
    let mut held = 0;
    let mut bytes = Vec::with_capacity(20);
    for c in text.bytes() {
        let v = match c.to_ascii_uppercase() {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | u64::from(v);
        held += 5;
        if held >= 8 {
            held -= 8;
            bytes.push((bits >> held) as u8);
        }
    }
    Some(hex(&bytes))
}

/// Where a torrent's file is kept below its torrent's folder: each part
/// of its path in the key made safe on every system, as the module says.
#[must_use]
pub fn stored(path: &Path) -> PathBuf {
    path.iter()
        .map(|p| stored_part(&p.to_string_lossy()))
        .collect()
}

fn stored_part(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_control() || crate::naming::reorders(c) {
                '_'
            } else {
                crate::naming::lookalike(c).unwrap_or(c)
            }
        })
        .collect();
    if out.starts_with('.') {
        out.replace_range(..1, "．");
    }
    let kept = out.trim_end_matches(['.', ' ']).len();
    let tail: String = out[kept..]
        .chars()
        .map(|c| if c == '.' { '．' } else { '\u{3000}' })
        .collect();
    out.truncate(kept);
    out.push_str(&tail);
    if crate::naming::reserved(&out) {
        let at = out.find('.').unwrap_or(out.len());
        out.insert(at, '_');
    }
    shortened(out, name)
}

/// `out`, the part `name` spelled for every system, cut to its first
/// [`crate::store::PART_BYTES`] bytes or so with a hash of `name` after,
/// its extension kept, where it runs longer: a torrent's file is known by
/// its metainfo, not by the name it is kept at, and lookalikes take more
/// bytes than what they stand for.
fn shortened(out: String, name: &str) -> String {
    const HASH: usize = 8;
    if out.len() <= crate::store::PART_BYTES {
        return out;
    }
    let ext = Path::new(&out)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 10 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let room = crate::store::PART_BYTES - HASH - 1 - ext.len();
    let mut cut = room.min(out.len());
    while !out.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}~{}{ext}",
        &out[..cut],
        &hex(&Sha1::digest(name.as_bytes()))[..HASH]
    )
}

/// A bencoded value; a dictionary keeps the bytes it was read from.
#[derive(Debug)]
enum Value<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>, &'a [u8]),
}

impl Value<'_> {
    fn text(&self) -> Option<String> {
        match self {
            Self::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
}

fn get<'v, 'a>(dict: &'v [(&'a [u8], Value<'a>)], key: &str) -> Option<&'v Value<'a>> {
    dict.iter()
        .find(|(k, _)| *k == key.as_bytes())
        .map(|(_, v)| v)
}

/// A string a dictionary holds, by its `<key>.utf-8` first, as older
/// clients wrote names in another encoding beside it.
fn text(dict: &[(&[u8], Value<'_>)], key: &str) -> Option<String> {
    get(dict, &format!("{key}.utf-8"))
        .or_else(|| get(dict, key))
        .and_then(Value::text)
}

/// One bencoded value at the start of `bytes`, and what follows it.
fn parse(bytes: &[u8], depth: usize) -> Result<(Value<'_>, &[u8])> {
    ensure!(depth < MAX_DEPTH, "nested too deep");
    match bytes.first() {
        Some(b'i') => {
            let end = bytes
                .iter()
                .position(|&b| b == b'e')
                .context("an unended integer")?;
            let n = std::str::from_utf8(&bytes[1..end])?.parse()?;
            Ok((Value::Int(n), &bytes[end + 1..]))
        }
        Some(b'l') => {
            let mut rest = &bytes[1..];
            let mut items = Vec::new();
            while rest.first() != Some(&b'e') {
                let (item, after) = parse(rest, depth + 1)?;
                items.push(item);
                rest = after;
            }
            Ok((Value::List(items), &rest[1..]))
        }
        Some(b'd') => {
            let mut rest = &bytes[1..];
            let mut entries = Vec::new();
            while rest.first() != Some(&b'e') {
                let (Value::Bytes(key), after) = parse(rest, depth + 1)? else {
                    bail!("a dictionary key that is no string");
                };
                let (value, after) = parse(after, depth + 1)?;
                entries.push((key, value));
                rest = after;
            }
            let raw = &bytes[..=bytes.len() - rest.len()];
            Ok((Value::Dict(entries, raw), &rest[1..]))
        }
        Some(b'0'..=b'9') => {
            let colon = bytes
                .iter()
                .position(|&b| b == b':')
                .context("an unended length")?;
            let len: usize = std::str::from_utf8(&bytes[..colon])?.parse()?;
            let start = colon + 1;
            let end = start
                .checked_add(len)
                .filter(|&e| e <= bytes.len())
                .context("a cut string")?;
            Ok((Value::Bytes(&bytes[start..end]), &bytes[end..]))
        }
        _ => bail!("no bencoded value"),
    }
}

#[cfg(test)]
mod tests;
