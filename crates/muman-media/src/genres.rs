//! The genres a MusicBrainz record gives a song, of those MusicBrainz's
//! users voted for.
//!
//! A record keeps the votes for genres of its recording, of its release
//! group and of its release, and of each artist it credits
//! ([`crate::record`]). The recording's speak for the song itself, so
//! they come first; a recording few vote on often has none, and then its
//! release group's, its release's, or its artists' summed stand in, the
//! first of these with any vote. Of those, the most voted are given, at
//! most [`MOST`], and none with under half the votes of the most voted:
//! one vote for a stray genre beside forty for the rest says little.
//!
//! MusicBrainz names genres in lower case, as `alternative rock` or
//! `j-pop`. Each is written with every word capitalized, a hyphen
//! starting a word too and a short joining word left small, as
//! `Alternative Rock`, `J-Pop` and `Drum and Bass`; `genres/spelled.txt`
//! spells those that rule spells wrong, as `R&B` or `UK Garage`.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::record::{Genre, Record};

/// The most genres a record gives.
pub const MOST: usize = 3;

/// Words that join a genre's others, left small but first.
const SMALL: [&str; 6] = ["and", "of", "the", "n", "in", "de"];

static SPELLED: LazyLock<BTreeMap<&'static str, &'static str>> = LazyLock::new(|| {
    include_str!("genres/spelled.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split_once('\t'))
        .collect()
});

/// The genres `record` gives, as the module says, written.
#[must_use]
pub fn of(record: &Record) -> Vec<String> {
    let details = record.release.as_ref().map(|r| &r.details);
    let mut artists: BTreeMap<&str, i32> = BTreeMap::new();
    for g in record.credited_artists.iter().flat_map(|a| &a.genres) {
        *artists.entry(&g.name).or_default() += g.count;
    }
    let artists: Vec<Genre> = artists
        .into_iter()
        .map(|(name, count)| Genre {
            name: name.to_string(),
            count,
        })
        .collect();
    let tiers: [&[Genre]; 4] = [
        &record.genres,
        details.map_or(&[], |d| &d.group_genres),
        details.map_or(&[], |d| &d.genres),
        &artists,
    ];
    let voted = |g: &&Genre| g.count > 0 && !g.name.trim().is_empty();
    let Some(tier) = tiers.into_iter().find(|t| t.iter().any(|g| voted(&g))) else {
        return Vec::new();
    };
    let mut ranked: Vec<&Genre> = tier.iter().filter(voted).collect();
    ranked.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    let most = ranked.first().map_or(0, |g| g.count);
    ranked
        .into_iter()
        .filter(|g| g.count.saturating_mul(2) >= most)
        .take(MOST)
        .map(|g| spelled(&g.name))
        .collect()
}

/// `name`, a MusicBrainz genre, as it is written.
#[must_use]
pub fn spelled(name: &str) -> String {
    let name = name.trim();
    if let Some(written) = SPELLED.get(name.to_lowercase().as_str()) {
        return (*written).to_string();
    }
    let mut out = String::with_capacity(name.len());
    for (at, word) in name.split(' ').enumerate() {
        if at > 0 {
            out.push(' ');
        }
        if at > 0 && SMALL.contains(&word) {
            out.push_str(word);
            continue;
        }
        let parts: Vec<String> = word.split('-').map(capitalized).collect();
        out.push_str(&parts.join("-"));
    }
    out
}

fn capitalized(part: &str) -> String {
    let mut chars = part.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genre(name: &str, count: i32) -> Genre {
        Genre {
            name: name.into(),
            count,
        }
    }

    #[test]
    fn a_genre_is_written_with_each_word_capitalized_or_as_the_table_spells_it() {
        assert_eq!(spelled("alternative rock"), "Alternative Rock");
        assert_eq!(spelled("j-pop"), "J-Pop");
        assert_eq!(spelled("drum and bass"), "Drum and Bass");
        assert_eq!(spelled("r&b"), "R&B");
        assert_eq!(spelled("uk garage"), "UK Garage");
    }

    #[test]
    fn the_recording_s_most_voted_genres_come_first_and_a_stray_vote_counts_for_none() {
        let mut record: Record = serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "title": "Lantern Weather",
            "artists": ["Marlo Venn"],
        }))
        .unwrap();
        record.credited_artists = vec![crate::record::CreditedArtist {
            id: "00000000-0000-0000-0000-0000000000b1".into(),
            name: "Marlo Venn".into(),
            sort_name: String::new(),
            aliases: Vec::new(),
            genres: vec![genre("folk", 3), genre("indie folk", 2)],
        }];
        assert_eq!(of(&record), ["Folk", "Indie Folk"], "its artists' alone");
        record.genres = vec![
            genre("shoegaze", 1),
            genre("dream pop", 9),
            genre("indie rock", 6),
            genre("noise pop", 5),
            genre("post-rock", 5),
        ];
        assert_eq!(of(&record), ["Dream Pop", "Indie Rock", "Noise Pop"]);
    }
}
