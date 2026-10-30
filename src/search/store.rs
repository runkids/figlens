//! One index generation as SQLite rows: records, terms with their postings, relation edges,
//! the definitions they point at and the generation's own fields. A query loads only its terms' postings and the
//! records they rank, into a partial `Index` that persisted and stateless search rank through
//! the same code.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{ErrorKind, FigError};
use rusqlite::{Connection, params};
use serde_json::{Value as Json, json};

use rusqlite::OptionalExtension;

use super::fingerprint::{self, Print};
use super::usages::{self, Definition, Edge, Graph, Key, RELATIONS, STATES};
use super::{
    BYTES, DEPTH, Index, IndexTerm, MAX_MARKS, NEVER, Page, Posting, RANKING, Record, TEXT_BYTES,
    WORK, tokenizer,
};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Rows per write transaction; between batches the caller reports progress and checks space.
pub(crate) const BATCH: usize = 2000;
/// Estimated row bytes per write transaction: a batch also ends here, so one batch adds about
/// this much plus b-tree overhead to the WAL; a single row estimated larger is its own batch.
pub(crate) const BATCH_BYTES: usize = if cfg!(test) { 1024 } else { 8 * 1024 * 1024 };

/// `items` in runs of at most `BATCH` rows and, past each run's first row, `BATCH_BYTES` of
/// `size`.
pub(crate) fn batches<T>(items: &[T], size: impl Fn(&T) -> usize) -> impl Iterator<Item = &[T]> {
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
    json!({"ranking": RANKING, "tokenizer": tokenizer(), "relations": RELATIONS,
        "fingerprint": fingerprint::VERSION, "budgets": [WORK, BYTES, TEXT_BYTES, DEPTH, MAX_MARKS],
        "properties": PROPERTIES})
    .to_string()
}

/// How records keep their component properties (`records.props`, schema 10); a generation
/// built before them is rebuilt, so no model layer reads a record whose properties were not kept.
const PROPERTIES: &str = "props-1";

