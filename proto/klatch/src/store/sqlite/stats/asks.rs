//! The ask section of the usage report (#1377): asks created in the window, answered, closed, scoped and referenced.
//! The only place a message body is read: at most its first 48 characters, matched and dropped.

use super::{LIVE, has, one, rows};
use crate::domain::{
    AskCounts, StatsOptions, answer_target, closed_target, is_ask, median_and_p90, vocab,
};
use crate::domain::{Counts, Result};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The characters of a body the ask fold may read.
const ASK_PREFIX_CHARS: usize = 48;

/// An ask is named by its room and the sequence number of its message.
type AskKey = (String, i64);

/// One message that starts with an ask word: where it is, who sent it, when, under which label, and its first words.
struct AskWord {
    room: String,
    seq: i64,
    sender: String,
    ts: i64,
    label: String,
    text: String,
}

fn ask_words(conn: &Connection, o: &StatsOptions) -> Result<Vec<AskWord>> {
    rows(
        conn,
        &format!(
            "SELECT room,seq,sender,ts,label,substr(body,1,{ASK_PREFIX_CHARS}) FROM messages \
             WHERE kind IS NOT 'archived' AND (?1 IS NULL OR room=?1) AND ?2>=0 \
             AND (body LIKE 'seeking:%' OR body LIKE 'answer to #%' OR body LIKE 'closed #%') ORDER BY room,seq"
        ),
        o,
        |r| {
            Ok(AskWord {
                room: r.get(0)?,
                seq: r.get(1)?,
                sender: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ts: r.get(3)?,
                label: r.get(4)?,
                text: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            })
        },
    )
}

/// The asks created in the window, each with its asker and its time.
fn created_asks(
    words: &[AskWord],
    sealed: &HashSet<String>,
    since: i64,
) -> HashMap<AskKey, (String, i64)> {
    words
        .iter()
        .filter(|w| !sealed.contains(&w.label) && is_ask(&w.text) && w.ts >= since)
        .map(|w| ((w.room.clone(), w.seq), (w.sender.clone(), w.ts)))
        .collect()
}

/// The ask this message answers, and the seconds from the ask to it, when someone other than the asker answers later.
fn answer_delay(w: &AskWord, created: &HashMap<AskKey, (String, i64)>) -> Option<(AskKey, i64)> {
    let n = answer_target(&w.text)?;
    let (asker, at) = created.get(&(w.room.clone(), n))?;
    let by_someone_else = w.sender != *asker && w.seq > n;
    by_someone_else.then(|| ((w.room.clone(), n), (w.ts - *at).max(0)))
}

/// The ask this message closes, when that ask was created in the window.
fn closed_ask(w: &AskWord, created: &HashMap<AskKey, (String, i64)>) -> Option<AskKey> {
    let key = (w.room.clone(), closed_target(&w.text)?);
    created.contains_key(&key).then_some(key)
}

/// Seconds to the first answer by ask, and the set of asks closed.
fn answers_and_closes(
    words: &[AskWord],
    sealed: &HashSet<String>,
    created: &HashMap<AskKey, (String, i64)>,
) -> (HashMap<AskKey, i64>, HashSet<AskKey>) {
    let mut first: HashMap<AskKey, i64> = HashMap::new();
    let mut closed: HashSet<AskKey> = HashSet::new();
    for w in words.iter().filter(|w| !sealed.contains(&w.label)) {
        if let Some((key, secs)) = answer_delay(w, created) {
            let e = first.entry(key).or_insert(i64::MAX);
            *e = (*e).min(secs);
        }
        closed.extend(closed_ask(w, created));
    }
    (first, closed)
}

/// How many created asks declared a scope.
fn with_scope(
    conn: &Connection,
    o: &StatsOptions,
    created: &HashMap<AskKey, (String, i64)>,
) -> Result<i64> {
    if !has(conn, "ask_scopes")? {
        return Ok(0);
    }
    let scoped: Vec<AskKey> = rows(
        conn,
        "SELECT room,seq FROM ask_scopes WHERE (?1 IS NULL OR room=?1)",
        o,
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(scoped.iter().filter(|k| created.contains_key(*k)).count() as i64)
}

/// How many created asks carry refs, and the refs by kind.
fn with_refs(
    conn: &Connection,
    o: &StatsOptions,
    created: &HashMap<AskKey, (String, i64)>,
) -> Result<(i64, Counts)> {
    if !has(conn, "ask_refs")? {
        return Ok((0, Vec::new()));
    }
    let refs: Vec<(String, i64, String)> = rows(
        conn,
        "SELECT room,seq,kind FROM ask_refs WHERE (?1 IS NULL OR room=?1)",
        o,
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut with: HashSet<AskKey> = HashSet::new();
    let mut kinds: BTreeMap<String, i64> = BTreeMap::new();
    for (room, seq, kind) in refs {
        if created.contains_key(&(room.clone(), seq)) {
            with.insert((room, seq));
            *kinds.entry(vocab(&kind)).or_default() += 1;
        }
    }
    Ok((with.len() as i64, kinds.into_iter().collect()))
}

/// The asks, their answers and closes, from the first characters of ask-word messages outside sealed labels.
pub(super) fn asks(
    conn: &Connection,
    o: &StatsOptions,
    sealed: &HashSet<String>,
) -> Result<AskCounts> {
    let sealed_unread = one(
        conn,
        &format!(
            "SELECT COUNT(*) FROM messages WHERE {LIVE} AND label IN (SELECT substr(key,8) FROM policy WHERE key LIKE 'sealed:%')"
        ),
        o,
    )?;
    let words = ask_words(conn, o)?;
    let created = created_asks(&words, sealed, o.since.unwrap_or(0));
    let (first, closed) = answers_and_closes(&words, sealed, &created);
    let (first_answer_median, first_answer_p90) = median_and_p90(first.values().copied().collect());
    let (with_refs, ref_kinds) = with_refs(conn, o, &created)?;
    Ok(AskCounts {
        sealed_unread,
        created: created.len() as i64,
        answered: first.len() as i64,
        closed: closed.len() as i64,
        first_answer_median,
        first_answer_p90,
        with_scope: with_scope(conn, o, &created)?,
        with_refs,
        ref_kinds,
    })
}
