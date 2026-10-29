//! Scoped lexical search over effective content: stored layers plus the instance sublayers
//! REST output resolves, indexed in full before any candidate is retrieved.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use caseless::Caseless;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use unicode_normalization::char::{canonical_combining_class, decompose_compatible};
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfkc_quick};

use crate::expand::{Bounds, Expansion, Stop, expand_instances_bounded, parent_of, plugin_type};
use crate::{Document, ErrorKind, FigError, Tree};

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 200;
const MAX_QUERY_CHARS: usize = 256;
/// Query terms are tracked in a `u32` mask.
/// A CJK run of n characters searches n-1 bigrams, so a 65-character sentence still fits.
const MAX_TERMS: usize = 64;
const MAX_TYPES: usize = 16;
const MAX_SCOPE_BYTES: usize = 1024;
const MAX_OFFSET: usize = 100_000;

/// Work bounds of one scope extraction, independent of the result limit.
const SOURCE_BYTES: u64 = 512 * 1024 * 1024;
/// Expanding a 179 MB held-out file charged 7.1M; each override value walked counts once.
const WORK: usize = 10_000_000;
const BYTES: usize = 512 * 1024 * 1024;
const TEXT_BYTES: usize = 64 * 1024 * 1024;
const DEPTH: usize = 256;
/// Combining marks in a row NFKC may buffer; stream-safe text keeps at most 30.
const MAX_MARKS: usize = 1024;
const DEADLINE: Duration = Duration::from_secs(15);
/// Same-stamp reuse lasts at most this long before the bytes are hashed again.
const REHASH: Duration = Duration::from_secs(60);

const RANKING: &str = "lexical-1";
const WEIGHTS: [f64; 2] = [2.0, 1.0];
const K1: f64 = 1.2;
const B: f64 = 0.75;
const NAME: u8 = 0;
const TEXT: u8 = 1;

fn tokenizer() -> String {
    let (n, c) = (
        unicode_normalization::UNICODE_VERSION,
        caseless::UNICODE_VERSION,
    );
    format!(
        "nfkc-{}.{}-casefold-{}.{}-cjk-bigram-camel-2",
        n.0, n.1, c.0, c.1
    )
}

/// The first 16 hex digits of a SHA-256: enough to tell sources, queries and builds apart.
fn short_hash(data: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(data))[..16].to_owned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn invalid(message: impl Into<String>) -> Box<dyn Error> {
    FigError::new(ErrorKind::InvalidArguments, message).into()
}

/// One search call: the query, its stored scope root, filters and page.
#[derive(Clone, Debug)]
pub struct Request {
    pub query: String,
    pub scope: String,
    pub types: Vec<String>,
    pub include_hidden: bool,
    pub include_internal: bool,
    pub limit: usize,
    pub cursor: Option<String>,
}

/// A request that passed validation, before any source is read.
pub struct Prepared {
    phrase: String,
    terms: Vec<String>,
    offset: usize,
    fingerprint: Option<String>,
}

impl Request {
    /// Checks the query, filters, limit and cursor syntax; cheap, so callers run it before I/O.
    pub fn validate(&self) -> Result<Prepared, Box<dyn Error>> {
        if self.query.chars().count() > MAX_QUERY_CHARS {
            return Err(invalid(format!(
                "search query takes at most {MAX_QUERY_CHARS} characters"
            )));
        }
        let units = normalize(&self.query);
        let phrase: String = units.iter().map(|u| u.c).collect();
        let mut terms: Vec<String> = Vec::new();
        for token in tokens(&units, true) {
            if !terms.contains(&token.term) {
                terms.push(token.term);
            }
        }
        if terms.is_empty() {
            return Err(invalid(
                "search needs a query with letters, digits, CJK characters or emoji",
            ));
        }
        if terms.len() > MAX_TERMS {
            return Err(invalid(format!(
                "search query has more than {MAX_TERMS} distinct terms"
            )));
        }
        if self.scope.is_empty() || self.scope.len() > MAX_SCOPE_BYTES {
            return Err(invalid(format!(
                "search node takes a node id of 1-{MAX_SCOPE_BYTES} bytes"
            )));
        }
        if !(1..=MAX_LIMIT).contains(&self.limit) {
            return Err(invalid(format!("search limit takes 1-{MAX_LIMIT}")));
        }
        if self.types.len() > MAX_TYPES
            || self.types.iter().any(|t| {
                t.is_empty()
                    || t.len() > 32
                    || !t.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
            })
        {
            return Err(invalid(format!(
                "search types take up to {MAX_TYPES} node types such as TEXT or FRAME"
            )));
        }
        let (offset, fingerprint) = match &self.cursor {
            None => (0, None),
            Some(cursor) => {
                let parsed = cursor
                    .strip_prefix("s1.")
                    .and_then(|c| c.split_once('.'))
                    .filter(|(fp, _)| fp.len() == 16 && fp.bytes().all(|b| b.is_ascii_hexdigit()))
                    .and_then(|(fp, offset)| Some((offset.parse::<usize>().ok()?, fp)))
                    .filter(|(offset, _)| (1..=MAX_OFFSET).contains(offset));
                let (offset, fp) = parsed.ok_or_else(|| {
                    invalid("invalid search cursor; pass nextCursor from the previous reply")
                })?;
                (offset, Some(fp.to_owned()))
            }
        };
        Ok(Prepared {
            phrase,
            terms,
            offset,
            fingerprint,
        })
    }

