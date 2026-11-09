//! A value read as its parts: a title as its core and what decorates
//! it, each decoration with what it is; a credit as its names and what
//! joins them.
//!
//! Comparing, cleaning and writing a value each need to know which of
//! its words name the song and which say something about it: `Lantern
//! Weather (Live at the Pier) [Remastered] - Radio Edit` names one song,
//! live, remastered and cut for radio. Here it is read once into parts,
//! and what compares or writes values reads the parts rather than
//! splitting the text again its own way.
//!
//! A title's decorations are what brackets hold, `(…)`, `[…]`, `{…}`, the
//! CJK brackets and a pair of wave dashes, `〜…〜`, nested brackets
//! inside one, and what follows a spaced dash, ` - `, as one more. Each is
//! read as what it says ([`Kind`]): packaging naming the video or the
//! release rather than the recording, as `Official Video`
//! ([`crate::clean::is_packaging`]); a featured artist, after `feat.`;
//! a version naming another recording, as `Live` ([`similar::VERSIONS`]);
//! a soft version naming another master or cut, as `Remastered`
//! ([`similar::SOFT_VERSIONS`]); or something else, as a subtitle. A
//! title of decorations alone takes the first that is no packaging as
//! its core.
//!
//! A credit's names are what its joiners part ([`similar::JOINERS`]): a
//! comma, `&`, `/`, `;`, `×` and their CJK forms, and `feat.`, `ft.`,
//! `featuring`, `vs.`, `with`, `and`, `x` or a spaced `-` between words;
//! the names after the first `feat.`, `ft.` or `featuring` are featured.
//!
//! Every part's text is found in the value as written, in order: nothing
//! is added or respelled.

use crate::similar;

/// What a decoration says about the title it decorates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The video or release the title came on, not the recording.
    Packaging,
    /// An artist the song features.
    Featuring,
    /// Another recording than the title alone names, by the conflict
    /// [`similar::VERSIONS`] names.
    Version(&'static str),
    /// Another master or cut of one recording.
    Soft,
    Other,
}

/// One decoration of a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoration {
    /// Its text, trimmed, without what sets it off.
    pub text: String,
    /// The brackets that held it; none for one after a spaced dash.
    pub brackets: Option<(char, char)>,
    pub kind: Kind,
}

/// A title as its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title {
    /// What names the song, trimmed.
    pub core: String,
    /// What decorates it, in the order written.
    pub decorations: Vec<Decoration>,
}

/// The words that open a featured artist's decoration.
const FEATURING: [&str; 4] = ["feat", "ft", "featuring", "with"];

impl Decoration {
    fn new(text: &str, brackets: Option<(char, char)>) -> Self {
        let words = similar::words(text);
        let kind = if crate::clean::is_packaging(text) {
            Kind::Packaging
        } else if words
            .first()
            .is_some_and(|w| FEATURING.contains(&w.as_str()))
        {
            Kind::Featuring
        } else if let Some((version, _)) = similar::VERSIONS
            .iter()
            .find(|(_, entries)| entries.iter().any(|e| similar::holds_words(&words, e)))
        {
            Kind::Version(version)
        } else if similar::SOFT_VERSIONS
            .iter()
            .any(|e| similar::holds_words(&words, e))
        {
            Kind::Soft
        } else {
            Kind::Other
        };
        Self {
            text: text.to_string(),
            brackets,
            kind,
        }
    }
}

/// A credit as its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit<'a> {
    /// Each name, trimmed, in the order written.
    pub names: Vec<&'a str>,
    /// What joins each name to the one before it, trimmed: one fewer than
    /// the names.
    pub joiners: Vec<&'a str>,
    /// Where the featured names begin, after `feat.`; none where the
    /// credit features no one.
    pub featured: Option<usize>,
}

/// `s` read as a credit's parts, as the module says.
#[must_use]
pub fn credit(s: &str) -> Credit<'_> {
    let mut parts = Credit {
        names: Vec::new(),
        joiners: Vec::new(),
        featured: None,
    };
    let mut joined = None;
    let mut start = 0;
    for m in similar::JOINERS.find_iter(s) {
        push(&mut parts, &mut joined, s[start..m.start()].trim());
        joined = Some(m.as_str().trim());
        start = m.end();
    }
    push(&mut parts, &mut joined, s[start..].trim());
    parts
}

/// `name` added to `parts`, the last joiner met, `joined`, joining it to
/// the name before; an empty name, between two joiners, adds nothing.
fn push<'a>(parts: &mut Credit<'a>, joined: &mut Option<&'a str>, name: &'a str) {
    if name.is_empty() {
        return;
    }
    if !parts.names.is_empty() {
        let before = joined.unwrap_or_default();
        if parts.featured.is_none() && FEATURING[..3].iter().any(|f| starts_word(before, f)) {
            parts.featured = Some(parts.names.len());
        }
        parts.joiners.push(before);
    }
    *joined = None;
    parts.names.push(name);
}

/// Whether `joiner` is the word `word`, case and a closing dot aside.
fn starts_word(joiner: &str, word: &str) -> bool {
    joiner.trim_end_matches('.').eq_ignore_ascii_case(word)
}

/// `s` read as a title's parts, as the module says.
#[must_use]
pub fn title(s: &str) -> Title {
    let mut core = String::new();
    let mut decorations = Vec::new();
    let mut rest = s;
    while let Some((at, open, close)) = rest
        .char_indices()
        .find_map(|(i, c)| similar::opener(c).map(|close| (i, c, close)))
    {
        let inner_at = at + open.len_utf8();
        let Some(len) = similar::closing(&rest[inner_at..], open, close) else {
            break;
        };
        core.push_str(&rest[..at]);
        let inner = rest[inner_at..inner_at + len].trim();
        if !inner.is_empty() {
            decorations.push(Decoration::new(inner, Some((open, close))));
        }
        rest = &rest[inner_at + len + close.len_utf8()..];
    }
    core.push_str(rest);
    let mut core = core.trim().to_string();
    if let Some((at, len)) = similar::SUFFIXES
        .iter()
        .filter_map(|s| core.find(s).map(|at| (at, s.len())))
        .min()
        .filter(|&(at, _)| at > 0)
    {
        let suffix = core[at + len..].trim().to_string();
        if !suffix.is_empty() {
            decorations.push(Decoration::new(&suffix, None));
        }
        core.truncate(at);
    }
    if core.trim().is_empty()
        && let Some(first) = decorations.iter().position(|d| d.kind != Kind::Packaging)
    {
        core = decorations.remove(first).text;
    }
    Title {
        core: core.trim().to_string(),
        decorations,
    }
}

#[cfg(test)]
mod tests;
