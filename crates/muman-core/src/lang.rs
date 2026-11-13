//! Languages, as BCP 47 tags: what lyrics are written in, what a song is
//! sung in and what its listener reads.
//!
//! A language reaches muman spelled many ways: yt-dlp's subtitle codes
//! (`en`, `pt-BR`, `zh-Hans`, `iw`), a stream's or a file's ISO 639-2 tag
//! (`jpn`, `ger`), MusicBrainz's ISO 639-3 (`cmn`) and the song list's
//! own. Each is read into one canonical tag by Unicode's rules (UTS #35,
//! through ICU4X), so `ger`, `deu` and `de` are one language, as are `iw`
//! and `he`, and `tl` is `fil`. `ID3v2` and the `LANGUAGE` tag take ISO
//! 639-2's terminologic code instead, [`Lang::iso639`], read from
//! isolang's ISO 639-3 table: for each language with a two-letter code,
//! its ISO 639-3 code is ISO 639-2's terminologic one. A tag well formed
//! but of a language no table names, as `xx`, has no such code, so the
//! `LANGUAGE` tag refuses it.
//!
//! A preference matches a language as RFC 4647's lookup does, refined by
//! CLDR's likely subtags ([`Lang::fit`]). A bare language takes any of its
//! forms: `en` takes `en-GB`, `zh` takes `zh-Hant`. A script or a region
//! narrows it to that script, read off the region where the tag names
//! none: `zh-TW` is written in Han's traditional form, so it takes
//! `zh-Hant` and never `zh-Hans`. A region named is preferred, not
//! required: `pt-BR` takes `pt-PT` after any `pt-BR`. Norwegian, `no`,
//! and its Bokmål, `nb`, are one, as nearly every text in one is the
//! other.
//!
//! Lyrics are then the song's own, a romanization of them or a
//! translation ([`Role`]): the original is in the language sung, a
//! romanization in that language's Latin letters where it is written in
//! others (`ja-Latn` of a song sung in `ja`), and anything else a
//! translation.
//!
//! The codes that name no language, `und`, `mul`, `mis` and `zxx`, are no
//! [`Lang`]: they match nothing.
//!
//! A text's [`Writing`], the scripts of its letters or those a script
//! code names, tells which languages it may be in: Japanese is written in
//! Han and kana, Chinese in Han alone and Korean in Hangul and Han, so a
//! title in kana is no Chinese and no Korean. Latin says nothing, as any
//! language may be romanized, and neither do the characters every script
//! shares. A language whose script is not known may be in any writing.

use std::collections::BTreeSet;
use std::fmt;

use icu_locale::subtags::{Language, Script, script};
use icu_locale::{LanguageIdentifier, Locale, LocaleCanonicalizer, LocaleExpander};
use icu_properties::props::Script as Letters;
use icu_properties::{CodePointMapData, PropertyParser};

/// A language, as a canonical BCP 47 tag of a language and, where
/// given, a script and a region.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Lang(LanguageIdentifier);

/// How a preference fits a language it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// As named, or the preference named no region.
    Exact,
    /// The language and script named, of another region.
    Near,
}

/// What lyrics are to a song, by their language and the one it is sung in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// In the language sung.
    Original,
    /// In the Latin letters of a language written in others.
    Romanization,
    /// In another language.
    Translation,
}