    /// Binds a cursor to this query and filters, the source, snapshot and index generation it
    /// paged, and the ranking versions.
    fn fingerprint(&self, q: &Prepared, source: &Json, index: &Index) -> String {
        let key = json!([
            source["sourceId"],
            source["snapshotId"],
            index.generation,
            self.scope,
            q.phrase,
            q.terms,
            self.types,
            self.include_hidden,
            self.include_internal,
            self.limit,
            RANKING,
            tokenizer()
        ]);
        short_hash(key.to_string())
    }
}

// ---- Normalization and tokens ----

/// A normalized character and the original bytes it came from.
#[derive(Clone, Copy, Debug)]
struct Unit {
    c: char,
    start: usize,
    end: usize,
    /// A lowercase-to-uppercase step in the compatibility form starts a word here.
    camel: bool,
}

/// NFKC, full Unicode case folding and NFKC again, applied per base character with its
/// combining marks so each output character maps back to original bytes. Whitespace runs
/// become one space; leading and trailing whitespace is dropped.
/// Whether NFKC may merge `c` into the character before it: its compatibility decomposition
/// starts with a combining mark or a starter that composes backward, such as a jamo vowel.
/// Anything else starts a new segment, so each unit keeps the span of its own original.
fn joins_previous(c: char) -> bool {
    if c.is_ascii() {
        return false;
    }
    let mut first = None;
    decompose_compatible(c, |d| {
        first.get_or_insert(d);
    });
    let first = first.unwrap_or(c);
    canonical_combining_class(first) != 0
        || is_nfkc_quick(std::iter::once(first)) == IsNormalized::Maybe
}

fn normalize(text: &str) -> Vec<Unit> {
    let mut out: Vec<Unit> = Vec::new();
    let mut lower = false;
    let mut chars = text.char_indices().peekable();
    while let Some((start, first)) = chars.next() {
        let mut end = start + first.len_utf8();
        while let Some(&(i, c)) = chars.peek()
            && joins_previous(c)
        {
            end = i + c.len_utf8();
            chars.next();
        }
        for k in text[start..end].nfkc() {
            let mut camel = k.is_uppercase() && lower;
            lower = k.is_lowercase();
            let folded: String = std::iter::once(k).default_case_fold().nfkc().collect();
            for c in folded.chars() {
                if c.is_whitespace() {
                    if out.last().is_some_and(|u| u.c != ' ') {
                        out.push(Unit {
                            c: ' ',
                            start,
                            end,
                            camel: false,
                        });
                    }
                    continue;
                }
                out.push(Unit {
                    c,
                    start,
                    end,
                    camel,
                });
                camel = false;
            }
        }
    }
    while out.last().is_some_and(|u| u.c == ' ') {
        out.pop();
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Word,
    Cjk,
    Symbol,
    /// Joins emoji: ZWJ, variation selectors, skin tones and tags.
    Join,
    Gap,
}

fn class(c: char) -> Class {
    match c as u32 {
        0x1100..=0x11FF
        | 0x2E80..=0x2FDF
        | 0x3005..=0x3007
        | 0x3040..=0x30FF
        | 0x3130..=0x318F
        | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF
        | 0x20000..=0x3FFFF => Class::Cjk,
        0x200D | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F => Class::Join,
        _ if c.is_alphanumeric() || canonical_combining_class(c) != 0 => Class::Word,
        0x2190..=0x2BFF | 0x1F000..=0x1FAFF => Class::Symbol,
        _ => Class::Gap,
    }
}

struct Token {
    term: String,
    /// Unit range.
    start: usize,
    end: usize,
}

/// Words split at spaces, punctuation and slashes, fields also at camel case; CJK runs as character bigrams
/// plus single characters (a query uses single characters only for a one-character run);
/// emoji clusters as one term each.
fn tokens(units: &[Unit], query: bool) -> Vec<Token> {
    let mut out = Vec::new();
    let term = |a: usize, b: usize| units[a..b].iter().map(|u| u.c).collect::<String>();
    let mut i = 0;
    while i < units.len() {
        let mut j = i + 1;
        match class(units[i].c) {
            Class::Word => {
                while j < units.len() && class(units[j].c) == Class::Word {
                    j += 1;
                }
                // Fields index camel-case parts plus the whole word, so a query matches whether
                // or not it repeats the capitalization; queries keep the whole word.
                let mut start = i;
                for k in (i + 1..j).filter(|&k| !query && units[k].camel).chain([j]) {
                    if start > i || k < j {
                        out.push(Token {
                            term: term(start, k),
                            start,
                            end: k,
                        });
                    }
                    start = k;
                }
                out.push(Token {
                    term: term(i, j),
                    start: i,
                    end: j,
                });
            }
            Class::Cjk => {
                while j < units.len() && class(units[j].c) == Class::Cjk {
                    j += 1;
                }
                for k in i..j {
                    if !query || j - i == 1 {
                        out.push(Token {
                            term: term(k, k + 1),
                            start: k,
                            end: k + 1,
                        });
                    }
                    if k + 1 < j {
                        out.push(Token {
                            term: term(k, k + 2),
                            start: k,
                            end: k + 2,
                        });
                    }
                }
            }
            Class::Symbol => {
                while j < units.len()
                    && (class(units[j].c) == Class::Join || units[j - 1].c == '\u{200D}')
                {
                    j += 1;
                }
                out.push(Token {
                    term: term(i, j),
                    start: i,
                    end: j,
                });
            }
            Class::Join | Class::Gap => {}
        }
        i = j;
    }
    out
}

// ---- Index ----

struct Record {
    id: String,
    name: String,
    kind: String,
    text: Option<String>,
    norm: [String; 2],
    lens: [u32; 2],
    parent: Option<u32>,
    /// The component node an expanded sublayer copies.
    source: Option<String>,
    text_source: Option<&'static str>,
    hidden: bool,
    page: Option<u32>,
    root: Option<u32>,
}

struct Posting {
    record: u32,
    field: u8,
    tf: u16,
}

#[derive(Default)]
struct IndexTerm {
    df: u32,
    postings: Vec<Posting>,
}

struct Page {
    id: String,
    name: String,
    internal: bool,
}

pub struct Index {
    records: Vec<Record>,
    terms: BTreeMap<String, IndexTerm>,
    pages: Vec<Page>,
    roots: Vec<String>,
    /// Stored ancestors above the scope root, top-down.
    above: Vec<(String, String)>,
    avg: [f64; 2],
    stop: Option<&'static str>,
    hidden_content: usize,
    text_bytes: usize,
    generation: String,
}

fn kiwi_text<'a>(node: &'a kiwi_schema::Value) -> Option<&'a str> {
    Some(node.get("textData")?.get("characters")?.as_string())
}

