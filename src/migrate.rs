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
//! | [`Step::Reshape`] | As a rename | By its shape: the old name is in its old table |
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
//! in the file, so only its header changes. A setting reshaped, as
//! `[library] lyrics = "both"` into a `[[lyrics.out]]` for each place the
//! lyrics go, takes the comments `new.toml` gives its new shape, placed
//! after the table it joins.
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

use crate::provider::Provider;
use crate::query::{Closed, Have};
use crate::settings::{WRITTEN, defaults};
use crate::units::{Bitrate, Frequency, Size, Time};

/// The edition this muman writes: the last in [`HISTORY`].
pub const EDITION: i64 = 8;

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
    Edition {
        number: 7,
        steps: &[reshaped("library", "lyrics", "lyrics", "out", lyrics_out)],
    },
    Edition {
        number: 8,
        steps: &[Step::Rewrite(Rewrite {
            old: "trigger",
            convert: triggers,
        })],
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
    Reshape(Reshape),
    Rewrite(Rewrite),
    Default(Change),
}

/// A top-level list of tables that is no setting any more, rewritten into
/// the settings that do what it did.
#[derive(Debug, Clone, Copy)]
pub struct Rewrite {
    pub old: &'static str,
    /// The file with the list rewritten and taken out: what was written.
    pub convert: fn(&mut DocumentMut) -> Result<Vec<Renamed>>,
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

/// A setting replaced by one of another shape, in another table.
#[derive(Debug, Clone, Copy)]
pub struct Reshape {
    /// The top-level table it was in, and its name there.
    pub from: &'static str,
    pub old: &'static str,
    /// The top-level table it is in now, and its name there.
    pub to: &'static str,
    pub new: &'static str,
    /// The old value as the new shape, `None` for one it never took.
    pub convert: fn(&Value) -> Option<Item>,
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

const fn reshaped(
    from: &'static str,
    old: &'static str,
    to: &'static str,
    new: &'static str,
    convert: fn(&Value) -> Option<Item>,
) -> Step {
    Step::Reshape(Reshape {
        from,
        old,
        to,
        new,
        convert,
    })
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

/// `[library] lyrics`, where lyrics went, as the `[[lyrics.out]]` writing
/// them there: `new.toml`'s, with its comments, for a file beside the
/// song, and the same with `to = "tag"` for inside it.
fn lyrics_out(v: &Value) -> Option<Item> {
    let defaults = defaults();
    let file = defaults
        .get("lyrics")?
        .get("out")?
        .as_array_of_tables()?
        .get(0)?
        .clone();
    let mut tag = file.clone();
    tag.insert("to", value("tag"));
    let mut out = toml_edit::ArrayOfTables::new();
    match v.as_str()? {
        "sidecar" => out.push(file),
        "embedded" => out.push(tag),
        "both" => {
            tag.decor_mut().clear();
            out.push(file);
            out.push(tag);
        }
        _ => return None,
    }
    Some(Item::ArrayOfTables(out))
}

/// `[library] lyrics` as `out` set over `[lyrics]` from outside the song
/// list, as `export --lyrics` sets it: each place as an inline table.
pub(crate) fn lyrics_out_value(placement: &str) -> Option<Value> {
    let Item::ArrayOfTables(out) = lyrics_out(&Value::from(placement))? else {
        return None;
    };
    let places: toml_edit::Array = out
        .iter()
        .map(|t| Value::InlineTable(t.clone().into_inline_table()))
        .collect();
    Some(Value::Array(places))
}

fn mib(v: &Value) -> Option<Value> {
    whole(v, |n| {
        Some(Size(u64::try_from(n).ok()?.checked_mul(1 << 20)?).exact())
    })
}

/// A `[[trigger]]` as edition 7 read it: a song with a source from any
/// of `from`, and none from `find`, looked `find` up while `when` held.
struct Trigger {
    from: Vec<Provider>,
    find: Provider,
    when: String,
}

/// The triggers edition 7 ran when a song list wrote none.
const BUILT_IN: [(&[&str], &str, &str); 6] = [
    (&["youtube"], "youtube-music", "always"),
    (&["youtube-music"], "youtube", "no-timed-lyrics"),
    (
        &["manual", "youtube", "youtube-music"],
        "lrclib",
        "no-timed-lyrics",
    ),
    (
        &["manual", "youtube", "youtube-music"],
        "acoustid",
        "no-album",
    ),
    (
        &["manual", "youtube", "youtube-music"],
        "musicbrainz",
        "no-album",
    ),
    (
        &["manual", "youtube", "youtube-music"],
        "coverart",
        "small-cover",
    ),
];

/// A query every song matches: each has a key.
const EVERY_SONG: &str = "key::.";

/// The condition a `when` held for, as a query: `None` for `always`.
fn condition(when: &str) -> Result<Option<Closed>> {
    Ok(Some(match when {
        "always" => return Ok(None),
        "no-lyrics" => Closed::Lyrics(Have::Some),
        "no-timed-lyrics" => Closed::Lyrics(Have::Better),
        "no-album" => Closed::Has(crate::tags::Field::Album.vorbis().to_string()),
        "small-cover" => Closed::Cover(Have::Better),
        _ => bail!(
            "`{when}` is no condition: always, no-lyrics, no-timed-lyrics, no-album, small-cover"
        ),
    }))
}

fn read_trigger(t: &Table, what: &str) -> Result<Trigger> {
    let from = match t
        .get("from")
        .with_context(|| format!("{what} names no `from`"))?
    {
        Item::Value(Value::String(s)) => vec![Provider::named(s.value())?],
        Item::Value(Value::Array(a)) => a
            .iter()
            .map(|v| Provider::named(v.as_str().unwrap_or_default()))
            .collect::<Result<_>>()?,
        _ => bail!("{what}: `from` must name a provider or a list of them"),
    };
    let find = t
        .get("find")
        .and_then(Item::as_str)
        .with_context(|| format!("{what} names no `find`"))?;
    let find = Provider::named(find)?;
    if !find.findable() {
        bail!("{what}: {find} cannot be looked up");
    }
    let when = match t.get("when") {
        None => "always".to_string(),
        Some(w) => w
            .as_str()
            .with_context(|| format!("{what}: `when` must be text"))?
            .to_string(),
    };
    condition(&when).with_context(|| what.to_string())?;
    Ok(Trigger { from, find, when })
}

/// What `mine`, the triggers finding `p`, set of `[providers.<p>]`, with
/// how they are named in the notice.
fn settings_of(p: Provider, mine: &[&Trigger]) -> Result<(String, Vec<(&'static str, Value)>)> {
    let mut set: Vec<(&str, Value)> = Vec::new();
    let Some(first) = mine.first() else {
        // Every song skipped, not the provider turned off, which would stop
        // what it found before being fetched again, and AcoustID with
        // MusicBrainz.
        set.push(("skip", [EVERY_SONG].into_iter().collect()));
        return Ok((format!("no trigger finds {p}"), set));
    };
    let same: Vec<&&Trigger> = mine.iter().filter(|t| t.when == first.when).collect();
    let mut from: Vec<Provider> = same.iter().flat_map(|t| t.from.clone()).collect();
    from.sort();
    from.dedup();
    let mut said = format!("find = \"{p}\", when = \"{}\"", first.when);
    if same.len() < mine.len() {
        said = format!("{said}; its other triggers' conditions are dropped");
    }
    let built_in = BUILT_IN.iter().any(|(f, find, when)| {
        *find == p.name()
            && *when == first.when
            && from.len() == f.len()
            && from.iter().all(|q| f.contains(&q.name()))
    });
    if built_in {
        return Ok((said, set));
    }
    let yields = Closed::From(p.yields());
    let mut skip = Vec::new();
    let until = match condition(&first.when)? {
        Some(c) if p.gives(&c) => c,
        Some(c) => {
            skip.push(c.to_string());
            yields
        }
        None => yields,
    };
    let own = [Provider::Manual, Provider::YouTube, Provider::YouTubeMusic];
    if !own.iter().all(|q| from.contains(q))
        && p != Provider::YouTubeMusic
        && p != Provider::YouTube
    {
        let none: Vec<String> = from.iter().map(|q| format!("^from:{q}")).collect();
        skip.push(none.join(" "));
    }
    if p.defaults().until != [until.clone()] {
        set.push(("until", Value::from(until.to_string())));
    }
    if !skip.is_empty() {
        set.push(("skip", skip.iter().map(String::as_str).collect()));
    }
    Ok((said, set))
}

/// `[[trigger]]` as `[providers.<name>]` settings: a provider each
/// trigger found asks `until` what its `when` held for, or, for a
/// condition of what another provider finds, until it found something,
/// skipping songs that have what the condition asked for; a `from` that
/// leaves out a source's provider skips the songs with none of `from`. A
/// trigger as the built-in one writes nothing, so the provider takes
/// today's default. A provider no trigger found was never looked up, and
/// skips every song; of a provider several found, the first trigger's
/// condition is kept.
fn triggers(doc: &mut DocumentMut) -> Result<Vec<Renamed>> {
    let Some(list) = doc.get("trigger") else {
        return Ok(Vec::new());
    };
    let list = list
        .as_array_of_tables()
        .context("`trigger` must be a list of [[trigger]] tables")?;
    let read: Vec<Trigger> = list
        .iter()
        .enumerate()
        .map(|(n, t)| read_trigger(t, &format!("trigger {}", n + 1)))
        .collect::<Result<_>>()?;
    let mut written = Vec::new();
    for p in crate::lookup::ORDER {
        let mine: Vec<&Trigger> = read.iter().filter(|t| t.find == p).collect();
        let (said, set) = settings_of(p, &mine)?;
        if set.is_empty() {
            continue;
        }
        let providers = doc
            .entry("providers")
            .or_insert_with(|| {
                let mut t = Table::new();
                t.set_implicit(true);
                Item::Table(t)
            })
            .as_table_like_mut()
            .context("`providers` must be a table")?;
        let table = providers
            .entry(p.name())
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_like_mut()
            .with_context(|| format!("`providers.{p}` must be a table"))?;
        for (key, _) in &set {
            if table.contains_key(key) {
                bail!(
                    "[[trigger]] and [providers.{p}] `{key}` both say what {p} is asked; \
                     keep [providers.{p}] and delete [[trigger]]"
                );
            }
        }
        let shown: Vec<String> = set
            .iter()
            .map(|(k, v)| format!("{k} = {}", text(v)))
            .collect();
        for (key, v) in set {
            table.insert(key, Item::Value(v));
        }
        written.push(Renamed {
            table: "trigger".to_string(),
            old: Some(said),
            new: format!("[providers.{p}] {}", shown.join(", ")),
        });
    }
    doc.remove("trigger");
    Ok(written)
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
        Step::Move(_)
        | Step::MoveTable(_)
        | Step::Reshape(_)
        | Step::Rewrite(_)
        | Step::Default(_) => None,
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
            Step::Reshape(r) => renamed.extend(reshape(doc, r)?),
            Step::Rewrite(r) if doc.contains_key(r.old) => renamed.extend((r.convert)(doc)?),
            Step::Rewrite(_) | Step::Rename(_) | Step::Default(_) => {}
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

/// `r` made in `doc`: the old setting taken out, and its new shape put
/// after the table it joins, which is made where the file has none. What
/// was reshaped.
fn reshape(doc: &mut DocumentMut, r: &Reshape) -> Result<Option<Renamed>> {
    let Some(old) = table_at(doc, &[r.from])
        .and_then(|t| t.get(r.old))
        .and_then(Item::as_value)
        .cloned()
    else {
        return Ok(None);
    };
    if table_at(doc, &[r.to]).is_some_and(|t| t.contains_key(r.new)) {
        bail!(
            "[{}] `{}` and [{}] `{}`, its new place, are both set; keep [{}] `{}`",
            r.from,
            r.old,
            r.to,
            r.new,
            r.to,
            r.new
        );
    }
    let mut item = (r.convert)(&old).with_context(|| {
        format!(
            "[{}] `{} = {}` is not a value it took; set [{}] `{}` instead",
            r.from,
            r.old,
            text(&old),
            r.to,
            r.new
        )
    })?;
    let from_at = doc
        .get(r.from)
        .and_then(Item::as_table)
        .and_then(Table::position);
    if let Some(from) = table_at(doc, &[r.from]) {
        from.remove(r.old);
    }
    let to = doc.entry(r.to).or_insert_with(|| {
        let mut t = Table::new();
        t.set_position(from_at);
        Item::Table(t)
    });
    let at = to.as_table().and_then(Table::position);
    if let Item::ArrayOfTables(tables) = &mut item {
        for t in tables.iter_mut() {
            t.set_position(at);
        }
    }
    let shown: Vec<String> = match &item {
        Item::ArrayOfTables(tables) => tables
            .iter()
            .map(|t| format!("[[{}.{}]] {}", r.to, r.new, inline(t)))
            .collect(),
        other => vec![format!("[{}] {} = {other}", r.to, r.new)],
    };
    to.as_table_like_mut()
        .with_context(|| format!("[{}] must be a table", r.to))?
        .insert(r.new, item);
    Ok(Some(Renamed {
        table: r.from.to_string(),
        old: Some(format!("{} = {}", r.old, text(&old))),
        new: shown.join(", "),
    }))
}

/// `table`'s settings on one line, as `to = "file", suffix = ""`.
fn inline(table: &Table) -> String {
    table
        .iter()
        .filter_map(|(k, i)| Some(format!("{k} = {}", text(i.as_value()?))))
        .collect::<Vec<_>>()
        .join(", ")
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
fn meaning(
    path: &[&str],
    text: &str,
) -> Option<(crate::settings::Settings, crate::provider::Config)> {
    let (key, tables) = path.split_last()?;
    let doc: DocumentMut = format!("[{}]\n{key} = {text}\n", tables.join("."))
        .parse()
        .ok()?;
    Some((
        crate::settings::read(&doc).ok()?,
        crate::provider::read(&doc).ok()?,
    ))
}

/// Each setting of `doc` still at the default of the edition it names,
/// where a later edition changed that default. `doc` is read renamed.
#[must_use]
pub fn stale(doc: &DocumentMut, editions: &Editions) -> Vec<Stale> {
    let file = edition_of(doc);
    let defaults = defaults();
    let mut keys = Vec::new();
    for table in WRITTEN {
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
