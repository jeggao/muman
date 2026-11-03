//! The home's state store, `state.db`: one SQLite file the state is to
//! move into from `state.json`, opened here, and written only through
//! [`Db::write`]. Nothing reads or writes it yet.
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

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, Transaction, TransactionBehavior};

pub(crate) const FILE: &str = "state.db";

/// The store's `application_id`, `muSt`.
const APPLICATION_ID: i32 = i32::from_be_bytes(*b"muSt");

/// The schema this muman writes: 1 holds each row of the state as JSON.
pub(crate) const SCHEMA: i32 = 1;

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
        let file = home.join(FILE);
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
                tx.pragma_update(None, "application_id", APPLICATION_ID)?;
                tx.pragma_update(None, "user_version", SCHEMA)?;
                Ok(())
            })
            .with_context(|| format!("making {}", db.file.display()))?;
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
}
