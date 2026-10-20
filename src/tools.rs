//! The programs muman runs, ffmpeg, ffprobe, yt-dlp and aria2c: where
//! each is found, how its version is read, and the oldest release of each
//! that takes every option muman passes.
//!
//! Which program fills each role is the song list's `[tools.*]`
//! ([`Named`]), so a fork, or a wrapper that speaks a tool's options, is
//! named once for every machine the list is used on. A tool is looked
//! for, in order:
//!
//! 1. In its variable (`MUMAN_FFMPEG`, `MUMAN_FFPROBE`, `MUMAN_YT_DLP`,
//!    `MUMAN_ARIA2C`):
//!    a path or a name, or a whole command such as `python -m yt_dlp`.
//!    The variable is this machine's, so it overrides the song list.
//! 1. As the song list's `command` names it: a name beside the muman
//!    executable, as a bundle ships them, then on `PATH`, with Windows'
//!    `PATHEXT` extensions such as `.cmd`; a path as it is.
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
//!
//! aria2c fetches torrents. Every option muman passes it is older than
//! aria2 1.35.0, the oldest release a Debian or Ubuntu still supported
//! ships, which is its floor; only a run that fetches a torrent needs it.
//!
//! A fork may number its releases its own way, and one dated as yt-dlp
//! is may still lack an option; `version_check = false` runs it whatever
//! it prints for its version, or if it prints none, and each option it
//! lacks fails where it is passed.

use std::ffi::{OsStr, OsString};
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

