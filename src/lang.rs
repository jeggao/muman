//! Languages, as BCP 47 tags: what lyrics are written in and what a
//! listener reads.
//!
//! A language reaches muman spelled many ways: yt-dlp's subtitle codes
//! (`en`, `pt-BR`, `zh-Hans`, `iw`), a stream's or a file's ISO 639-2 tag
//! (`jpn`, `ger`), MusicBrainz's ISO 639-3 (`cmn`) and the song list's
//! own. Each is read into one canonical tag by Unicode's rules (UTS #35,
//! through ICU4X), so `ger`, `deu` and `de` are one language, as are `iw`
//! and `he`, and `tl` is `fil`. `ID3v2` and the `LANGUAGE` tag take ISO
//! 639-2's terminologic code instead, [`Lang::iso639_2`]: the table of
//! two-letter codes and their three-letter ones is ISO 639-2's own list,
//! as the Library of Congress publishes it.
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
//! The codes that name no language, `und`, `mul`, `mis` and `zxx`, are no
//! [`Lang`]: they match nothing.

use std::fmt;

use icu_locale::subtags::{Language, Script};
use icu_locale::{LanguageIdentifier, Locale, LocaleCanonicalizer, LocaleExpander};

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

    /// Its ISO 639-2 code, as `ID3v2` and the `LANGUAGE` tag hold it: the
    /// terminologic one where a language has two, `deu` rather than `ger`.
    /// A language ISO 639-1 has no code for keeps its own three letters.
    #[must_use]
    pub fn iso639_2(&self) -> String {
        let code = self.0.language.as_str();
        ISO639_1
            .lines()
            .find_map(|pair| pair.strip_prefix(code)?.strip_prefix(' '))
            .unwrap_or(code)
            .to_string()
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

/// Each ISO 639-1 code and its ISO 639-2 terminologic one, a pair a line.
const ISO639_1: &str = "\
aa aar
ab abk
ae ave
af afr
ak aka
am amh
an arg
ar ara
as asm
av ava
ay aym
az aze
ba bak
be bel
bg bul
bi bis
bm bam
bn ben
bo bod
br bre
bs bos
ca cat
ce che
ch cha
co cos
cr cre
cs ces
cu chu
cv chv
cy cym
da dan
de deu
dv div
dz dzo
ee ewe
el ell
en eng
eo epo
es spa
et est
eu eus
fa fas
ff ful
fi fin
fj fij
fo fao
fr fra
fy fry
ga gle
gd gla
gl glg
gn grn
gu guj
gv glv
ha hau
he heb
hi hin
ho hmo
hr hrv
ht hat
hu hun
hy hye
hz her
ia ina
id ind
ie ile
ig ibo
ii iii
ik ipk
io ido
is isl
it ita
iu iku
ja jpn
jv jav
ka kat
kg kon
ki kik
kj kua
kk kaz
kl kal
km khm
kn kan
ko kor
kr kau
ks kas
ku kur
kv kom
kw cor
ky kir
la lat
lb ltz
lg lug
li lim
ln lin
lo lao
lt lit
lu lub
lv lav
mg mlg
mh mah
mi mri
mk mkd
ml mal
mn mon
mr mar
ms msa
mt mlt
my mya
na nau
nb nob
nd nde
ne nep
ng ndo
nl nld
nn nno
no nor
nr nbl
nv nav
ny nya
oc oci
oj oji
om orm
or ori
os oss
pa pan
pi pli
pl pol
ps pus
pt por
qu que
rm roh
rn run
ro ron
ru rus
rw kin
sa san
sc srd
sd snd
se sme
sg sag
si sin
sk slk
sl slv
sm smo
sn sna
so som
sq sqi
sr srp
ss ssw
st sot
su sun
sv swe
sw swa
ta tam
te tel
tg tgk
th tha
ti tir
tk tuk
tl tgl
tn tsn
to ton
tr tur
ts tso
tt tat
tw twi
ty tah
ug uig
uk ukr
ur urd
uz uzb
ve ven
vi vie
vo vol
wa wln
wo wol
xh xho
yi yid
yo yor
za zha
zh zho
zu zul";

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
            ("zh-Hant", "zho"),
            ("nb", "nob"),
            ("fil", "fil"),
            ("yue", "yue"),
        ] {
            assert_eq!(lang(text).iso639_2(), code, "{text}");
        }
    }

    #[test]
    fn the_table_pairs_each_two_letter_code_with_one_of_three() {
        let pairs: Vec<&str> = ISO639_1.lines().collect();
        assert_eq!(pairs.len(), 183);
        assert!(
            pairs
                .iter()
                .all(|p| p.len() == 6 && p.as_bytes()[2] == b' ')
        );
        assert!(pairs.windows(2).all(|w| w[0] < w[1]));
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
}
