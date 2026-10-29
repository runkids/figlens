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
/// Sources one workspace search reads together; the supported workspace holds at most 10.
pub(crate) const MAX_SOURCES: usize = 16;

/// Work bounds of one scope extraction, independent of the result limit.
const SOURCE_BYTES: u64 = 512 * 1024 * 1024;
/// Expanding a 179 MB held-out file charged 7.1M; each override value walked counts once.
const WORK: usize = 10_000_000;
const BYTES: usize = 512 * 1024 * 1024;
const TEXT_BYTES: usize = 64 * 1024 * 1024;
const DEPTH: usize = 256;
/// Combining marks in a row NFKC may buffer; stream-safe text keeps at most 30.
const MAX_MARKS: usize = 1024;
/// Bytes one snippet may copy, its context and matched span together.
const SNIPPET_BYTES: usize = 2048;
/// Names, ids and paths one page of results may copy; the MCP reply cap, so CLI and MCP agree.
const REPLY_BYTES: usize = 8 * 1024 * 1024;
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
        "nfkc-{}.{}-casefold-{}.{}-segment-cjk-bigram-camel-3",
        n.0, n.1, c.0, c.1
    )
}

/// The first 16 hex digits of a SHA-256: enough to tell sources, queries and builds apart.
fn short_hash(data: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(data))[..16].to_owned()
}

