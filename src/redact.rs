//! The song list with the secrets it holds replaced by [`PLACEHOLDER`],
//! for `export --redact` to hand on: every other byte, comments and
//! formatting included, stays as written, so the list still reads and
//! says where a secret was.
//!
//! | Where | Replaced |
//! |---|---|
//! | `[providers.*] key` | The key |
//! | `[ytdlp] args` | The value of each option in [`YTDLP`] |
//! | `[ytdlp] extractor_args`, and `--extractor-args` in `args` | Each argument that holds a token, a password or a session, as `po_token` |
//! | `[torrent] args` | The value of each option in [`ARIA2C`] |
//! | `[torrents.*] trackers` | All after each tracker's host, where a passkey may be |
//! | Every string | The user name and password in an address, as `https://user:pass@host`, and each query value named as a secret is, as `?token=` or `&client=` |
//!
//! The options are the ones a trace hides, read from one table
//! ([`crate::runner::YTDLP_SECRET`], [`crate::runner::ARIA2C_SECRET`]),
//! so the two never disagree. An option's value is replaced given apart
//! (`--password x`), joined by `=` (`--password=x`), or, for a short
//! option, joined on (`-px`). A header keeps its field's name, so
//! `--add-header Cookie:x` says which header was sent. An address an
//! option takes, as a proxy or a tracker, keeps its scheme and host
//! alone, with or without a scheme, as `user:pass@proxy:3128`: a
//! tracker's passkey may be in its path. A path is no secret: `--cookies`
//! and `--load-cookies` name a file the export does not hold, and
//! `--netrc-cmd` a command that prints one. A hook's or a program's own
//! options are not known, so of their words only an address's
//! credentials and query can be told apart; any other secret in them
//! stays, which the docs say.

use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use toml_edit::{DocumentMut, Item, TableLike, Value};

/// What a secret is replaced by.
pub const PLACEHOLDER: &str = "<redacted>";

pub use crate::runner::Takes;
use crate::runner::{ARIA2C_SECRET as ARIA2C, YTDLP_SECRET as YTDLP, extractor_secret};

/// `text`, a song list, with each secret replaced, and how many were.
pub fn redact(text: &str) -> Result<(String, usize)> {
    let mut doc: DocumentMut = text
        .trim_start_matches('\u{feff}')
        .parse()
        .context("the song list is not valid TOML")?;
    let mut n = 0;
    if let Some(ytdlp) = table(&mut doc, "ytdlp") {
        n += options(ytdlp.get_mut("args"), YTDLP);
        if let Some(by) = ytdlp
            .get_mut("extractor_args")
            .and_then(Item::as_table_like_mut)
        {
            for (_, args) in by.iter_mut() {
                n += strings(args, &mut extractor_args);
            }
        }
    }
    if let Some(torrent) = table(&mut doc, "torrent") {
        n += options(torrent.get_mut("args"), ARIA2C);
    }
    if let Some(providers) = table(&mut doc, "providers") {
        for (_, provider) in providers.iter_mut() {
            if let Some(key) = provider.as_table_like_mut().and_then(|p| p.get_mut("key")) {
                n += strings(key, &mut |s| {
                    *s = PLACEHOLDER.into();
                    1
                });
            }
        }
    }
    if let Some(torrents) = table(&mut doc, "torrents") {
        for (_, torrent) in torrents.iter_mut() {
            if let Some(trackers) = torrent
                .as_table_like_mut()
                .and_then(|t| t.get_mut("trackers"))
            {
                n += strings(trackers, &mut address_value);
            }
        }
    }
    // Every string, as a secret's shape is the same wherever it is.
    for (_, item) in doc.iter_mut() {
        n += strings(item, &mut address);
    }
    Ok((doc.to_string(), n))
}

fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> Option<&'a mut dyn TableLike> {
    doc.get_mut(name).and_then(Item::as_table_like_mut)
}

/// Apply `f` to every string in `item`, each one changed written back
/// where it was; the sum of what `f` returns.
fn strings(item: &mut Item, f: &mut dyn FnMut(&mut String) -> usize) -> usize {
    match item {
        Item::Value(v) => value_strings(v, f),
        Item::Table(t) => t.iter_mut().map(|(_, i)| strings(i, f)).sum(),
        Item::ArrayOfTables(a) => a
            .iter_mut()
            .flat_map(toml_edit::Table::iter_mut)
            .map(|(_, i)| strings(i, f))
            .sum(),
        Item::None => 0,
    }
}

