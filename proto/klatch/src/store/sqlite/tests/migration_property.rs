//! Every older schema version migrates to the current one with its data intact (#603).
//!
//! A database "at version N" is built the only way it can be without freezing 32 fixture files: start from the
//! prototype schema (version 0, written out below) and run the first N migrations. The schema of each version is
//! then read back with `PRAGMA table_info`, every table is filled with generated rows through raw SQL, and the
//! database is migrated to the current version. Two things must hold: every row survives with the value of every
//! column it had at version N (all migrations are additive), and the migrated schema has the same tables and columns
//! as a fresh database (plus the legacy columns the prototype had).
//!
//! The version-N schema comes from the migrations themselves, so this does not catch a migration that is wrong
//! about what older databases looked like; the frozen real databases in `fixtures/store` and the hand-built cases in
//! `migrations.rs` cover that for a few versions. What this adds is that no migration, at any version, loses or
//! rewrites existing data, and that the steps compose.

use super::super::schema::{MIGRATIONS, SCHEMA, migrate};
use super::*;
use rusqlite::types::Value;
use std::collections::BTreeSet;

/// The schema of the original prototype (version 0), the start of every chain.
const PROTOTYPE: &str = "CREATE TABLE members(room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_id INTEGER DEFAULT 0, PRIMARY KEY(room,name));
 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, sender TEXT, body TEXT, ts INTEGER);
 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, path TEXT, owner TEXT, reason TEXT, expires INTEGER);";

/// Columns the prototype had that a fresh database no longer creates; migrating keeps them (nothing drops a column).
const LEGACY_COLUMNS: [(&str, &str); 1] = [("members", "last_id")];

type Rows = Vec<Vec<String>>;

fn tables(c: &Connection) -> Vec<String> {
    let mut st = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    st.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}

/// (name, declared type) of each column of a table.
fn columns(c: &Connection, table: &str) -> Vec<(String, String)> {
    let mut st = c.prepare(&format!("PRAGMA table_info({table})")).unwrap();
    st.query_map([], |r| Ok((r.get(1)?, r.get::<_, String>(2)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}

/// A database at schema version `version`, built by running the first `version` migrations over the prototype.
fn database_at(version: usize) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(PROTOTYPE).unwrap();
    for step in &MIGRATIONS[..version] {
        step(&c).unwrap();
    }
    c
}

/// Fill every table with `count` rows. Every value is distinct per row, so primary keys and unique indexes hold.
fn fill(c: &Connection, count: usize, seed: u64) {
    for table in tables(c) {
        let cols = columns(c, &table);
        let names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
        let marks = vec!["?"; cols.len()].join(",");
        let sql = format!("INSERT INTO {table}({}) VALUES({marks})", names.join(","));
        for i in 0..count {
            let n = (seed % 997) as i64 + i as i64 * 1000;
            let values: Vec<Value> = cols
                .iter()
                .enumerate()
                .map(|(k, (name, ty))| match ty.to_uppercase().as_str() {
                    "INTEGER" => Value::Integer(n + k as i64),
                    "BLOB" => Value::Blob(format!("{name}{n}").into_bytes()),
                    _ => Value::Text(format!("{name}-{n}")),
                })
                .collect();
            c.execute(&sql, rusqlite::params_from_iter(values)).unwrap();
        }
    }
}

/// Every row of `table` restricted to `cols`, rendered and sorted so the order of rows does not matter.
fn project(c: &Connection, table: &str, cols: &[String]) -> Rows {
    let mut st = c
        .prepare(&format!("SELECT {} FROM {table}", cols.join(",")))
        .unwrap();
    let mut rows: Rows = st
        .query_map([], |r| {
            (0..cols.len())
                .map(|i| r.get::<_, Value>(i).map(|v| format!("{v:?}")))
                .collect()
        })
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    rows.sort();
    rows
}

fn schema_of(c: &Connection) -> BTreeSet<(String, String, String)> {
    tables(c)
        .into_iter()
        .flat_map(|t| {
            columns(c, &t)
                .into_iter()
                .map(move |(n, ty)| (t.clone(), n, ty))
        })
        .collect()
}

/// Build version `version`, fill it, migrate it to the current version and check nothing was lost.
fn migrates_with_data_intact(version: usize, rows: usize, seed: u64) {
    let c = database_at(version);
    fill(&c, rows, seed);
    let before: Vec<(String, Vec<String>, Rows)> = tables(&c)
        .into_iter()
        .map(|t| {
            let cols: Vec<String> = columns(&c, &t).into_iter().map(|(n, _)| n).collect();
            let data = project(&c, &t, &cols);
            (t, cols, data)
        })
        .collect();

    migrate(&c, version as i64).unwrap();

    for (table, cols, data) in &before {
        assert_eq!(
            &project(&c, table, cols),
            data,
            "migrating from version {version} changed the rows of {table}"
        );
    }
    let fresh = Connection::open_in_memory().unwrap();
    fresh.execute_batch(SCHEMA).unwrap();
    let (migrated, fresh) = (schema_of(&c), schema_of(&fresh));
    let missing: Vec<_> = fresh.difference(&migrated).collect();
    let extra: Vec<_> = migrated.difference(&fresh).collect();
    assert!(
        missing.is_empty(),
        "a database migrated from version {version} lacks what a fresh one has: {missing:?}"
    );
    assert_eq!(
        extra
            .iter()
            .map(|(t, n, _)| (t.as_str(), n.as_str()))
            .collect::<Vec<_>>(),
        LEGACY_COLUMNS,
        "a database migrated from version {version} has columns a fresh one does not"
    );
}

#[test]
fn the_chain_of_migrations_has_one_step_per_older_version() {
    assert_eq!(MIGRATIONS.len() as i64, SCHEMA_VERSION);
}

/// SCHEMA_VERSION is the length of the list, so a merge of two PRs that each added a step with the same number (a
/// union merge keeps both lines) must fail here, not pass quietly (#1070): the list must read migrate_v0, migrate_v1,
/// and so on, each once and in order.
#[test]
fn the_list_names_each_step_once_in_order() {
    let source = include_str!("../schema.rs");
    let start = source.find("const MIGRATIONS").expect("the list");
    let listed: Vec<&str> = source[start..]
        .lines()
        .skip(1)
        .take_while(|l| l.trim() != "];")
        .map(str::trim)
        .collect();
    let want: Vec<String> = (0..listed.len())
        .map(|n| format!("migrate_v{n},"))
        .collect();
    assert_eq!(
        listed, want,
        "MIGRATIONS must list migrate_v0.. once each, in order"
    );
    assert_eq!(listed.len() as i64, SCHEMA_VERSION);
}

#[test]
fn every_older_version_migrates_to_the_current_one_with_its_data_intact() {
    for version in 0..MIGRATIONS.len() {
        migrates_with_data_intact(version, 3, version as u64);
    }
}

#[test]
fn an_empty_database_at_every_older_version_migrates_to_the_current_schema() {
    for version in 0..MIGRATIONS.len() {
        migrates_with_data_intact(version, 0, 0);
    }
}

/// Generated version, row count and values on top of the exhaustive tests above. Cheap (an in-memory database per
/// case), so it stays inside the gate's property budget.
#[hegel::test]
fn a_database_of_any_older_version_migrates_with_generated_data_intact(tc: hegel::TestCase) {
    use hegel::generators as gs;
    let version = tc.draw(gs::integers::<usize>().max_value(MIGRATIONS.len() - 1));
    let rows = tc.draw(gs::integers::<usize>().max_value(6));
    let seed = tc.draw(gs::integers::<u64>());
    migrates_with_data_intact(version, rows, seed);
}