fn kiwi_name<'a>(node: &'a kiwi_schema::Value) -> &'a str {
    node.get("name").map_or("", |v| v.as_string())
}

fn kiwi_hidden(node: &kiwi_schema::Value) -> bool {
    node.get("visible").map(|v| v.as_bool()) == Some(false)
}

fn kiwi_internal(node: &kiwi_schema::Value) -> bool {
    node.get("internalOnly").map(|v| v.as_bool()) == Some(true)
}

/// Peak bytes normalizing and tokenizing `text` allocates: per compatibility character at
/// most three folded units, each with its string byte and a token share. NFKC buffers each run
/// of combining marks between starters, so a raw scan refuses a run over `MAX_MARKS` (None) before
/// any normalization; the count then checks the deadline as it goes.
fn transient(text: &str, deadline: Instant) -> Result<usize, Option<Stop>> {
    let mut run = 0usize;
    for c in text.chars() {
        let mut starter = c.is_ascii();
        if !starter {
            decompose_compatible(c, |d| starter |= canonical_combining_class(d) == 0);
        }
        run = if starter { 0 } else { run + 1 };
        if run > MAX_MARKS {
            return Err(None);
        }
    }
    let mut n = 0usize;
    for _ in text.nfkc() {
        n += 1;
        if n.is_multiple_of(65_536) && Instant::now() >= deadline {
            return Err(Some(Stop::Deadline));
        }
    }
    Ok(n * 3 * 160)
}