impl Lang {
    /// The language `text` names, as any of the spellings above; `None`
    /// for one that names no language or is no tag.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().replace('_', "-");
        let mut locale = Locale::try_from_str(&text).ok()?;
        // A const fn over data compiled in: making one allocates nothing.
        LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
        let id = LanguageIdentifier {
            language: locale.id.language,
            script: locale.id.script,
            region: locale.id.region,
            variants: icu_locale::subtags::Variants::new(),
        };
        let none = ["und", "mul", "mis", "zxx"];
        (!none.contains(&id.language.as_str())).then_some(Self(id))
    }

    /// Its three-letter ISO 639 code, as the `LANGUAGE` tag holds it: ISO
    /// 639-2's terminologic one where a language has two, `deu` rather
    /// than `ger`, or ISO 639-3's for one ISO 639-2 has none of its own
    /// for, as `yue`; none for a code no ISO 639 table names, as `xx`.
    #[must_use]
    pub fn iso639(&self) -> Option<String> {
        let code = self.0.language.as_str();
        isolang::Language::from_639_1(code)
            .or_else(|| isolang::Language::from_639_3(code))
            .map(|l| l.to_639_3().to_string())
    }

    /// [`Lang::iso639`], or its own code where no table names it, as an
    /// `ID3v2` frame's language, which falls back on `XXX` for one not of
    /// three letters.
    #[must_use]
    pub fn iso639_2(&self) -> String {
        self.iso639()
            .unwrap_or_else(|| self.0.language.as_str().to_string())
    }

    /// The script it is written in: the one it names, else the one its
    /// language and region are most likely written in.
    fn script(&self) -> Option<Script> {
        self.0.script.or_else(|| {
            let mut full = self.0.clone();
            LocaleExpander::new_extended().maximize(&mut full);
            full.script
        })
    }

    /// Whether it is written in Latin letters, where its script is known.
    #[must_use]
    pub fn latin(&self) -> Option<bool> {
        self.script().map(|s| s.as_str() == "Latn")
    }

    fn same_language(&self, other: &Self) -> bool {
        let norwegian = |l: Language| matches!(l.as_str(), "no" | "nb");
        let (a, b) = (self.0.language, other.0.language);
        a == b || (norwegian(a) && norwegian(b))
    }

    /// How this preference fits `lang`, if it takes it at all.
    #[must_use]
    pub fn fit(&self, lang: &Self) -> Option<Fit> {
        if !self.same_language(lang) {
            return None;
        }
        let narrowed = self.0.script.is_some() || self.0.region.is_some();
        if narrowed && self.script() != lang.script() {
            return None;
        }
        match self.0.region {
            Some(region) if lang.0.region != Some(region) => Some(Fit::Near),
            _ => Some(Fit::Exact),
        }
    }

    /// What lyrics in this language are to a song sung in `sung`.
    #[must_use]
    pub fn role(&self, sung: &Self) -> Role {
        let latin = |l: &Self| l.script() == Some(script!("Latn"));
        if !self.same_language(sung) {
            Role::Translation
        } else if latin(self) && !latin(sung) {
            Role::Romanization
        } else {
            Role::Original
        }
    }
}

/// The scripts a text is written in, as Unicode names a letter's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Writing(BTreeSet<Letters>);

impl Writing {
    /// What the letters of `text` are written in.
    #[must_use]
    pub fn of_text(text: &str) -> Self {
        let map = CodePointMapData::<Letters>::new();
        Self(text.chars().map(|c| map.get(c)).collect())
    }

    /// What an ISO 15924 code names, as a release's `SCRIPT` tag holds
    /// one: `Jpan` is Han and kana, `Kore` Hangul and Han, `Hans` and
    /// `Hant` Han. A code naming no script Unicode knows, as `Qaaa`,
    /// says nothing.
    #[must_use]
    pub fn of_code(code: &str) -> Self {
        let scripts = match code.trim() {
            "Jpan" => vec![Letters::Han, Letters::Hiragana, Letters::Katakana],
            "Kore" => vec![Letters::Hangul, Letters::Han],
            "Hans" | "Hant" => vec![Letters::Han],
            "Hanb" => vec![Letters::Han, Letters::Bopomofo],
            "Hrkt" => vec![Letters::Hiragana, Letters::Katakana],
            "Jamo" => vec![Letters::Hangul],
            code => PropertyParser::<Letters>::new()
                .get_strict(code)
                .into_iter()
                .collect(),
        };
        Self(scripts.into_iter().collect())
    }

    /// This writing and `other`'s together.
    #[must_use]
    pub fn and(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        self
    }