const TEXT_SOURCES: [&str; 3] = ["stored", "override", "component"];
const STOPS: [&str; 6] = [
    "text_bytes_limit",
    "combining_run_limit",
    "work_limit",
    "byte_limit",
    "depth_limit",
    "deadline",
];

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
fn postings(blob: &[u8], total: usize, cancel: &AtomicBool) -> Result<Vec<Posting>> {
    let (mut at, mut record, mut out) = (0, None::<u32>, Vec::new());
    while at < blob.len() {
        stopped(cancel)?;
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
    // JSON escaping can grow names past what `load` reads back; refuse before writing anything.
    let info = info_json(index, INFO_BYTES)?;
    let total = index.records.len()
        + index.terms.len()
        + index.definitions.len()
        + index.fingerprints.len();
    let mut done = 0;
    let record_bytes = |r: &Record| {
        let text = r.text.as_ref().map_or(0, String::len);
        let edges: usize = r
            .edges
            .iter()
            .chain(&r.library)
            .map(|e| 64 + e.target.as_ref().map_or(0, String::len) + e.evidence.len())
            .sum();
        r.id.len()
            + r.name.len()
            + r.kind.len()
            + text
            + r.norm[0].len()
            + r.norm[1].len()
            + r.props
                .iter()
                .map(|(n, v)| 8 + n.len() + v.as_ref().map_or(4, String::len))
                .sum::<usize>()
            + 64
            + edges
    };
    for chunk in batches(&index.records, record_bytes) {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        guard(&tx)?;
        {
            let mut record = tx.prepare_cached(
                "INSERT INTO records (gen, rid, node_id, name, kind, text, norm0, norm1, len0, len1,
                 parent, source, text_source, hidden, page, root, props) VALUES
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?;
            let mut edge = tx.prepare_cached(
                "INSERT INTO edges (gen, rid, relation, seq, target, state, basis, evidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
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
                    r.root,
                    (!r.props.is_empty()).then(|| json!(r.props).to_string())
                ])?;
                for e in r.edges.iter().chain(&r.library) {
                    edge.execute(params![
                        gen_id, rid, e.relation, e.seq, e.target, e.state, e.basis, e.evidence
                    ])?;
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
    let definitions: Vec<(&String, &Definition)> = index.definitions.iter().collect();
    let definition_bytes = |d: &(&String, &Definition)| {
        let opt = |v: &Option<String>| v.as_ref().map_or(0, String::len);
        d.0.len() + d.1.name.len() + d.1.kind.len() + opt(&d.1.set) + opt(&d.1.page) + 32
    };
    for chunk in batches(&definitions, definition_bytes) {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        guard(&tx)?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO definitions (gen, id, name, kind, set_id, page) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (id, d) in chunk {
                stmt.execute(params![gen_id, id, d.name, d.kind, d.set, d.page])?;
            }
        }
        tx.commit()?;
        done += chunk.len();
        between(conn, done, total)?;
    }
    let print_bytes = |_: &(u32, [Print; 4])| 4 * fingerprint::ROW_BYTES;
    for chunk in batches(&index.fingerprints, print_bytes) {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        guard(&tx)?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO fingerprints (gen, rid, facet, state, digest) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (rid, print) in chunk {
                for (facet, (state, digest)) in fingerprint::FACETS.iter().zip(print) {
                    stmt.execute(params![
                        gen_id,
                        rid,
                        facet,
                        state,
                        digest.map(|d| d.to_vec())
                    ])?;
                }
            }
        }
        tx.commit()?;
        done += chunk.len();
        between(conn, done, total)?;
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    guard(&tx)?;
    tx.execute(
        "UPDATE generations SET info = ?2, records = ?3, complete = ?4, stop = ?5 WHERE gen = ?1",
        params![
            gen_id,
            info,
            index.records.len() as i64,
            index.stop.is_none(),
            index.stop
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Keeps at most `cap` bytes and fails the write that would pass it.
pub(super) struct Capped {
    pub(super) out: Vec<u8>,
    cap: usize,
}

impl Capped {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            out: Vec::new(),
            cap,
        }
    }
}

impl std::io::Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.out.len() + bytes.len() > self.cap {
            return Err(std::io::Error::other("cap"));
        }
        self.out.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The generation's `info` JSON, serialized from borrowed fields into at most `cap` bytes;
/// keys in sorted order, as `json!` wrote them.
pub(super) fn info_json(index: &Index, cap: usize) -> Result<String> {
    let mut w = Capped::new(cap);
    let pages: Vec<_> = index
        .pages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str(), p.internal))
        .collect();
    (|| -> serde_json::Result<()> {
        use std::io::Write as _;
        let key =
            |w: &mut Capped, k: &str| w.write_all(k.as_bytes()).map_err(serde_json::Error::io);
        key(&mut w, "{\"above\":")?;
        serde_json::to_writer(&mut w, &index.above)?;
        key(&mut w, ",\"generation\":")?;
        serde_json::to_writer(&mut w, &index.generation)?;
        key(&mut w, ",\"hiddenContent\":")?;
        serde_json::to_writer(&mut w, &index.hidden_content)?;
        key(&mut w, ",\"lens\":")?;
        serde_json::to_writer(&mut w, &index.lens)?;
        key(&mut w, ",\"pages\":")?;
        serde_json::to_writer(&mut w, &pages)?;
        key(&mut w, ",\"roots\":")?;
        serde_json::to_writer(&mut w, &index.roots)?;
        key(&mut w, ",\"stop\":")?;
        serde_json::to_writer(&mut w, &index.stop)?;
        key(&mut w, ",\"textBytes\":")?;
        serde_json::to_writer(&mut w, &index.text_bytes)?;
        key(&mut w, ",\"unreadFields\":")?;
        serde_json::to_writer(&mut w, &index.unread_fields)?;
        key(&mut w, ",\"unverifiedLinks\":")?;
        serde_json::to_writer(&mut w, &index.unverified_links)?;
        key(&mut w, "}")
    })()
    .map_err(|_| crate::parse::exceeded("index metadata bytes", cap))?;
    Ok(String::from_utf8(w.out)?)
}

/// Largest stored generation fields JSON; pages, roots and ancestors are bounded by the build.
const INFO_BYTES: usize = 64 * 1024 * 1024;
/// Largest node ID, type or component ID: an effective ID gains one `sessionID:localID` GUID
/// (at most 21 bytes) plus a separator and prefix per nested instance, up to `MAX_DEPTH` deep.
pub(crate) const ID_BYTES: usize = (crate::expand::MAX_DEPTH + 1) * 23;

/// An error once `cancel` is set; the cancellable search reports it as `Cancelled`.
fn stopped(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(FigError::new(ErrorKind::Cancelled, "search cancelled").into());
    }
    Ok(())
}