/// Indexes every stored and effective layer under `scope`, in document order, until the
/// shared bounds run out. Each step is charged before the visit, copy or allocation it pays for.
fn build(tree: &Tree, scope: &str, bounds: Bounds) -> Index {
    // An eighth of the work and bytes is kept from expansion, so a stop there still leaves room to
    // index what was expanded and the stored layers; only a passed deadline ends both. Indexing
    // a 179 MB held-out file took 139k work and 35 MB against its expansion's 7.1M and 394 MB.
    let reserve = (bounds.work / 8, bounds.bytes / 8);
    let (ex, mut bounds) = expand_instances_bounded(
        tree,
        scope,
        Bounds {
            work: bounds.work - reserve.0,
            bytes: bounds.bytes - reserve.1,
            ..bounds
        },
    );
    let expansion = bounds.stop.take_if(|stop| *stop != Stop::Deadline);
    bounds.work += reserve.0;
    bounds.bytes += reserve.1;
    let mut index = Index {
        records: Vec::new(),
        terms: BTreeMap::new(),
        pages: Vec::new(),
        roots: Vec::new(),
        above: Vec::new(),
        avg: [0.0; 2],
        stop: None,
        hidden_content: 0,
        text_bytes: 0,
        generation: String::new(),
    };
    // Stored ancestors give the scope its page, navigation root and inherited visibility.
    let mut chain = Vec::new();
    let mut cur = tree.nodes.get(scope).and_then(|n| parent_of(n));
    while let Some((id, node)) = cur.and_then(|id| Some((id.clone(), *tree.nodes.get(&id)?))) {
        if !bounds.charge(1, 128 + 2 * (id.len() + kiwi_name(node).len())) {
            break;
        }
        cur = parent_of(node);
        chain.push((id, node));
    }
    let (mut page, mut root, mut hidden, mut parent_canvas) = (None, None, false, false);
    for (id, node) in chain.into_iter().rev() {
        hidden |= kiwi_hidden(node);
        let kind = plugin_type(node);
        if kind == "CANVAS" {
            page = Some(index.pages.len() as u32);
            index.pages.push(Page {
                id: id.clone(),
                name: kiwi_name(node).to_owned(),
                internal: kiwi_internal(node),
            });
        } else if parent_canvas {
            root = Some(index.roots.len() as u32);
            index.roots.push(id.clone());
        }
        parent_canvas = kind == "CANVAS";
        index.above.push((id, kiwi_name(node).to_owned()));
    }
    let mut stack = vec![(scope.to_owned(), None, hidden, page, root, parent_canvas)];
    let mut lens = [0u64; 2];
    while let Some((id, parent, hidden, page, root, parent_canvas)) = stack.pop() {
        let (name, kind, text, text_source, source, own_hidden) = match ex.nodes.get(&id) {
            Some(e) => {
                let main = tree.nodes[&e.source];
                let own = e
                    .props
                    .get("textData")
                    .and_then(|t| t["characters"].as_str());
                let text_source = match (own, kiwi_text(main)) {
                    (Some(_), _) => Some("override"),
                    (None, Some(_)) => Some("component"),
                    _ => None,
                };
                let visible = e.props.get("visible").and_then(Json::as_bool);
                let own_hidden = visible.map_or_else(|| kiwi_hidden(main), |v| !v);
                let text = own.or_else(|| kiwi_text(main));
                (
                    e.name.as_str(),
                    e.kind.clone(),
                    text,
                    text_source,
                    Some(&e.source),
                    own_hidden,
                )
            }
            None => {
                let node = tree.nodes[&id];
                let text = kiwi_text(node);
                (
                    kiwi_name(node),
                    plugin_type(node),
                    text,
                    text.map(|_| "stored"),
                    None,
                    kiwi_hidden(node),
                )
            }
        };
        let text_len = text.map_or(0, str::len);
        if index.text_bytes + text_len > TEXT_BYTES {
            index.stop = Some("text_bytes_limit");
            break;
        }
        // Originals and normalized copies, ids and the record, before allocating them.
        let retained = 256 + 2 * id.len() + 3 * (name.len() + text_len) + kind.len();
        let peak = [Some(name), text]
            .into_iter()
            .flatten()
            .map(|t| transient(t, bounds.deadline))
            .sum::<Result<usize, Option<Stop>>>();
        match peak {
            Err(Some(stop)) => {
                bounds.stop = Some(stop);
                break;
            }
            Err(None) => {
                index.stop = Some("combining_run_limit");
                break;
            }
            Ok(peak) if peak > bounds.bytes => {
                bounds.stop = Some(Stop::Bytes);
                break;
            }
            Ok(_) => {}
        }
        if !bounds.charge(1, retained) {
            break;
        }
        let mut norm = [String::new(), String::new()];
        let mut record_lens = [0u32; 2];
        let mut tf: HashMap<String, [u16; 2]> = HashMap::new();
        for (f, value) in [Some(name), text].into_iter().enumerate() {
            let Some(value) = value else { continue };
            let units = normalize(value);
            norm[f] = units.iter().map(|u| u.c).collect();
            let found = tokens(&units, false);
            record_lens[f] = found.len().min(u32::MAX as usize) as u32;
            for token in found {
                let n = &mut tf.entry(token.term).or_default()[f];
                *n = n.saturating_add(1);
            }
        }
        let hidden = hidden || own_hidden;
        let children = tree.children.get(&id).or_else(|| ex.children.get(&id));
        let kids = children.map_or(&[][..], Vec::as_slice);
        let queued = kids.iter().map(|c| 96 + c.len()).sum::<usize>();
        // Postings and queued children; nothing is committed to the index until both charges pass.
        if !bounds.charge(kids.len(), 24 * tf.len() + queued) {
            break;
        }
        index.text_bytes += text_len;
        lens[0] += u64::from(record_lens[0]);
        lens[1] += u64::from(record_lens[1]);
        if hidden && children.is_none() && hides_content(tree, &ex, &id, &kind) {
            index.hidden_content += 1;
        }
        let record = index.records.len() as u32;
        let (page, root) = if kind == "CANVAS" {
            index.pages.push(Page {
                id: id.clone(),
                name: name.to_owned(),
                internal: tree.nodes.get(&id).is_some_and(|n| kiwi_internal(n)),
            });
            (Some(index.pages.len() as u32 - 1), None)
        } else if parent_canvas {
            index.roots.push(id.clone());
            (page, Some(index.roots.len() as u32 - 1))
        } else {
            (page, root)
        };
        for (term, counts) in tf {
            let entry = index.terms.entry(term).or_default();
            entry.df += 1;
            for (field, tf) in counts.into_iter().enumerate().filter(|(_, n)| *n > 0) {
                entry.postings.push(Posting {
                    record,
                    field: field as u8,
                    tf,
                });
            }
        }
        index.records.push(Record {
            id,
            name: name.to_owned(),
            kind: kind.clone(),
            text: text.map(str::to_owned),
            norm,
            lens: record_lens,
            parent,
            source: source.cloned(),
            text_source,
            hidden,
            page,
            root,
        });
        for child in kids.iter().rev() {
            stack.push((
                child.clone(),
                Some(record),
                hidden,
                page,
                root,
                kind == "CANVAS",
            ));
        }
    }
    if index.stop.is_none() {
        index.stop = bounds.stop.or(expansion).map(|stop| match stop {
            Stop::Work => "work_limit",
            Stop::Bytes => "byte_limit",
            Stop::Depth => "depth_limit",
            Stop::Deadline => "deadline",
        });
    }
    let n = index.records.len().max(1) as f64;
    index.avg = [(lens[0] as f64 / n).max(1.0), (lens[1] as f64 / n).max(1.0)];
    index.generation = generation(&index);
    index
}

