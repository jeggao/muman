//! Files and folders given to `add`: how far a link inside one is
//! followed, and the one way a file is read out of one.
//!
//! A path named on the command line is taken as it is, a link to a file
//! or a folder followed: the user named it. Inside a folder given, a
//! link, to a file or to a folder, is followed only while what it leads
//! to is inside that folder too, the two compared as their real paths,
//! every link and `..` resolved (`dunce::canonicalize`, which resolves
//! Windows' junctions as it does its symbolic links, and keeps a path in
//! the plain form the rest of muman compares). A folder downloaded or
//! unpacked from elsewhere may hold `Song.lrc -> ../../.ssh/id_ed25519`
//! or an `extras` linked to the home folder, and what they lead to,
//! copied into the sources, would be in the library and in every export
//! of it. A link leading out is passed over and named
//! ([`say_outside`]); one leading nowhere is passed over as a file
//! deleted during the walk is, and one leading back to a folder it is
//! inside is walked no further, since it would hold the same files again
//! without end. A file given brings its sidecars from its own folder,
//! which bounds them as a folder given bounds its files, and the folder
//! `add --from` names bounds what a torrent is taken up from
//! ([`crate::aria2::take_up`]).
//!
//! Only regular files are read: a pipe or a device would hold a copy up
//! forever or never end it, and a socket holds nothing.
//!
//! Every walk of a folder given and every copy out of one goes through
//! [`Folder`], as `tests/architecture.rs` keeps it. muman's own folders,
//! the sources and the library, are walked by `store::files_below`,
//! which follows a link wherever it leads: one there was made by the
//! user, as a folder linked into the manual folder is.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// A file or folder given to `add`, the bound a link inside it is
/// followed within.
#[derive(Debug, Clone)]
pub struct Folder {
    path: PathBuf,
    /// Its real path; `None` when it cannot be read, so nothing is
    /// inside it.
    real: Option<PathBuf>,
}

/// What a walk of a [`Folder`] found: its regular files, and the links
/// inside it leading out of it, passed over.
#[derive(Debug, Default, Clone)]
pub struct Walked {
    pub files: Vec<PathBuf>,
    pub outside: Vec<PathBuf>,
}

