//! A song list written by an earlier muman brought up to this one: its
//! settings renamed, rewritten in the units they take now, and offered
//! the defaults that changed since. One history, [`HISTORY`], holds each
//! edition's steps, oldest first, and the top-level `edition` names the
//! last edition a file was brought up to: its settings' names, their
//! units and their defaults.
//!
//! | Step | Made | How a file is told to need it |
//! |---|---|---|
//! | [`Step::Rename`] | On every read, in memory, and on every write, for good | By its shape: the old name is there |
//! | [`Step::Move`] | As a rename | By its shape: the old name is in its old table |
//! | [`Step::MoveTable`] | As a rename | By its shape: the old table is there |
//! | [`Step::Default`] | By `sync --update-defaults`, when asked | By its edition, and a value that reads as the old default |
//!
//! A rename loses nothing, so it is made without asking, and is found by
//! the old name wherever it is rather than by the edition: a file left
//! at an old edition, a table pasted from old docs or a key typed from
//! memory reads the same. Renames chain: each step converts the value
//! the step before wrote. Making one twice changes nothing, since the old
//! name is gone after the first. A file naming a setting by both names
//! is refused, as neither can be told the one meant. A move is a rename
//! into another table, its value kept as it is; the table it leaves is
//! dropped once it holds nothing. A table moved whole, as `[quality.purity]`
//! to `[audio.pick.purity]`, keeps its settings, its comments and its place
//! in the file, so only its header changes.
//!
//! A changed default is offered, not made: a default written out looks
//! like a value chosen. A setting still at an edition's old default, read
//! by what it means rather than how it is spelled, so `"160kbps"` is
//! `"160 kb/s"`, is stale ([`stale`]); `sync`, `status` and `check` say
//! so, and `sync --update-defaults` moves it ([`update`]). Any other
//! value is the user's and stays. The edition rises on any write that
//! finds nothing stale, so a value set later to an old default is not
//! taken for one left behind.
//!
//! Reading never writes: a command that only reads, as `status`, reads an
//! old file as the current one and names each rename the next write
//! makes ([`notices`]). The write keeps every comment and the order of
//! keys, and `undo` puts back the song list it replaced.
//!
//! A file whose edition is newer than [`EDITION`] was written by a later
//! muman. It reads while it holds nothing this one does not know; a
//! setting it does not is refused by name, with the editions said, as
//! is any unknown setting.
//!
//! To change a setting's name, unit or default: add an [`Edition`] to
//! [`HISTORY`], numbered one above the last, with a step for each change,
//! and raise [`EDITION`] to it; write the new name or default in
//! `new.toml`. The tests check that the editions count up from 2, that
//! every renamed setting's new name is in `new.toml` and no old name is,
//! and that a file holding every old name reads as its renamed self.

use std::fmt;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table, TableLike, Value, value};

use crate::settings::{TABLES, defaults};
use crate::units::{Bitrate, Frequency, Size, Time};

/// The edition this muman writes: the last in [`HISTORY`].
pub const EDITION: i64 = 6;

/// The edition of a song list that names none: those written before
/// editions were.
pub const FIRST: i64 = 1;

