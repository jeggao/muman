//! The home's state store, `state.db`: one SQLite file the state is to
//! move into from `state.json`, opened here, and written only through
//! [`Db::write`].
//!
//! The store loads into the [`State`] every reader knows, in one
//! snapshot ([`Db::snapshot`]), which remembers a digest of each row it
//! read; a save ([`Db::save_diff`]) writes only the rows whose digest
//! differs and deletes only those it read that the state no longer holds.
//! So a save that changed nothing writes nothing, and a row another
//! process wrote after the snapshot, which this one never read, stays.
//!
//! SQLite, since several processes reach one home: a step without the
//! run's lock writes what it measured while a run holds it, and a
//! transaction lets each write its own rows where a file is written
//! whole by whoever saves last. Its format is stable across versions, and
//! the C library is built into muman (`bundled`), so no system SQLite of
//! another version is met.
//!
//! The store is muman's by its `application_id`, and of a schema by its
//! `user_version`: a file of another application, or of a schema newer
//! than this muman's, is refused, as a newer `state.json` is. Each value
//! is a row of JSON text, as `state.json` holds it, so a fact or a plan
//! of another shape needs no change to the tables, and is read as
//! leniently as the file is.
//!
//! The journal is WAL, so a reader is never blocked by a writer, and a
//! commit waits for the disk: `synchronous` is `FULL`, with `fullfsync`
//! on Apple platforms, whose `fsync` does not reach the disk. A writer
//! waits for another's transaction as long as the caller allows. Tests
//! commit without waiting for the disk, and each commit is a change
//! `testing::kill_at` can stop a test at, just before it lands:
//! a write stopped there is rolled back whole, as a write killed before
//! its rename leaves the file as it was.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};

use super::{Caches, State, VERSION};
use crate::facts::Facts;
use crate::source::SourceKey;

pub(crate) const FILE: &str = "state.db";

/// The store's `application_id`, `muSt`.
const APPLICATION_ID: i32 = i32::from_be_bytes(*b"muSt");

/// The schema this muman writes: 1 holds each row of the state as JSON.
pub(crate) const SCHEMA: i32 = 1;

/// The renamings a step without the run's lock recorded of a source's
/// parts, in the order made, which a load applies to the kept plans and
/// a whole save, writing them renamed, clears ([`Db::keep`]).
const RENAMINGS: &str = "
    CREATE TABLE IF NOT EXISTS renamings (
        seq INTEGER PRIMARY KEY AUTOINCREMENT, key TEXT NOT NULL, old TEXT NOT NULL,
        new TEXT NOT NULL
    );
";

/// The tables of [`SCHEMA`], each value a row of JSON text; a `digest`
/// is of that text.
const TABLES: &str = "
    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE outputs (path TEXT PRIMARY KEY, row TEXT NOT NULL, digest BLOB NOT NULL);
    CREATE TABLE replaced (old TEXT PRIMARY KEY, new TEXT NOT NULL);
    CREATE TABLE listed (key TEXT PRIMARY KEY);
    CREATE TABLE damaged (key TEXT PRIMARY KEY, audio TEXT NOT NULL);
    CREATE TABLE trusted (name TEXT PRIMARY KEY, digest TEXT NOT NULL);
    CREATE TABLE facts (key TEXT PRIMARY KEY, row TEXT NOT NULL, digest BLOB NOT NULL);
    CREATE TABLE alignments (
        a_audio TEXT NOT NULL, b_audio TEXT NOT NULL, method TEXT NOT NULL, row TEXT NOT NULL,
        PRIMARY KEY (a_audio, b_audio, method)
    );
    CREATE TABLE mixes (
        audio TEXT NOT NULL, layout TEXT NOT NULL, row TEXT NOT NULL,
        PRIMARY KEY (audio, layout)
    );
    CREATE TABLE failures (key TEXT PRIMARY KEY, row TEXT NOT NULL);
    CREATE TABLE lookups (
        source TEXT NOT NULL, find TEXT NOT NULL, row TEXT NOT NULL,
        PRIMARY KEY (source, find)
    );
    CREATE TABLE sizes (plan_key TEXT PRIMARY KEY, row TEXT NOT NULL);
