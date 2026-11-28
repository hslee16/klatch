//! The schema a migrated store ends up with equals the schema of a fresh one (#1780, #1785).
//!
//! Three things are compared, for a database migrated from every older version and from the `SCHEMA` constant a fresh store
//! was created from at each version (frozen in `schema_history/`, one file per version): every index, trigger and view
//! by its normalised sql, and every table by its facts (each column's declared type, NOT NULL, default, primary key
//! position and hidden flag; the primary key and UNIQUE constraints by their columns; foreign keys). The comparison is
//! checked by self-tests that feed it deliberately wrong schemas, and by running it against wrong migrations.
//!
//! Column order is not compared: `ALTER TABLE ADD COLUMN` appends, so a migrated table can never list its columns in a
//! fresh table's order, and no query reads a column by position.

use super::super::schema::{MIGRATIONS, SCHEMA, migrate};
use super::*;
use std::collections::BTreeSet;

/// Lower-case the sql outside quoted strings, collapse whitespace outside them and drop `IF NOT EXISTS` (a migration
/// says it, the fresh schema does not). A string literal is kept exactly: `'allow'` and `'ALLOW'` differ.
pub(super) fn normalise_sql(sql: &str) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut pending_space = false;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    out.push(chars.next().unwrap());
                } else {
                    in_string = false;
                }
            }
        } else if c.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.extend(c.to_lowercase());
            in_string = c == '\'';
        }
    }
    out.replace(" if not exists", "")
}

/// Every index, trigger and view as (type, name, table, normalised sql). Automatic indexes have no sql and follow from
/// the table's keys, which `table_facts` compares.
pub(super) fn objects_of(c: &Connection) -> BTreeSet<(String, String, String, String)> {
    let mut st = c
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('index','trigger','view') AND sql IS NOT NULL")
        .unwrap();
    st.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            normalise_sql(&r.get::<_, String>(3)?),
        ))
    })
    .unwrap()
    .collect::<std::result::Result<_, _>>()
    .unwrap()
}

/// What each table is, as one line per fact, so two databases compare as sets: `members.col.room.notnull=0`,
/// `members.unique(room,name)`, `x.fk(a->t.b,no action,no action)`.
pub(super) fn table_facts(c: &Connection) -> BTreeSet<String> {
    let mut facts = BTreeSet::new();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for t in tables {
        let mut st = c.prepare(&format!("PRAGMA table_xinfo({t})")).unwrap();
        let cols = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .unwrap();
        for col in cols {
            let (name, ty, notnull, default, pk, hidden) = col.unwrap();
            let p = format!("{t}.col.{name}");
            facts.insert(format!("{p}.type={}", ty.to_lowercase()));
            facts.insert(format!("{p}.notnull={notnull}"));
            facts.insert(format!(
                "{p}.default={}",
                default.unwrap_or_else(|| "NULL".into())
            ));
            facts.insert(format!("{p}.pk={pk}"));
            facts.insert(format!("{p}.hidden={hidden}"));
        }
        let mut st = c.prepare(&format!("PRAGMA index_list({t})")).unwrap();
        let indexes: Vec<(String, i64, String)> = st
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for (index, unique, origin) in indexes {
            // `c` is a CREATE INDEX, compared by its sql; `u` and `pk` are constraints of the table
            if origin == "c" {
                continue;
            }
            let mut st = c.prepare(&format!("PRAGMA index_info({index})")).unwrap();
            let cols: Vec<String> = st
                .query_map([], |r| r.get::<_, String>(2))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            facts.insert(format!(
                "{t}.{}({})unique={unique}",
                if origin == "pk" { "pk" } else { "unique" },
                cols.join(",")
            ));
        }
        let mut st = c.prepare(&format!("PRAGMA foreign_key_list({t})")).unwrap();
        let fks = st
            .query_map([], |r| {
                Ok(format!(
                    "{t}.fk({}->{}.{},{},{})",
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?
                ))
            })
            .unwrap();
        for fk in fks {
            facts.insert(fk.unwrap());
        }
    }
    facts
}

/// The differences between a database and a fresh one: facts and objects found on one side only, as printable lines
/// (`-` only in the fresh database, `+` only in this one).
pub(super) fn differences(c: &Connection, fresh: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    let (a, b) = (table_facts(c), table_facts(fresh));
    out.extend(b.difference(&a).map(|f| format!("- {f}")));
    out.extend(a.difference(&b).map(|f| format!("+ {f}")));
    let (a, b) = (objects_of(c), objects_of(fresh));
    out.extend(
        b.difference(&a)
            .map(|o| format!("- {} {} on {}: {}", o.0, o.1, o.2, o.3)),
    );
    out.extend(
        a.difference(&b)
            .map(|o| format!("+ {} {} on {}: {}", o.0, o.1, o.2, o.3)),
    );
    out
}