/// Every edition since the first, oldest first.
pub const HISTORY: &[Edition] = &[
    Edition {
        number: 2,
        steps: &[
            rename("audio", "opus_kbps", "opus_bitrate", kbps),
            rename("audio", "opus_surround_kbps", "opus_surround_bitrate", kbps),
            rename("audio", "vorbis_kbps", "vorbis_bitrate", kbps),
            rename("audio", "aac_kbps", "aac_bitrate", kbps),
            rename("audio", "mp3_kbps", "mp3_bitrate", kbps),
            rename("audio", "min_kbps", "min_bitrate", kbps),
            rename("quality.purity", "step_ms", "step", ms),
            rename("quality.bandwidth", "step_hz", "step", hz),
            rename("ytdlp", "partial_days", "keep_partial", days),
            rename("history", "max_mib", "max_size", mib),
            rename("providers.*", "recheck_days", "recheck", days),
            rename("song", "lyrics_offset_ms", "lyrics_offset", ms),
        ],
    },
    Edition {
        number: 3,
        steps: &[
            changed("audio.codecs", r#"["opus", "flac"]"#),
            changed("quality.stereo.weight", "5"),
            changed("ytdlp.format", r#""bv*+ba/b""#),
        ],
    },
    Edition {
        number: 4,
        steps: &[changed("ytdlp.sub_langs", r#""all,-live_chat""#)],
    },
    Edition {
        number: 5,
        steps: &[moved("defaults", "lyrics", "lyrics", "read")],
    },
    Edition {
        number: 6,
        steps: &[
            moved_table("clean.lyrics", "lyrics.clean"),
            moved_table("clean", "tags.clean"),
            moved_table("quality.purity", "audio.pick.purity"),
            moved_table("quality.stereo", "audio.pick.stereo"),
            moved_table("quality.bandwidth", "audio.pick.bandwidth"),
            moved_table("quality.clipping", "audio.pick.clipping"),
            moved_table("quality.square", "cover.pick.square"),
            moved_table("quality.resolution", "cover.pick.resolution"),
            moved_table("quality.blockiness", "cover.pick.blockiness"),
        ],
    },
];

/// The editions this muman knows.
pub const EDITIONS: Editions = Editions {
    current: EDITION,
    history: HISTORY,
};

/// The editions a song list is brought up through.
#[derive(Debug, Clone, Copy)]
pub struct Editions {
    pub current: i64,
    pub history: &'static [Edition],
}

/// What changed in one edition.
#[derive(Debug, Clone, Copy)]
pub struct Edition {
    pub number: i64,
    pub steps: &'static [Step],
}

#[derive(Debug, Clone, Copy)]
pub enum Step {
    Rename(Rename),
    Move(Move),
    MoveTable(MoveTable),
    Default(Change),
}

/// A setting renamed, its value written anew.
#[derive(Debug, Clone, Copy)]
pub struct Rename {
    /// The tables it is in: `history` or `quality.purity`; `providers.*`
    /// for each provider's, `song` for each song's.
    pub table: &'static str,
    pub old: &'static str,
    pub new: &'static str,
    /// The old value as the new name takes it, `None` for one that was
    /// never valid.
    pub convert: fn(&Value) -> Option<Value>,
}

/// A setting moved into another table, its value as it was.
#[derive(Debug, Clone, Copy)]
pub struct Move {
    /// The top-level table it was in, and its name there.
    pub from: &'static str,
    pub old: &'static str,
    /// The top-level table it is in now, and its name there.
    pub to: &'static str,
    pub new: &'static str,
}

/// A table moved whole, its settings and the tables in it as they were.
#[derive(Debug, Clone, Copy)]
pub struct MoveTable {
    /// Its path before and after, as `quality.purity` and
    /// `audio.pick.purity`.
    pub from: &'static str,
    pub to: &'static str,
}

/// A default changed.
#[derive(Debug, Clone, Copy)]
pub struct Change {
    /// The key as its edition named it, as `audio.opus_bitrate` or
    /// `quality.stereo.weight`; a later [`MoveTable`] moves it.
    pub key: &'static str,
    /// Its default in the edition before, as TOML.
    pub was: &'static str,
}

const fn rename(
    table: &'static str,
    old: &'static str,
    new: &'static str,
    convert: fn(&Value) -> Option<Value>,
) -> Step {
    Step::Rename(Rename {
        table,
        old,
        new,
        convert,
    })
}

const fn moved(from: &'static str, old: &'static str, to: &'static str, new: &'static str) -> Step {
    Step::Move(Move { from, old, to, new })
}

const fn moved_table(from: &'static str, to: &'static str) -> Step {
    Step::MoveTable(MoveTable { from, to })
}

const fn changed(key: &'static str, was: &'static str) -> Step {
    Step::Default(Change { key, was })
}

/// A whole number of `unit`, written as the new name takes it.
fn whole(v: &Value, write: impl Fn(i64) -> Option<String>) -> Option<Value> {
    v.as_integer().and_then(write).map(Value::from)
}

fn kbps(v: &Value) -> Option<Value> {
    whole(v, |n| Some(Bitrate(u32::try_from(n).ok()?).exact()))
}

fn ms(v: &Value) -> Option<Value> {
    whole(v, |n| Some(Time(n).exact()))
}

fn hz(v: &Value) -> Option<Value> {
    whole(v, |n| Some(Frequency(u32::try_from(n).ok()?).exact()))
}

fn days(v: &Value) -> Option<Value> {
    whole(v, |n| (n >= 0).then(|| Time::days(n).exact()))
}

fn mib(v: &Value) -> Option<Value> {
    whole(v, |n| {
        Some(Size(u64::try_from(n).ok()?.checked_mul(1 << 20)?).exact())
    })
}

/// One rename made: where, and the setting before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    /// The table as `audio`, `providers.lrclib` or `song 3`.
    pub table: String,
    /// The setting as it was; none for the table moved whole.
    pub old: Option<String>,
    /// The setting as it is, or the table's path.
    pub new: String,
}

impl fmt::Display for Renamed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.old {
            Some(old) => write!(f, "[{}] `{old}` is `{}` now", self.table, self.new),
            None => write!(f, "[{}] is [{}] now", self.table, self.new),
        }
    }
}

/// What to say of `renamed`, and of `also` renamed besides, to a command
/// that only read the file.
#[must_use]
pub fn notices(renamed: &[Renamed], also: Option<String>) -> Vec<String> {
    let mut lines: Vec<String> = renamed
        .iter()
        .map(ToString::to_string)
        .chain(also)
        .collect();
    if !lines.is_empty() {
        lines.push("The next `sync` writes the song list with these names".into());
    }
    lines
}

/// The tables `pattern` names in `doc`, as [`Rename::table`] names them,
/// each with its name.
fn tables_named<'a>(
    doc: &'a mut DocumentMut,
    pattern: &str,
) -> Vec<(String, &'a mut dyn TableLike)> {
    let mut parts = pattern.split('.');
    let first = parts.next().unwrap_or_default();
    let Some(mut item) = doc.get_mut(first) else {
        return Vec::new();
    };
    if item.is_array_of_tables() {
        return item
            .as_array_of_tables_mut()
            .into_iter()
            .flat_map(|list| list.iter_mut().enumerate())
            .map(|(n, t)| (format!("{first} {}", n + 1), t as &mut dyn TableLike))
            .collect();
    }
    let mut name = first.to_string();
    for part in parts {
        if part == "*" {
            return item
                .as_table_like_mut()
                .into_iter()
                .flat_map(|t| t.iter_mut())
                .filter_map(|(k, i)| {
                    let label = format!("{name}.{}", k.get());
                    Some((label, i.as_table_like_mut()?))
                })
                .collect();
        }
        match item.get_mut(part) {
            Some(next) => item = next,
            None => return Vec::new(),
        }
        name = format!("{name}.{part}");
    }
    item.as_table_like_mut()
        .map(|t| (name, t))
        .into_iter()
        .collect()
}

