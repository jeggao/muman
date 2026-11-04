//! Digests of bytes and of files: the first 128 bits of a SHA-256, in
//! hex. Facts name a source's parts by them, the state its trusted
//! settings, and the history the files it keeps.

use std::io::Read as _;
use std::path::Path;

use sha2::{Digest as _, Sha256};

/// The digest of `bytes`: the first 128 bits of their SHA-256 in hex,
/// which no two contents share by chance and a person can still read.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes)[..16])
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// [`digest`] of a file's bytes, read a block at a time.
pub(crate) fn digest_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut block = [0; 8 * 1024];
    loop {
        match file.read(&mut block) {
            Ok(0) => break,
            Ok(n) => hasher.update(&block[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(hex(&hasher.finalize()[..16]))
}
