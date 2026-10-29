//! One index generation as SQLite rows: records, terms with their postings, `instance_of`
//! edges and the generation's own fields. A query loads only its terms' postings and the
//! records they rank, into a partial `Index` that persisted and stateless search rank through
//! the same code.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use crate::{ErrorKind, FigError};
use rusqlite::{Connection, params};
use serde_json::{Value as Json, json};

use rusqlite::OptionalExtension;

use super::{
    BYTES, DEPTH, Index, IndexTerm, Instance, MAX_MARKS, Page, Posting, RANKING, Record,
    TEXT_BYTES, WORK, tokenizer,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Rows per write transaction; between batches the caller reports progress and checks space.
pub(crate) const BATCH: usize = 2000;
/// Estimated row bytes per write transaction: a batch also ends here, so one batch adds about
/// this much plus b-tree overhead to the WAL; a single row estimated larger is its own batch.
pub(crate) const BATCH_BYTES: usize = if cfg!(test) { 1024 } else { 8 * 1024 * 1024 };

/// `items` in runs of at most `BATCH` rows and, past each run's first row, `BATCH_BYTES` of
/// `size`.
fn batches<T>(items: &[T], size: impl Fn(&T) -> usize) -> impl Iterator<Item = &[T]> {
    let mut rest = items;
    std::iter::from_fn(move || {
        let (mut n, mut bytes) = (0, 0);
        while n < rest.len() && n < BATCH && (n == 0 || bytes + size(&rest[n]) <= BATCH_BYTES) {
            bytes += size(&rest[n]);
            n += 1;
        }
        let (run, tail) = rest.split_at(n);
        rest = tail;
        (!run.is_empty()).then_some(run)
    })
}

/// What a published generation was built with; a generation built by other versions is rebuilt,
/// never reused.
pub(crate) fn versions() -> String {
    json!({"ranking": RANKING, "tokenizer": tokenizer(),
        "budgets": [WORK, BYTES, TEXT_BYTES, DEPTH, MAX_MARKS]})
    .to_string()
}

const TEXT_SOURCES: [&str; 3] = ["stored", "override", "component"];
const STOPS: [&str; 6] = [
    "text_bytes_limit",
    "combining_run_limit",
    "work_limit",
    "byte_limit",
    "depth_limit",
    "deadline",
];
const BASES: [&str; 2] = ["recorded", "resolved"];
const EVIDENCE: [&str; 3] = ["symbolData", "definition", "swap"];

fn known(list: &'static [&'static str], value: Option<String>) -> Result<Option<&'static str>> {
    match value {
        None => Ok(None),
        Some(v) => list
            .iter()
            .find(|k| **k == v)
            .map(|k| Some(*k))
            .ok_or_else(|| format!("unknown stored value {v:?}; rebuild the index").into()),
    }
}

fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// One LEB128 value; a corrupt value is an error, never a wrapped number.
fn read_varint(data: &[u8], at: &mut usize) -> Result<u64> {
    let mut n = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *data
            .get(*at)
            .ok_or("truncated postings; rebuild the index")?;
        *at += 1;
        // The tenth byte carries only bit 63.
        if shift == 63 && byte > 1 {
            break;
        }
        n |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(n);
        }
    }
    Err("invalid postings; rebuild the index".into())
}

/// The postings of one term, record IDs checked against the generation's `total`.
fn postings(blob: &[u8], total: usize) -> Result<Vec<Posting>> {
    let (mut at, mut record, mut out) = (0, None::<u32>, Vec::new());
    while at < blob.len() {
        let delta = u32::try_from(read_varint(blob, &mut at)?)?;
        let next = match record {
            None => Some(delta),
            Some(r) => r.checked_add(delta),
        }
        .filter(|r| (*r as usize) < total)
        .ok_or("invalid postings; rebuild the index")?;
        record = Some(next);
        let packed = read_varint(blob, &mut at)?;
        out.push(Posting {
            record: next,
            field: (packed & 1) as u8,
            tf: u16::try_from(packed >> 1)?,
        });
    }
    Ok(out)
}