/// Every rename of `editions` made in `doc`, oldest first, each in its
/// place and with its comments. A table setting a setting by both names,
/// or the old to a value it never took, is refused. What was renamed.
pub fn rename_all(doc: &mut DocumentMut, editions: &Editions) -> Result<Vec<Renamed>> {
    let mut renamed = Vec::new();
    let steps = editions.history.iter().flat_map(|e| e.steps);
    for r in steps.clone().filter_map(|s| match s {
        Step::Rename(r) => Some(r),
        Step::Move(_) | Step::MoveTable(_) | Step::Default(_) => None,
    }) {
        for (name, table) in tables_named(doc, r.table) {
            let Some(old) = table.get(r.old).and_then(Item::as_value) else {
                continue;
            };
            if table.contains_key(r.new) {
                bail!(
                    "[{name}] sets both `{}` and `{}`, its old name; keep `{}`",
                    r.new,
                    r.old,
                    r.new
                );
            }
            let mut new = (r.convert)(old).with_context(|| {
                format!(
                    "[{name}] `{} = {}` is not a value it took; set `{}` instead",
                    r.old,
                    text(old),
                    r.new
                )
            })?;
            *new.decor_mut() = old.decor().clone();
            renamed.push(Renamed {
                table: name,
                old: Some(format!("{} = {}", r.old, text(old))),
                new: format!("{} = {}", r.new, text(&new)),
            });
            replace_key(table, r.old, r.new, &Item::Value(new));
        }
    }
    for step in steps {
        match step {
            Step::Move(m) => renamed.extend(move_setting(doc, m)?),
            Step::MoveTable(m) => renamed.extend(move_table(doc, m)?),
            Step::Rename(_) | Step::Default(_) => {}
        }
    }
    Ok(renamed)
}