pub(crate) fn bad() -> Box<dyn Error> {
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
/// `spent` is the record text a query has copied so far, shared by every load and `add`.
pub(crate) fn load(
    conn: &Connection,
    gen_id: i64,
    terms: &[String],
    spent: &mut usize,
) -> Result<Index> {
    load_with(conn, gen_id, terms, spent, &NEVER)
}

/// `load`, stopped with an error once `cancel` is set, checked per term, posting and record.
pub(crate) fn load_with(
    conn: &Connection,
    gen_id: i64,
    terms: &[String],
    spent: &mut usize,
    cancel: &AtomicBool,
) -> Result<Index> {
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
        stopped(cancel)?;
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
                    let list = postings(r.get_ref(2)?.as_blob()?, total, cancel)
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
        unverified_links: info["unverifiedLinks"].as_u64().ok_or_else(bad)? as usize,
        unread_fields: info["unreadFields"].as_u64().ok_or_else(bad)? as usize,
        library_unread: false,
        text_bytes: info["textBytes"].as_u64().ok_or_else(bad)? as usize,
        generation: text(&info["generation"])?,
        definitions: BTreeMap::new(),
        relations: true,
        reverse: Default::default(),
        fingerprints: Vec::new(),
    };
    let mut wanted: Vec<u32> = survivors.into_iter().collect();
    if total > 0 {
        wanted.push(0);
    }
    add_with(conn, gen_id, &mut index, &wanted, spent, cancel)?;
    Ok(index)
}

/// Loads records `rids` of generation `gen` into the partial `index`, with their edges; ones
/// already loaded are skipped. Every text is checked against its budget before it is copied:
/// its own cap, and the query's `BYTES` with the `spent` it shares across every load.
pub(crate) fn add(
    conn: &Connection,
    gen_id: i64,
    index: &mut Index,
    rids: &[u32],
    spent: &mut usize,
) -> Result<()> {
    add_with(conn, gen_id, index, rids, spent, &NEVER)
}

/// `add`, stopped with an error once `cancel` is set, checked per row read.
pub(crate) fn add_with(
    conn: &Connection,
    gen_id: i64,
    index: &mut Index,
    rids: &[u32],
    spent: &mut usize,
    cancel: &AtomicBool,
) -> Result<()> {
    let missing: BTreeSet<u32> = rids
        .iter()
        .copied()
        .filter(|r| index.rids.binary_search(r).is_err())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let list = serde_json::to_string(&missing)?;
    let mut edges: BTreeMap<u32, Vec<Edge>> = BTreeMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT rid, relation, seq, target, state, basis, evidence FROM edges
         WHERE gen = ?1 AND rid IN (SELECT value FROM json_each(?2)) AND relation <> ?3
         ORDER BY rid, relation, seq",
    )?;
    let mut rows = stmt.query(params![gen_id, list, usages::LIBRARY])?;
    let mut count = 0usize;
    while let Some(row) = rows.next()? {
        stopped(cancel)?;
        count += 1;
        if count > WORK {
            return Err(bad());
        }
        // Target and evidence, measured in SQLite's buffer and charged before they are copied.
        let size = 64
            + [3, 6]
                .into_iter()
                .map(|i| text_len(row, i))
                .sum::<rusqlite::Result<usize>>()?;
        charge_query(spent, size)?;
        edges.entry(row.get(0)?).or_default().push(edge(row, 1)?);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT rid, node_id, name, kind, text, norm0, norm1, len0, len1, parent, source,
         text_source, hidden, page, root FROM records
         WHERE gen = ?1 AND rid IN (SELECT value FROM json_each(?2)) ORDER BY rid",
    )?;
    let mut rows = stmt.query(params![gen_id, list])?;
    let mut loaded: Vec<(u32, Record)> = Vec::new();
    while let Some(row) = rows.next()? {
        stopped(cancel)?;
        let rid: u32 = row.get(0)?;
        // Name, text and both normalized fields, measured in SQLite's buffer before any copy.
        let size = [2, 4, 5, 6]
            .into_iter()
            .map(|i| text_len(row, i))
            .sum::<rusqlite::Result<usize>>()?;
        charge_query(spent, size)?;
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
            edges: edges.remove(&rid).unwrap_or_default(),
            library: Vec::new(),
            props: Vec::new(),
        };
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

