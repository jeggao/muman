//! muman's yt-dlp plugins, shipped inside the binary and written out the
//! first time a run fetches.
//!
//! - `OriginalSubs`, a postprocessor, drops machine-translated captions
//!   before download: they run to hundreds per video and trip YouTube's
//!   rate limit.
//! - `AlbumArt`, a postprocessor, makes a YouTube Music track's square
//!   album art its thumbnail, in place of a 16:9 video frame.
//! - `archive_item`, an override of yt-dlp's archive.org extractor, gives
//!   each file of an item of several the item's creator, dates, licence
//!   and description where the file has none of its own. yt-dlp gives
//!   them to a file fetched by its own address and to the one file of an
//!   item, but not to the files of an item of several, so a file's tags
//!   would depend on how it was reached, and change when a sync fetched
//!   it again by its own address. The extractor's key stays `ArchiveOrg`,
//!   so keys do not change.
//!
//! They go to `<cache>/yt-dlp-plugins/<hash>/muman/yt_dlp_plugins/`, each
//! in the folder of its kind, named by a hash of their text, so two runs
//! at once or two muman versions never write over each other's, and an
//! unchanged copy is never written again. yt-dlp is handed `<hash>`: it
//! takes each folder inside a plugin directory as a package holding
//! `yt_dlp_plugins`, and loads its extractors without being asked.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Each plugin: the folder of its kind, its name and its text.
const FILES: [(&str, &str, &str); 3] = [
    (
        "postprocessor",
        "original_subs.py",
        include_str!("../assets/yt-dlp/original_subs.py"),
    ),
    (
        "postprocessor",
        "album_art.py",
        include_str!("../assets/yt-dlp/album_art.py"),
    ),
    (
        "extractor",
        "archive_item.py",
        include_str!("../assets/yt-dlp/archive_item.py"),
    ),
];

/// The plugin folder under `cache`, written if missing, to hand yt-dlp
/// as `--plugin-dirs`.
pub fn folder(cache: &Path) -> Result<PathBuf> {
    let root = cache
        .join("yt-dlp-plugins")
        .join(format!("{:016x}", hash()));
    let plugins = root.join("muman").join("yt_dlp_plugins");
    for (kind, name, text) in FILES {
        let folder = plugins.join(kind);
        if std::fs::read_to_string(folder.join(name)).is_ok_and(|t| t == text) {
            continue;
        }
        std::fs::create_dir_all(&folder)
            .with_context(|| format!("creating {}", folder.display()))?;
        crate::atomic::write(&folder, &crate::atomic::Name::new(name)?, text.as_bytes())?;
    }
    Ok(root)
}

/// FNV-1a over every file's name and text: stable across builds and
/// platforms, unlike the standard library's hasher.
fn hash() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (kind, name, text) in FILES {
        for b in kind.bytes().chain(name.bytes()).chain(text.bytes()) {
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
        let extractor = root
            .join("muman")
            .join("yt_dlp_plugins")
            .join("extractor")
            .join("archive_item.py");
        assert!(
            std::fs::read_to_string(&extractor)
                .unwrap()
                .contains("plugin_name=\"muman\"")
        );
        assert_eq!(folder(dir.path()).unwrap(), root);
    }
}