/// `m` made in `doc`, the setting keeping its comments: what moved.
fn move_setting(doc: &mut DocumentMut, m: &Move) -> Result<Option<Renamed>> {
    let Some(from) = doc.get_mut(m.from).and_then(Item::as_table_like_mut) else {
        return Ok(None);
    };
    let Some((key, item)) = from
        .get_key_value(m.old)
        .map(|(k, i)| (k.clone(), i.clone()))
    else {
        return Ok(None);
    };
    let Some(old) = item.as_value() else {
        bail!(
            "[{}] `{}` must be a value; set [{}] `{}` instead",
            m.from,
            m.old,
            m.to,
            m.new
        );
    };
    if doc.get(m.to).and_then(|t| t.get(m.new)).is_some() {
        bail!(
            "[{}] `{}` and [{}] `{}`, its new place, are both set; keep [{}] `{}`",
            m.from,
            m.old,
            m.to,
            m.new,
            m.to,
            m.new
        );
    }
    let moved = Renamed {
        table: m.from.to_string(),
        old: Some(format!("{} = {}", m.old, text(old))),
        new: format!("[{}] {} = {}", m.to, m.new, text(old)),
    };
    if let Some(from) = doc.get_mut(m.from).and_then(Item::as_table_like_mut) {
        from.remove(m.old);
        if from.is_empty() {
            doc.remove(m.from);
        }
    }
    let to = doc
        .entry(m.to)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_like_mut()
        .with_context(|| format!("[{}] must be a table", m.to))?;
    to.insert(m.new, item);
    if let Some(mut k) = to.key_mut(m.new) {
        *k.leaf_decor_mut() = key.leaf_decor().clone();
    }
    Ok(Some(moved))
}

/// `m` made in `doc`, the table keeping its settings, comments and place:
/// what moved. The tables it leaves empty are dropped, and those it goes
/// under are made, holding nothing of their own.
fn move_table(doc: &mut DocumentMut, m: &MoveTable) -> Result<Option<Renamed>> {
    let from: Vec<&str> = m.from.split('.').collect();
    let to: Vec<&str> = m.to.split('.').collect();
    let (Some((name, above)), Some((new, under))) = (from.split_last(), to.split_last()) else {
        return Ok(None);
    };
    let Some((key, item)) = table_at(doc, above)
        .and_then(|t| t.get_key_value(name))
        .map(|(k, i)| (k.clone(), i.clone()))
    else {
        return Ok(None);
    };
    if !item.is_table_like() {
        bail!("[{}] must be a table; set [{}] instead", m.from, m.to);
    }
    if table_at(doc, under).is_some_and(|t| t.contains_key(new)) {
        bail!(
            "[{}] and [{}], its new place, are both set; keep [{}]",
            m.from,
            m.to,
            m.to
        );
    }
    if let Some(t) = table_at(doc, above) {
        t.remove(name);
    }
    for depth in (1..=above.len()).rev() {
        if !table_at(doc, &above[..depth]).is_some_and(|t| t.is_empty()) {
            break;
        }
        if let Some(t) = table_at(doc, &above[..depth - 1]) {
            t.remove(above[depth - 1]);
        }
    }
    let mut parent: &mut dyn TableLike = doc.as_table_mut();
    for part in under {
        parent = parent
            .entry(part)
            .or_insert_with(|| {
                let mut t = Table::new();
                t.set_implicit(true);
                Item::Table(t)
            })
            .as_table_like_mut()
            .with_context(|| format!("[{}] must be a table", m.to))?;
    }
    parent.insert(new, item);
    if let Some(mut k) = parent.key_mut(new) {
        *k.leaf_decor_mut() = key.leaf_decor().clone();
        *k.dotted_decor_mut() = key.dotted_decor().clone();
    }
    Ok(Some(Renamed {
        table: m.from.to_string(),
        old: None,
        new: m.to.to_string(),
    }))
}

