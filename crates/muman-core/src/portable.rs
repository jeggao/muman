//! What a name must avoid to be taken on every system: the characters
//! Windows refuses, each with the full-width lookalike that stands for
//! it, the names it keeps for devices, and the characters that reorder
//! the text around them. A song's path in the library (`naming.rs`) and
//! a torrent's files in the store ([`crate::torrent`]) are both spelled
//! by these.

use icu_properties::CodePointSetData;
use icu_properties::props::BidiControl;

/// Each character Windows refuses in a name, besides control
/// characters, and its full-width lookalike.
pub const LOOKALIKES: [(&str, &str); 9] = [
    ("/", "⧸"),
    ("\\", "⧹"),
    (":", "："),
    ("*", "＊"),
    ("?", "？"),
    ("\"", "＂"),
    ("<", "＜"),
    (">", "＞"),
    ("|", "｜"),
];

/// Whether `c` reorders the text around it, as `U+202E` makes a name
/// ending in `gpj.exe` read as one ending in `exe.jpg`: Unicode's
/// `Bidi_Control` set, the marks, embeddings, overrides and isolates.
#[must_use]
pub fn reorders(c: char) -> bool {
    CodePointSetData::new::<BidiControl>().contains(c)
}

/// The full-width lookalike of a character Windows refuses in a name.
pub(crate) fn lookalike(c: char) -> Option<char> {
    LOOKALIKES
        .iter()
        .find(|(from, _)| from.starts_with(c))
        .and_then(|(_, to)| to.chars().next())
}

/// A name Windows keeps for a device, extension or not: `CON`, `nul.txt`.
#[must_use]
pub fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && stem.chars().count() == 4
            && stem
                .chars()
                .nth(3)
                .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_device_names_get_an_underscore() {
        assert!(reserved("CON") && reserved("nul.txt") && reserved("COM1") && reserved("lpt²"));
        assert!(!reserved("CONTACT") && !reserved("COM") && !reserved("Console.opus"));
    }

    #[test]
    fn every_bidi_control_reorders() {
        for c in [
            '\u{61C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202E}', '\u{2066}', '\u{2069}',
        ] {
            assert!(reorders(c), "{c:?}");
        }
        assert!(!reorders('a') && !reorders('\u{200B}') && !reorders('\u{3000}'));
    }
}