/// The stable id of the local source at the canonical `path`; a workspace keeps it on relocate.
pub(crate) fn source_id(path: &Path) -> String {
    format!("local:{}", short_hash(path.to_string_lossy().as_bytes()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
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
    /// The generations a workspace cursor paged, `None` for a stateless cursor.
    gens: Option<Vec<i64>>,
}

impl Prepared {
    pub(crate) fn terms(&self) -> &[String] {
        &self.terms
    }

    /// The generations of the sources a workspace cursor paged.
    pub(crate) fn cursor_gens(&self) -> Option<&[i64]> {
        self.gens.as_deref()
    }
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
        let (offset, fingerprint, gens) = match &self.cursor {
            None => (0, None, None),
            Some(cursor) => {
                // A workspace cursor also lists the generations it paged: `w1.fp.offset.g-g`.
                let (rest, gens) = match cursor.strip_prefix("w1.") {
                    Some(rest) => match rest.rsplit_once('.') {
                        Some((rest, gens)) => (
                            Some(rest),
                            gens.split('-')
                                .map(|g| g.parse::<i64>().ok())
                                .collect::<Option<Vec<_>>>()
                                .filter(|g| (1..=MAX_SOURCES).contains(&g.len())),
                        ),
                        None => (None, None),
                    },
                    None => (cursor.strip_prefix("s1."), None),
                };
                let parsed = rest
                    .filter(|_| cursor.starts_with("s1.") || gens.is_some())
                    .and_then(|c| c.split_once('.'))
                    .filter(|(fp, _)| fp.len() == 16 && fp.bytes().all(|b| b.is_ascii_hexdigit()))
                    .and_then(|(fp, offset)| Some((offset.parse::<usize>().ok()?, fp)))
                    .filter(|(offset, _)| (1..=MAX_OFFSET).contains(offset));
                let (offset, fp) = parsed.ok_or_else(|| {
                    invalid("invalid search cursor; pass nextCursor from the previous reply")
                })?;
                (offset, Some(fp.to_owned()), gens)
            }
        };
        Ok(Prepared {
            phrase,
            terms,
            offset,
            fingerprint,
            gens,
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

/// NFKC, full Unicode case folding and NFKC again, applied per segment (a base character with
/// its combining marks) so each output character maps back to original bytes. Whitespace runs
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
        // Camel steps are read per compatibility character, then the whole segment is folded and
        // composed together, since folding can enable composition across its characters.
        let mut camel = Vec::new();
        for k in text[start..end].nfkc() {
            camel.push(k.is_uppercase() && lower);
            lower = k.is_lowercase();
            camel.extend(std::iter::repeat_n(
                false,
                std::iter::once(k).default_case_fold().count() - 1,
            ));
        }
        let folded: Vec<char> = text[start..end].nfkc().default_case_fold().nfkc().collect();
        // shortcut: a fold that changes the length keeps only the segment's first camel step.
        if folded.len() != camel.len() {
            camel.truncate(1);
        }
        for (i, &c) in folded.iter().enumerate() {
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
                camel: camel.get(i).copied().unwrap_or(false),
            });
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

pub(crate) struct Record {
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
    /// Its relations, `contains` excepted (that is `parent`), in relation and sequence order.
    edges: Vec<usages::Edge>,
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
    /// Postings before a partial load kept only the surviving records'; ranking orders terms
    /// by it, so scores sum in the same order as over the whole index.
    full: Option<usize>,
}

impl IndexTerm {
    fn size(&self) -> usize {
        self.full.unwrap_or(self.postings.len())
    }
}

struct Page {
    id: String,
    name: String,
    internal: bool,
}

pub struct Index {
    records: Vec<Record>,
    /// The record IDs of `records` when only some were loaded, ascending; empty when all were.
    rids: Vec<u32>,
    /// Records in the whole index, loaded or not.
    total: usize,
    terms: BTreeMap<String, IndexTerm>,
    pages: Vec<Page>,
    roots: Vec<String>,
    /// Stored ancestors above the scope root, top-down.
    above: Vec<(String, String)>,
    /// Token counts summed per field, so several indexes share one average.
    lens: [u64; 2],
    stop: Option<&'static str>,
    hidden_content: usize,
    /// Prototype links inherited from a component definition, left out as unverified.
    unverified_links: usize,
    /// Binding fields left out for being too large to read.
    unread_fields: usize,
    text_bytes: usize,
    generation: String,
    /// Whether every relation kind was extracted; a text-only build (ordinary stateless search)
    /// holds only `TEXT_ONLY` kinds and never answers usages.
    relations: bool,
    /// Edge targets and component-set members described for usages; a partial index holds the
    /// ones a query read.
    definitions: BTreeMap<String, usages::Definition>,
    /// Reverse lookups over a whole index, built on its first usages query.
    reverse: std::sync::OnceLock<usages::Reverse>,
}

impl Index {
    /// Record `rid`, which a partial index must have loaded.
    fn record(&self, rid: u32) -> &Record {
        if self.rids.is_empty() {
            &self.records[rid as usize]
        } else {
            &self.records[self.rids.binary_search(&rid).expect("loaded record")]
        }
    }

    /// The scope root's record, the first in document order.
    fn scope(&self) -> Option<&Record> {
        (self.total > 0).then(|| self.record(0))
    }
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
#[cfg(test)]
fn build(tree: &Tree, scope: &str, bounds: Bounds) -> Index {
    build_as(tree, scope, bounds, true)
}

fn build_as(tree: &Tree, scope: &str, bounds: Bounds, relations: bool) -> Index {
    // A full build describes definitions within the caller's bounds: an eighth is kept back
    // from expansion and indexing, and whatever those leave unused is added to it. A text-only
    // build keeps nothing back.
    let keep = match relations {
        true => (bounds.work / 8, bounds.bytes / 8),
        false => (0, 0),
    };
    let bounds = Bounds {
        work: bounds.work - keep.0,
        bytes: bounds.bytes - keep.1,
        ..bounds
    };
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
        rids: Vec::new(),
        total: 0,
        terms: BTreeMap::new(),
        pages: Vec::new(),
        roots: Vec::new(),
        above: Vec::new(),
        lens: [0; 2],
        stop: None,
        hidden_content: 0,
        unverified_links: 0,
        unread_fields: 0,
        text_bytes: 0,
        generation: String::new(),
        relations: false,
        definitions: BTreeMap::new(),
        reverse: Default::default(),
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
        // Kept before allocating them: the record and its id, name, text, kind and source
        // copies, plus a page's id and name or a top-level frame's id. Normalized copies are
        // charged at their real size below, since NFKC can expand a character many times over.
        let listed = match kind.as_str() {
            "CANVAS" => id.len() + name.len(),
            _ if parent_canvas => id.len(),
            _ => 0,
        };
        let retained = 256
            + id.len()
            + listed
            + name.len()
            + text_len
            + kind.len()
            + source.map_or(0, |s| s.len());
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
        let Some(mut edges) = usages::extract(
            tree,
            &id,
            &kind,
            ex.nodes.get(&id),
            &mut bounds,
            relations.then_some((&mut index.unverified_links, &mut index.unread_fields)),
        ) else {
            break;
        };
        edges.sort_by(|a, b| (a.relation, a.seq).cmp(&(b.relation, b.seq)));
        let children = tree.children.get(&id).or_else(|| ex.children.get(&id));
        let kids = children.map_or(&[][..], Vec::as_slice);
        let queued = kids.iter().map(|c| 96 + c.len()).sum::<usize>();
        let normalized =
            norm.iter().map(String::len).sum::<usize>() + tf.keys().map(String::len).sum::<usize>();
        // Normalized copies, terms, postings and queued children; nothing is committed to the
        // index until both charges pass.
        if !bounds.charge(kids.len(), 24 * tf.len() + queued + normalized) {
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
            edges,
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
    // The kept share means a build its records stopped still describes the targets it kept; a
    // stop here leaves some undescribed and marks coverage partial below.
    let mut own = Bounds {
        work: keep.0 + bounds.work,
        bytes: keep.1 + bounds.bytes,
        depth: DEPTH,
        deadline: bounds.deadline,
        stop: None,
    };
    if relations {
        index.definitions = usages::definitions(tree, &index.records, &mut own).unwrap_or_default();
    }
    index.relations = relations;
    bounds.stop = bounds.stop.or(own.stop);
    if index.stop.is_none() {
        index.stop = bounds.stop.or(expansion).map(|stop| match stop {
            Stop::Work => "work_limit",
            Stop::Bytes => "byte_limit",
            Stop::Depth => "depth_limit",
            Stop::Deadline => "deadline",
        });
    }
    index.lens = lens;
    index.total = index.records.len();
    index.generation = generation(&index);
    index
}

fn average(n: usize, lens: [u64; 2]) -> [f64; 2] {
    let n = n.max(1) as f64;
    [(lens[0] as f64 / n).max(1.0), (lens[1] as f64 / n).max(1.0)]
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
    /// Position of the hit's index in the corpus.
    source: usize,
    record: u32,
    tier: u8,
    score: f64,
}

fn idf(n: usize, df: u32) -> f64 {
    let (n, df) = (n as f64, f64::from(df));
    (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
}

/// Ranked page of `req` over every index of `corpus` together: candidates must contain every
/// query term; exact name, name phrase and text phrase matches lead, then field-weighted BM25,
/// then corpus order and document order. Record counts, document frequencies and field lengths
/// are summed over the whole corpus before any record is scored, so one source's hits are never
/// ranked alone and merged afterwards; one index ranks exactly as it always did.
fn retrieve(corpus: &[&Index], q: &Prepared, req: &Request) -> (Vec<Hit>, bool, [usize; 2]) {
    let n: usize = corpus.iter().map(|i| i.total).sum();
    let lens = corpus
        .iter()
        .fold([0u64; 2], |a, i| [a[0] + i.lens[0], a[1] + i.lens[1]]);
    let avg = average(n, lens);
    let df: Vec<u32> = q
        .terms
        .iter()
        .map(|t| {
            corpus
                .iter()
                .filter_map(|i| i.terms.get(t))
                .map(|t| t.df)
                .sum()
        })
        .collect();
    let mut excluded = [0usize; 2];
    let mut hits: Vec<Hit> = Vec::new();
    for (source, index) in corpus.iter().enumerate() {
        let mut lists: Vec<(&IndexTerm, u32)> = Vec::new();
        for (term, &df) in q.terms.iter().zip(&df) {
            match index.terms.get(term) {
                Some(found) => lists.push((found, df)),
                None => break,
            }
        }
        if lists.len() < q.terms.len() {
            continue;
        }
        lists.sort_by_key(|(t, _)| t.size());
        // Score and the last term each record matched: after term i only records matching it
        // stay, so the survivors match every term, with no bitmask width to outgrow.
        let mut acc: HashMap<u32, (f64, usize)> = HashMap::new();
        for (i, (term, df)) in lists.iter().enumerate() {
            let idf = idf(n, *df);
            for p in &term.postings {
                if i > 0 && !acc.contains_key(&p.record) {
                    continue;
                }
                let r = index.record(p.record);
                let f = usize::from(p.field);
                let tf = f64::from(p.tf);
                let norm = 1.0 - B + B * f64::from(r.lens[f]) / avg[f];
                let entry = acc.entry(p.record).or_insert((0.0, 0));
                entry.0 += idf * WEIGHTS[f] * tf * (K1 + 1.0) / (tf + K1 * norm);
                entry.1 = i;
            }
            acc.retain(|_, (_, last)| *last == i);
        }
        for (record, (score, _)) in acc {
            let r = index.record(record);
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
                source,
                record,
                tier,
                score,
            });
        }
    }
    let order = |a: &Hit, b: &Hit| {
        b.tier
            .cmp(&a.tier)
            .then(b.score.total_cmp(&a.score))
            .then(a.source.cmp(&b.source))
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
    // Stream-safe text attaches at most 30 marks; a longer crafted run is cut there.
    for _ in 0..32 {
        if a == 0 || !attached(text, a) {
            break;
        }
        a -= text[..a].chars().next_back().unwrap().len_utf8();
    }
    for _ in 0..32 {
        if b == text.len() || !attached(text, b) {
            break;
        }
        b += text[b..].chars().next().unwrap().len_utf8();
    }
    // Composed characters can widen the matched span itself, so the copy is cut at a fixed size.
    if b - a > SNIPPET_BYTES {
        b = a + SNIPPET_BYTES;
        while !text.is_char_boundary(b) {
            b -= 1;
        }
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
    let mut from = 0;
    while let Some(i) = norm[from..].find(phrase) {
        let at = from + i;
        if !joins(norm[..at].chars().next_back(), first)
            && !joins(norm[at + phrase.len()..].chars().next(), last)
        {
            return Some(at);
        }
        // Occurrences may overlap, so resume one character on; a phrase starting with a word
        // character can only start a word, so the rest of this word is skipped.
        from = at + first.len_utf8();
        if class(first) == Class::Word {
            from += norm[from..]
                .find(|c| class(c) != Class::Word)
                .unwrap_or(norm.len() - from);
        }
    }
    None
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

/// Bytes `result` copies for `hit`: its name, every path entry and its page, a capped snippet,
/// the matched terms of both fields and a fixed share for refs and spans. Counted without
/// allocating; serialization adds escaping, which MCP checks on the complete reply.
fn result_bytes(index: &Index, hit: &Hit, q: &Prepared) -> usize {
    SNIPPET_BYTES + 2 * q.phrase.len() + location_bytes(index, hit.record)
}

/// Bytes `location` copies for record `rid`: its name, page and path, plus a fixed share.
fn location_bytes(index: &Index, rid: u32) -> usize {
    let entry = |id: &str, name: &str| 64 + id.len() + name.len();
    let r = index.record(rid);
    let mut bytes = 1024 + r.name.len();
    bytes += r.page.map_or(0, |p| index.pages[p as usize].name.len());
    let mut cur = Some(rid);
    while let Some(i) = cur {
        let node = index.record(i);
        bytes += entry(&node.id, &node.name);
        cur = node.parent;
    }
    bytes
        + index
            .above
            .iter()
            .map(|(id, name)| entry(id, name))
            .sum::<usize>()
}

fn result(index: &Index, hit: &Hit, rank: usize, q: &Prepared, source: &Json) -> Json {
    let r = index.record(hit.record);
    let matches: Vec<Json> = [explain(NAME, &r.name, q)]
        .into_iter()
        .chain([r.text.as_deref().and_then(|t| explain(TEXT, t, q))])
        .flatten()
        .collect();
    let mut out = location(index, hit.record, source);
    out["rank"] = json!(rank);
    out["matches"] = json!(matches);
    if let (Some(text), Some(from)) = (&r.text, r.text_source) {
        out["textSource"] = json!(from);
        out["textLength"] = json!(utf16(text));
    }
    out
}

/// The exact ref of record `rid` with what navigates to it: type, origin, page, scope root and
/// path.
fn location(index: &Index, rid: u32, source: &Json) -> Json {
    let r = index.record(rid);
    let mut path: Vec<Json> = Vec::new();
    let mut cur = Some(rid);
    while let Some(i) = cur {
        let node = index.record(i);
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
    let mut out = json!({
        "ref": {"sourceId": source["sourceId"], "snapshotId": source["snapshotId"], "nodeId": r.id},
        "name": r.name,
        "type": r.kind,
        "origin": if r.source.is_some() { "instance-effective" } else { "stored" },
        "hidden": r.hidden,
        "internal": page.is_some_and(|p| p.internal),
        "page": page.map(|p| json!({"id": p.id, "name": p.name})),
        "scopeRoot": r.root.map(|i| &index.roots[i as usize]),
        "path": path,
    });
    if let Some(source) = &r.source {
        out["sourceNodeId"] = json!(source);
    }
    out
}

/// The relations layer of a full build: every kind in `usages::KINDS` is extracted for each
/// indexed record (`built` marks what a text-only one skipped); `library_identity`
/// (cross-file identity) is not, so the layer is never `ready`.
pub(crate) fn relations(state: &str) -> Json {
    let kinds: serde_json::Map<String, Json> = usages::KINDS
        .iter()
        .map(|k| ((*k).to_owned(), json!(state)))
        .collect();
    json!({"state": if state == "ready" { "partial" } else { state },
        "kinds": kinds, "version": usages::RELATIONS,
        "reasons": [{"code": "relation_unavailable", "relations": ["library_identity"]}]})
}

/// `layer` as a text-only index has it: the kinds it never extracted are `not_built`, which says
/// nothing about their absence and leaves text coverage alone.
fn built(index: &Index, mut layer: Json) -> Json {
    if !index.relations {
        for k in usages::KINDS
            .iter()
            .filter(|k| !usages::TEXT_ONLY.contains(k))
        {
            layer["kinds"][*k] = json!("not_built");
        }
    }
    gapped(index.unverified_links, index.unread_fields, layer)
}

/// `layer` with the kinds extraction left unverified marked partial and a reason for each:
/// prototype links it could not attribute, and binding fields too large to read. Text coverage
/// is not affected.
pub(crate) fn gapped(links: usize, fields: usize, mut layer: Json) -> Json {
    let mut add = |kinds: &[&str], counts: Json| {
        for k in kinds {
            layer["kinds"][*k] = json!("partial");
        }
        layer["state"] = json!("partial");
        layer["reasons"]
            .as_array_mut()
            .unwrap()
            .push(json!({"code": "relation_unverified", "relations": kinds, "counts": counts}));
    };
    if fields > 0 {
        add(&usages::FIELDED, json!({"fields": fields}));
    }
    if links > 0 {
        add(&["prototype_target"], json!({"links": links}));
    }
    layer
}

fn coverage(index: &Index, req: &Request, excluded: [usize; 2]) -> Json {
    let scanned = index.total;
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
        "layers": {"text": {"state": if complete { "ready" } else { "partial" }},
            "relations": built(index, relations(if complete { "ready" } else { "partial" }))},
        "reasons": reasons,
        "excluded": {"hidden": excluded[0], "internal": excluded[1]},
        "knownGaps": [{"code": "component_property_values_not_indexed"},
            {"code": "non_text_layer_content_not_indexed"}],
        "limits": {"work": WORK, "bytes": BYTES, "textBytes": TEXT_BYTES,
            "depth": DEPTH, "deadlineMs": DEADLINE.as_millis() as u64, "sourceBytes": SOURCE_BYTES,
            "combiningRun": MAX_MARKS, "replyBytes": REPLY_BYTES},
    })
}

/// The reply for `req` over an index whose source and snapshot `source` names.
fn reply(index: &Index, q: &Prepared, req: &Request, source: Json) -> Result<Json, Box<dyn Error>> {
    let fingerprint = req.fingerprint(q, &source, index);
    if q.gens.is_some() || q.fingerprint.as_ref().is_some_and(|fp| *fp != fingerprint) {
        return Err(invalid(
            "search cursor belongs to another query, filter set, source, snapshot or index generation; search again without cursor",
        ));
    }
    let (hits, more, excluded) = retrieve(&[index], q, req);
    let over = || -> Box<dyn Error> {
        FigError::new(
            ErrorKind::ResponseBudget,
            format!("search results would copy more than {REPLY_BYTES} bytes of names, paths and snippets; lower limit or search a narrower node"),
        )
        .into()
    };
    if hits
        .iter()
        .map(|hit| result_bytes(index, hit, q))
        .sum::<usize>()
        > REPLY_BYTES
    {
        return Err(over());
    }
    let results: Vec<Json> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| result(index, hit, q.offset + i + 1, q, &source))
        .collect();
    let scope = index.scope();
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

/// One source of a workspace search: its loaded generation and its result identity
/// (`sourceId`, `snapshotId`), plus the generation and cursor key the cursor binds.
pub(crate) struct Part<'a> {
    pub index: &'a Index,
    pub source: Json,
    pub gen_id: i64,
}

/// Coverage of several sources: summed counts, and each stop as its source's reason.
fn corpus_coverage(parts: &[Part], req: &Request, excluded: [usize; 2]) -> Json {
    let mut reasons = Vec::new();
    let (mut scanned, mut text_bytes, mut hidden) = (0, 0, 0);
    let (mut links, mut fields) = (0, 0);
    for part in parts {
        let index = part.index;
        scanned += index.total;
        text_bytes += index.text_bytes;
        hidden += index.hidden_content;
        links += index.unverified_links;
        fields += index.unread_fields;
        if let Some(code) = index.stop {
            reasons.push(json!({"code": code, "sourceId": part.source["sourceId"],
                "counts": {"scannedRecords": index.total}}));
        }
    }
    if req.include_hidden && hidden > 0 {
        reasons.push(json!({"code": "hidden_content_not_expanded", "counts": {"layers": hidden}}));
    }
    let complete = reasons.is_empty();
    let state = if complete { "ready" } else { "partial" };
    json!({
        "complete": complete,
        "scannedRecords": scanned,
        "totalRecords": if complete { json!(scanned) } else { Json::Null },
        "indexedTextBytes": text_bytes,
        "layers": {"text": {"state": state}, "relations": gapped(links, fields, relations(state))},
        "reasons": reasons,
        "excluded": {"hidden": excluded[0], "internal": excluded[1]},
        "knownGaps": [{"code": "component_property_values_not_indexed"},
            {"code": "non_text_layer_content_not_indexed"}],
        "limits": {"work": WORK, "bytes": BYTES, "textBytes": TEXT_BYTES,
            "depth": DEPTH, "deadlineMs": DEADLINE.as_millis() as u64, "sourceBytes": SOURCE_BYTES,
            "combiningRun": MAX_MARKS, "replyBytes": REPLY_BYTES},
    })
}

/// One page of a corpus-wide ranking, before its records' paths are known.
pub(crate) struct Ranked {
    hits: Vec<Hit>,
    more: bool,
    excluded: [usize; 2],
    fingerprint: String,
}

impl Ranked {
    /// (part, record) of every hit on the page, which display reads with its ancestors.
    pub(crate) fn shown(&self) -> impl Iterator<Item = (usize, u32)> + '_ {
        self.hits.iter().map(|h| (h.source, h.record))
    }
}

/// The parent of a record, for a caller that loads a partial index's display ancestors.
pub(crate) fn parent(index: &Index, rid: u32) -> Option<u32> {
    index.record(rid).parent
}

/// Ranks `req` over every part together: corpus-wide statistics and phrase tiers for every
/// survivor first, then the global page. `key` identifies the sources used (ids, memberships,
/// snapshots, generations and versions); the cursor binds it with the corpus statistics the
/// scores came from, and lists the generations it paged.
pub(crate) fn corpus_rank(
    parts: &[Part],
    q: &Prepared,
    req: &Request,
    key: &Json,
) -> Result<Ranked, Box<dyn Error>> {
    let corpus: Vec<&Index> = parts.iter().map(|p| p.index).collect();
    let n: usize = corpus.iter().map(|i| i.total).sum();
    let lens = corpus
        .iter()
        .fold([0u64; 2], |a, i| [a[0] + i.lens[0], a[1] + i.lens[1]]);
    let df: Vec<u32> = q
        .terms
        .iter()
        .map(|t| {
            corpus
                .iter()
                .filter_map(|i| i.terms.get(t))
                .map(|t| t.df)
                .sum()
        })
        .collect();
    let fingerprint = short_hash(
        json!([
            key,
            n,
            lens,
            df,
            req.scope,
            q.phrase,
            q.terms,
            req.types,
            req.include_hidden,
            req.include_internal,
            req.limit,
            RANKING,
            tokenizer()
        ])
        .to_string(),
    );
    if q.fingerprint.as_ref().is_some_and(|fp| *fp != fingerprint)
        || (q.fingerprint.is_some() && q.gens.is_none())
    {
        return Err(invalid(
            "search cursor belongs to another query, filter set or workspace search; search again without cursor",
        ));
    }
    let (hits, more, excluded) = retrieve(&corpus, q, req);
    Ok(Ranked {
        hits,
        more,
        excluded,
        fingerprint,
    })
}

/// The reply for a ranked page; every part has the shown records and their ancestors.
pub(crate) fn corpus_page(
    parts: &[Part],
    q: &Prepared,
    req: &Request,
    ranked: Ranked,
) -> Result<Json, Box<dyn Error>> {
    let corpus: Vec<&Index> = parts.iter().map(|p| p.index).collect();
    let Ranked {
        hits,
        more,
        excluded,
        fingerprint,
    } = ranked;
    let bytes: usize = hits
        .iter()
        .map(|hit| result_bytes(corpus[hit.source], hit, q))
        .sum();
    if bytes > REPLY_BYTES {
        return Err(FigError::new(
            ErrorKind::ResponseBudget,
            format!("search results would copy more than {REPLY_BYTES} bytes of names, paths and snippets; lower limit or search a narrower node"),
        )
        .into());
    }
    let results: Vec<Json> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| {
            let part = &parts[hit.source];
            result(part.index, hit, q.offset + i + 1, q, &part.source)
        })
        .collect();
    let gens: Vec<String> = parts.iter().map(|p| p.gen_id.to_string()).collect();
    let scope = corpus.iter().find_map(|i| i.scope());
    Ok(json!({
        "query": {"text": req.query, "normalized": q.phrase, "mode": "text", "terms": q.terms},
        "scope": {"nodeId": req.scope, "name": scope.map(|r| r.name.as_str())},
        "ranking": {"version": RANKING, "tokenizer": tokenizer(), "weights": {"name": WEIGHTS[0], "text": WEIGHTS[1]},
            "k1": K1, "b": B, "order": ["exact-name", "name-phrase", "text-phrase", "score", "source-order", "document-order"]},
        "results": results,
        "coverage": corpus_coverage(parts, req, excluded),
        "hasMore": more,
        "nextCursor": (more && q.offset + req.limit <= MAX_OFFSET)
            .then(|| format!("w1.{fingerprint}.{}.{}", q.offset + req.limit, gens.join("-"))),
    }))
}

// ---- Source snapshot and cache ----

pub(crate) type Stamp = (SystemTime, u64);

pub(crate) fn stamp(path: &Path) -> std::io::Result<Stamp> {
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
pub(crate) fn stage(path: &Path) -> Result<(Vec<u8>, Stamp), Box<dyn Error>> {
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
    /// What text search reads: a text-only build, or a full one that indexed the same corpus.
    text: Option<Arc<Index>>,
    /// What usages reads: a full build. It can stop earlier than the text one under the same
    /// bounds, so it replaces the text one only when it indexed the same text corpus.
    full: Option<Arc<Index>>,
}

impl Cached {
    /// The index that answers `relations` (usages) or text search, if this entry holds one.
    fn pick(&self, relations: bool) -> Option<Arc<Index>> {
        if relations {
            self.full.clone()
        } else {
            self.text.clone()
        }
    }

    /// Keeps `index` in its slot; a full build whose text corpus matches the text one serves
    /// both, so the entry never holds two copies of one corpus.
    fn keep(&mut self, index: Arc<Index>) {
        let shares = |a: &Index, b: &Index| a.generation == b.generation && a.stop == b.stop;
        if index.relations {
            // A stopped full build may hold less text than a text-only one would.
            let serves = match &self.text {
                Some(t) => shares(t, &index),
                None => index.stop.is_none(),
            };
            if serves {
                self.text = Some(index.clone());
            }
            self.full = Some(index);
        } else {
            let full = self.full.as_ref().filter(|f| shares(f, &index));
            self.text = Some(full.cloned().unwrap_or(index));
        }
    }
}

// shortcut: one source and scope per process, at most a text and a full index of it, reused
// while path, scope and file stamp match; persistent multi-scope indexes are #90's workspace.
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

/// The content identity of staged source bytes.
pub(crate) fn snapshot_id(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

/// The index of `scope` in staged source bytes, within the bounds every search build uses.
pub(crate) fn index_bytes(
    bytes: Vec<u8>,
    label: &str,
    scope: &str,
    deadline: Instant,
) -> Result<Index, Box<dyn Error>> {
    decode_index(bytes, label, scope, deadline, true)
}

fn decode_index(
    bytes: Vec<u8>,
    label: &str,
    scope: &str,
    deadline: Instant,
    relations: bool,
) -> Result<Index, Box<dyn Error>> {
    let mut doc = Document::from_bytes(&bytes, label)?;
    drop(bytes);
    let message = doc.take_message()?;
    let tree = Tree::new(&message);
    if relations {
        index_tree(&tree, scope, deadline)
    } else {
        profiled(&tree, scope, deadline, false)
    }
}

/// The index of `scope` in an already decoded tree, within the bounds every search build uses,
/// so a caller that holds one revision's tree (the viewer) never decodes the file again.
pub(crate) fn index_tree(
    tree: &Tree,
    scope: &str,
    deadline: Instant,
) -> Result<Index, Box<dyn Error>> {
    profiled(tree, scope, deadline, true)
}

fn profiled(
    tree: &Tree,
    scope: &str,
    deadline: Instant,
    relations: bool,
) -> Result<Index, Box<dyn Error>> {
    require_scope(tree, scope)?;
    let bounds = Bounds {
        work: WORK,
        bytes: BYTES,
        depth: DEPTH,
        deadline,
        stop: None,
    };
    Ok(build_as(tree, scope, bounds, relations))
}

/// Searches a built index. `source` is the caller's exact identity of the revision the index
/// was built from (`sourceId`, `snapshotId`); it is copied into refs and cursors, never guessed.
pub(crate) fn search_index(
    index: &Index,
    req: &Request,
    source: Json,
) -> Result<Json, Box<dyn Error>> {
    reply(index, &req.validate()?, req, source)
}

/// Usages over a built index, with the caller's exact source identity as in `search_index`.
pub(crate) fn usages_index(
    index: &Index,
    req: &usages::Request,
    source: Json,
) -> Result<Json, Box<dyn Error>> {
    let q = req.validate()?;
    // A text-only index never stands in for missing relations.
    assert!(index.relations, "usages over a text-only index");
    let served = usages::Served {
        source: &source,
        gen_id: 0,
    };
    let mut out = usages::query(&mut usages::Whole(index), &q, req, served)?;
    out["source"] = source;
    Ok(out)
}

/// The build deadline of one index.
pub(crate) fn deadline() -> Instant {
    Instant::now() + DEADLINE
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Searches `scope` of the `.fig` at `path`. The snapshot is the SHA-256 of the exact bytes
/// decoded. A later call reuses the index while path, scope and file stamp are unchanged, and
/// hashes the bytes again once `REHASH` has passed, since a save can keep the stamp.
pub fn search(path: &Path, req: &Request) -> Result<Json, Box<dyn Error>> {
    req.validate()?;
    let (index, source) = loaded(path, &req.scope, false)?;
    search_index(&index, req, source)
}

/// Exact usages of a node or resource in `scope` of the `.fig` at `path`, from the same cached
/// index search uses; nothing is stored.
pub fn usages(path: &Path, req: &usages::Request) -> Result<Json, Box<dyn Error>> {
    req.validate()?;
    let (index, source) = loaded(path, &req.scope, true)?;
    usages_index(&index, req, source)
}

/// The index of `scope` and its source identity, from the process cache while it is fresh.
/// `relations` asks for a full build: a cached text-only one is replaced by one, once, and a
/// cached full one also serves text search.
fn loaded(path: &Path, scope: &str, relations: bool) -> Result<(Arc<Index>, Json), Box<dyn Error>> {
    let deadline = Instant::now() + DEADLINE;
    let canonical = std::fs::canonicalize(path)?;
    let source_id = source_id(&canonical);
    let now = stamp(&canonical)?;
    let cached = CACHE.lock().ok().and_then(|cache| {
        cache
            .as_ref()
            .filter(|c| c.path == canonical && c.stamp == now && c.scope == scope)
            .and_then(|c| Some((c.snapshot.clone(), c.pick(relations)?, c.hashed)))
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
            let snapshot = snapshot_id(&bytes);
            match cached.filter(|(s, ..)| *s == snapshot) {
                Some((snapshot, index, _)) => {
                    if let Ok(mut cache) = CACHE.lock()
                        && let Some(c) = cache.as_mut()
                        && (c.path == canonical && c.scope == scope && c.snapshot == snapshot)
                    {
                        c.hashed = hashed;
                    }
                    (snapshot, index, hashed, true)
                }
                None => {
                    let label = path.display().to_string();
                    let index = Arc::new(decode_index(bytes, &label, scope, deadline, relations)?);
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
                        // The other profile of this same revision stays beside it.
                        let same = cache.as_ref().is_some_and(|c| {
                            c.path == canonical
                                && c.stamp == staged
                                && c.scope == scope
                                && c.snapshot == snapshot
                        });
                        if !same {
                            *cache = Some(Cached {
                                path: canonical.clone(),
                                stamp: staged,
                                scope: scope.to_owned(),
                                snapshot: snapshot.clone(),
                                hashed,
                                text: None,
                                full: None,
                            });
                        }
                        if let Some(c) = cache.as_mut() {
                            c.keep(index.clone());
                        }
                    }
                    (snapshot, index, hashed, false)
                }
            }
        }
    };
    let source = json!({"sourceId": source_id, "snapshotId": snapshot, "freshness": {
        "reusedIndex": reused, "hashedAtMs": ms(hashed), "stampCheckedAtMs": ms(SystemTime::now()),
        "rehashAfterMs": REHASH.as_millis() as u64}});
    Ok((index, source))
}

pub(crate) mod store;
pub mod usages;

#[cfg(test)]
mod tests;