/// The table at `path` in `doc`, the root at none. Unlike
/// [`Item::get_mut`], it adds no key it finds missing.
fn table_at<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut dyn TableLike> {
    path.iter()
        .try_fold(doc.as_table_mut() as &mut dyn TableLike, |table, key| {
            table.get_mut(key)?.as_table_like_mut()
        })
}

/// `table` with `old` replaced by `new`, holding `item`, in its place
/// and with its key's comments.
fn replace_key(table: &mut dyn TableLike, old: &str, new: &str, item: &Item) {
    let entries: Vec<(toml_edit::Key, Item)> = table
        .iter()
        .filter_map(|(k, _)| {
            let (key, value) = table.get_key_value(k)?;
            Some((key.clone(), value.clone()))
        })
        .collect();
    table.clear();
    for (key, value) in entries {
        let (name, value) = if key.get() == old {
            (new, item.clone())
        } else {
            (key.get(), value)
        };
        table.insert(name, value);
        if let Some(mut k) = table.key_mut(name) {
            *k.leaf_decor_mut() = key.leaf_decor().clone();
            *k.dotted_decor_mut() = key.dotted_decor().clone();
        }
    }
}

/// A setting still at the default of the edition its song list was
/// written from, which a later edition changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    /// The key as `audio.opus_bitrate`.
    pub key: String,
    /// The file's value, that edition's default.
    pub was: String,
    pub now: String,
}

impl fmt::Display for Stale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (table, key) = self.key.rsplit_once('.').unwrap_or(("", &self.key));
        write!(f, "[{table}] {key} = {}", self.was)
    }
}

/// What to warn of for `stale`: each setting, then how to update them.
#[must_use]
pub fn stale_warnings(stale: &[Stale]) -> Vec<String> {
    let mut lines: Vec<String> = stale
        .iter()
        .map(|s| format!("`{s}` was the default; it is {} now", s.now))
        .collect();
    if !lines.is_empty() {
        lines.push(
            "`muman sync --update-defaults` moves these settings to the current defaults".into(),
        );
    }
    lines
}

/// The top-level tables a setting or a table was moved out of.
pub(crate) fn moved_from() -> impl Iterator<Item = &'static str> {
    HISTORY
        .iter()
        .flat_map(|e| e.steps)
        .filter_map(|s| match s {
            Step::Move(m) => Some(m.from),
            Step::MoveTable(m) => m.from.split('.').next(),
            _ => None,
        })
}

/// What a table every known table was moved out of still holds, which
/// no setting is, said as an error: as `[quality.typo]`, once its
/// measures went to `[audio.pick]` and `[cover.pick]`.
pub(crate) fn left_behind(doc: &DocumentMut) -> Option<String> {
    let moves: Vec<&MoveTable> = HISTORY
        .iter()
        .flat_map(|e| e.steps)
        .filter_map(|s| match s {
            Step::MoveTable(m) => Some(m),
            _ => None,
        })
        .collect();
    moves.iter().find_map(|m| {
        let root = m.from.split('.').next()?;
        let (key, _) = doc.get(root)?.as_table_like()?.iter().next()?;
        let mut went: Vec<String> = moves
            .iter()
            .filter(|n| n.from.split('.').next() == Some(root))
            .map(|n| format!("[{}]", n.to.rsplit_once('.').map_or(n.to, |(t, _)| t)))
            .collect();
        went.dedup();
        Some(format!(
            "[{root}.{key}] is no setting: what [{root}] held is under {} now",
            went.join(" and ")
        ))
    })
}

/// `key`, or a table's path, as edition `number` named it, as the
/// editions after it name it: under each table moved whole since.
fn now_named(key: &str, number: i64, editions: &Editions) -> String {
    let mut key = key.to_string();
    for step in editions
        .history
        .iter()
        .filter(|e| e.number > number)
        .flat_map(|e| e.steps)
    {
        if let Step::MoveTable(m) = step
            && let Some(rest) = key
                .strip_prefix(m.from)
                .filter(|r| r.is_empty() || r.starts_with('.'))
        {
            key = format!("{}{rest}", m.to);
        }
    }
    key
}

/// The names a setting called `new` now had in earlier editions.
pub(crate) fn old_names(new: &str) -> impl Iterator<Item = &'static str> {
    HISTORY
        .iter()
        .flat_map(|e| e.steps)
        .filter_map(move |s| match s {
            Step::Rename(r) if r.new == new => Some(r.old),
            _ => None,
        })
}