fn value_strings(v: &mut Value, f: &mut dyn FnMut(&mut String) -> usize) -> usize {
    match v {
        Value::String(s) => {
            let mut text = s.value().clone();
            let n = f(&mut text);
            if text != *s.value() {
                let decor = s.decor().clone();
                *s = toml_edit::Formatted::new(text);
                *s.decor_mut() = decor;
            }
            n
        }
        Value::Array(a) => a.iter_mut().map(|v| value_strings(v, f)).sum(),
        Value::InlineTable(t) => t.iter_mut().map(|(_, v)| value_strings(v, f)).sum(),
        _ => 0,
    }
}

/// The arguments `item` lists with each value of `known`'s options, and
/// each address's credentials, replaced.
fn options(item: Option<&mut Item>, known: &[(&str, Takes)]) -> usize {
    let Some(array) = item.and_then(Item::as_array_mut) else {
        return 0;
    };
    let mut words: Vec<String> = array
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    let n = redact_words(&mut words, known);
    for (v, word) in array.iter_mut().zip(words) {
        if v.as_str().is_some_and(|s| s != word) {
            value_strings(v, &mut |s| {
                s.clone_from(&word);
                0
            });
        }
    }
    n
}

/// `words` with each value of `known`'s options, and each address's
/// credentials, replaced; how many were.
fn redact_words(words: &mut [String], known: &[(&str, Takes)]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < words.len() {
        n += address(&mut words[i]);
        let word = words[i].clone();
        if let Some((_, takes)) = known.iter().find(|(o, _)| *o == word) {
            if let Some(next) = words.get_mut(i + 1) {
                n += redact_value(*takes, next);
                i += 1;
            }
        } else if let Some((option, takes, mut value)) = joined(&word, known) {
            n += redact_value(takes, &mut value);
            words[i] = format!("{option}{value}");
        }
        i += 1;
    }
    n
}

/// `word` as an option of `known` with its value joined on: the option
/// as written up to the value, how its value is read, and the value.
fn joined(word: &str, known: &[(&str, Takes)]) -> Option<(String, Takes, String)> {
    known.iter().find_map(|(option, takes)| {
        if option.starts_with("--") {
            let value = word.strip_prefix(option)?.strip_prefix('=')?;
            Some((format!("{option}="), *takes, value.to_string()))
        } else {
            let value = word.strip_prefix(option).filter(|v| !v.is_empty())?;
            Some(((*option).to_string(), *takes, value.to_string()))
        }
    })
}

/// `value`, given to an option that `takes` it, with its secret
/// replaced; how many were.
fn redact_value(takes: Takes, value: &mut String) -> usize {
    match takes {
        Takes::Secret => *value = PLACEHOLDER.into(),
        Takes::Header => {
            *value = match value.split_once(':') {
                Some((field, _)) => format!("{field}:{PLACEHOLDER}"),
                None => PLACEHOLDER.into(),
            };
        }
        Takes::ExtractorArgs => {
            let Some((extractor, args)) = value.split_once(':') else {
                return 0;
            };
            let mut args = args.to_string();
            let n = extractor_args(&mut args);
            *value = format!("{extractor}:{args}");
            return n;
        }
        Takes::Address => return address_value(value),
    }
    1
}

/// `value`, addresses split by `,` as aria2c's `--bt-tracker` takes
/// them, each with its login and all after its host replaced, with or
/// without a scheme: a tracker's passkey may be in its path. How many
/// were.
fn address_value(value: &mut String) -> usize {
    let mut n = 0;
    let parts: Vec<String> = value
        .split(',')
        .map(|part| {
            let (scheme, rest) = match part.find("://") {
                Some(at) => part.split_at(at + 3),
                None => ("", part),
            };
            let (login, rest) = match rest.rsplit_once('@') {
                Some((login, rest)) if !login.contains('/') => (Some(login), rest),
                _ => (None, rest),
            };
            let (host, after) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
            let login = login.filter(|l| *l != PLACEHOLDER);
            let after = (!after.is_empty() && after != format!("/{PLACEHOLDER}")).then_some(after);
            if login.is_some() || after.is_some() {
                n += 1;
            }
            let login = if login.is_some() || rest.len() < part.len() - scheme.len() {
                format!("{PLACEHOLDER}@")
            } else {
                String::new()
            };
            let after = if after.is_some() || rest.len() > host.len() {
                format!("/{PLACEHOLDER}")
            } else {
                String::new()
            };
            format!("{scheme}{login}{host}{after}")
        })
        .collect();
    *value = parts.join(",");
    n
}