";

/// The home's state store, open.
pub(crate) struct Db {
    conn: Connection,
    file: PathBuf,
}

impl Db {
    /// The store in `home`, made with the tables of [`SCHEMA`] when there
    /// is none, a writer waiting up to `wait` for another's transaction.
    pub(crate) fn open(home: &Path, wait: Duration) -> Result<Self> {
        Self::open_file(&home.join(FILE), wait)
    }

    /// The store at `file`, as [`Db::open`] opens one.
    pub(crate) fn open_file(file: &Path, wait: Duration) -> Result<Self> {
        let file = file.to_path_buf();
        let conn =
            Connection::open(&file).with_context(|| format!("opening {}", file.display()))?;
        conn.busy_timeout(wait)?;
        let id: i32 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
        if id != APPLICATION_ID && (id != 0 || version != 0 || tables != 0) {
            bail!("{} is not muman's state", file.display());
        }
        if version > SCHEMA {
            bail!(
                "{} is format version {version}, newer than this muman's {SCHEMA}",
                file.display()
            );
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", if cfg!(test) { "OFF" } else { "FULL" })?;
        if cfg!(target_vendor = "apple") {
            conn.pragma_update(None, "fullfsync", true)?;
        }
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        let mut db = Self { conn, file };
        if version == 0 {
            db.write(|tx| {
                tx.execute_batch(TABLES)?;
                tx.execute_batch(RENAMINGS)?;
                tx.pragma_update(None, "application_id", APPLICATION_ID)?;
                tx.pragma_update(None, "user_version", SCHEMA)?;
                Ok(())
            })
            .with_context(|| format!("making {}", db.file.display()))?;
        } else {
            db.conn
                .execute_batch(RENAMINGS)
                .with_context(|| format!("opening {}", db.file.display()))?;
        }
        Ok(db)
    }

    /// Run `f` in one transaction, which commits only when it returns
    /// `Ok`, a writer at a time.
    pub(crate) fn write<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .with_context(|| format!("writing {}", self.file.display()))?;
        let done = f(&tx)?;
        #[cfg(test)]
        crate::testing::change_point(&self.file);
        tx.commit()
            .with_context(|| format!("writing {}", self.file.display()))?;
        Ok(done)
    }

    /// Run `f` over one snapshot of the store, whatever a writer commits
    /// meanwhile.
    pub(crate) fn read<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .with_context(|| format!("reading {}", self.file.display()))?;
        f(&tx)
    }
}

/// A table of [`SCHEMA`]: its key columns, and the column of its value,
/// if it has one, with a `digest` beside it, if it has one.
struct Table {
    name: &'static str,
    keys: &'static [&'static str],
    value: Option<&'static str>,
    digest: bool,
}

const META: Table = table("meta", &["key"], Some("value"), false);
const OUTPUTS: Table = table("outputs", &["path"], Some("row"), true);
const REPLACED: Table = table("replaced", &["old"], Some("new"), false);
const LISTED: Table = table("listed", &["key"], None, false);
const DAMAGED: Table = table("damaged", &["key"], Some("audio"), false);
const TRUSTED: Table = table("trusted", &["name"], Some("digest"), false);
const FACTS: Table = table("facts", &["key"], Some("row"), true);
const ALIGNMENTS: Table = table(
    "alignments",
    &["a_audio", "b_audio", "method"],
    Some("row"),
    false,
);
const MIXES: Table = table("mixes", &["audio", "layout"], Some("row"), false);
const FAILURES: Table = table("failures", &["key"], Some("row"), false);
const LOOKUPS: Table = table("lookups", &["source", "find"], Some("row"), false);
const SIZES: Table = table("sizes", &["plan_key"], Some("row"), false);

const fn table(
    name: &'static str,
    keys: &'static [&'static str],
    value: Option<&'static str>,
    digest: bool,
) -> Table {
    Table {
        name,
        keys,
        value,
        digest,
    }
}

/// Every table, in the order a snapshot reads them.
const TABLES_OF_STATE: [&Table; 12] = [
    &META,
    &OUTPUTS,
    &REPLACED,
    &LISTED,
    &DAMAGED,
    &TRUSTED,
    &FACTS,
    &ALIGNMENTS,
    &MIXES,
    &FAILURES,
    &LOOKUPS,
    &SIZES,
];

