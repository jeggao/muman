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
//! | Those, `[[hook]]`, `[tools]`, `[providers]` and `[sites]` | The user name and password in an address, as `https://user:pass@host` |
//!
//! An option's value is replaced given apart (`--password x`), joined by
//! `=` (`--password=x`), or, for a short option, joined on (`-px`). A
//! header keeps its field's name, so `--add-header Cookie:x` says which
//! header was sent. A path is no secret: `--cookies` and `--load-cookies`
//! name a file the export does not hold, and `--netrc-cmd` a command
//! that prints one. A hook's or a program's own options are not known,
//! so of their words only an address's credentials can be told apart;
//! any other secret in them stays, which the docs say.

use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use toml_edit::{DocumentMut, Item, TableLike, Value};

/// What a secret is replaced by.
pub const PLACEHOLDER: &str = "<redacted>";

/// What follows an option that takes a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takes {
    /// A secret whole: a password, a user name, a token.
    Secret,
    /// `Field:value`, the field kept.
    Header,
    /// `extractor:key=value;...`, as yt-dlp's `--extractor-args`.
    ExtractorArgs,
}

/// yt-dlp's options that take a secret: its account, two-factor and TV
/// provider logins, a video's password, a client certificate's password,
/// headers sent, and extractor arguments.
pub const YTDLP: &[(&str, Takes)] = &[
    ("--username", Takes::Secret),
    ("-u", Takes::Secret),
    ("--password", Takes::Secret),
    ("-p", Takes::Secret),
    ("--twofactor", Takes::Secret),
    ("-2", Takes::Secret),
    ("--video-password", Takes::Secret),
    ("--ap-username", Takes::Secret),
    ("--ap-password", Takes::Secret),
    ("--client-certificate-password", Takes::Secret),
    ("--add-header", Takes::Header),
    ("--add-headers", Takes::Header),
    ("--extractor-args", Takes::ExtractorArgs),
];

/// aria2c's options that take a secret: HTTP, FTP and proxy logins, the
/// RPC interface's secret and login, and headers sent.
pub const ARIA2C: &[(&str, Takes)] = &[
    ("--http-user", Takes::Secret),
    ("--http-passwd", Takes::Secret),
    ("--ftp-user", Takes::Secret),
    ("--ftp-passwd", Takes::Secret),
    ("--all-proxy-user", Takes::Secret),
    ("--all-proxy-passwd", Takes::Secret),
    ("--http-proxy-user", Takes::Secret),
    ("--http-proxy-passwd", Takes::Secret),
    ("--https-proxy-user", Takes::Secret),
    ("--https-proxy-passwd", Takes::Secret),
    ("--ftp-proxy-user", Takes::Secret),
    ("--ftp-proxy-passwd", Takes::Secret),
    ("--rpc-secret", Takes::Secret),
    ("--rpc-user", Takes::Secret),
    ("--rpc-passwd", Takes::Secret),
    ("--header", Takes::Header),
];

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
    for name in ["providers", "hook", "tools", "sites"] {
        if let Some(item) = doc.get_mut(name) {
            n += strings(item, &mut address);
        }
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
                n += takes.redact(next);
                i += 1;
            }
        } else if let Some((option, takes, mut value)) = joined(&word, known) {
            n += takes.redact(&mut value);
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

impl Takes {
    fn redact(self, value: &mut String) -> usize {
        match self {
            Self::Secret => *value = PLACEHOLDER.into(),
            Self::Header => {
                *value = match value.split_once(':') {
                    Some((field, _)) => format!("{field}:{PLACEHOLDER}"),
                    None => PLACEHOLDER.into(),
                };
            }
            Self::ExtractorArgs => {
                let Some((extractor, args)) = value.split_once(':') else {
                    return 0;
                };
                let mut args = args.to_string();
                let n = extractor_args(&mut args);
                *value = format!("{extractor}:{args}");
                return n;
            }
        }
        1
    }
}

/// yt-dlp's extractor arguments, `key=value` pairs split by `;`, with
/// each secret one's value replaced.
fn extractor_args(text: &mut String) -> usize {
    let mut n = 0;
    let parts: Vec<String> = text
        .split(';')
        .map(|part| match part.split_once('=') {
            Some((key, _)) if secret_argument(key) => {
                n += 1;
                format!("{key}={PLACEHOLDER}")
            }
            _ => part.to_string(),
        })
        .collect();
    *text = parts.join(";");
    n
}

/// Whether an extractor argument named `key` holds a secret: a token, as
/// YouTube's `po_token`, a password or a cookie, or what names a
/// signed-in account's session, YouTube's `visitor_data` and
/// `data_sync_id`.
fn secret_argument(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    ["token", "password", "secret", "cookie", "api_key"]
        .iter()
        .any(|s| key.contains(s))
        || ["visitor_data", "data_sync_id"].contains(&key.as_str())
}

/// `word` with the user name and password of each address in it
/// replaced; how many were.
fn address(word: &mut String) -> usize {
    static CREDENTIALS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([A-Za-z][A-Za-z0-9+.-]*://)([^/?#@\s]+)@").expect("valid"));
    let mut n = 0;
    let replaced = CREDENTIALS.replace_all(word, |c: &regex::Captures<'_>| {
        if &c[2] != PLACEHOLDER {
            n += 1;
        }
        format!("{}{PLACEHOLDER}@", &c[1])
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
        let want = "--proxy=socks5://<redacted>@proxy.example:1080/x https://tunes.example/a@b";
        assert_eq!(args(given, YTDLP), (want.to_string(), 1));
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
        crate::provider::read(&doc).unwrap();
        let new = crate::manifest::NEW;
        assert_eq!(redact(new).unwrap(), (new.to_string(), 0));
    }
}