/// Loads the component properties (`records.props`, schema 10) of every loaded record of `index`,
/// charged to `spent` before each copy; only a model layer reads them.
pub(crate) fn add_properties(
    conn: &Connection,
    gen_id: i64,
    index: &mut Index,
    spent: &mut usize,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "SELECT rid, props FROM records WHERE gen = ?1 AND props IS NOT NULL ORDER BY rid",
    )?;
    let mut rows = stmt.query([gen_id])?;
    while let Some(row) = rows.next()? {
        stopped(cancel)?;
        let rid: u32 = row.get(0)?;
        let at = match index.rids.is_empty() {
            true => Some(rid as usize).filter(|&at| at < index.records.len()),
            false => index.rids.binary_search(&rid).ok(),
        };
        let Some(at) = at else { continue };
        // The text and the values parsed from it.
        charge_query(spent, 2 * text_len(row, 1)?)?;
        index.records[at].props =
            serde_json::from_str(&need(text_at(row, 1, TEXT_BYTES)?)?).map_err(|_| bad())?;
    }
    Ok(())
}

/// The length of text or blob column `i` in SQLite's buffer, without copying it.
fn text_len(row: &rusqlite::Row, i: usize) -> rusqlite::Result<usize> {
    Ok(match row.get_ref(i)? {
        rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) => t.len(),
        _ => 0,
    })
}

/// Adds `size` to the bytes a query has copied, refusing past `BYTES`.
pub(super) fn charge_query(spent: &mut usize, size: usize) -> Result<()> {
    *spent = spent.saturating_add(size);
    if *spent > BYTES {
        return Err(FigError::new(
            ErrorKind::QueryWorkLimit,
            format!("the records this search or comparison needs exceed {BYTES} bytes"),
        )
        .into());
    }
    Ok(())
}

/// One edge from `row`'s columns `at..at + 6`: relation, seq, target, state, basis, evidence.
fn edge(row: &rusqlite::Row, at: usize) -> Result<Edge> {
    Ok(Edge {
        relation: known(&usages::STORED, text_at(row, at, ID_BYTES)?)?.ok_or_else(bad)?,
        seq: row.get(at + 1)?,
        target: text_at(row, at + 2, ID_BYTES)?,
        state: known(&STATES, text_at(row, at + 3, ID_BYTES)?)?.ok_or_else(bad)?,
        basis: known(&usages::BASES, text_at(row, at + 4, ID_BYTES)?)?.ok_or_else(bad)?,
        evidence: need(text_at(row, at + 5, ID_BYTES)?)?,
    })
}

/// One stored generation read through bounded, indexed queries; records load on demand.
/// The edges that point at one target from a cursor on, in key order, straight from
/// `edges_target` (named: without statistics the planner prefers the primary key, which reads
/// every later edge of the generation); the caller filters relation and visibility while it reads.
pub(super) const INBOUND: &str =
    "SELECT e.rid, e.relation, e.seq, e.target, e.state, e.basis, e.evidence, r.hidden
     FROM edges e INDEXED BY edges_target JOIN records r ON r.gen = e.gen AND r.rid = e.rid
     WHERE e.gen = ?1 AND e.target = ?2 AND (e.rid, e.relation, e.seq) > (?3, ?4, ?5)
     ORDER BY e.rid, e.relation, e.seq";

/// The children of one record from the cursor's rid on, from `records_parent`.
pub(super) const CHILDREN: &str =
    "SELECT rid, source IS NOT NULL, hidden FROM records INDEXED BY records_parent
     WHERE gen = ?1 AND parent = ?2 AND rid >= ?3 ORDER BY rid";

pub(crate) struct Stored<'c> {
    pub conn: &'c Connection,
    pub gen_id: i64,
    pub index: Index,
    /// The query's record budget spent so far, carried into every ancestor load.
    pub spent: usize,
}