/// One row: its key columns' text, and its value's.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    keys: Vec<String>,
    value: Option<String>,
}

impl Row {
    fn new(keys: Vec<String>, value: Option<String>) -> Self {
        Self { keys, value }
    }

    /// Its key, its columns' text joined by a NUL, which no key holds.
    fn key(&self) -> String {
        self.keys.join("\0")
    }

    /// A digest of its value's text.
    fn digest(&self) -> [u8; 16] {
        let whole = Sha256::digest(self.value.as_deref().unwrap_or_default());
        let mut short = [0; 16];
        short.copy_from_slice(&whole[..16]);
        short
    }
}

/// The digest of each row a snapshot read or a save wrote, by its
/// table and key: a save writes the rows whose digest differs, and
/// deletes those of these the state no longer holds.
#[derive(Debug, Clone, Default)]
pub(crate) struct Loaded(BTreeMap<(&'static str, String), [u8; 16]>, i64);

fn json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// `text` read as a `T` whose JSON is a string, as a key is.
fn key_of<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(text.to_string())).ok()
}

/// The tables a step without the run's lock keeps what it measured in.
const CACHES: [&Table; 5] = [&FACTS, &ALIGNMENTS, &MIXES, &FAILURES, &SIZES];

/// The state the rows `read`, by table, hold; a table not read is empty.
/// A cache row an earlier version wrote in another shape is dropped; an
/// output that does not read refuses the store at `file`.
fn state_of(read: &BTreeMap<&'static str, Vec<Row>>, file: &Path) -> Result<State> {
    let parse = |t: &Table| read.get(t.name).into_iter().flatten();
    let value = |r: &Row| r.value.clone().unwrap_or_default();
    let mut state = State {
        version: VERSION,
        ..State::default()
    };
    for row in parse(&META) {
        if row.keys[0] == "library" {
            state.library = serde_json::from_str(&value(row)).ok();
        }
    }
    for row in parse(&OUTPUTS) {
        let refused = || {
            anyhow::anyhow!(
                "{} cannot be read; move it aside to start over, which forgets which library files are muman's",
                file.display()
            )
        };
        let path = crate::relpath::recorded(&row.keys[0]).map_err(|_| refused())?;
        let written = serde_json::from_str(&value(row)).map_err(|_| refused())?;
        state.outputs.insert(path, written);
    }
    for row in parse(&REPLACED) {
        if let (Some(old), Some(new)) = (key_of(&row.keys[0]), key_of(&value(row))) {
            state.replaced.insert(old, new);
        }
    }
    state.listed = parse(&LISTED).filter_map(|r| key_of(&r.keys[0])).collect();
    for row in parse(&DAMAGED) {
        if let Some(k) = key_of(&row.keys[0]) {
            state.damaged.insert(k, value(row));
        }
    }
    for row in parse(&TRUSTED) {
        state.trusted.insert(row.keys[0].clone(), value(row));
    }
    for row in parse(&FACTS) {
        if let (Some(k), Ok(f)) = (key_of(&row.keys[0]), serde_json::from_str(&value(row))) {
            state.facts.insert(k, f);
        }
    }
    state.alignments = parse(&ALIGNMENTS)
        .filter_map(|r| serde_json::from_str(&value(r)).ok())
        .collect();
    state.mixes = parse(&MIXES)
        .filter_map(|r| serde_json::from_str(&value(r)).ok())
        .collect();
    for row in parse(&FAILURES) {
        if let (Some(k), Ok(f)) = (key_of(&row.keys[0]), serde_json::from_str(&value(row))) {
            state.failures.insert(k, f);
        }
    }
    state.lookups = parse(&LOOKUPS)
        .filter_map(|r| serde_json::from_str(&value(r)).ok())
        .collect();
    for row in parse(&SIZES) {
        if let Ok(m) = serde_json::from_str(&value(row)) {
            state.sizes.insert(row.keys[0].clone(), m);
        }
    }
    Ok(state)
}

