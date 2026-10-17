//! The programs muman runs, ffmpeg, ffprobe and yt-dlp: where each is
//! found, how its version is read, and the oldest release of each that
//! takes every option muman passes.
//!
//! A tool is looked for, in order:
//!
//! 1. In its variable (`MUMAN_FFMPEG`, `MUMAN_FFPROBE`, `MUMAN_YT_DLP`):
//!    a path or a name, and for yt-dlp a whole command such as
//!    `python -m yt_dlp`.
//! 1. Beside the muman executable, as a bundle ships them.
//! 1. On `PATH`, with Windows' `PATHEXT` extensions such as `.cmd`.
//!
//! # Floors
//!
//! muman hands ffmpeg `aresample=ochl=`, the channel layout option of
//! the API FFmpeg 5.1 brought, and yt-dlp `--no-plugin-dirs`, which
//! yt-dlp 2025.03.21 added. An older release fails each song, or each
//! fetch, with an error naming the option and nothing else; a yt-dlp
//! older still cannot read YouTube at all, which changes under it within
//! months. So each tool's version is read when it is first found, by one
//! short run of it, and one below its [`Tool::floor`] stops the run as a
//! missing tool does, saying where a newer one comes from. A version
//! that does not read, as a build of FFmpeg's git master names itself
//! `N-118000-g1a2b3c4`, is taken as new enough: it is newer than any
//! release. Reading yt-dlp's version is a Python start, up to a second,
//! once in each run that fetches.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;

/// One program muman runs.
#[derive(Debug)]
pub struct Tool {
    pub name: &'static str,
    /// The environment variable that names it.
    pub variable: &'static str,
    /// The oldest release that takes every option muman passes.
    pub floor: Version,
    /// The argument that has it print its version and exit.
    pub version_flag: &'static str,
}

pub const TOOLS: [Tool; 3] = [
    Tool {
        name: "ffmpeg",
        variable: "MUMAN_FFMPEG",
        floor: Version::new([5, 1, 0]),
        version_flag: "-version",
    },
    Tool {
        name: "ffprobe",
        variable: "MUMAN_FFPROBE",
        floor: Version::new([5, 1, 0]),
        version_flag: "-version",
    },
    Tool {
        name: "yt-dlp",
        variable: "MUMAN_YT_DLP",
        floor: Version::new([2025, 3, 21]),
        version_flag: "--version",
    },
];

/// The tool called `name`, if muman runs one.
#[must_use]
pub fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// A release number: FFmpeg's `6.1.1`, yt-dlp's date `2025.03.21`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version([u32; 3]);

impl Version {
    #[must_use]
    pub const fn new(numbers: [u32; 3]) -> Self {
        Self(numbers)
    }

    /// The version in what a tool prints for its version flag: the
    /// number after the word `version` on the first line, as `ffmpeg
    /// version 6.1.1-3ubuntu5` or `ffmpeg version n7.1`, else the first
    /// word, as yt-dlp prints `2025.03.21` alone. `None` when no number
    /// leads it, as a git build's `N-118000-g1a2b3c4`.
    #[must_use]
    pub fn parse(output: &str) -> Option<Self> {
        let line = output.lines().next()?;
        let word = match line.find("version ") {
            Some(at) => line[at + "version ".len()..].split_whitespace().next()?,
            None => line.split_whitespace().next()?,
        };
        let word = word.strip_prefix('n').unwrap_or(word);
        let mut numbers = [0; 3];
        let mut found = 0;
        for (slot, part) in numbers.iter_mut().zip(word.split('.')) {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                break;
            }
            *slot = digits.parse().ok()?;
            found += 1;
            if digits.len() < part.len() {
                break;
            }
        }
        (found > 0).then_some(Self(numbers))
    }

    /// Whether the number is a date, as yt-dlp's are.
    fn is_date(self) -> bool {
        self.0[0] >= 2000
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c] = self.0;
        if self.is_date() {
            write!(f, "{a:04}.{b:02}.{c:02}")
        } else if c == 0 {
            write!(f, "{a}.{b}")
        } else {
            write!(f, "{a}.{b}.{c}")
        }
    }
}