/// The records, terms and edges of `index` as generation `gen`, in transactions of `BATCH`
/// rows or `BATCH_BYTES`; `between` runs after each committed batch with the rows written so
/// far and in total.
/// `guard` runs first inside every write transaction, the final generation update included,
/// so a writer that lost its right to write commits nothing.
pub(crate) fn save(
    conn: &mut Connection,
    gen_id: i64,
    index: &Index,
    guard: &dyn Fn(&Connection) -> Result<()>,
    between: &mut dyn FnMut(&Connection, usize, usize) -> Result<()>,
) -> Result<()> {
    let total = index.records.len() + index.terms.len();
    let mut done = 0;
    let record_bytes = |r: &Record| {
        let text = r.text.as_ref().map_or(0, String::len);
        r.id.len() + r.name.len() + r.kind.len() + text + r.norm[0].len() + r.norm[1].len() + 64
    };
    for chunk in batches(&index.records, record_bytes) {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        guard(&tx)?;
        {
            let mut record = tx.prepare_cached(
                "INSERT INTO records (gen, rid, node_id, name, kind, text, norm0, norm1, len0, len1,
                 parent, source, text_source, hidden, page, root) VALUES
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            )?;
            let mut edge = tx.prepare_cached(
                "INSERT INTO edges (gen, rid, relation, target, basis, evidence)
                 VALUES (?1, ?2, 'instance_of', ?3, ?4, ?5)",
            )?;
            for (i, r) in chunk.iter().enumerate() {
                let rid = (done + i) as i64;
                record.execute(params![
                    gen_id,
                    rid,
                    r.id,
                    r.name,
                    r.kind,
                    r.text,
                    r.norm[0],
                    r.norm[1],
                    r.lens[0],
                    r.lens[1],
                    r.parent,
                    r.source,
                    r.text_source,
                    r.hidden,
                    r.page,
                    r.root
                ])?;
                if let Some(inst) = &r.instance {
                    edge.execute(params![gen_id, rid, inst.target, inst.basis, inst.evidence])?;
                }
            }
        }
        tx.commit()?;
        done += chunk.len();
        between(conn, done, total)?;
    }
    let terms: Vec<(&String, &IndexTerm)> = index.terms.iter().collect();
    // A posting packs into at most 10 bytes of varints.
    let term_bytes = |t: &(&String, &IndexTerm)| t.0.len() + t.1.postings.len() * 10 + 16;
    for chunk in batches(&terms, term_bytes) {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        guard(&tx)?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO terms (gen, term, df, postings) VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut blob = Vec::new();
            for (term, entry) in chunk {
                blob.clear();
                let mut last = 0u32;
                for p in &entry.postings {
                    varint(&mut blob, u64::from(p.record - last));
                    varint(&mut blob, u64::from(p.tf) << 1 | u64::from(p.field));
                    last = p.record;
                }
                stmt.execute(params![gen_id, term, entry.df, blob])?;
            }
        }
        tx.commit()?;
        done += chunk.len();
        between(conn, done, total)?;
    }
    let pages: Vec<Json> = index
        .pages
        .iter()
        .map(|p| json!([p.id, p.name, p.internal]))
        .collect();
    let info = json!({"pages": pages, "roots": index.roots, "above": index.above,
        "lens": index.lens, "stop": index.stop, "hiddenContent": index.hidden_content,
        "textBytes": index.text_bytes, "generation": index.generation});
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    guard(&tx)?;
    tx.execute(
        "UPDATE generations SET info = ?2, records = ?3, complete = ?4, stop = ?5 WHERE gen = ?1",
        params![
            gen_id,
            info.to_string(),
            index.records.len() as i64,
            index.stop.is_none(),
            index.stop
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Largest stored generation fields JSON; pages, roots and ancestors are bounded by the build.
const INFO_BYTES: usize = 64 * 1024 * 1024;
/// Largest node ID, type or component ID: an effective ID gains one `sessionID:localID` GUID
/// (at most 21 bytes) plus a separator and prefix per nested instance, up to `MAX_DEPTH` deep.
const ID_BYTES: usize = (crate::expand::MAX_DEPTH + 1) * 23;

fn bad() -> Box<dyn Error> {
    "invalid stored index; rebuild the index".into()
}

/// A text column checked against `cap` before it is copied.
fn text_at(row: &rusqlite::Row, i: usize, cap: usize) -> Result<Option<String>> {
    match row.get_ref(i)? {
        rusqlite::types::ValueRef::Null => Ok(None),
        v => {
            let s = v.as_str()?;
            if s.len() > cap {
                return Err(bad());
            }
            Ok(Some(s.to_owned()))
        }
    }
}

fn need(value: Option<String>) -> Result<String> {
    value.ok_or_else(bad)
}

/// The part of generation `gen` that ranks `terms`: its corpus fields, the terms' document
/// frequencies, and the records every term matches with only their postings, plus the scope
/// root. Nothing else of the generation is read; `add` loads display ancestors later.
pub(crate) fn load(conn: &Connection, gen_id: i64, terms: &[String]) -> Result<Index> {
    let (info, total): (Option<String>, i64) = conn.query_row(
        "SELECT info, records FROM generations WHERE gen = ?1",
        [gen_id],
        |r| {
            Ok((
                text_at(r, 0, INFO_BYTES).map_err(|_| rusqlite::Error::InvalidQuery)?,
                r.get(1)?,
            ))
        },
    )?;
    let total = usize::try_from(total)
        .ok()
        .filter(|t| *t <= WORK)
        .ok_or_else(bad)?;
    let info: Json = serde_json::from_str(&need(info)?)?;
    let mut found = BTreeMap::new();
    let mut survivors: Option<BTreeSet<u32>> = None;
    for term in terms {
        if found.contains_key(term) {
            continue;
        }
        let row: Option<(u32, Vec<Posting>)> = conn
            .query_row(
                "SELECT df, length(postings), postings FROM terms WHERE gen = ?1 AND term = ?2",
                params![gen_id, term],
                |r| {
                    let (df, len): (u32, i64) = (r.get(0)?, r.get(1)?);
                    // A posting takes at most 8 bytes, and a record has at most one per field.
                    if df as usize > total || len as u64 > 16 * total as u64 + 16 {
                        return Err(rusqlite::Error::IntegralValueOutOfRange(1, len));
                    }
                    let list = postings(r.get_ref(2)?.as_blob()?, total)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                    Ok((df, list))
                },
            )
            .optional()?;
        let Some((df, list)) = row else {
            // A source lacking a term matches nothing; its zero df adds nothing.
            survivors = Some(BTreeSet::new());
            continue;
        };
        let records: BTreeSet<u32> = list.iter().map(|p| p.record).collect();
        survivors = Some(match survivors {
            None => records,
            Some(s) => s.intersection(&records).copied().collect(),
        });
        found.insert(
            term.clone(),
            IndexTerm {
                df,
                postings: list,
                full: None,
            },
        );
    }
    let survivors = survivors.unwrap_or_default();
    for entry in found.values_mut() {
        entry.full = Some(entry.postings.len());
        entry.postings.retain(|p| survivors.contains(&p.record));
    }
    let text = |v: &Json| v.as_str().map(str::to_owned).ok_or_else(bad);
    let pages = info["pages"]
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|p| {
            Ok(Page {
                id: text(&p[0])?,
                name: text(&p[1])?,
                internal: p[2].as_bool().ok_or_else(bad)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let roots = info["roots"]
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(text)
        .collect::<Result<Vec<_>>>()?;
    let above = info["above"]
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|a| Ok((text(&a[0])?, text(&a[1])?)))
        .collect::<Result<Vec<_>>>()?;
    let mut index = Index {
        records: Vec::new(),
        rids: Vec::new(),
        total,
        terms: found,
        pages,
        roots,
        above,
        lens: [
            info["lens"][0].as_u64().ok_or_else(bad)?,
            info["lens"][1].as_u64().ok_or_else(bad)?,
        ],
        stop: known(&STOPS, info["stop"].as_str().map(str::to_owned))?,
        hidden_content: info["hiddenContent"].as_u64().ok_or_else(bad)? as usize,
        text_bytes: info["textBytes"].as_u64().ok_or_else(bad)? as usize,
        generation: text(&info["generation"])?,
    };
    let mut wanted: Vec<u32> = survivors.into_iter().collect();
    if total > 0 {
        wanted.push(0);
    }
    add(conn, gen_id, &mut index, &wanted)?;
    Ok(index)
}

/// Loads records `rids` of generation `gen` into the partial `index`, with their edges; ones
/// already loaded are skipped. Every text is checked against its budget before it is copied.
pub(crate) fn add(conn: &Connection, gen_id: i64, index: &mut Index, rids: &[u32]) -> Result<()> {
    let missing: BTreeSet<u32> = rids
        .iter()
        .copied()
        .filter(|r| index.rids.binary_search(r).is_err())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let list = serde_json::to_string(&missing)?;
    let mut edges: BTreeMap<u32, Instance> = BTreeMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT rid, target, basis, evidence FROM edges
         WHERE gen = ?1 AND rid IN (SELECT value FROM json_each(?2))",
    )?;
    let mut rows = stmt.query(params![gen_id, list])?;
    while let Some(row) = rows.next()? {
        edges.insert(
            row.get(0)?,
            Instance {
                target: text_at(row, 1, ID_BYTES)?,
                basis: known(&BASES, text_at(row, 2, ID_BYTES)?)?.ok_or_else(bad)?,
                evidence: known(&EVIDENCE, text_at(row, 3, ID_BYTES)?)?.ok_or_else(bad)?,
            },
        );
    }
    let mut stmt = conn.prepare_cached(
        "SELECT rid, node_id, name, kind, text, norm0, norm1, len0, len1, parent, source,
         text_source, hidden, page, root FROM records
         WHERE gen = ?1 AND rid IN (SELECT value FROM json_each(?2)) ORDER BY rid",
    )?;
    let mut rows = stmt.query(params![gen_id, list])?;
    let mut loaded: Vec<(u32, Record)> = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = rows.next()? {
        let rid: u32 = row.get(0)?;
        let record = Record {
            id: need(text_at(row, 1, ID_BYTES)?)?,
            name: need(text_at(row, 2, TEXT_BYTES)?)?,
            kind: need(text_at(row, 3, ID_BYTES)?)?,
            text: text_at(row, 4, TEXT_BYTES)?,
            norm: [
                need(text_at(row, 5, TEXT_BYTES)?)?,
                need(text_at(row, 6, TEXT_BYTES)?)?,
            ],
            lens: [row.get(7)?, row.get(8)?],
            parent: row.get(9)?,
            source: text_at(row, 10, ID_BYTES)?,
            text_source: known(&TEXT_SOURCES, text_at(row, 11, ID_BYTES)?)?,
            hidden: row.get(12)?,
            page: row.get(13)?,
            root: row.get(14)?,
            instance: edges.remove(&rid),
        };
        bytes += record.name.len()
            + record.text.as_ref().map_or(0, String::len)
            + record.norm[0].len()
            + record.norm[1].len();
        if bytes > BYTES {
            return Err(FigError::new(
                ErrorKind::QueryWorkLimit,
                format!("the records this search needs exceed {BYTES} bytes"),
            )
            .into());
        }
        if record.page.is_some_and(|p| p as usize >= index.pages.len())
            || record.root.is_some_and(|p| p as usize >= index.roots.len())
            // Builds number a parent before its children, so a stored parent cycle is corrupt.
            || record.parent.is_some_and(|p| p >= rid)
        {
            return Err(bad());
        }
        loaded.push((rid, record));
    }
    if loaded.len() != missing.len() {
        return Err(bad());
    }
    let mut all: Vec<(u32, Record)> = std::mem::take(&mut index.rids)
        .into_iter()
        .zip(std::mem::take(&mut index.records))
        .chain(loaded)
        .collect();
    all.sort_unstable_by_key(|(rid, _)| *rid);
    (index.rids, index.records) = all.into_iter().unzip();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_batches_end_at_their_byte_budget_or_row_count() {
        let sizes = [400, 400, 400, 5000, 10];
        let runs: Vec<usize> = batches(&sizes, |s| *s).map(<[_]>::len).collect();
        assert_eq!(BATCH_BYTES, 1024);
        assert_eq!(
            runs,
            [2, 1, 1, 1],
            "a row past the budget starts the next batch"
        );
        let small = vec![0; BATCH + 1];
        let runs: Vec<usize> = batches(&small, |s| *s).map(<[_]>::len).collect();
        assert_eq!(runs, [BATCH, 1]);
    }

    #[test]
    fn the_longest_effective_id_bounded_expansion_builds_reads_back() {
        let guid = format!("{}:{}", u32::MAX, u32::MAX);
        let id = format!("I{}", vec![guid; crate::expand::MAX_DEPTH + 1].join(";"));
        let conn = Connection::open_in_memory().unwrap();
        let read = conn
            .query_row("SELECT ?1", [&id], |row| {
                Ok(text_at(row, 0, ID_BYTES).map_err(|e| e.to_string()))
            })
            .unwrap();
        assert_eq!(read, Ok(Some(id)));
    }

    #[test]
    fn postings_round_trip_and_reject_corrupt_values() {
        let mut blob = Vec::new();
        for (delta, packed) in [(3u64, 5u64), (300, 2)] {
            varint(&mut blob, delta);
            varint(&mut blob, packed);
        }
        let list = postings(&blob, 304).unwrap();
        assert_eq!(
            list.iter()
                .map(|p| (p.record, p.tf, p.field))
                .collect::<Vec<_>>(),
            [(3, 2, 1), (303, 1, 0)]
        );
        // A tenth byte above 1 would shift payload bits out of a u64.
        let mut wide = vec![0xff; 9];
        wide.push(0x02);
        assert!(read_varint(&wide, &mut 0).is_err());
        // Deltas that wrap past u32::MAX, or name a record past the generation, are corrupt.
        let mut wrap = Vec::new();
        for n in [u64::from(u32::MAX), 0, 1, 0] {
            varint(&mut wrap, n);
        }
        assert!(postings(&wrap, usize::MAX).is_err());
        assert!(postings(&blob, 303).is_err());
    }
}