impl Graph for Stored<'_> {
    fn index(&self) -> &Index {
        &self.index
    }
    fn rid_of(&mut self, node: &str) -> Result<Option<u32>> {
        Ok(self
            .conn
            .query_row(
                "SELECT rid FROM records WHERE gen = ?1 AND node_id = ?2",
                params![self.gen_id, node],
                |r| r.get(0),
            )
            .optional()?)
    }
    fn definition(&mut self, id: &str) -> Result<Option<Definition>> {
        if let Some(d) = self.index.definitions.get(id) {
            return Ok(Some(d.clone()));
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT name, kind, set_id, page FROM definitions WHERE gen = ?1 AND id = ?2",
        )?;
        let mut rows = stmt.query(params![self.gen_id, id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let d = Definition {
            name: need(text_at(row, 0, TEXT_BYTES)?)?,
            kind: need(text_at(row, 1, ID_BYTES)?)?,
            set: text_at(row, 2, ID_BYTES)?,
            page: text_at(row, 3, ID_BYTES)?,
        };
        self.index.definitions.insert(id.to_owned(), d.clone());
        Ok(Some(d))
    }
    fn members(&mut self, set: &str, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM definitions WHERE gen = ?1 AND set_id = ?2 ORDER BY id LIMIT ?3",
        )?;
        let mut rows = stmt.query(params![self.gen_id, set, limit as i64])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(need(text_at(row, 0, ID_BYTES)?)?);
        }
        Ok(out)
    }
    fn targeted(&mut self, target: &str) -> Result<Option<&'static str>> {
        let state: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM edges WHERE gen = ?1 AND target = ?2 LIMIT 1",
                params![self.gen_id, target],
                |r| r.get(0),
            )
            .optional()?;
        known(&STATES, state)
    }
    fn inbound(
        &mut self,
        targets: &[String],
        relations: &[&'static str],
        parent: Option<u32>,
        hidden: bool,
        after: Option<&Key>,
        limit: usize,
    ) -> Result<usages::Inbound> {
        // One index range per target, read in key order from the cursor on, so a page costs
        // what it returns plus the rows its filters pass over, never every earlier match.
        let (rid, relation, seq) = match after {
            Some(a) => (i64::from(a.0), a.1.clone(), a.2),
            None => (-1, String::new(), 0),
        };
        let mut out = Vec::new();
        let (mut scanned, mut left) = (0, None);
        let mut stmt = self.conn.prepare_cached(INBOUND)?;
        for target in targets {
            let mut rows = stmt.query(params![self.gen_id, target, rid, relation, seq])?;
            let mut kept = 0;
            while kept < limit
                && let Some(row) = rows.next()?
            {
                scanned += 1;
                if scanned > usages::SCAN {
                    return Err(usages::scan_limit());
                }
                let e = edge(row, 1)?;
                if !relations.contains(&e.relation) {
                    continue;
                }
                if row.get::<_, bool>(7)? && !hidden {
                    usages::lowest(&mut left, (row.get(0)?, e.relation.into(), e.seq));
                    continue;
                }
                out.push((row.get(0)?, e));
                kept += 1;
            }
        }
        if let Some(parent) = parent.filter(|_| relations.contains(&"contains")) {
            let mut stmt = self.conn.prepare_cached(CHILDREN)?;
            let mut rows = stmt.query(params![self.gen_id, parent, rid])?;
            let mut kept = 0;
            while kept < limit
                && let Some(row) = rows.next()?
            {
                let child: u32 = row.get(0)?;
                if after.is_some_and(|a| (child, "contains", 0) <= (a.0, a.1.as_str(), a.2)) {
                    continue;
                }
                scanned += 1;
                if scanned > usages::SCAN {
                    return Err(usages::scan_limit());
                }
                if row.get::<_, bool>(2)? && !hidden {
                    usages::lowest(&mut left, (child, "contains".into(), 0));
                    continue;
                }
                let expanded: bool = row.get(1)?;
                out.push((
                    child,
                    Edge {
                        relation: "contains",
                        seq: 0,
                        target: None,
                        state: "resolved",
                        basis: if expanded { "resolved" } else { "recorded" },
                        evidence: "parent".into(),
                    },
                ));
                kept += 1;
            }
        }
        out.sort_by(|a, b| (a.0, a.1.relation, a.1.seq).cmp(&(b.0, b.1.relation, b.1.seq)));
        out.truncate(limit);
        Ok((out, left))
    }
    fn outbound(&mut self, rid: u32) -> Result<Vec<Edge>> {
        self.load(&[rid])?;
        let r = self.index.record(rid);
        let mut out: Vec<Edge> = r.edges.clone();
        if let Some(p) = r.parent {
            let kind = if r.source.is_some() {
                "resolved"
            } else {
                "recorded"
            };
            out.push(Edge {
                relation: "contains",
                seq: 0,
                target: Some(self.index.record(p).id.clone()),
                state: "resolved",
                basis: kind,
                evidence: "parent".into(),
            });
        }
        out.sort_by(|a, b| (a.relation, a.seq).cmp(&(b.relation, b.seq)));
        Ok(out)
    }
    fn library(&mut self, rid: u32) -> Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT rid, relation, seq, target, state, basis, evidence FROM edges
             WHERE gen = ?1 AND rid = ?2 AND relation = ?3 ORDER BY seq",
        )?;
        let mut rows = stmt.query(params![self.gen_id, rid, usages::LIBRARY])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            if out.len() >= WORK {
                return Err(bad());
            }
            // Measured in SQLite's buffer and charged before it is copied, as `add` does.
            let size = 64 + text_len(row, 3)? + text_len(row, 6)?;
            charge_query(&mut self.spent, size)?;
            out.push(edge(row, 1)?);
        }
        Ok(out)
    }
    fn load(&mut self, rids: &[u32]) -> Result<()> {
        let mut rids = rids.to_vec();
        // Each round reads one more ancestor level, so paths and parents are readable.
        while !rids.is_empty() {
            add(
                self.conn,
                self.gen_id,
                &mut self.index,
                &rids,
                &mut self.spent,
            )?;
            rids = rids
                .iter()
                .filter_map(|r| super::parent(&self.index, *r))
                .collect();
            rids.sort_unstable();
            rids.dedup();
        }
        Ok(())
    }
}