    /// Whether a text so written may be in `langs`, together: each of its
    /// scripts that says anything is one they are written in.
    #[must_use]
    pub fn fits(&self, langs: &[Lang]) -> bool {
        let mut theirs = BTreeSet::new();
        for lang in langs {
            match lang.script() {
                Some(s) => theirs.extend(Self::of_code(s.as_str()).0),
                None => return true,
            }
        }
        let silent = [
            Letters::Latin,
            Letters::Common,
            Letters::Inherited,
            Letters::Unknown,
        ];
        self.0
            .iter()
            .filter(|s| !silent.contains(s))
            .all(|s| theirs.contains(s))
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl TryFrom<String> for Lang {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        Self::parse(&text).ok_or_else(|| {
            format!("{text:?} names no language: give a language tag, as \"en\" or \"pt-BR\"")
        })
    }
}

impl<'de> serde::Deserialize<'de> for Lang {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::try_from(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(text: &str) -> Lang {
        Lang::parse(text).unwrap()
    }

    #[test]
    fn every_spelling_of_a_language_reads_as_one_tag() {
        for (text, tag) in [
            ("jpn", "ja"),
            ("ger", "de"),
            ("deu", "de"),
            ("iw", "he"),
            ("tl", "fil"),
            ("cmn", "zh"),
            ("pt_BR", "pt-BR"),
            ("EN-us", "en-US"),
            ("zh-hant", "zh-Hant"),
        ] {
            assert_eq!(lang(text).to_string(), tag, "{text}");
        }
        for none in ["und", "zxx", "mul", "", "x-private", "not a tag"] {
            assert_eq!(Lang::parse(none), None, "{none}");
        }
    }

    #[test]
    fn its_iso_639_2_code_is_the_terminologic_one() {
        for (text, code) in [
            ("ja", "jpn"),
            ("de-AT", "deu"),
            ("ger", "deu"),
            ("fa", "fas"),
            ("sq", "sqi"),
            ("hy", "hye"),
            ("ms", "msa"),
            ("zh-Hant", "zho"),
            ("nb", "nob"),
            ("fil", "fil"),
            ("yue", "yue"),
        ] {
            assert_eq!(lang(text).iso639(), Some(code.to_string()), "{text}");
            assert_eq!(lang(text).iso639_2(), code, "{text}");
        }
        for (text, own) in [("xx", "xx"), ("zz-ZZ", "zz"), ("qaa", "qaa")] {
            assert_eq!(lang(text).iso639(), None, "{text}");
            assert_eq!(lang(text).iso639_2(), own, "{text}");
        }
    }

    #[test]
    fn a_preference_fits_a_language_as_lookup_and_likely_scripts_say() {
        let fit = |pref: &str, l: &str| lang(pref).fit(&lang(l));
        assert_eq!(fit("en", "en-GB"), Some(Fit::Exact));
        assert_eq!(fit("zh", "zh-Hant"), Some(Fit::Exact));
        assert_eq!(fit("zh-TW", "zh-Hant"), Some(Fit::Near));
        assert_eq!(fit("zh-TW", "zh-Hans"), None);
        assert_eq!(fit("zh-Hans", "zh"), Some(Fit::Exact));
        assert_eq!(fit("pt-BR", "pt-BR"), Some(Fit::Exact));
        assert_eq!(fit("pt-BR", "pt-PT"), Some(Fit::Near));
        assert_eq!(fit("no", "nb"), Some(Fit::Exact));
        assert_eq!(fit("ja", "ko"), None);
    }

    #[test]
    fn lyrics_are_original_romanized_or_translated() {
        let role = |l: &str, sung: &str| lang(l).role(&lang(sung));
        assert_eq!(role("ja", "ja"), Role::Original);
        assert_eq!(role("ja-JP", "jpn"), Role::Original);
        assert_eq!(role("ja-Latn", "ja"), Role::Romanization);
        assert_eq!(role("en", "ja"), Role::Translation);
        assert_eq!(role("zh-Hant", "zh"), Role::Original);
        assert_eq!(role("en-GB", "en"), Role::Original);
    }

    #[test]
    fn a_text_s_letters_say_which_languages_it_may_be_in() {
        let fits = |text: &str, l: &str| Writing::of_text(text).fits(&[lang(l)]);
        assert!(fits("ルララリの夜", "ja"));
        assert!(!fits("ルララリの夜", "zh"));
        assert!(!fits("ルララリの夜", "ko"));
        assert!(!fits("ルララリの夜", "th"));
        assert!(fits("灯籠の天気", "ja"));
        assert!(fits("灯籠天気", "zh-Hant"));
        assert!(fits("灯籠天気", "ja"), "Han alone may be Japanese");
        assert!(fits("등불 날씨", "ko"));
        assert!(!fits("등불 날씨", "ja"));
        assert!(fits("Glass Orchard", "ja"), "romanized");
        assert!(fits("ノート (Lantern Mix)", "ja"));
        assert!(!fits("ノート", "en"));
        assert!(fits("ノート", "qaa"), "a language of no known script");
        assert!(Writing::of_text("ノート 夜").fits(&[lang("en"), lang("ja")]));
    }

    #[test]
    fn a_script_code_names_the_scripts_its_texts_hold() {
        let fits = |code: &str, l: &str| Writing::of_code(code).fits(&[lang(l)]);
        assert!(fits("Jpan", "ja"));
        assert!(!fits("Jpan", "zh"));
        assert!(!fits("Kore", "ja"));
        assert!(fits("Hant", "zh"));
        assert!(fits("Hani", "ja"));
        assert!(fits("Cyrl", "ru"));
        assert!(!fits("Cyrl", "en"));
        assert!(fits("Latn", "ko"));
        assert!(fits("Qaaa", "ko"), "several scripts say nothing");
        assert!(
            Writing::of_code("Latn")
                .and(Writing::of_text("ルララリの夜"))
                .fits(&[lang("ja")])
        );
    }
}