impl Folder {
    /// The file or folder at `path` as given, a link to it followed; an
    /// empty path is the working folder.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        let at = if path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            path
        };
        Self {
            path: path.to_path_buf(),
            real: dunce::canonicalize(at).ok(),
        }
    }

    /// The path as given.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether `path` leads to a place inside the folder, through
    /// whatever links it passes.
    fn leads_inside(&self, path: &Path) -> bool {
        self.real
            .as_ref()
            .is_some_and(|root| dunce::canonicalize(path).is_ok_and(|real| real.starts_with(root)))
    }

    /// Whether `path` leads somewhere outside the folder, as a link out
    /// of it does; a path leading nowhere leads nowhere outside.
    #[must_use]
    pub fn leads_out(&self, path: &Path) -> bool {
        dunce::canonicalize(path).is_ok() && !self.leads_inside(path)
    }

    /// Whether `path` is a regular file inside the folder.
    #[must_use]
    pub fn holds(&self, path: &Path) -> bool {
        self.leads_inside(path) && std::fs::metadata(path).is_ok_and(|m| m.is_file())
    }

    /// The regular files below the folder, at most `depth` folders down,
    /// in path order, none hidden or a download unfinished
    /// (`store::visible`), with the links leading out of it.
    ///
    /// # Errors
    ///
    /// A folder below that cannot be read.
    pub fn walk(&self, depth: usize) -> Result<Walked> {
        let root = if self.path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.path
        };
        let mut walked = Walked::default();
        let entries = walkdir::WalkDir::new(root)
            .follow_links(true)
            .min_depth(1)
            .max_depth(depth)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                if !crate::store::visible(&e.file_name().to_string_lossy()) {
                    return false;
                }
                let inside = !e.path_is_symlink() || self.leads_inside(e.path());
                if !inside {
                    walked.outside.push(e.path().to_path_buf());
                }
                inside
            });
        let mut files = Vec::new();
        for entry in entries {
            match entry {
                Ok(e) if e.file_type().is_file() => files.push(e.into_path()),
                Ok(_) => {}
                Err(e) if e.loop_ancestor().is_some() || e.path().is_some_and(is_link) => {}
                Err(e)
                    if e.io_error()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) => {}
                Err(e) => return Err(e.into()),
            }
        }
        walked.files = files;
        if root != self.path.as_path() {
            for path in walked.files.iter_mut().chain(&mut walked.outside) {
                if let Ok(rel) = path.strip_prefix(root) {
                    *path = rel.to_path_buf();
                }
            }
        }
        walked.files.sort();
        walked.outside.sort();
        Ok(walked)
    }

    /// Copy `from`, a regular file inside the folder, to `to`, replacing
    /// it, read at its real path so no link is followed out on the way.
    ///
    /// # Errors
    ///
    /// `from` leading out of the folder or being no regular file, or the
    /// copy failing.
    pub fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let real =
            dunce::canonicalize(from).with_context(|| format!("reading {}", from.display()))?;
        if !self
            .real
            .as_ref()
            .is_some_and(|root| real.starts_with(root))
        {
            bail!("{} leads out of {}", from.display(), self.path.display());
        }
        if !std::fs::metadata(&real).is_ok_and(|m| m.is_file()) {
            bail!("{} is no regular file", from.display());
        }
        std::fs::copy(&real, to)
            .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        Ok(())
    }
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Warn of the links in `dir`, a folder given, that lead out of it and
/// were passed over, each named below `dir`.
///
/// # Errors
///
/// Writing to `out`.
pub fn say_outside<W: Write>(dir: &Path, outside: &[PathBuf], out: &mut W) -> Result<()> {
    if outside.is_empty() {
        return Ok(());
    }
    let named: Vec<String> = outside
        .iter()
        .map(|f| f.strip_prefix(dir).unwrap_or(f).display().to_string())
        .collect();
    let shown = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    crate::ui::warning(
        out,
        &format!(
            "Left out of {}, as links leading out of it: {}",
            shown.display(),
            named.join(", ")
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::link;

    fn touch(root: &Path, rel: &str) -> PathBuf {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rel.as_bytes()).unwrap();
        path
    }

    fn names(root: &Path, paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| crate::relpath::show(p.strip_prefix(root).unwrap()))
            .collect()
    }

    #[test]
    fn a_link_inside_is_followed_and_one_leading_out_named_and_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Album");
        touch(&album, "Paper Comets.flac");
        touch(&album, "art/front.jpg");
        touch(dir.path(), "secret/notes.txt");
        touch(dir.path(), "elsewhere/Glass Orchards.flac");
        let linked = link(
            &album.join("art").join("front.jpg"),
            &album.join("cover.jpg"),
        ) && link(&album.join("art"), &album.join("scans"))
            && link(
                &dir.path().join("secret").join("notes.txt"),
                &album.join("Paper Comets.lrc"),
            )
            && link(&dir.path().join("elsewhere"), &album.join("extras"));
        if !linked {
            return;
        }
        let walked = Folder::new(&album).walk(usize::MAX).unwrap();
        assert_eq!(
            names(&album, &walked.files),
            [
                "Paper Comets.flac",
                "art/front.jpg",
                "cover.jpg",
                "scans/front.jpg"
            ]
        );
        assert_eq!(
            names(&album, &walked.outside),
            ["Paper Comets.lrc", "extras"]
        );

        let mut out = Vec::new();
        say_outside(&album, &walked.outside, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("Paper Comets.lrc") && text.contains("extras"),
            "{text}"
        );
    }

    #[test]
    fn a_folder_given_as_a_link_is_followed_and_bounds_what_is_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("rips").join("Album");
        touch(&real, "Paper Comets.flac");
        touch(dir.path(), "rips/Other/Glass Orchards.flac");
        let given = dir.path().join("given");
        let linked = link(&real, &given)
            && link(
                &real.join("Paper Comets.flac"),
                &real.join("Paper Comets (copy).flac"),
            )
            && link(&dir.path().join("rips").join("Other"), &real.join("other"));
        if !linked {
            return;
        }
        let walked = Folder::new(&given).walk(usize::MAX).unwrap();
        assert_eq!(
            names(&given, &walked.files),
            ["Paper Comets (copy).flac", "Paper Comets.flac"]
        );
        assert_eq!(names(&given, &walked.outside), ["other"]);
    }

    #[test]
    fn a_link_leading_nowhere_or_round_is_passed_over_in_silence() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Album");
        touch(&album, "Paper Comets.flac");
        let linked = link(&album.join("gone.lrc"), &album.join("Paper Comets.lrc"))
            && link(&album, &album.join("up"))
            && link(&album.join("round"), &album.join("round"));
        if !linked {
            return;
        }
        let walked = Folder::new(&album).walk(usize::MAX).unwrap();
        assert_eq!(names(&album, &walked.files), ["Paper Comets.flac"]);
        assert!(walked.outside.is_empty(), "{:?}", walked.outside);
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_is_no_file_to_copy() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Album");
        touch(&album, "Paper Comets.flac");
        let socket = album.join("Paper Comets.lrc");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let folder = Folder::new(&album);
        let walked = folder.walk(usize::MAX).unwrap();
        assert_eq!(names(&album, &walked.files), ["Paper Comets.flac"]);
        assert!(!folder.holds(&socket));
        let e = folder
            .copy(&socket, &dir.path().join("copy.lrc"))
            .unwrap_err();
        assert!(format!("{e:#}").contains("no regular file"), "{e:#}");
    }

    #[test]
    fn a_copy_reads_no_link_out_of_its_folder_but_follows_the_path_given() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Album");
        touch(&album, "Paper Comets.flac");
        let secret = touch(dir.path(), "secret/notes.txt");
        let lrc = album.join("Paper Comets.lrc");
        if !link(&secret, &lrc) {
            return;
        }
        let to = dir.path().join("copy.lrc");
        let folder = Folder::new(&album);
        assert!(folder.leads_out(&lrc));
        assert!(!folder.holds(&lrc));
        let e = folder.copy(&lrc, &to).unwrap_err();
        assert!(format!("{e:#}").contains("leads out of"), "{e:#}");
        assert!(!to.exists());

        Folder::new(&lrc).copy(&lrc, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"secret/notes.txt");
    }
}