pub const TOOLS: [Tool; 4] = [
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
    Tool {
        name: "aria2c",
        variable: "MUMAN_ARIA2C",
        floor: Version::new([1, 35, 0]),
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

/// A tool as the song list's `[tools.*]` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// The program, a name or a whole path, then its arguments.
    pub command: Vec<OsString>,
    /// Refuse a release below the tool's floor.
    pub version_check: bool,
}

impl Named {
    /// `tool` as a song list that sets nothing for it names it.
    #[must_use]
    pub fn default_for(tool: &Tool) -> Self {
        Self {
            command: vec![tool.name.into()],
            version_check: true,
        }
    }

    /// The command as the song list writes it, when it is not the tool's
    /// own name.
    pub(crate) fn other_than(&self, tool: &Tool) -> Option<String> {
        (self.command != [tool.name]).then(|| {
            self.command
                .iter()
                .map(|w| w.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        })
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
    /// Not found in its variable, beside muman or on `PATH`; `named` is
    /// the song list's command where it is not the tool's own name.
    Missing {
        tool: &'static Tool,
        named: Option<String>,
    },
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
            "aria2c" => format!("install aria2 {} or newer", tool.floor),
            _ => format!("install ffmpeg {} or newer", tool.floor),
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { tool, named } => {
                match (
                    std::env::var_os(tool.variable).filter(|v| !v.is_empty()),
                    named,
                ) {
                    (Some(value), _) => write!(
                        f,
                        "{} names {}, which is no program found: give its path, or unset {} \
                         to find {} as the song list names it",
                        tool.variable,
                        value.to_string_lossy(),
                        tool.variable,
                        tool.name
                    ),
                    (None, Some(named)) => write!(
                        f,
                        "[tools.{}] command names {named}, which is no program found: install \
                         it, or name another there or in {}",
                        tool.name, tool.variable
                    ),
                    (None, None) => write!(
                        f,
                        "{} not found: install it, or name it in [tools.{}] command or {}",
                        tool.name, tool.name, tool.variable
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
            Self::Old { tool, found } => {
                write!(
                    f,
                    "{} {found} is older than {}, the oldest muman runs: {}; or name a newer \
                     one in {}",
                    tool.name,
                    tool.floor,
                    Self::newer(tool),
                    tool.variable
                )?;
                if found.is_date() != tool.floor.is_date() {
                    write!(
                        f,
                        "; a fork numbered its own way runs with [tools.{}] version_check = false",
                        tool.name
                    )?;
                }
                Ok(())
            }
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

/// `tool` with the version its version run printed, or
/// [`ToolError::Old`] when `check` and below its floor.
pub fn accept(
    tool: &'static Tool,
    command: Vec<OsString>,
    printed: &str,
    check: bool,
) -> Result<Found, ToolError> {
    let version = Version::parse(printed);
    match version {
        Some(found) if check && found < tool.floor => Err(ToolError::Old { tool, found }),
        _ => Ok(Found { command, version }),
    }
}

/// How a tool is started: from `variable`'s value when set, else as
/// `named` names it.
#[must_use]
pub fn find(variable: Option<OsString>, named: &Named) -> Option<Vec<OsString>> {
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
    let (first, rest) = named.command.split_first()?;
    let program = if is_name(first) {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .and_then(|dir| which::which_in(first, Some(dir), ".").ok());
        beside.or_else(|| which::which(first).ok())?
    } else {
        which::which(first).ok()?
    };
    Some(
        std::iter::once(program.into_os_string())
            .chain(rest.iter().cloned())
            .collect(),
    )
}

/// Whether `word` is a program's bare name, to look for, rather than a
/// path to it.
pub(crate) fn is_name(word: &OsStr) -> bool {
    let mut parts = Path::new(word).components();
    matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none()
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
    fn aria2c_is_read_by_the_version_after_its_name() {
        let aria2c = tool("aria2c").unwrap();
        let printed = "aria2 version 1.34.0\nCopyright (C) 2006, 2019 Tatsuhiro Tsujikawa\n";
        let old = accept(aria2c, vec!["aria2c".into()], printed, true).unwrap_err();
        assert!(
            old.to_string().contains("install aria2 1.35 or newer"),
            "{old}"
        );
        let fine = accept(aria2c, vec!["aria2c".into()], "aria2 version 1.37.0", true).unwrap();
        assert_eq!(fine.version, Some(Version::new([1, 37, 0])));
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
            true,
        );
        assert!(matches!(old, Err(ToolError::Old { found, .. }) if found.to_string() == "4.4.2"));
        let text = old.unwrap_err().to_string();
        assert!(text.contains("4.4.2") && text.contains("5.1") && text.contains("MUMAN_FFMPEG"));
        let old = accept(ytdlp(), vec!["yt-dlp".into()], "2025.02.19", true);
        assert!(old.unwrap_err().to_string().contains("yt-dlp -U"));
        let fine = accept(ytdlp(), vec!["yt-dlp".into()], "2025.03.21", true).unwrap();
        assert_eq!(fine.version.unwrap().to_string(), "2025.03.21");
        let unread = accept(
            ffmpeg(),
            vec!["ffmpeg".into()],
            "ffmpeg version N-1-gabc",
            true,
        )
        .unwrap();
        assert_eq!(unread.version, None);
    }

    #[test]
    fn a_variable_names_a_whole_command() {
        let exe = std::env::current_exe().unwrap();
        let line = format!("\"{}\" -m yt_dlp", exe.display());
        let found = find(Some(line.into()), &Named::default_for(ytdlp())).unwrap();
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
        let found = find(Some(copy.clone().into()), &Named::default_for(ffmpeg())).unwrap();
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
        let named = Named::default_for(ffmpeg());
        assert!(find(Some("muman-no-such-tool".into()), &named).is_none());
        let named = Named {
            command: vec!["muman-no-such-tool".into()],
            version_check: true,
        };
        assert!(find(None, &named).is_none());
    }

    #[test]
    fn a_missing_tool_says_which_variable_names_it() {
        let missing = ToolError::Missing {
            tool: ffmpeg(),
            named: None,
        };
        let text = missing.to_string();
        assert!(text.contains("MUMAN_FFMPEG") && text.contains("[tools.ffmpeg]"));
    }

    #[test]
    fn the_song_list_names_a_program_by_path_with_its_arguments() {
        let exe = std::env::current_exe().unwrap();
        let named = Named {
            command: vec![exe.clone().into(), "-m".into(), "yt_dlp".into()],
            version_check: true,
        };
        let found = find(None, &named).unwrap();
        assert_eq!(found, [exe.into_os_string(), "-m".into(), "yt_dlp".into()]);
        assert_eq!(
            named.other_than(ytdlp()).map(|c| c.ends_with("-m yt_dlp")),
            Some(true)
        );
        assert_eq!(Named::default_for(ytdlp()).other_than(ytdlp()), None);
    }

    #[test]
    fn a_bare_name_is_looked_for_and_a_path_is_not() {
        assert!(is_name(OsStr::new("yt-dlp")));
        assert!(!is_name(OsStr::new("./yt-dlp")));
        assert!(!is_name(Path::new("tools").join("yt-dlp").as_os_str()));
    }

    #[test]
    fn a_tool_whose_version_is_not_checked_runs_below_the_floor() {
        let fork = accept(ytdlp(), vec!["yt-dlp".into()], "1.4.0", false).unwrap();
        assert_eq!(fork.version.unwrap().to_string(), "1.4");
        let refused = accept(ytdlp(), vec!["yt-dlp".into()], "1.4.0", true).unwrap_err();
        assert!(
            refused.to_string().contains("version_check = false"),
            "{refused}"
        );
        let old = accept(ytdlp(), vec!["yt-dlp".into()], "2025.02.19", true).unwrap_err();
        assert!(!old.to_string().contains("version_check"));
    }
}