/// The renamings recorded, in the order made, with each one's place; one
/// an earlier version wrote in another shape is dropped.
fn renamings_in(tx: &Transaction<'_>) -> Result<Vec<(i64, SourceKey, Facts, Facts)>> {
    let mut statement = tx.prepare("SELECT seq, key, old, new FROM renamings ORDER BY seq")?;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut renamings = Vec::new();
    for row in rows {
        let (seq, key, old, new) = row?;
        if let (Some(key), Ok(old), Ok(new)) = (
            key_of(&key),
            serde_json::from_str(&old),
            serde_json::from_str(&new),
        ) {
            renamings.push((seq, key, old, new));
        }
    }
    Ok(renamings)
}

/// The rows `state` is stored as, table by table: what muman keeps
/// first, then its caches.
fn rows_of(state: &State) -> Result<Vec<(&'static Table, Vec<Row>)>> {
    let key = |k: &SourceKey| k.to_string();
    let mut meta = Vec::new();
    if let Some(library) = &state.library {
        meta.push(Row::new(vec!["library".into()], Some(json(library)?)));
    }
    let mut tables = vec![(&META, meta)];
    tables.push((
        &OUTPUTS,
        state
            .outputs
            .iter()
            .map(|(p, w)| {
                Ok(Row::new(
                    vec![crate::relpath::to_portable(p)],
                    Some(json(w)?),
                ))
            })
            .collect::<Result<_>>()?,
    ));
    tables.push((
        &REPLACED,
        state
            .replaced
            .iter()
            .map(|(old, new)| Row::new(vec![key(old)], Some(key(new))))
            .collect(),
    ));
    tables.push((
        &LISTED,
        state
            .listed
            .iter()
            .map(|k| Row::new(vec![key(k)], None))
            .collect(),
    ));
    tables.push((
        &DAMAGED,
        state
            .damaged
            .iter()
            .map(|(k, audio)| Row::new(vec![key(k)], Some(audio.clone())))
            .collect(),
    ));
    tables.push((
        &TRUSTED,
        state
            .trusted
            .iter()
            .map(|(name, digest)| Row::new(vec![name.clone()], Some(digest.clone())))
            .collect(),
    ));
    tables.extend(cache_rows(state)?);
    Ok(tables)
}

/// The rows `state`'s caches are stored as, table by table.
fn cache_rows(state: &State) -> Result<Vec<(&'static Table, Vec<Row>)>> {
    let key = |k: &SourceKey| k.to_string();
    let mut tables = Vec::new();
    tables.push((
        &FACTS,
        state
            .facts
            .iter()
            .map(|(k, f)| Ok(Row::new(vec![key(k)], Some(json(f)?))))
            .collect::<Result<_>>()?,
    ));
    tables.push((
        &ALIGNMENTS,
        state
            .alignments
            .iter()
            .map(|a| {
                let keys = vec![a.revs.0.clone(), a.revs.1.clone(), a.method.clone()];
                Ok(Row::new(keys, Some(json(a)?)))
            })
            .collect::<Result<_>>()?,
    ));
    tables.push((
        &MIXES,
        state
            .mixes
            .iter()
            .map(|m| {
                let layout = serde_json::to_value(m.layout)?;
                let layout = layout.as_str().unwrap_or_default().to_string();
                Ok(Row::new(vec![m.audio.clone(), layout], Some(json(m)?)))
            })
            .collect::<Result<_>>()?,
    ));
    tables.push((
        &FAILURES,
        state
            .failures
            .iter()
            .map(|(k, f)| Ok(Row::new(vec![key(k)], Some(json(f)?))))
            .collect::<Result<_>>()?,
    ));
    tables.push((
        &LOOKUPS,
        state
            .lookups
            .iter()
            .map(|l| Ok(Row::new(vec![key(&l.from), l.find.clone()], Some(json(l)?))))
            .collect::<Result<_>>()?,
    ));
    tables.push((
        &SIZES,
        state
            .sizes
            .iter()
            .map(|(k, m)| Ok(Row::new(vec![k.clone()], Some(json(m)?))))
            .collect::<Result<_>>()?,
    ));
    Ok(tables)
}