fn fresh_store() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(SCHEMA).unwrap();
    c
}

/// What a store that started at version 0 (the prototype) differs in from a fresh one, and why. A prefix of a fact (the
/// part before the value), so it covers the fresh value and the migrated one. An entry that no database matches fails
/// `an_allowed_difference_that_no_database_shows_is_removed`, so this list cannot outlive what it excuses.
const VERSION_0_DIFFERENCES: [(&str, &str); 4] = [
    (
        "messages.col.seq.notnull=",
        "the prototype's messages table had no seq; the migration adds it nullable and fills it, and SQLite cannot add NOT NULL to a column afterwards",
    ),
    (
        "messages.col.room.notnull=",
        "the prototype declared room without NOT NULL and a table cannot be altered to add it",
    ),
    (
        "messages.col.mid.notnull=",
        "added by a migration as a nullable column and filled in; NOT NULL cannot be added afterwards",
    ),
    (
        "members.col.last_id.",
        "the prototype's cursor column, kept by migration because nothing drops a column",
    ),
];

/// The prototype schema (version 0) is built by `migration_property::database_at`; replay it from there.
fn chain_differences(version: usize) -> Vec<String> {
    let c = super::migration_property::database_at(version);
    migrate(&c, version as i64).unwrap();
    differences(&c, &fresh_store())
}

fn allowed(diff: &str) -> Option<usize> {
    let fact = diff.trim_start_matches(['-', '+', ' ']);
    VERSION_0_DIFFERENCES
        .iter()
        .position(|(prefix, _)| fact.starts_with(prefix))
}

#[test]
fn every_older_version_migrates_to_the_same_schema_as_a_fresh_database_but_for_the_documented_differences()
 {
    for version in 0..MIGRATIONS.len() {
        let bad: Vec<String> = chain_differences(version)
            .into_iter()
            .filter(|d| allowed(d).is_none())
            .collect();
        assert!(
            bad.is_empty(),
            "a database migrated from version {version} differs from a fresh one: {bad:#?}"
        );
    }
}

#[test]
fn an_allowed_difference_that_no_database_shows_is_removed() {
    let mut used = BTreeSet::new();
    for version in 0..MIGRATIONS.len() {
        used.extend(chain_differences(version).iter().filter_map(|d| allowed(d)));
    }
    for (n, (prefix, why)) in VERSION_0_DIFFERENCES.iter().enumerate() {
        assert!(
            used.contains(&n),
            "the allowed difference {prefix} ({why}) matched nothing: remove it"
        );
    }
}

/// The SCHEMA constant of every store schema version, as it stood in main when that version was current.
const HISTORY: [(usize, &str); 47] = [
    (1, include_str!("schema_history/v01.sql")),
    (2, include_str!("schema_history/v02.sql")),
    (3, include_str!("schema_history/v03.sql")),
    (4, include_str!("schema_history/v04.sql")),
    (5, include_str!("schema_history/v05.sql")),
    (6, include_str!("schema_history/v06.sql")),
    (7, include_str!("schema_history/v07.sql")),
    (8, include_str!("schema_history/v08.sql")),
    (9, include_str!("schema_history/v09.sql")),
    (10, include_str!("schema_history/v10.sql")),
    (11, include_str!("schema_history/v11.sql")),
    (12, include_str!("schema_history/v12.sql")),
    (13, include_str!("schema_history/v13.sql")),
    (14, include_str!("schema_history/v14.sql")),
    (15, include_str!("schema_history/v15.sql")),
    (16, include_str!("schema_history/v16.sql")),
    (17, include_str!("schema_history/v17.sql")),
    (18, include_str!("schema_history/v18.sql")),
    (19, include_str!("schema_history/v19.sql")),
    (20, include_str!("schema_history/v20.sql")),
    (21, include_str!("schema_history/v21.sql")),
    (22, include_str!("schema_history/v22.sql")),
    (23, include_str!("schema_history/v23.sql")),
    (24, include_str!("schema_history/v24.sql")),
    (25, include_str!("schema_history/v25.sql")),
    (26, include_str!("schema_history/v26.sql")),
    (27, include_str!("schema_history/v27.sql")),
    (28, include_str!("schema_history/v28.sql")),
    (29, include_str!("schema_history/v29.sql")),
    (30, include_str!("schema_history/v30.sql")),
    (31, include_str!("schema_history/v31.sql")),
    (32, include_str!("schema_history/v32.sql")),
    (33, include_str!("schema_history/v33.sql")),
    (34, include_str!("schema_history/v34.sql")),
    (35, include_str!("schema_history/v35.sql")),
    (36, include_str!("schema_history/v36.sql")),
    (37, include_str!("schema_history/v37.sql")),
    (38, include_str!("schema_history/v38.sql")),
    (39, include_str!("schema_history/v39.sql")),
    (40, include_str!("schema_history/v40.sql")),
    (41, include_str!("schema_history/v41.sql")),
    (42, include_str!("schema_history/v42.sql")),
    (43, include_str!("schema_history/v43.sql")),
    (44, include_str!("schema_history/v44.sql")),
    (45, include_str!("schema_history/v45.sql")),
    (46, include_str!("schema_history/v46.sql")),
    (47, include_str!("schema_history/v47.sql")),
];

