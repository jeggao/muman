//! Paths inside the library and the manual folder, written portably.
//!
//! A path muman records, in `songs.toml` as a `manual:` key or in
//! `state.json` and a run's record, is relative and written with `/`
//! between its parts on every platform, in Unicode NFC. So a home made
//! on one system reads the same on another, and a name macOS's HFS+
//! handed back decomposed (NFD) is the same key as the one typed.
//!
//! In memory such a path is a `PathBuf` built part by part, which the
//! platform joins with its own separator; [`portable`], [`portable_opt`]
//! and [`portable_keys`] are the serde adapters that write it with `/`.
//!
//! Read back, a recorded path is joined to the library or the home and
//! handed to a removal, a rename or a copy, so the adapters refuse one
//! with any part but a plain name ([`recorded`]): a `..`, or on Windows a
//! drive such as `C:x`, which a join takes in place of the whole path. A
//! `state.json` or a run's record edited by hand or by someone else then
//! fails to read, naming the file, rather than reach outside its folder.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use unicode_normalization::UnicodeNormalization;

/// `path` with `/` between its parts, in NFC.
#[must_use]
pub fn to_portable(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().nfc().collect::<String>()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A path inside the library or the home as muman shows it: with `/`,
/// as the song list and state write it, on every platform. A path from
/// the root is shown as the platform writes it, by `display`.
#[must_use]
pub fn show(path: &Path) -> String {
    debug_assert!(path.is_relative(), "{} is not relative", path.display());
    to_portable(path)
}

/// A path written with `/`, in NFC. On Windows a `\` typed by hand
/// separates too; elsewhere it is a character a name may hold.
#[must_use]
pub fn from_portable(text: &str) -> PathBuf {
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    text.split(separators)
        .filter(|p| !p.is_empty() && *p != ".")
        .map(|p| p.nfc().collect::<String>())
        .collect()
}

/// A path [`to_portable`] wrote, read back as [`from_portable`] reads it,
/// refused unless it names something inside the folder it is joined to:
/// each part a plain name, and at least one.
pub fn recorded(text: &str) -> Result<PathBuf, String> {
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let mut path = PathBuf::new();
    for part in text
        .split(separators)
        .filter(|p| !p.is_empty() && *p != ".")
    {
        let part: String = part.nfc().collect();
        let mut parts = Path::new(&part).components();
        let plain = matches!(
            (parts.next(), parts.next()),
            (Some(Component::Normal(name)), None) if name == part.as_str()
        );
        if !plain {
            return Err(format!("`{text}` names a path outside its folder"));
        }
        path.push(part);
    }
    if path.as_os_str().is_empty() {
        return Err(format!("`{text}` names no file"));
    }
    Ok(path)
}

/// `path` in NFC, part by part; a name read from disk becomes the key
/// it is listed under.
#[must_use]
pub fn normalized(path: &Path) -> PathBuf {
    from_portable(&to_portable(path))
}

/// How a case-insensitive, normalization-insensitive filesystem (NTFS,
/// exFAT, APFS, HFS+) tells two names apart: if two paths fold alike, they
/// may be one file. They fold to upper case, as NTFS's and exFAT's tables
/// do: lower case would tell a final sigma, `ς`, from `σ`, both `Σ` there.
#[must_use]
pub fn folded(path: &Path) -> String {
    to_portable(path).to_uppercase()
}

/// For a path.
pub mod portable {
    use super::{Deserialize, Deserializer, Path, PathBuf, Serializer, recorded, to_portable};

    pub fn serialize<S: Serializer>(path: &Path, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&to_portable(path))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        recorded(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

pub mod portable_opt {
    use super::{Deserialize, Deserializer, PathBuf, Serializer, recorded, to_portable};

    pub fn serialize<S: Serializer>(path: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
        match path {
            Some(p) => s.serialize_some(&to_portable(p)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|t| recorded(&t))
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}

/// For a map or a set whose keys are paths.
pub mod portable_keys {
    use super::{
        BTreeMap, BTreeSet, Deserialize, Deserializer, PathBuf, Serialize, Serializer, recorded,
        to_portable,
    };

    pub trait Keyed: Sized {
        fn write<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error>;
        fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error>;
    }

    impl<V: Serialize + for<'de> Deserialize<'de>> Keyed for BTreeMap<PathBuf, V> {
        fn write<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.collect_map(self.iter().map(|(k, v)| (to_portable(k), v)))
        }

        fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            // Two spellings of one path, as `A//b` and `A/b` or a name in
            // NFD and NFC, are one entry; the first stands.
            let raw = BTreeMap::<String, V>::deserialize(d)?;
            let mut out = BTreeMap::new();
            for (k, v) in raw {
                out.entry(recorded(&k).map_err(serde::de::Error::custom)?)
                    .or_insert(v);
            }
            Ok(out)
        }
    }

    impl Keyed for BTreeSet<PathBuf> {
        fn write<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.collect_seq(self.iter().map(|p| to_portable(p)))
        }

        fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            Vec::<String>::deserialize(d)?
                .iter()
                .map(|t| recorded(t).map_err(serde::de::Error::custom))
                .collect()
        }
    }

    pub fn serialize<T: Keyed, S: Serializer>(value: &T, s: S) -> Result<S::Ok, S::Error> {
        value.write(s)
    }

    pub fn deserialize<'de, T: Keyed, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
        T::read(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_written_with_slashes_and_read_back_whole() {
        let path: PathBuf = ["Artist", "Album", "01 Song.opus"].iter().collect();
        assert_eq!(to_portable(&path), "Artist/Album/01 Song.opus");
        assert_eq!(from_portable("Artist/Album/01 Song.opus"), path);
        if cfg!(windows) {
            assert_eq!(from_portable(r"Artist\Album\01 Song.opus"), path);
        }
    }

    #[test]
    fn a_decomposed_name_is_the_composed_key() {
        let nfd = "Cafe\u{301}/Ole\u{301}.flac";
        assert_eq!(to_portable(&from_portable(nfd)), "Caf\u{e9}/Ol\u{e9}.flac");
    }

    #[test]
    fn names_differing_in_case_or_composition_fold_alike() {
        assert_eq!(
            folded(&from_portable("A/Song.opus")),
            folded(&from_portable("a/song.OPUS"))
        );
        assert_eq!(
            folded(Path::new("Cafe\u{301}")),
            folded(Path::new("caf\u{e9}"))
        );
    }

    #[test]
    fn maps_and_sets_of_paths_round_trip_through_json() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Record {
            #[serde(with = "portable_keys")]
            outputs: BTreeMap<PathBuf, u8>,
            #[serde(with = "portable_keys")]
            kept: BTreeSet<PathBuf>,
            #[serde(with = "portable_opt")]
            lyrics: Option<PathBuf>,
        }
        let record = Record {
            outputs: BTreeMap::from([(from_portable("A/b.opus"), 1)]),
            kept: BTreeSet::from([from_portable("A/c.flac")]),
            lyrics: Some(from_portable("A/b.lrc")),
        };
        let json = serde_json::to_string(&record).unwrap();
        assert_eq!(
            json,
            r#"{"outputs":{"A/b.opus":1},"kept":["A/c.flac"],"lyrics":"A/b.lrc"}"#
        );
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), record);
    }

    #[test]
    fn a_recorded_path_reaching_outside_its_folder_is_refused() {
        fn json(text: &str) -> serde_json::Deserializer<serde_json::de::StrRead<'_>> {
            serde_json::Deserializer::from_str(text)
        }
        assert_eq!(recorded("A/./b.opus"), Ok(from_portable("A/b.opus")));
        assert_eq!(recorded("/A/b.opus"), Ok(from_portable("A/b.opus")));
        for text in ["../x.opus", "A/../../x.opus", "..", "", "/", "."] {
            assert!(recorded(text).is_err(), "{text}");
        }
        assert_eq!(recorded("C:x").is_err(), cfg!(windows));
        assert_eq!(recorded(r"A\..\..\x").is_err(), cfg!(windows));
        let keys: Result<BTreeMap<PathBuf, u8>, _> =
            portable_keys::deserialize(&mut json(r#"{"A/x.opus":1,"../../x.opus":1}"#));
        let e = keys.unwrap_err();
        assert!(e.to_string().contains("outside its folder"), "{e}");
        let set: Result<BTreeSet<PathBuf>, _> =
            portable_keys::deserialize(&mut json(r#"["A/../../x"]"#));
        assert!(set.is_err());
        assert!(portable_opt::deserialize(&mut json(r#""A/../../x""#)).is_err());
        assert!(portable::deserialize(&mut json(r#""../x""#)).is_err());
    }

    #[test]
    fn names_a_case_insensitive_filesystem_holds_as_one_fold_alike() {
        let fold = |s: &str| folded(Path::new(s));
        assert_eq!(fold("ΟΔΟΣ"), fold("οδοσ"));
        assert_eq!(fold("Song"), fold("SONG"));
        assert_eq!(fold("e\u{301}"), fold("é"));
        assert_ne!(fold("Song"), fold("Songs"));
    }
}