/// The edition `doc` names, the first when it names none.
#[must_use]
pub fn edition_of(doc: &DocumentMut) -> i64 {
    doc.get("edition")
        .and_then(Item::as_integer)
        .unwrap_or(FIRST)
}

/// Every key the settings tables of `table` hold a value at, with its
/// path from the root.
pub(crate) fn leaves<'a>(
    path: &[&'a str],
    table: &'a Table,
    out: &mut Vec<(Vec<&'a str>, &'a Value)>,
) {
    for (key, item) in table {
        let mut at = path.to_vec();
        at.push(key);
        match item {
            Item::Table(t) => leaves(&at, t, out),
            Item::Value(v) => out.push((at, v)),
            Item::None | Item::ArrayOfTables(_) => {}
        }
    }
}

pub(crate) fn value_at<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(doc.as_item(), |item, key| item.get(key))?
        .as_value()
}

fn value_at_mut<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut Value> {
    path.iter()
        .try_fold(doc.as_item_mut(), |item, key| item.get_mut(key))?
        .as_value_mut()
}

fn text(value: &Value) -> String {
    let mut value = value.clone();
    value.decor_mut().clear();
    value.to_string()
}

/// The settings a song list setting only `path` to `text` reads as, when
/// it reads: what a value means however it is spelled.
fn meaning(path: &[&str], text: &str) -> Option<crate::settings::Settings> {
    let (key, tables) = path.split_last()?;
    let doc: DocumentMut = format!("[{}]\n{key} = {text}\n", tables.join("."))
        .parse()
        .ok()?;
    crate::settings::read(&doc).ok()
}

/// Each setting of `doc` still at the default of the edition it names,
/// where a later edition changed that default. `doc` is read renamed.
#[must_use]
pub fn stale(doc: &DocumentMut, editions: &Editions) -> Vec<Stale> {
    let file = edition_of(doc);
    let defaults = defaults();
    let mut keys = Vec::new();
    for table in TABLES {
        if let Some(t) = defaults.get(table).and_then(Item::as_table) {
            leaves(&[table], t, &mut keys);
        }
    }
    let mut stale = Vec::new();
    for (path, now) in keys {
        let key = path.join(".");
        let Some(then) = editions
            .history
            .iter()
            .filter(|e| e.number > file)
            .flat_map(|e| e.steps.iter().map(move |s| (e.number, s)))
            .find_map(|(number, s)| match s {
                Step::Default(c) if now_named(c.key, number, editions) == key => Some(c),
                _ => None,
            })
        else {
            continue;
        };
        let Some(value) = value_at(doc, &path) else {
            continue;
        };
        let read = meaning(&path, &text(value));
        if read.is_some() && read == meaning(&path, then.was) && read != meaning(&path, &text(now))
        {
            stale.push(Stale {
                key,
                was: then.was.to_string(),
                now: text(now),
            });
        }
    }
    stale
}

/// Every setting [`stale`] finds moved to its current default, keeping
/// its comments, and `edition` raised to the current. What moved.
pub fn update(doc: &mut DocumentMut, editions: &Editions) -> Vec<Stale> {
    let _ = rename_all(doc, editions);
    let stale = stale(doc, editions);
    let defaults = defaults();
    for s in &stale {
        let path: Vec<&str> = s.key.split('.').collect();
        let Some(now) = value_at(&defaults, &path).cloned() else {
            continue;
        };
        if let Some(value) = value_at_mut(doc, &path) {
            let decor = value.decor().clone();
            *value = now;
            *value.decor_mut() = decor;
        }
    }
    if edition_of(doc) < editions.current {
        doc["edition"] = value(editions.current);
    }
    stale
}

/// `error`, reading `doc`, with what its edition says of it: a file a
/// later muman wrote may set what this one does not know.
#[must_use]
pub fn explain(error: anyhow::Error, doc: &DocumentMut) -> anyhow::Error {
    let file = edition_of(doc);
    if file > EDITION {
        error.context(format!(
            "the song list is of edition {file}, written by a later muman; this one knows \
             editions up to {EDITION}: update muman"
        ))
    } else {
        error
    }
}

#[cfg(test)]
mod tests;
