//! muman's two yt-dlp postprocessors, shipped inside the binary and
//! written out the first time a run fetches.
//!
//! - `OriginalSubs` drops machine-translated captions before download:
//!   they run to hundreds per video and trip YouTube's rate limit.
//! - `AlbumArt` makes a YouTube Music track's square album art its
//!   thumbnail, in place of a 16:9 video frame.
//!
//! They go to `<cache>/yt-dlp-plugins/<hash>/muman/yt_dlp_plugins/
//! postprocessor/`, named by a hash of their text, so two runs at once or
//! two muman versions never write over each other's, and an unchanged
//! copy is never written again. yt-dlp is handed `<hash>`: it takes each
//! folder inside a plugin directory as a package holding
//! `yt_dlp_plugins`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const FILES: [(&str, &str); 2] = [
    (
        "original_subs.py",
        include_str!("../assets/yt-dlp/original_subs.py"),
    ),
    (
        "album_art.py",
        include_str!("../assets/yt-dlp/album_art.py"),
    ),
];

/// The plugin folder under `cache`, written if missing, to hand yt-dlp
/// as `--plugin-dirs`.
pub fn folder(cache: &Path) -> Result<PathBuf> {
    let root = cache
        .join("yt-dlp-plugins")
        .join(format!("{:016x}", hash()));
    let postprocessors = root
        .join("muman")
        .join("yt_dlp_plugins")
        .join("postprocessor");
    for (name, text) in FILES {
        let path = postprocessors.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
            continue;
        }
        std::fs::create_dir_all(&postprocessors)
            .with_context(|| format!("creating {}", postprocessors.display()))?;
        crate::atomic::write(&postprocessors, name, text.as_bytes())?;
    }
    Ok(root)
}

/// FNV-1a over every file's name and text: stable across builds and
/// platforms, unlike the standard library's hasher.
fn hash() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, text) in FILES {
        for b in name.bytes().chain(text.bytes()) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugins_are_written_once_where_yt_dlp_looks() {
        let dir = tempfile::tempdir().unwrap();
        let root = folder(dir.path()).unwrap();
        let file = root
            .join("muman")
            .join("yt_dlp_plugins")
            .join("postprocessor")
            .join("album_art.py");
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("AlbumArtPP")
        );
        assert_eq!(folder(dir.path()).unwrap(), root);
    }
}