/// yt-dlp's extractor arguments, `key=value` pairs split by `;`, with
/// each secret one's value replaced.
fn extractor_args(text: &mut String) -> usize {
    let mut n = 0;
    let parts: Vec<String> = text
        .split(';')
        .map(|part| match part.split_once('=') {
            Some((key, _)) if extractor_secret(key) => {
                n += 1;
                format!("{key}={PLACEHOLDER}")
            }
            _ => part.to_string(),
        })
        .collect();
    *text = parts.join(";");
    n
}

/// `word` with each address's user name and password replaced, and the
/// value of each of its query's parameters named as a secret is, as
/// `?token=` or `&client=`; how many were.
fn address(word: &mut String) -> usize {
    static CREDENTIALS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([A-Za-z][A-Za-z0-9+.-]*://)([^/?#@\s]+)@").expect("valid"));
    static QUERY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)([?&](?:[a-z0-9_.-]*(?:token|key|secret|passw(?:or)?d|pass|auth|session|sig|signature)|client|client_id|passkey|apikey|uid)=)([^&#\s]*)",
        )
        .expect("valid")
    });
    let mut n = 0;
    let replaced = CREDENTIALS.replace_all(word, |c: &regex::Captures<'_>| {
        if &c[2] != PLACEHOLDER {
            n += 1;
        }
        format!("{}{PLACEHOLDER}@", &c[1])
    });
    let replaced = QUERY.replace_all(&replaced, |c: &regex::Captures<'_>| {
        if &c[2] != PLACEHOLDER {
            n += 1;
        }
        format!("{}{PLACEHOLDER}", &c[1])
    });
    if n > 0 {
        *word = replaced.into_owned();
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `line`'s words, split at spaces, as `known`'s arguments redacted.
    fn args(line: &str, known: &[(&str, Takes)]) -> (String, usize) {
        let mut words: Vec<String> = line.split(' ').map(String::from).collect();
        let n = redact_words(&mut words, known);
        (words.join(" "), n)
    }

    #[test]
    fn yt_dlp_s_logins_headers_and_tokens_are_redacted_apart_joined_and_short() {
        let given = "--username marlo --password=hunter2 -psecret -u venn --video-password v \
                     --ap-password=b -2 123456 --no-progress";
        let want = "--username <redacted> --password=<redacted> -p<redacted> -u <redacted> \
                    --video-password <redacted> --ap-password=<redacted> -2 <redacted> \
                    --no-progress";
        assert_eq!(args(given, YTDLP), (want.to_string(), 7));
        let given = "--add-header Cookie:session=1 --add-headers=Authorization:Bearer_x \
                     --cookies /home/someone/cookies.txt --netrc-cmd pass";
        let want = "--add-header Cookie:<redacted> --add-headers=Authorization:<redacted> \
                    --cookies /home/someone/cookies.txt --netrc-cmd pass";
        assert_eq!(args(given, YTDLP), (want.to_string(), 2));
        let given = "--extractor-args youtube:po_token=xyz;skip=dash \
                     --extractor-args=youtube:player_skip=js";
        let want = "--extractor-args youtube:po_token=<redacted>;skip=dash \
                    --extractor-args=youtube:player_skip=js";
        assert_eq!(args(given, YTDLP), (want.to_string(), 1));
        let mut text = "po_token=web.gvs+AAAA;player_client=web;visitor_data=CgtB".to_string();
        assert_eq!(extractor_args(&mut text), 2);
        assert_eq!(
            text,
            "po_token=<redacted>;player_client=web;visitor_data=<redacted>"
        );
    }

    #[test]
    fn aria2c_s_logins_secrets_and_headers_are_redacted() {
        let given = "--http-user=marlo --http-passwd p --ftp-user f --ftp-passwd=q \
                     --rpc-secret=s --rpc-user r --rpc-passwd=t --header=X-Key:abc \
                     --max-upload-limit=50K";
        let want = "--http-user=<redacted> --http-passwd <redacted> --ftp-user <redacted> \
                    --ftp-passwd=<redacted> --rpc-secret=<redacted> --rpc-user <redacted> \
                    --rpc-passwd=<redacted> --header=X-Key:<redacted> --max-upload-limit=50K";
        assert_eq!(args(given, ARIA2C), (want.to_string(), 8));
    }

    #[test]
    fn an_address_loses_its_credentials_only() {
        let given = "--proxy=socks5://marlo:pw@proxy.example:1080/x https://tunes.example/a@b";
        let want = "--proxy=socks5://<redacted>@proxy.example:1080/<redacted> \
                    https://tunes.example/a@b";
        assert_eq!(args(given, YTDLP), (want.to_string(), 2));
    }

    /// A song list with a marked secret in each form a secret takes.
    const CANARIES: &str = r#"version = 4
[ytdlp]
args = ["--username", "canary01", "--password=canary02", "-pcanary03",
        "--proxy", "canary04:pw@proxy.example:3128", "--add-header", "X-Key:canary05",
        "--extractor-args", "youtube:po_token=canary06",
        "--geo-verification-proxy=https://canary07@geo.example"]
extractor_args = { youtube = "data_sync_id=canary08;player_client=web" }

[torrent]
args = ["--bt-tracker=https://tracker.example/announce?passkey=canary09",
        "--bt-tracker", "https://tracker.example/canary10/announce",
        "--all-proxy=marlo:canary11@proxy.example:3128", "--http-passwd", "canary12",
        "--http-proxy", "http://canary13:x@proxy.example"]

[providers.acoustid]
key = "canary14"
url = "https://aid.example/v2?client=canary15&format=json"

[[hook]]
on = "changed"
run = ["curl", "https://hooks.example/ping?token=canary16",
       "https://marlo:canary17@hooks.example/"]
"#;

    #[test]
    fn an_address_an_option_takes_keeps_its_scheme_and_host() {
        let cases = [
            (
                "https://tracker.example/announce?passkey=x",
                "https://tracker.example/<redacted>",
                1,
            ),
            (
                "marlo:pw@proxy.example:3128",
                "<redacted>@proxy.example:3128",
                1,
            ),
            ("proxy.example:3128", "proxy.example:3128", 0),
            (
                "https://tracker.example/a@b",
                "https://tracker.example/<redacted>",
                1,
            ),
            (
                "udp://a.example:80/x,udp://b.example:80",
                "udp://a.example:80/<redacted>,udp://b.example:80",
                1,
            ),
        ];
        for (given, want, n) in cases {
            let mut value = given.to_string();
            assert_eq!(address_value(&mut value), n, "{given}");
            assert_eq!(value, want, "{given}");
            assert_eq!(address_value(&mut value), 0, "again: {given}");
            assert_eq!(value, want, "again: {given}");
        }
    }

    #[test]
    fn no_secret_of_any_form_is_left() {
        let (text, n) = redact(CANARIES).unwrap();
        assert!(!text.contains("canary"), "{text}");
        assert!(n >= 17, "{n}: {text}");
        assert!(
            text.contains("proxy.example:3128"),
            "the host stays: {text}"
        );
        assert!(text.contains("format=json"), "{text}");
    }

    #[test]
    fn a_song_list_keeps_all_but_its_secrets_as_written() {
        let text = r#"version = 1
# my list
[ytdlp]
args = ["--username", "marlo",   # mine
        "--password", "hunter2"]
extractor_args = { youtube = "po_token=AAAA;player_client=web" }

[torrent]
args = ["--rpc-secret=s3"]

[providers.acoustid]
key = "k0000000" # from acoustid.org

[[hook]]
on = "changed"
run = ["curl", "https://marlo:tok@hooks.example/ping"]

[[song]]
sources = ["youtube.com:vid00000001"]
tags.title = "Paper Comets"
"#;
        let want = text
            .replace("\"marlo\"", "\"<redacted>\"")
            .replace("hunter2", PLACEHOLDER)
            .replace("AAAA", PLACEHOLDER)
            .replace("=s3", "=<redacted>")
            .replace("k0000000", PLACEHOLDER)
            .replace("marlo:tok", PLACEHOLDER);
        assert_eq!(redact(text).unwrap(), (want.clone(), 6));
        let doc: DocumentMut = want.parse().unwrap();
        crate::settings::read(&doc).unwrap();
        crate::providers::read(&doc).unwrap();
        let new = crate::manifest::NEW;
        assert_eq!(redact(new).unwrap(), (new.to_string(), 0));
    }
}