#[test]
fn a_store_created_at_any_historic_version_migrates_to_exactly_the_schema_of_a_fresh_one() {
    assert_eq!(
        HISTORY.len(),
        MIGRATIONS.len(),
        "a version has no frozen schema: add its file (see the first line of any)"
    );
    for (version, schema) in HISTORY {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(schema).unwrap();
        migrate(&c, version as i64).unwrap();
        let bad = differences(&c, &fresh_store());
        assert!(
            bad.is_empty(),
            "a store created at version {version} differs from a fresh one after migrating: {bad:#?}"
        );
    }
}

/// A one-table database and a one-index database from sql, to feed the comparison wrong schemas on purpose.
fn db(sql: &str) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(sql).unwrap();
    c
}

#[test]
fn the_comparison_sees_each_kind_of_difference_it_exists_to_catch() {
    let base = "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('allow','redact');";
    assert!(differences(&db(base), &db(base)).is_empty());
    for (what, wrong) in [
        (
            "a dropped default",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL, b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('allow','redact');",
        ),
        (
            "a dropped NOT NULL",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT DEFAULT 'x', b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('allow','redact');",
        ),
        (
            "a changed default",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'y', b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('allow','redact');",
        ),
        (
            "a changed type",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b TEXT, UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('allow','redact');",
        ),
        (
            "a lost UNIQUE",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER); CREATE INDEX t_b ON t(b) WHERE a IN ('allow','redact');",
        ),
        (
            "a changed primary key",
            "CREATE TABLE t(id INTEGER, a TEXT NOT NULL DEFAULT 'x', b INTEGER, PRIMARY KEY(id,b), UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('allow','redact');",
        ),
        (
            "a changed index column",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(a) WHERE a IN ('allow','redact');",
        ),
        (
            "a unique index",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER, UNIQUE(a,b)); CREATE UNIQUE INDEX t_b ON t(b) WHERE a IN ('allow','redact');",
        ),
        (
            "a changed index where",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('allow');",
        ),
        (
            "a removed index where",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(b);",
        ),
        (
            "a case change inside an index literal",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER, UNIQUE(a,b)); CREATE INDEX t_b ON t(b) WHERE a IN ('ALLOW','redact');",
        ),
        (
            "a missing index",
            "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT NOT NULL DEFAULT 'x', b INTEGER, UNIQUE(a,b));",
        ),
    ] {
        assert!(
            !differences(&db(wrong), &db(base)).is_empty(),
            "{what} went unseen"
        );
    }
}

#[test]
fn a_string_literal_keeps_its_case_in_a_trigger_and_a_view_and_sql_outside_it_does_not() {
    let t = |lit: &str, kw: &str| {
        format!(
            "CREATE TABLE t(a TEXT); CREATE {kw} TRIGGER g AFTER INSERT ON t WHEN new.a = '{lit}' BEGIN SELECT 1; END; CREATE VIEW v AS SELECT a FROM t WHERE a = '{lit}';"
        )
    };
    assert!(differences(&db(&t("allow", "temp")), &db(&t("allow", "temp"))).is_empty());
    assert!(!differences(&db(&t("ALLOW", "temp")), &db(&t("allow", "temp"))).is_empty());
    let (a, b) = (db(&t("it''s", "temp")), db(&t("It''s", "temp")));
    assert!(
        !differences(&a, &b).is_empty(),
        "an escaped quote ended the literal"
    );
    // keywords, spacing and IF NOT EXISTS are not differences
    assert_eq!(
        normalise_sql("CREATE INDEX IF NOT EXISTS  X ON T(A)\n WHERE A='Q'"),
        normalise_sql("create index x on t(a) where a='Q'")
    );
}