/// Identifies the indexed corpus, so a cursor never pages over another partial build of the
/// same snapshot: every record's identity, normalized fields and flags, plus the stop code.
fn generation(index: &Index) -> String {
    let mut h = Sha256::new();
    for r in &index.records {
        for part in [
            r.id.as_str(),
            &r.kind,
            &r.norm[0],
            &r.norm[1],
            r.text_source.unwrap_or(""),
        ] {
            h.update(part.len().to_le_bytes());
            h.update(part);
        }
        h.update([
            u8::from(r.hidden),
            u8::from(r.page.is_some()),
            u8::from(r.root.is_some()),
        ]);
    }
    h.update(index.stop.unwrap_or(""));
    hex(&h.finalize())[..16].to_owned()
}

/// A hidden layer whose content expansion leaves out: a stored instance without stored
/// children, or an expanded sublayer whose definition has children.
fn hides_content(tree: &Tree, ex: &Expansion, id: &str, kind: &str) -> bool {
    match ex.nodes.get(id) {
        Some(e) => kind == "INSTANCE" || tree.children.contains_key(&e.source),
        None => kind == "INSTANCE",
    }
}

// ---- Retrieval ----

struct Hit {
    record: u32,
    tier: u8,
    score: f64,
}

fn idf(n: usize, df: u32) -> f64 {
    let (n, df) = (n as f64, f64::from(df));
    (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
}

/// Ranked page of `req` over `index`: candidates must contain every query term; exact name,
/// name phrase and text phrase matches lead, then field-weighted BM25, then document order.
fn retrieve(index: &Index, q: &Prepared, req: &Request) -> (Vec<Hit>, bool, [usize; 2]) {
    let n = index.records.len();
    let mut lists: Vec<&IndexTerm> = Vec::new();
    for term in &q.terms {
        match index.terms.get(term) {
            Some(found) => lists.push(found),
            None => return (Vec::new(), false, [0; 2]),
        }
    }
    lists.sort_by_key(|t| t.postings.len());
    // Score and the last term each record matched: after term i only records matching it stay,
    // so the survivors match every term, with no bitmask width to outgrow.
    let mut acc: HashMap<u32, (f64, usize)> = HashMap::new();
    for (i, term) in lists.iter().enumerate() {
        let idf = idf(n, term.df);
        for p in &term.postings {
            if i > 0 && !acc.contains_key(&p.record) {
                continue;
            }
            let r = &index.records[p.record as usize];
            let f = usize::from(p.field);
            let tf = f64::from(p.tf);
            let norm = 1.0 - B + B * f64::from(r.lens[f]) / index.avg[f];
            let entry = acc.entry(p.record).or_insert((0.0, 0));
            entry.0 += idf * WEIGHTS[f] * tf * (K1 + 1.0) / (tf + K1 * norm);
            entry.1 = i;
        }
        acc.retain(|_, (_, last)| *last == i);
    }
    let mut excluded = [0usize; 2];
    let mut hits: Vec<Hit> = Vec::new();
    for (record, (score, _)) in acc {
        let r = &index.records[record as usize];
        if !req.types.is_empty() && !req.types.contains(&r.kind) {
            continue;
        }
        let internal = r.page.is_some_and(|p| index.pages[p as usize].internal);
        if r.hidden && !req.include_hidden {
            excluded[0] += 1;
            continue;
        }
        if internal && !req.include_internal {
            excluded[1] += 1;
            continue;
        }
        let [name, text] = &r.norm;
        let tier = if *name == q.phrase {
            3
        } else if phrase_at(name, &q.phrase).is_some() {
            2
        } else if phrase_at(text, &q.phrase).is_some() {
            1
        } else {
            0
        };
        hits.push(Hit {
            record,
            tier,
            score,
        });
    }
    let order = |a: &Hit, b: &Hit| {
        b.tier
            .cmp(&a.tier)
            .then(b.score.total_cmp(&a.score))
            .then(a.record.cmp(&b.record))
    };
    // Select only the requested page plus one before sorting it.
    let need = q.offset + req.limit + 1;
    if hits.len() > need {
        hits.select_nth_unstable_by(need - 1, order);
        hits.truncate(need);
    }
    hits.sort_unstable_by(order);
    let more = hits.len() > q.offset + req.limit;
    let page = hits.into_iter().skip(q.offset).take(req.limit).collect();
    (page, more, excluded)
}

fn utf16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Keeps a character with its marks and emoji joiners.
fn attached(text: &str, at: usize) -> bool {
    let next = text[at..].chars().next();
    let prev = text[..at].chars().next_back();
    next.is_some_and(|c| canonical_combining_class(c) != 0 || class(c) == Class::Join)
        || prev == Some('\u{200D}')
}

fn snippet(text: &str, start: usize, end: usize) -> Json {
    let (mut a, mut b) = (start, end);
    for _ in 0..40 {
        match text[..a].chars().next_back() {
            Some(c) => a -= c.len_utf8(),
            None => break,
        }
    }
    for _ in 0..80 {
        match text[b..].chars().next() {
            Some(c) => b += c.len_utf8(),
            None => break,
        }
    }
    while a > 0 && attached(text, a) {
        a -= text[..a].chars().next_back().unwrap().len_utf8();
    }
    while b < text.len() && attached(text, b) {
        b += text[b..].chars().next().unwrap().len_utf8();
    }
    json!({"text": &text[a..b], "sourceStart": utf16(&text[..a]),
        "truncatedStart": a > 0, "truncatedEnd": b < text.len()})
}

/// The first byte offset where `phrase` occurs in `norm` without splitting a word: letters and
/// digits on both sides of an edge mean a longer word, while CJK has no such boundary.
fn phrase_at(norm: &str, phrase: &str) -> Option<usize> {
    let (first, last) = (phrase.chars().next()?, phrase.chars().next_back()?);
    let joins = |a: Option<char>, b: char| {
        a.is_some_and(|a| class(a) == Class::Word && class(b) == Class::Word)
    };
    norm.match_indices(phrase).map(|(at, _)| at).find(|&at| {
        !joins(norm[..at].chars().next_back(), first)
            && !joins(norm[at + phrase.len()..].chars().next(), last)
    })
}

/// Why `original` matched: reason, matched terms and the UTF-16 span in the original field.
fn explain(field: u8, original: &str, q: &Prepared) -> Option<Json> {
    let units = normalize(original);
    let norm: String = units.iter().map(|u| u.c).collect();
    let found = tokens(&units, false);
    let terms: Vec<&String> = q
        .terms
        .iter()
        .filter(|t| found.iter().any(|k| k.term == **t))
        .collect();
    let (reason, range) = if norm == q.phrase {
        ("exact", Some((0, units.len())))
    } else if let Some(at) = phrase_at(&norm, &q.phrase) {
        let a = norm[..at].chars().count();
        ("phrase", Some((a, a + q.phrase.chars().count())))
    } else {
        let first = found.iter().find(|k| q.terms.contains(&k.term));
        ("terms", first.map(|k| (k.start, k.end)))
    };
    let (a, b) = range.filter(|(a, b)| a < b)?;
    let (start, end) = (units[a].start, units[b - 1].end);
    let mut out = json!({
        "field": if field == NAME { "name" } else { "text" },
        "reason": reason,
        "terms": terms,
        "span": {"start": utf16(&original[..start]), "end": utf16(&original[..end]), "unit": "utf16"},
    });
    if field == TEXT {
        out["snippet"] = snippet(original, start, end);
    }
    Some(out)
}

fn result(index: &Index, hit: &Hit, rank: usize, q: &Prepared, source: &Json) -> Json {
    let r = &index.records[hit.record as usize];
    let mut path: Vec<Json> = Vec::new();
    let mut cur = Some(hit.record);
    while let Some(i) = cur {
        let node = &index.records[i as usize];
        path.push(json!({"id": node.id, "name": node.name}));
        cur = node.parent;
    }
    path.extend(
        index
            .above
            .iter()
            .rev()
            .map(|(id, name)| json!({"id": id, "name": name})),
    );
    path.reverse();
    let page = r.page.map(|p| &index.pages[p as usize]);
    let matches: Vec<Json> = [explain(NAME, &r.name, q)]
        .into_iter()
        .chain([r.text.as_deref().and_then(|t| explain(TEXT, t, q))])
        .flatten()
        .collect();
    let mut out = json!({
        "ref": {"sourceId": source["sourceId"], "snapshotId": source["snapshotId"], "nodeId": r.id},
        "rank": rank,
        "name": r.name,
        "type": r.kind,
        "origin": if r.source.is_some() { "instance-effective" } else { "stored" },
        "hidden": r.hidden,
        "internal": page.is_some_and(|p| p.internal),
        "page": page.map(|p| json!({"id": p.id, "name": p.name})),
        "scopeRoot": r.root.map(|i| &index.roots[i as usize]),
        "path": path,
        "matches": matches,
    });
    if let Some(source) = &r.source {
        out["sourceNodeId"] = json!(source);
    }
    if let (Some(text), Some(from)) = (&r.text, r.text_source) {
        out["textSource"] = json!(from);
        out["textLength"] = json!(utf16(text));
    }
    out
}

fn coverage(index: &Index, req: &Request, excluded: [usize; 2]) -> Json {
    let scanned = index.records.len();
    let mut reasons = Vec::new();
    if let Some(code) = index.stop {
        reasons.push(json!({"code": code, "counts": {"scannedRecords": scanned}}));
    }
    // REST output omits what hidden instances and hidden sublayers contain, so no exact ID exists.
    if req.include_hidden && index.hidden_content > 0 {
        reasons.push(json!({"code": "hidden_content_not_expanded", "counts": {"layers": index.hidden_content}}));
    }
    let complete = reasons.is_empty();
    json!({
        "complete": complete,
        "scannedRecords": scanned,
        "totalRecords": if complete { json!(scanned) } else { Json::Null },
        "indexedTextBytes": index.text_bytes,
        "layers": {"text": {"state": if complete { "ready" } else { "partial" }}},
        "reasons": reasons,
        "excluded": {"hidden": excluded[0], "internal": excluded[1]},
        "knownGaps": [{"code": "component_property_values_not_indexed"},
            {"code": "non_text_layer_content_not_indexed"}],
        "limits": {"work": WORK, "bytes": BYTES, "textBytes": TEXT_BYTES,
            "depth": DEPTH, "deadlineMs": DEADLINE.as_millis() as u64, "sourceBytes": SOURCE_BYTES,
            "combiningRun": MAX_MARKS},
    })
}

/// The reply for `req` over an index whose source and snapshot `source` names.
fn reply(index: &Index, q: &Prepared, req: &Request, source: Json) -> Result<Json, Box<dyn Error>> {
    let fingerprint = req.fingerprint(q, &source, index);
    if q.fingerprint.as_ref().is_some_and(|fp| *fp != fingerprint) {
        return Err(invalid(
            "search cursor belongs to another query, filter set, source, snapshot or index generation; search again without cursor",
        ));
    }
    let (hits, more, excluded) = retrieve(index, q, req);
    let results: Vec<Json> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| result(index, hit, q.offset + i + 1, q, &source))
        .collect();
    let scope = index.records.first();
    Ok(json!({
        "query": {"text": req.query, "normalized": q.phrase, "mode": "text", "terms": q.terms},
        "source": source,
        "scope": {"nodeId": req.scope, "name": scope.map(|r| r.name.as_str())},
        "ranking": {"version": RANKING, "tokenizer": tokenizer(), "weights": {"name": WEIGHTS[0], "text": WEIGHTS[1]},
            "k1": K1, "b": B, "order": ["exact-name", "name-phrase", "text-phrase", "score", "document-order"]},
        "results": results,
        "coverage": coverage(index, req, excluded),
        "hasMore": more,
        "nextCursor": (more && q.offset + req.limit <= MAX_OFFSET)
            .then(|| format!("s1.{fingerprint}.{}", q.offset + req.limit)),
    }))
}

