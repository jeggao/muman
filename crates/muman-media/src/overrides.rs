//! What the song list says of every source of one torrent or one
//! channel, as `[[tags.from]]` holds it: fields its sources offer that
//! are never to be believed, whether their credits name anyone, and how
//! their titles read. One rip tags every file the same wrong way, and
//! one channel titles every upload one way, so a rule for each source
//! group mends what no cleaning rule could tell from the values alone.
//!
//! An override names a torrent by its info hash, or a channel by the
//! name an upload credits it with where it names no artist of its own,
//! case aside. It applies to each source it names before cleaning: its
//! title is read by `title`, a pattern of `%title%`, `%artist%`,
//! `%album%`, `%track%` and `%dummy%` for a stretch read as nothing, as
//! `%artist% - %title%` or `%track%. %title% (%dummy%)`, each stretch
//! taken as the source offering it in its own field; then `drop`'s
//! fields go, and with `credits = false` its artist, artists and album
//! artist. A title the pattern does not fit is left as it is.

use regex::Regex;

use crate::source::SourceKey;
use crate::tags::{Field, Offers};

/// What one `[[tags.from]]` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    /// The torrent named, by its info hash.
    pub torrent: Option<String>,
    /// The channel named, by the name its uploads credit it with.
    pub channel: Option<String>,
    /// The fields its sources' offers of which are never believed.
    pub drop: Vec<Field>,
    /// Whether its sources' artist, artists and album artist count.
    pub credits: bool,
    /// How its sources' titles read.
    pub title: Option<Pattern>,
}

impl Override {
    /// Whether it names `key`, a source offering `offers`.
    #[must_use]
    pub fn names(&self, key: &SourceKey, offers: &Offers) -> bool {
        let torrent = self.torrent.as_deref().is_some_and(|hash| {
            matches!(key, SourceKey::Torrent(t) if t.hash().eq_ignore_ascii_case(hash.trim()))
        });
        let channel = self.channel.as_deref().is_some_and(|name| {
            offers
                .get(&Field::Artist)
                .filter(|o| !o.structured)
                .is_some_and(|o| {
                    o.values
                        .iter()
                        .any(|v| v.trim().eq_ignore_ascii_case(name.trim()))
                })
        });
        torrent || channel
    }
}

/// The fields a pattern's placeholders take, by name.
const PLACEHOLDERS: [(&str, Option<Field>); 5] = [
    ("title", Some(Field::Title)),
    ("artist", Some(Field::Artist)),
    ("album", Some(Field::Album)),
    ("track", Some(Field::Track)),
    ("dummy", None),
];

/// How the titles of one source group read: a pattern of placeholders
/// and the text between them, as the module says.
#[derive(Debug, Clone)]
pub struct Pattern {
    written: String,
    regex: Regex,
    /// The field each capture group takes, in order.
    fields: Vec<Field>,
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.written == other.written
    }
}

impl Eq for Pattern {}

impl Pattern {
    /// `written` as a pattern; why not, where it names no `%title%` or a
    /// placeholder there is none of.
    pub fn parse(written: &str) -> Result<Self, String> {
        let mut regex = String::from(r"^\s*");
        let mut fields = Vec::new();
        let mut rest = written;
        while let Some(start) = rest.find('%') {
            regex.push_str(&regex::escape(&rest[..start]));
            let after = &rest[start + 1..];
            let end = after
                .find('%')
                .ok_or_else(|| format!("`{written}`: a `%` opens no placeholder it closes"))?;
            let name = &after[..end];
            let (_, field) = PLACEHOLDERS
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(|| format!("`{written}`: there is no placeholder `%{name}%`"))?;
            match field {
                Some(Field::Track) => regex.push_str(r"(\d+)"),
                Some(_) => regex.push_str("(.+?)"),
                None => regex.push_str(".+?"),
            }
            fields.extend(*field);
            rest = &after[end + 1..];
        }
        regex.push_str(&regex::escape(rest));
        regex.push_str(r"\s*$");
        if !fields.contains(&Field::Title) {
            return Err(format!("`{written}` names no `%title%`"));
        }
        let regex = Regex::new(&regex).map_err(|e| format!("`{written}`: {e}"))?;
        Ok(Self {
            written: written.to_string(),
            regex,
            fields,
        })
    }

    /// What `title` reads as, each field with its value; none where the
    /// pattern does not fit it.
    #[must_use]
    pub fn read(&self, title: &str) -> Option<Vec<(Field, String)>> {
        let caps = self.regex.captures(title)?;
        let read: Vec<(Field, String)> = self
            .fields
            .iter()
            .zip(caps.iter().skip(1))
            .filter_map(|(f, m)| {
                let value = m?.as_str().trim();
                let value = match f {
                    Field::Track => value.trim_start_matches('0'),
                    _ => value,
                };
                Some((*f, value.to_string()))
            })
            .filter(|(_, v)| !v.is_empty())
            .collect();
        read.iter().any(|(f, _)| *f == Field::Title).then_some(read)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::Offer;

    #[test]
    fn a_pattern_reads_a_title_s_parts_and_leaves_one_it_does_not_fit() {
        let p = Pattern::parse("%artist% - %title% (%dummy%)").unwrap();
        assert_eq!(
            p.read("Marlo Venn - Night Ferry (Official Video)"),
            Some(vec![
                (Field::Artist, "Marlo Venn".to_string()),
                (Field::Title, "Night Ferry".to_string())
            ])
        );
        assert_eq!(p.read("Night Ferry"), None);
        let numbered = Pattern::parse("%track%. %title%").unwrap();
        assert_eq!(
            numbered.read("07. Lantern Weather"),
            Some(vec![
                (Field::Track, "7".to_string()),
                (Field::Title, "Lantern Weather".to_string())
            ])
        );
        assert!(Pattern::parse("%artist% - %name%").is_err());
        assert!(Pattern::parse("%artist% only").is_err(), "no title");
        assert!(Pattern::parse("100% %title%").is_err(), "a stray %");
    }

    #[test]
    fn an_override_names_a_torrent_by_its_hash_and_a_channel_by_its_uploads_credit() {
        let torrent = SourceKey::parse(&format!("torrent:{}/a.flac", "a".repeat(40))).unwrap();
        let o = Override {
            torrent: Some("A".repeat(40)),
            channel: Some("orchard archive".into()),
            drop: Vec::new(),
            credits: true,
            title: None,
        };
        assert!(o.names(&torrent, &Offers::new()));
        let upload = SourceKey::youtube("vid00000001");
        let mut offers = Offers::new();
        let channel = Offer {
            values: vec!["Orchard Archive".into()],
            structured: false,
        };
        offers.insert(Field::Artist, channel);
        assert!(o.names(&upload, &offers));
        offers.get_mut(&Field::Artist).unwrap().structured = true;
        assert!(!o.names(&upload, &offers), "an artist of its own");
    }
}