/// Every row of `table`, in the order it was last written.
fn rows_in(tx: &Transaction<'_>, table: &Table) -> Result<Vec<Row>> {
    let columns: Vec<&str> = table.keys.iter().copied().chain(table.value).collect();
    let mut select = tx.prepare(&format!(
        "SELECT {} FROM {} ORDER BY rowid",
        columns.join(", "),
        table.name
    ))?;
    let keys = table.keys.len();
    let rows = select
        .query_map([], |r| {
            let mut texts = Vec::with_capacity(columns.len());
            for n in 0..columns.len() {
                texts.push(r.get::<_, String>(n)?);
            }
            let value = (texts.len() > keys).then(|| texts.pop().unwrap_or_default());
            Ok(Row::new(texts, value))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

impl Db {
    /// The state the store holds, read in one snapshot, and what was read.
    /// A cache row an earlier version wrote in another shape is dropped,
    /// as `state.json` drops one; an output that does not read refuses
    /// the store, since losing it would lose which library files are
    /// muman's.
    pub(crate) fn snapshot(&mut self) -> Result<(State, Loaded)> {
        let file = self.file.clone();
        let (read, renamings) = self.read(|tx| {
            let read = TABLES_OF_STATE
                .iter()
                .map(|t| Ok((t.name, rows_in(tx, t)?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            Ok((read, renamings_in(tx)?))
        })?;
        let mut loaded = Loaded::default();
        for (name, rows) in &read {
            for row in rows {
                loaded.0.insert((name, row.key()), row.digest());
            }
        }
        let mut state = state_of(&read, &file)?;
        for (seq, key, old, new) in renamings {
            state.rebase(&key, &old, &new);
            loaded.1 = seq;
        }
        Ok((state, loaded))
    }

    /// Keep what a step `measured` without the run's lock in the caches as
    /// they are now, in one transaction that reads and writes theirs
    /// alone: a fact replaces the one kept, a failure the step cleared is
    /// cleared, and a comparison or size is recorded by its key. A fact
    /// that names a source's parts anew renames the comparisons here, and
    /// is recorded among the renamings that every load applies to the
    /// kept plans until a whole save writes them renamed: what the library
    /// holds is left to the run that holds its lock.
    pub(crate) fn keep(&mut self, measured: &Caches) -> Result<()> {
        let file = self.file.clone();
        self.write(|tx| {
            let read = CACHES
                .iter()
                .map(|t| Ok((t.name, rows_in(tx, t)?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            let mut state = state_of(&read, &file)?;
            for (key, old, new) in state.merge_measured(measured) {
                tx.execute(
                    "INSERT INTO renamings (key, old, new) VALUES (?1, ?2, ?3)",
                    rusqlite::params![key.to_string(), json(&old)?, json(&new)?],
                )?;
            }
            for (table, rows) in rows_of(&state)? {
                let Some(was) = read.get(table.name) else {
                    continue;
                };
                let was: BTreeMap<String, [u8; 16]> =
                    was.iter().map(|r| (r.key(), r.digest())).collect();
                let mut now = BTreeSet::new();
                for row in rows {
                    let digest = row.digest();
                    if was.get(&row.key()) != Some(&digest) {
                        put_row(tx, table, &row, &digest)?;
                    }
                    now.insert(row.key());
                }
                for key in was.keys().filter(|k| !now.contains(*k)) {
                    delete_row(tx, table, key)?;
                }
            }
            Ok(())
        })
    }

    /// Write the rows of `state` whose digest differs from `loaded`'s, and
    /// delete the rows `loaded` holds that `state` does not, in one
    /// transaction; nothing is written when nothing differs. A row
    /// another process wrote that `loaded` never held stays. Returns
    /// whether anything was written.
    pub(crate) fn save_diff(&mut self, state: &State, loaded: &mut Loaded) -> Result<bool> {
        let tables = rows_of(state)?;
        let mut upserts: Vec<(&Table, Row, [u8; 16])> = Vec::new();
        let mut held: BTreeSet<(&'static str, String)> = BTreeSet::new();
        for (table, rows) in tables {
            for row in rows {
                let key = (table.name, row.key());
                let digest = row.digest();
                if loaded.0.get(&key) != Some(&digest) {
                    upserts.push((table, row, digest));
                }
                held.insert(key);
            }
        }
        let gone: Vec<(&'static str, String)> = loaded
            .0
            .keys()
            .filter(|k| !held.contains(*k))
            .cloned()
            .collect();
        let renamed = loaded.1;
        if upserts.is_empty() && gone.is_empty() && renamed == 0 {
            return Ok(false);
        }
        self.write(|tx| {
            // The plans written here hold the renamings the load applied.
            tx.execute("DELETE FROM renamings WHERE seq <= ?1", [renamed])?;
            for (table, row, digest) in &upserts {
                put_row(tx, table, row, digest)?;
            }
            for (name, key) in &gone {
                let Some(table) = TABLES_OF_STATE.iter().find(|t| t.name == *name) else {
                    continue;
                };
                delete_row(tx, table, key)?;
            }
            Ok(())
        })?;
        for (table, row, digest) in upserts {
            loaded.0.insert((table.name, row.key()), digest);
        }
        for key in gone {
            loaded.0.remove(&key);
        }
        loaded.1 = 0;
        Ok(true)
    }

    /// Write `state`'s output at `path` alone, or delete it where the
    /// state holds none, in a commit of its own, as [`Db::save_diff`]
    /// writes it among the rest: so a song written is recorded as it is,
    /// at a cost that does not grow with the library.
    pub(crate) fn save_output(
        &mut self,
        state: &State,
        path: &Path,
        loaded: &mut Loaded,
    ) -> Result<()> {
        let key = crate::relpath::to_portable(path);
        let row = match state.outputs.get(path) {
            Some(w) => Some(Row::new(vec![key.clone()], Some(json(w)?))),
            None => None,
        };
        let held = (OUTPUTS.name, key.clone());
        match &row {
            Some(row) if loaded.0.get(&held) == Some(&row.digest()) => return Ok(()),
            None if !loaded.0.contains_key(&held) => return Ok(()),
            _ => {}
        }
        self.write(|tx| match &row {
            Some(row) => put_row(tx, &OUTPUTS, row, &row.digest()),
            None => delete_row(tx, &OUTPUTS, &key),
        })?;
        match row {
            Some(row) => loaded.0.insert(held, row.digest()),
            None => loaded.0.remove(&held),
        };
        Ok(())
    }
}

/// Write `row` into `table`, over the row of its key.
fn put_row(tx: &Transaction<'_>, table: &Table, row: &Row, digest: &[u8; 16]) -> Result<()> {
    let mut columns: Vec<&str> = table.keys.to_vec();
    let mut values: Vec<rusqlite::types::Value> =
        row.keys.iter().map(|k| k.clone().into()).collect();
    if let Some(column) = table.value {
        columns.push(column);
        values.push(row.value.clone().unwrap_or_default().into());
    }
    if table.digest {
        columns.push("digest");
        values.push(digest.to_vec().into());
    }
    let marks = vec!["?"; columns.len()].join(", ");
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO {} ({}) VALUES ({marks})",
            table.name,
            columns.join(", ")
        ),
        rusqlite::params_from_iter(values),
    )?;
    Ok(())
}

/// Delete the row of `table` whose key is `key`, its columns joined by a
/// NUL.
fn delete_row(tx: &Transaction<'_>, table: &Table, key: &str) -> Result<()> {
    let matching: Vec<String> = table.keys.iter().map(|k| format!("{k} = ?")).collect();
    tx.execute(
        &format!(
            "DELETE FROM {} WHERE {}",
            table.name,
            matching.join(" AND ")
        ),
        rusqlite::params_from_iter(key.split('\0')),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    fn count(db: &mut Db, table: &str) -> i64 {
        db.read(|tx| Ok(tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?))
            .unwrap()
    }

    #[test]
    fn a_new_store_has_its_tables_and_one_opened_again_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        db.write(|tx| {
            Ok(tx.execute(
                "INSERT INTO listed VALUES ('manual:Lantern Weather.flac')",
                [],
            )?)
        })
        .unwrap();
        drop(db);
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        assert_eq!(count(&mut db, "listed"), 1);
        let version: i32 = db
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA);
    }

    #[test]
    fn a_newer_store_or_another_application_s_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path(), WAIT).unwrap();
        db.conn
            .pragma_update(None, "user_version", SCHEMA + 1)
            .unwrap();
        drop(db);
        let e = Db::open(dir.path(), WAIT).err().unwrap();
        assert!(
            format!("{e:#}").contains("newer than this muman's"),
            "{e:#}"
        );

        let other = tempfile::tempdir().unwrap();
        let conn = Connection::open(other.path().join(FILE)).unwrap();
        conn.execute_batch("CREATE TABLE songs (title TEXT)")
            .unwrap();
        drop(conn);
        let e = Db::open(other.path(), WAIT).err().unwrap();
        assert!(format!("{e:#}").contains("is not muman's state"), "{e:#}");
    }

    #[test]
    fn a_write_stopped_before_its_commit_leaves_the_store_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        let kill = crate::testing::kill_at(dir.path(), 1);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            db.write(|tx| {
                Ok(tx.execute("INSERT INTO listed VALUES ('manual:Low Orchard.flac')", [])?)
            })
        }));
        drop(kill);
        assert!(stopped.is_err(), "stopped at the commit");
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        assert_eq!(count(&mut db, "listed"), 0);
    }

    /// A state with a row in every table.
    fn every_row() -> State {
        use crate::state::{
            Aligned, Failure, Looked, Measured, Mix, Outcome, Sidecar, Step, Written,
        };
        let key = SourceKey::youtube("aaaaaaaaaaa");
        let manual = SourceKey::Manual("Lantern Weather.flac".into());
        let mut state = State {
            version: VERSION,
            library: Some(PathBuf::from("Music")),
            ..State::default()
        };
        state.outputs.insert(
            PathBuf::from("Marlo Venn/Harbor Lights/01 Lantern Weather.flac"),
            Written {
                sources: vec![key.clone()],
                sidecars: vec![Sidecar::at(
                    "Marlo Venn/Harbor Lights/01 Lantern Weather.lrc".into(),
                )],
                plan: Some(crate::testing::every_part(crate::resolve::Format::Copy {
                    codec: crate::codec::Codec::Flac,
                })),
                stamp: Some("10:20".into()),
                digest: Some("d".into()),
            },
        );
        state
            .replaced
            .insert(SourceKey::youtube("bbbbbbbbbbb"), key.clone());
        state.listed.push(manual.clone());
        state.damaged.insert(key.clone(), "a1".into());
        state.trusted.insert("hooks".into(), "t1".into());
        state
            .facts
            .insert(key.clone(), crate::facts::Facts::unreadable("10:20".into()));
        state.alignments.push(Aligned {
            a: key.clone(),
            b: manual.clone(),
            method: crate::align::METHOD.into(),
            revs: ("1".into(), "2".into()),
            offset_ms: 120,
            stretch_ppm: 0,
            score: 0.9,
            coverage: 0.8,
            a_ms: 1000,
            b_ms: 900,
            a_extra_ms: 100,
            a_lacks_ms: 0,
            b_lacks_ms: 0,
        });
        state.mixes.push(Mix {
            source: key.clone(),
            audio: "1".into(),
            layout: crate::codec::Layout::STEREO,
            filter: crate::codec::MIXING,
            analysis: crate::analysis::Analysis::default(),
        });
        state.failures.insert(
            manual.clone(),
            Failure {
                step: Step::Measure,
                rev: Some("10:20".into()),
                error: "unreadable".into(),
                count: 2,
                last: 1_000,
            },
        );
        state.lookups.push(Looked {
            from: key.clone(),
            find: "lrclib".into(),
            method: "m".into(),
            at: 1_000,
            outcome: Outcome::Nothing,
            unchanged: 1,
        });
        state.sizes.insert(
            "0123456789abcdef".into(),
            Measured {
                source: key,
                audio: 4_000_000,
                beside: vec![120],
            },
        );
        state
    }

    #[test]
    fn a_state_saved_reads_back_whole_and_a_save_of_it_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        let state = every_row();
        assert!(db.save_diff(&state, &mut Loaded::default()).unwrap());
        let (back, mut loaded) = db.snapshot().unwrap();
        assert_eq!(back, state);
        let before = db.conn.total_changes();
        assert!(
            !db.save_diff(&back, &mut loaded).unwrap(),
            "nothing differs"
        );
        assert_eq!(db.conn.total_changes(), before);
    }

    #[test]
    fn an_output_saved_alone_writes_its_row_and_a_whole_save_after_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        db.save_diff(&every_row(), &mut Loaded::default()).unwrap();
        let (mut state, mut loaded) = db.snapshot().unwrap();
        let path = PathBuf::from("Marlo Venn/Harbor Lights/02 Paper Comets.flac");
        let mut written = state.outputs.values().next().unwrap().clone();
        written.stamp = Some("10:40".into());
        state.outputs.insert(path.clone(), written);
        state.facts.clear();

        let before = db.conn.total_changes();
        db.save_output(&state, &path, &mut loaded).unwrap();
        assert_eq!(db.conn.total_changes() - before, 1, "its row alone");
        db.save_output(&state, &path, &mut loaded).unwrap();
        assert_eq!(db.conn.total_changes() - before, 1, "as it was, nothing");
        let (back, _) = db.snapshot().unwrap();
        assert_eq!(back.outputs, state.outputs);
        assert!(!back.facts.is_empty(), "the facts left as they were");

        state.outputs.remove(&path);
        db.save_output(&state, &path, &mut loaded).unwrap();
        assert_eq!(db.conn.total_changes() - before, 2, "its row deleted");
        let (back, _) = db.snapshot().unwrap();
        assert_eq!(back.outputs, state.outputs);
        let before = db.conn.total_changes();
        assert!(db.save_diff(&state, &mut loaded).unwrap(), "the facts go");
        assert_eq!(
            db.conn.total_changes() - before,
            u64::try_from(every_row().facts.len()).unwrap(),
            "the output's row not written twice"
        );
    }

    #[test]
    fn a_save_writes_the_rows_that_changed_and_leaves_another_s_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        db.save_diff(&every_row(), &mut Loaded::default()).unwrap();
        let (mut state, mut loaded) = db.snapshot().unwrap();

        let mut other = Db::open(dir.path(), WAIT).unwrap();
        other
            .write(
                |tx| Ok(tx.execute("INSERT INTO listed VALUES ('manual:Low Orchard.flac')", [])?),
            )
            .unwrap();

        let key = SourceKey::youtube("aaaaaaaaaaa");
        state.facts.get_mut(&key).unwrap().rev = "10:30".into();
        state.trusted.clear();
        let before = db.conn.total_changes();
        assert!(db.save_diff(&state, &mut loaded).unwrap());
        assert_eq!(
            db.conn.total_changes() - before,
            2,
            "one fact written, one trust deleted"
        );
        let (back, _) = db.snapshot().unwrap();
        assert_eq!(back.facts[&key].rev, "10:30");
        assert!(back.trusted.is_empty());
        assert_eq!(
            back.listed,
            [
                SourceKey::Manual("Lantern Weather.flac".into()),
                SourceKey::Manual("Low Orchard.flac".into())
            ],
            "the other's row stays"
        );
    }

    #[test]
    fn a_cache_row_of_another_shape_is_dropped_and_an_output_that_does_not_read_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(dir.path(), WAIT).unwrap();
        db.save_diff(&every_row(), &mut Loaded::default()).unwrap();
        db.write(|tx| {
            Ok(tx.execute(
                "INSERT INTO facts VALUES ('youtube:bbbbbbbbbbb', '{\"rev\": 3}', x'00')",
                [],
            )?)
        })
        .unwrap();
        let (state, _) = db.snapshot().unwrap();
        assert_eq!(state.facts.len(), 1, "the old shape's fact is made again");
        assert_eq!(state.outputs.len(), 1);

        db.write(|tx| Ok(tx.execute("UPDATE outputs SET row = '{}'", [])?))
            .unwrap();
        let e = db.snapshot().err().unwrap();
        assert!(
            format!("{e:#}").contains("which library files are muman's"),
            "{e:#}"
        );
    }
}