/// The kind and fingerprint of stored node `node` in generation `gen_id`, its first record by
/// rid; `None` when it is no record or was not fingerprinted. Rows are checked as they are read.
pub(crate) fn print(
    conn: &Connection,
    gen_id: i64,
    node: &str,
) -> Result<Option<(String, [Print; 4])>> {
    // Every stored value is measured in SQLite's buffer before it is copied.
    let mut stmt = conn.prepare_cached(
        "SELECT rid, kind FROM records INDEXED BY records_node WHERE gen = ?1 AND node_id = ?2
         ORDER BY rid LIMIT 1",
    )?;
    let mut found = stmt.query(params![gen_id, node])?;
    let Some(row) = found.next()? else {
        return Ok(None);
    };
    let (rid, kind): (i64, String) = (row.get(0)?, need(text_at(row, 1, ID_BYTES)?)?);
    let mut stmt = conn.prepare_cached(
        "SELECT facet, state, digest FROM fingerprints WHERE gen = ?1 AND rid = ?2 LIMIT 5",
    )?;
    let mut rows = stmt.query(params![gen_id, rid])?;
    let (mut print, mut seen) = ([("unread", None); 4], 0);
    while let Some(row) = rows.next()? {
        let at = fingerprint::FACETS
            .iter()
            .position(|f| row.get_ref(0).ok().and_then(|v| v.as_str().ok()) == Some(*f))
            .ok_or_else(bad)?;
        let state = known(&fingerprint::STATES, text_at(row, 1, ID_BYTES)?)?.ok_or_else(bad)?;
        let digest = row.get_ref(2)?.as_blob_or_null()?;
        let digest = match (state, digest) {
            ("read", Some(d)) => Some(<[u8; 32]>::try_from(d).map_err(|_| bad())?),
            (_, None) if state != "read" => None,
            _ => return Err(bad()),
        };
        print[at] = (state, digest);
        seen |= 1 << at;
    }
    Ok(match seen {
        0 => None,
        0b1111 => Some((kind, print)),
        _ => return Err(bad()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generation_with_other_fingerprints_is_not_reused() {
        // Stored fingerprints hash their version, so it must be part of the reuse key.
        let v: Json = serde_json::from_str(&versions()).unwrap();
        assert_eq!(v["fingerprint"], fingerprint::VERSION);
    }

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
        let list = postings(&blob, 304, &NEVER).unwrap();
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
        assert!(postings(&wrap, usize::MAX, &NEVER).is_err());
        assert!(postings(&blob, 303, &NEVER).is_err());
    }
}