/// A tool as it was found: how it is started, and its version where it
/// read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub command: Vec<OsString>,
    pub version: Option<Version>,
}

/// Why a tool cannot be run; `run` exits 2 on any of these.
#[derive(Debug, Clone)]
pub enum ToolError {
    /// Not found in its variable, beside muman or on `PATH`.
    Missing { tool: &'static Tool },
    /// Found, but its version run failed: a shim whose Python is gone, a
    /// file that is no program.
    Unusable {
        tool: &'static Tool,
        program: String,
        error: String,
    },
    /// Older than the floor.
    Old { tool: &'static Tool, found: Version },
}

impl ToolError {
    /// Where a tool at or above the floor comes from.
    fn newer(tool: &Tool) -> String {
        match tool.name {
            "yt-dlp" => "update it: `yt-dlp -U` for the standalone program, `pipx upgrade \
                         yt-dlp` for one pip installed, else your package manager"
                .to_string(),
            _ => format!("install ffmpeg {} or newer", tool.floor),
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { tool } => {
                match std::env::var_os(tool.variable).filter(|v| !v.is_empty()) {
                    Some(named) => write!(
                        f,
                        "{} names {}, which is no program found: give its path, or unset {} \
                         to find {} on the PATH",
                        tool.variable,
                        named.to_string_lossy(),
                        tool.variable,
                        tool.name
                    ),
                    None => write!(
                        f,
                        "{} not found: install it, or name it in {}",
                        tool.name, tool.variable
                    ),
                }
            }
            Self::Unusable {
                tool,
                program,
                error,
            } => write!(
                f,
                "{program} ({}) does not run: {error}; install it again, or name another in {}",
                tool.name, tool.variable
            ),
            Self::Old { tool, found } => write!(
                f,
                "{} {found} is older than {}, the oldest muman runs: {}; or name a newer one \
                 in {}",
                tool.name,
                tool.floor,
                Self::newer(tool),
                tool.variable
            ),
        }
    }
}

impl std::error::Error for ToolError {}

/// A tool as `muman info` reports it.
#[derive(Debug, Clone)]
pub struct Status {
    pub tool: &'static Tool,
    pub found: Result<Found, ToolError>,
}

/// `tool` with the version its version run printed, or [`ToolError::Old`].
pub fn accept(
    tool: &'static Tool,
    command: Vec<OsString>,
    printed: &str,
) -> Result<Found, ToolError> {
    let version = Version::parse(printed);
    match version {
        Some(found) if found < tool.floor => Err(ToolError::Old { tool, found }),
        _ => Ok(Found { command, version }),
    }
}

/// How `tool` is started: from `variable`'s value when set, else beside
/// the running executable, else from `PATH`.
#[must_use]
pub fn find(tool: &Tool, variable: Option<OsString>) -> Option<Vec<OsString>> {
    if let Some(value) = variable.filter(|v| !v.is_empty()) {
        // A path or name first, whatever it holds: `C:\Tools\ffmpeg.exe`
        // and `/Applications/My Tools/ffmpeg` are one word each.
        if let Ok(program) = which::which(&value) {
            return Some(vec![program.into_os_string()]);
        }
        // Then a command with arguments, as `python -m yt_dlp`.
        let text = value.to_string_lossy();
        let words: Vec<String> = if cfg!(windows) {
            windows_words(&text)
        } else {
            shell_words::split(&text).ok()?
        };
        let (first, rest) = words.split_first()?;
        let program = which::which(first).ok()?;
        return Some(
            std::iter::once(program.into_os_string())
                .chain(rest.iter().map(OsString::from))
                .collect(),
        );
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .and_then(|dir| which::which_in(tool.name, Some(dir), ".").ok());
    beside
        .or_else(|| which::which(tool.name).ok())
        .map(|p| vec![p.into_os_string()])
}

/// `text` split into words the way Windows programs read a command line:
/// at spaces outside double quotes, the quotes dropped, and `\` taken
/// as it is, since it separates folders.
pub(crate) fn windows_words(text: &str) -> Vec<String> {
    let (mut words, mut word, mut quoted) = (Vec::new(), String::new(), false);
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffmpeg() -> &'static Tool {
        tool("ffmpeg").unwrap()
    }

    fn ytdlp() -> &'static Tool {
        tool("yt-dlp").unwrap()
    }

    #[test]
    fn a_version_is_read_from_what_each_tool_prints() {
        let v = |s| Version::parse(s).map(|v| v.to_string());
        assert_eq!(
            v("ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023"),
            Some("6.1.1".into())
        );
        assert_eq!(v("ffmpeg version n7.1 Copyright"), Some("7.1".into()));
        assert_eq!(
            v("ffmpeg version 7.1.1-full_build-www.gyan.dev Copyright"),
            Some("7.1.1".into())
        );
        assert_eq!(v("ffprobe version 5.1.4-0+deb12u1"), Some("5.1.4".into()));
        assert_eq!(v("2025.03.21\n"), Some("2025.03.21".into()));
        assert_eq!(v("2025.03.21.232815"), Some("2025.03.21".into()));
        assert_eq!(v("ffmpeg version N-118000-g1a2b3c4 Copyright"), None);
        assert_eq!(v(""), None);
    }

    #[test]
    fn a_git_build_dated_by_year_reads_as_new() {
        let v = Version::parse("ffmpeg version 2025-01-01-git-abc123-full_build").unwrap();
        assert!(v > ffmpeg().floor);
    }

    #[test]
    fn a_release_below_the_floor_is_refused_and_an_unread_one_accepted() {
        let old = accept(
            ffmpeg(),
            vec!["ffmpeg".into()],
            "ffmpeg version 4.4.2-0ubuntu0.22.04.1",
        );
        assert!(matches!(old, Err(ToolError::Old { found, .. }) if found.to_string() == "4.4.2"));
        let text = old.unwrap_err().to_string();
        assert!(text.contains("4.4.2") && text.contains("5.1") && text.contains("MUMAN_FFMPEG"));
        let old = accept(ytdlp(), vec!["yt-dlp".into()], "2025.02.19");
        assert!(old.unwrap_err().to_string().contains("yt-dlp -U"));
        let fine = accept(ytdlp(), vec!["yt-dlp".into()], "2025.03.21").unwrap();
        assert_eq!(fine.version.unwrap().to_string(), "2025.03.21");
        let unread = accept(ffmpeg(), vec!["ffmpeg".into()], "ffmpeg version N-1-gabc").unwrap();
        assert_eq!(unread.version, None);
    }

    #[test]
    fn a_variable_names_a_whole_command() {
        let exe = std::env::current_exe().unwrap();
        let line = format!("\"{}\" -m yt_dlp", exe.display());
        let found = find(ytdlp(), Some(line.into())).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[1], "-m");
    }

    #[test]
    fn a_variable_naming_a_path_with_spaces_is_one_program() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("My Tools");
        std::fs::create_dir(&folder).unwrap();
        let exe = std::env::current_exe().unwrap();
        let copy = folder.join(exe.file_name().unwrap());
        std::fs::copy(&exe, &copy).unwrap();
        let found = find(ffmpeg(), Some(copy.clone().into())).unwrap();
        assert_eq!(found, [copy.into_os_string()]);
    }

    #[test]
    fn windows_words_keep_quoted_spaces_and_backslashes() {
        assert_eq!(
            windows_words(r#""C:\Program Files\Python\python.exe" -m yt_dlp"#),
            [r"C:\Program Files\Python\python.exe", "-m", "yt_dlp"]
        );
    }

    #[test]
    fn a_tool_found_nowhere_is_none() {
        assert!(find(ffmpeg(), Some("muman-no-such-tool".into())).is_none());
    }

    #[test]
    fn a_missing_tool_says_which_variable_names_it() {
        let missing = ToolError::Missing { tool: ffmpeg() };
        assert!(missing.to_string().contains("MUMAN_FFMPEG"));
    }
}