// ---- Source snapshot and cache ----

type Stamp = (SystemTime, u64);

fn stamp(path: &Path) -> std::io::Result<Stamp> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "search reads a regular .fig file",
        ));
    }
    Ok((meta.modified()?, meta.len()))
}

/// The source's bytes, read once within `SOURCE_BYTES` while its size and time stayed put.
fn stage(path: &Path) -> Result<(Vec<u8>, Stamp), Box<dyn Error>> {
    for _ in 0..3 {
        let before = stamp(path)?;
        if before.1 > SOURCE_BYTES {
            return Err(crate::parse::exceeded("source bytes", SOURCE_BYTES).into());
        }
        let mut bytes = Vec::with_capacity(before.1 as usize);
        std::fs::File::open(path)?
            .take(SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 == before.1 && stamp(path)? == before {
            return Ok((bytes, before));
        }
    }
    Err(FigError::new(
        ErrorKind::SourceUnreadable,
        "the .fig kept changing while it was read; retry after the save finishes",
    )
    .into())
}

struct Cached {
    path: PathBuf,
    stamp: Stamp,
    scope: String,
    snapshot: String,
    hashed: SystemTime,
    index: Arc<Index>,
}

// shortcut: one index per process, reused while path, scope and file stamp match; persistent
// multi-scope indexes are #90's workspace.
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

fn require_scope(tree: &Tree, scope: &str) -> Result<(), Box<dyn Error>> {
    if tree.nodes.contains_key(scope) {
        return Ok(());
    }
    Err(if scope.starts_with('I') {
        FigError::new(ErrorKind::UnsupportedNode, format!(
            "node {scope} is an instance sublayer; search scopes are stored nodes, so search its stored instance or frame and filter the results"
        ))
    } else {
        FigError::new(
            ErrorKind::NodeNotFound,
            format!("node {scope} not found; list ids with `figlens tree <file.fig> --depth 2`"),
        )
    }
    .into())
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Searches `scope` of the `.fig` at `path`. The snapshot is the SHA-256 of the exact bytes
/// decoded. A later call reuses the index while path, scope and file stamp are unchanged, and
/// hashes the bytes again once `REHASH` has passed, since a save can keep the stamp.
pub fn search(path: &Path, req: &Request) -> Result<Json, Box<dyn Error>> {
    let q = req.validate()?;
    let deadline = Instant::now() + DEADLINE;
    let canonical = std::fs::canonicalize(path)?;
    let source_id = format!(
        "local:{}",
        short_hash(canonical.to_string_lossy().as_bytes())
    );
    let now = stamp(&canonical)?;
    let cached = CACHE.lock().ok().and_then(|cache| {
        cache
            .as_ref()
            .filter(|c| c.path == canonical && c.stamp == now && c.scope == req.scope)
            .map(|c| (c.snapshot.clone(), c.index.clone(), c.hashed))
    });
    let fresh = |hashed: SystemTime| hashed.elapsed().is_ok_and(|age| age < REHASH);
    let (snapshot, index, hashed, reused) = match cached {
        Some((snapshot, index, hashed)) if fresh(hashed) => {
            // Metadata alone does not prove the source is still readable.
            std::fs::File::open(&canonical)?;
            (snapshot, index, hashed, true)
        }
        cached => {
            let (bytes, staged) = stage(&canonical)?;
            let hashed = SystemTime::now();
            let snapshot = format!("sha256:{}", hex(&Sha256::digest(&bytes)));
            match cached.filter(|(s, ..)| *s == snapshot) {
                Some((snapshot, index, _)) => {
                    if let Ok(mut cache) = CACHE.lock()
                        && let Some(c) = cache.as_mut()
                        && (c.path == canonical && c.scope == req.scope && c.snapshot == snapshot)
                    {
                        c.hashed = hashed;
                    }
                    (snapshot, index, hashed, true)
                }
                None => {
                    let mut doc = Document::from_bytes(&bytes, &path.display().to_string())?;
                    drop(bytes);
                    let message = doc.take_message()?;
                    let tree = Tree::new(&message);
                    require_scope(&tree, &req.scope)?;
                    let bounds = Bounds {
                        work: WORK,
                        bytes: BYTES,
                        depth: DEPTH,
                        deadline,
                        stop: None,
                    };
                    let index = Arc::new(build(&tree, &req.scope, bounds));
                    if stamp(&canonical)? != staged {
                        return Err(FigError::new(
                            ErrorKind::SourceUnreadable,
                            "the .fig changed while it was searched; retry after the save finishes",
                        )
                        .into());
                    }
                    // A deadline stop depends on load, not the file; the next call rebuilds.
                    if index.stop != Some("deadline")
                        && let Ok(mut cache) = CACHE.lock()
                    {
                        *cache = Some(Cached {
                            path: canonical.clone(),
                            stamp: staged,
                            scope: req.scope.clone(),
                            snapshot: snapshot.clone(),
                            hashed,
                            index: index.clone(),
                        });
                    }
                    (snapshot, index, hashed, false)
                }
            }
        }
    };
    let source = json!({"sourceId": source_id, "snapshotId": snapshot, "freshness": {
        "reusedIndex": reused, "hashedAtMs": ms(hashed), "stampCheckedAtMs": ms(SystemTime::now()),
        "rehashAfterMs": REHASH.as_millis() as u64}});
    reply(&index, &q, req, source)
}

#[cfg(test)]
mod tests;
