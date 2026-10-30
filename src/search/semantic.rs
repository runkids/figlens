//! Semantic and hybrid search over one stored scope (#94). The model layer embeds each
//! record's name and effective text, chunked without truncation, once per source snapshot,
//! scope and profile; identical chunk texts share one vector. Retrieval is an exact scan, and
//! hybrid fuses the lexical and semantic rankings with `semantic::FUSION`. Semantic matches
//! name their field and chunk, never a text span.

use super::*;
use crate::semantic::{self as model, Kind, Mode, Profile, Stop, profile};

/// Records each channel ranks before fusion; pages past it are not offered.
pub const CANDIDATES: usize = 1000;
/// Unique chunk texts one layer keeps (4 KiB each at 1024 dimensions).
pub const MAX_TEXTS: usize = 50_000;
/// Embedding time one request may spend; an unfinished layer resumes on the next request.
const BUILD: Duration = Duration::from_secs(30);
const SEMANTIC_RANKING: &str = "semantic-exact-1";
/// What the layer embeds; property values and scope summaries are not model inputs yet.
const INPUTS: [&str; 2] = ["name", "text"];
/// Bytes one chunk mapping keeps beside its text.
const UNIT: usize = 24;
/// Extraction budget of one layer: work (one per chunk), bytes and time.
pub(crate) const LAYER_WORK: usize = 1_000_000;
pub(crate) const LAYER_BYTES: usize = 512 * 1024 * 1024;

fn field_name(field: u8) -> &'static str {
    if field == NAME { "name" } else { "text" }
}

/// One index's model layer under one profile: its chunks and unique texts, then their vectors.
pub(crate) struct Layer {
    /// The source snapshot, scope and profile it embeds.
    pub(crate) key: Json,
    /// `(record, field, chunk, text)` for every chunk, in record order.
    pub(crate) units: Vec<(u32, u8, u32, usize)>,
    pub(crate) texts: Vec<String>,
    /// One per text: `None` until embedded, then the vector (empty when the child refused it).
    pub(crate) vectors: Vec<Option<Vec<f32>>>,
    /// Chunks left out because the layer reached `MAX_TEXTS` texts or `model::MAX_SCAN` chunks.
    pub(crate) over: usize,
    /// The bound extraction stopped at, leaving later records out.
    pub(crate) stop: Option<&'static str>,
}

impl Layer {
    /// Extracts the chunks of every record, charging each copy and mapping to `bounds` before
    /// it is made and checking `cancel` per chunk; a spent bound leaves the layer partial.
    pub(crate) fn new(
        index: &Index,
        key: Json,
        dim: usize,
        bounds: &mut Bounds,
        cancel: &AtomicBool,
    ) -> Result<Layer, Stop> {
        let mut slot: HashMap<&str, usize> = HashMap::new();
        let mut layer = Layer {
            key,
            units: Vec::new(),
            texts: Vec::new(),
            vectors: Vec::new(),
            over: 0,
            stop: None,
        };
        let rids: Vec<u32> = if index.rids.is_empty() {
            (0..index.records.len() as u32).collect()
        } else {
            index.rids.clone()
        };
        'records: for rid in rids {
            let r = index.record(rid);
            for (field, text) in [(NAME, Some(r.name.as_str())), (TEXT, r.text.as_deref())] {
                for (i, chunk) in model::chunks(text.unwrap_or_default()).enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Stop::Cancelled);
                    }
                    let known = slot.get(chunk).copied();
                    // A new text costs its copy, its slot and the vector it will hold.
                    let cost = UNIT + known.map_or(2 * chunk.len() + 4 * dim, |_| 0);
                    if !bounds.charge(1, cost) {
                        break 'records;
                    }
                    let next = layer.texts.len();
                    let at = match known {
                        _ if layer.units.len() == model::MAX_SCAN => None,
                        Some(at) => Some(at),
                        None if next == MAX_TEXTS => None,
                        None => {
                            slot.insert(chunk, next);
                            layer.texts.push(chunk.to_owned());
                            Some(next)
                        }
                    };
                    match at {
                        Some(at) => layer.units.push((rid, field, i as u32, at)),
                        None => layer.over += 1,
                    }
                }
            }
        }
        layer.stop = bounds.stop.map(|stop| match stop {
            crate::expand::Stop::Work => "work_limit",
            crate::expand::Stop::Bytes => "byte_limit",
            crate::expand::Stop::Depth => "depth_limit",
            crate::expand::Stop::Deadline => "deadline",
        });
        layer.vectors = vec![None; layer.texts.len()];
        Ok(layer)
    }

    /// Embeds missing texts until done, `BUILD` passes or `cancel`. A refused batch is retried
    /// one text at a time so only the refused text stays uncovered.
    fn fill(&mut self, profile: &Profile, cancel: &AtomicBool) -> Result<(), Stop> {
        let end = Instant::now() + BUILD;
        let todo: Vec<usize> = (0..self.texts.len())
            .filter(|&i| self.vectors[i].is_none())
            .collect();
        for batch in todo.chunks(model::MAX_BATCH) {
            if Instant::now() >= end {
                break;
            }
            let texts: Vec<String> = batch.iter().map(|&i| self.texts[i].clone()).collect();
            match profile::with_runtime(profile, cancel, |rt| {
                rt.embed(Kind::Passage, &texts, cancel)
            }) {
                Ok(vs) => batch
                    .iter()
                    .zip(vs)
                    .for_each(|(&i, v)| self.vectors[i] = Some(v)),
                Err(Stop::Refused(_)) => {
                    for (&i, text) in batch.iter().zip(texts) {
                        let one = profile::with_runtime(profile, cancel, |rt| {
                            rt.embed(Kind::Passage, &[text], cancel)
                        });
                        self.vectors[i] = match one {
                            Ok(mut v) => Some(v.remove(0)),
                            Err(Stop::Refused(_)) => Some(Vec::new()),
                            Err(e) => return Err(e),
                        };
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// A record's best chunk: `(record, field, chunk)`.
type Best = (u32, u8, u32);
/// A record, its best chunk's similarity, field and chunk.
pub(crate) type Scored = (u32, f32, u8, u32);

/// The last layer built in this process; another snapshot, scope or profile replaces it.
static LAYER: Mutex<Option<Layer>> = Mutex::new(None);

fn unavailable(message: impl Into<String>) -> Box<dyn Error> {
    FigError::new(ErrorKind::ModeUnavailable, message)
        .with_recovery(
            json!([{"action": "models_status", "tool": "models", "arguments": {"action": "status"},
            "requires": "none"}]),
        )
        .into()
}

/// A model runtime that cannot run, with the status call that names its recovery.
fn runtime_failed(message: String) -> FigError {
    FigError::new(ErrorKind::OperationFailed, message)
        .with_recovery(json!([{"action": "models_status", "tool": "models",
            "arguments": {"action": "status"}, "requires": "none"}]))
}

/// The semantic layer's coverage when it cannot answer.
fn down(code: &str, message: Option<String>) -> Json {
    json!({"state": "unavailable", "reasons": [{"code": code, "message": message}]})
}

/// Searches `scope` of the `.fig` at `path` in `mode`. Text mode is exactly `search::search`.
/// Semantic mode needs a usable profile; hybrid falls back to text and says why.
pub fn search(
    path: &Path,
    req: &Request,
    mode: Mode,
    storage: &crate::workspace::Storage,
    cancel: &AtomicBool,
) -> Result<Json, Box<dyn Error>> {
    if mode == Mode::Text {
        return super::search(path, req);
    }
    let q = req.validate()?;
    let profile = match (profile::active(storage), mode) {
        (Ok(Some(p)), _) => Ok(p),
        (Ok(None), Mode::Semantic) => {
            return Err(unavailable(
                "semantic search needs a configured local model; run models status for setup",
            ));
        }
        (Err(e), Mode::Semantic) => return Err(e),
        (Ok(None), _) => Err(down("no_profile", None)),
        (Err(e), _) => Err(down("profile_unreadable", Some(e.to_string()))),
    };
    let (index, source) = loaded(path, &req.scope, false)?;
    reply_with(&index, &q, req, mode, profile, source, cancel)
}

fn reply_with(
    index: &Index,
    q: &Prepared,
    req: &Request,
    mode: Mode,
    profile: Result<Profile, Json>,
    source: Json,
    cancel: &AtomicBool,
) -> Result<Json, Box<dyn Error>> {
    let (lexical, excluded) = match mode {
        Mode::Hybrid => {
            // Lexical candidates: the same ranking as text mode, from its first page.
            let wide = Request {
                limit: CANDIDATES,
                cursor: None,
                ..req.clone()
            };
            let first = Prepared {
                offset: 0,
                fingerprint: None,
                gens: None,
                issued: None,
                phrase: q.phrase.clone(),
                terms: q.terms.clone(),
            };
            let (hits, _, excluded) = retrieve(&[index], &first, &wide, cancel);
            // A cancelled ranking stops early; never answer it as a complete page.
            if cancel.load(Ordering::SeqCst) {
                return Err(FigError::new(ErrorKind::Cancelled, "search cancelled").into());
            }
            (hits, excluded)
        }
        _ => (Vec::new(), [0, 0]),
    };
    let mut used = Vec::new();
    if mode == Mode::Hybrid {
        used.push("text");
    }
    let profile_id = profile.as_ref().ok().map(|p| p.id.clone());
    let (semantic, layer_json) = match profile {
        Ok(p) => match ranked(index, req, &p, &source, cancel) {
            Ok(found) => {
                if found.1["embeddedTexts"].as_u64().unwrap_or(0) > 0 {
                    used.push("semantic");
                }
                found
            }
            Err(Stop::Cancelled) => {
                return Err(FigError::new(ErrorKind::Cancelled, "search cancelled").into());
            }
            Err(e) if mode == Mode::Semantic => {
                return Err(runtime_failed(e.to_string()).into());
            }
            Err(e) => (Vec::new(), down("runtime_failed", Some(e.to_string()))),
        },
        Err(layer) => (Vec::new(), layer),
    };
    // A page of an unfinished layer or a downgraded hybrid is not replayable: the next call can
    // rank differently, so it gets no cursor.
    let ready = layer_json["state"] == "ready";
    let fingerprint = short_hash(
        json!([
            req.fingerprint(q, &source, index),
            req.query,
            mode.as_str(),
            profile_id,
            model::FUSION,
            SEMANTIC_RANKING,
            CANDIDATES
        ])
        .to_string(),
    );
    if q.gens.is_some()
        || q.fingerprint
            .as_ref()
            .is_some_and(|fp| *fp != fingerprint || !ready)
    {
        return Err(invalid(
            "search cursor belongs to another query, mode, profile, filter set, source, snapshot, index generation or model layer state; search again without cursor",
        ));
    }
    let lexical_ids: Vec<u32> = lexical.iter().map(|h| h.record).collect();
    let semantic_ids: Vec<u32> = semantic.iter().map(|s| s.0).collect();
    let order: Vec<u32> = match mode {
        Mode::Hybrid => model::fuse(&[&lexical_ids, &semantic_ids])
            .into_iter()
            .map(|f| f.0)
            .collect(),
        _ => semantic_ids,
    };
    let more = order.len() > q.offset + req.limit;
    let page: Vec<u32> = order
        .iter()
        .skip(q.offset)
        .take(req.limit)
        .copied()
        .collect();
    let bytes: usize = page
        .iter()
        .map(|&rid| SNIPPET_BYTES + 2 * q.phrase.len() + location_bytes(index, rid))
        .sum();
    if bytes > REPLY_BYTES {
        return Err(FigError::new(
            ErrorKind::ResponseBudget,
            format!("search results would copy more than {REPLY_BYTES} bytes of names, paths and snippets; lower limit or search a narrower node"),
        )
        .into());
    }
    let lexical_by: HashMap<u32, &Hit> = lexical.iter().map(|h| (h.record, h)).collect();
    let semantic_by: HashMap<u32, Best> = semantic.iter().map(|s| (s.0, *s)).collect();
    let results: Vec<Json> = page
        .iter()
        .enumerate()
        .map(|(i, &rid)| {
            let rank = q.offset + i + 1;
            let mut matches = Vec::new();
            let mut out = match lexical_by.get(&rid) {
                Some(hit) => {
                    let mut out = result(index, hit, rank, q, &source);
                    for m in out["matches"].as_array_mut().into_iter().flatten() {
                        m["basis"] = json!("lexical");
                        matches.push(m.take());
                    }
                    out
                }
                None => {
                    let mut out = location(index, rid, &source);
                    out["rank"] = json!(rank);
                    out
                }
            };
            if let Some(&(_, field, chunk)) = semantic_by.get(&rid) {
                matches
                    .push(json!({"field": field_name(field), "chunk": chunk, "basis": "semantic"}));
            }
            out["matches"] = json!(matches);
            out
        })
        .collect();
    let mut coverage = coverage(index, req, excluded);
    coverage["layers"]["semantic"] = layer_json;
    if !ready {
        coverage["complete"] = json!(false);
    }
    let scope = index.scope();
    Ok(json!({
        "query": {"text": req.query, "normalized": q.phrase, "mode": mode.as_str(), "requestedMode": mode.as_str(),
            "usedModes": used, "terms": q.terms},
        "source": source,
        "scope": {"nodeId": req.scope, "name": scope.map(|r| r.name.as_str())},
        "profileId": profile_id,
        "ranking": {"version": if mode == Mode::Hybrid { model::FUSION } else { SEMANTIC_RANKING },
            "channels": {"text": if mode == Mode::Hybrid { json!(RANKING) } else { Json::Null }, "semantic": SEMANTIC_RANKING},
            "candidatesPerChannel": CANDIDATES},
        "results": results,
        "coverage": coverage,
        "hasMore": more,
        "nextCursor": (ready && more && q.offset + req.limit <= MAX_OFFSET)
            .then(|| format!("s1.{fingerprint}.{}", q.offset + req.limit)),
    }))
}

/// Records ranked by their best chunk's similarity to the query (filters applied first), each
/// with that chunk, and the layer's coverage.
fn ranked(
    index: &Index,
    req: &Request,
    profile: &Profile,
    source: &Json,
    cancel: &AtomicBool,
) -> Result<(Vec<Best>, Json), Stop> {
    let key = json!([
        source["sourceId"],
        source["snapshotId"],
        req.scope,
        profile.id
    ]);
    let mut guard = model::lock(&LAYER, cancel)?;
    if !guard.as_ref().is_some_and(|l| l.key == key) {
        *guard = None;
        let bounds = Bounds {
            work: LAYER_WORK,
            bytes: LAYER_BYTES,
            depth: 0,
            deadline: Instant::now() + DEADLINE,
            stop: None,
        };
        *guard = Some(Layer::new(
            index,
            key,
            profile.dim,
            &mut { bounds },
            cancel,
        )?);
    }
    let layer = guard.as_mut().expect("set above");
    layer.fill(profile, cancel)?;
    let query = profile::with_runtime(profile, cancel, |rt| {
        rt.embed(Kind::Query, std::slice::from_ref(&req.query), cancel)
    })?
    .remove(0);
    let (order, excluded) = score(index, layer, &query, req, cancel)?;
    let order: Vec<Best> = order
        .into_iter()
        .take(CANDIDATES)
        .map(|(rid, _, field, chunk)| (rid, field, chunk))
        .collect();
    Ok((order, layer_coverage(layer, &profile.id, excluded)))
}

/// Every record of `index` that passes the filters, by its best chunk's similarity to `query`
/// (ties in record order), with that chunk, and the records the filters left out.
pub(crate) fn score(
    index: &Index,
    layer: &Layer,
    query: &[f32],
    req: &Request,
    cancel: &AtomicBool,
) -> Result<(Vec<Scored>, [usize; 2]), Stop> {
    // One dot product per unique text; a refused or missing text scores nothing.
    let scores: Vec<Option<f32>> = layer
        .vectors
        .iter()
        .map(|v| {
            v.as_ref()
                .filter(|v| !v.is_empty())
                .map(|v| query.iter().zip(v).map(|(a, b)| a * b).sum())
        })
        .collect();
    let mut best: BTreeMap<u32, (f32, u8, u32)> = BTreeMap::new();
    let mut excluded = [0usize; 2];
    for (n, &(rid, field, chunk, text)) in layer.units.iter().enumerate() {
        if n % 4096 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        let Some(score) = scores[text] else { continue };
        let r = index.record(rid);
        let internal = r.page.is_some_and(|p| index.pages[p as usize].internal);
        if !req.types.is_empty() && !req.types.contains(&r.kind) {
            continue;
        }
        if r.hidden && !req.include_hidden {
            excluded[0] += 1;
            continue;
        }
        if internal && !req.include_internal {
            excluded[1] += 1;
            continue;
        }
        let e = best.entry(rid).or_insert((f32::NEG_INFINITY, field, chunk));
        if score > e.0 {
            *e = (score, field, chunk);
        }
    }
    let mut order: Vec<Scored> = best
        .into_iter()
        .map(|(rid, (score, field, chunk))| (rid, score, field, chunk))
        .collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    if cancel.load(Ordering::Relaxed) {
        return Err(Stop::Cancelled);
    }
    Ok((order, excluded))
}

/// Estimated bytes a cached full generation and its layer keep: every record's strings,
/// texts, chunk mappings and the vectors read so far.
pub(crate) fn cached_bytes(index: &Index, layer: &Layer) -> usize {
    let vectors: usize = layer
        .vectors
        .iter()
        .map(|v| v.as_ref().map_or(0, |v| 24 + 4 * v.len()))
        .sum();
    let texts: usize = layer.texts.iter().map(|t| 24 + t.len()).sum();
    // Every string a loaded record keeps, its normalized copies included, plus a fixed share.
    let records: usize = index
        .records
        .iter()
        .map(|r| {
            256 + r.id.len()
                + r.name.len()
                + r.kind.len()
                + r.text.as_ref().map_or(0, String::len)
                + r.norm[0].len()
                + r.norm[1].len()
        })
        .sum();
    records + texts + UNIT * layer.units.len() + vectors
}

/// `coverage.layers.semantic` of one layer.
pub(crate) fn layer_coverage(layer: &Layer, profile_id: &str, excluded: [usize; 2]) -> Json {
    let embedded = layer.vectors.iter().filter(|v| v.is_some()).count();
    let refused = layer
        .vectors
        .iter()
        .filter(|v| v.as_ref().is_some_and(Vec::is_empty))
        .count();
    let complete =
        embedded == layer.texts.len() && refused == 0 && layer.over == 0 && layer.stop.is_none();
    let mut reasons = Vec::new();
    if embedded < layer.texts.len() {
        reasons.push(json!({"code": "semantic_layer_building", "counts": {"embeddedTexts": embedded, "texts": layer.texts.len()}}));
    }
    if refused > 0 {
        reasons.push(json!({"code": "chunk_refused", "counts": {"texts": refused}}));
    }
    if layer.over > 0 {
        reasons.push(
            json!({"code": "semantic_capacity", "counts": {"chunks": layer.over,
            "maxTexts": MAX_TEXTS, "maxChunks": model::MAX_SCAN}}),
        );
    }
    if let Some(code) = layer.stop {
        reasons.push(json!({"code": code, "counts": {"chunks": layer.units.len()},
            "limits": {"work": LAYER_WORK, "bytes": LAYER_BYTES, "deadlineMs": DEADLINE.as_millis() as u64}}));
    }
    json!({"state": if complete { "ready" } else { "partial" }, "profileId": profile_id,
        "inputs": INPUTS, "chunks": layer.units.len(), "texts": layer.texts.len(),
        "embeddedTexts": embedded, "excluded": {"hidden": excluded[0], "internal": excluded[1]},
        "reasons": reasons})
}

/// What a workspace semantic page adds to the text reply `corpus_page` renders.
pub(crate) struct Decor {
    /// The semantic match of each page row, in page order.
    rows: Vec<Option<(u8, u32)>>,
    lexical: Vec<bool>,
    used: Vec<&'static str>,
    profile_id: Option<String>,
    layer: Json,
    ready: bool,
}

/// One workspace source's semantic side: its fully loaded generation and model layer.
pub(crate) struct Side<'a> {
    pub index: &'a Index,
    pub layer: &'a Layer,
}

/// Ranks a workspace page in `mode`: the semantic ranking over every source's layer (one vector
/// space, filters applied first, ties by source then record) and, in hybrid, the lexical
/// candidates of `parts` fused by RRF. The returned `Ranked` pages `parts` like a text search,
/// so the caller loads and renders it as one; `decorate` then adds the semantic fields.
/// `sides[i]` belongs to `parts[i]`; `layer` is the sources' combined coverage.
#[allow(clippy::too_many_arguments)]
pub(crate) fn workspace_rank(
    parts: &[Part],
    sides: &[Side],
    mode: Mode,
    profile: &Profile,
    layer: Json,
    q: &Prepared,
    req: &Request,
    key: &Json,
    cancel: &AtomicBool,
) -> Result<(Ranked, Decor), Box<dyn Error>> {
    let ready = layer["state"] == "ready";
    let mut used = Vec::new();
    let lexical: Vec<Hit> = if mode == Mode::Hybrid {
        used.push("text");
        let wide = Request {
            limit: CANDIDATES,
            cursor: None,
            ..req.clone()
        };
        let first = Prepared {
            offset: 0,
            fingerprint: None,
            gens: None,
            issued: None,
            phrase: q.phrase.clone(),
            terms: q.terms.clone(),
        };
        corpus_rank(parts, &first, &wide, key, cancel)?.hits
    } else {
        Vec::new()
    };
    // No stored vector anywhere: nothing to compare, so the model is not started for the query.
    let any = sides.iter().any(|s| {
        s.layer
            .vectors
            .iter()
            .any(|v| v.as_ref().is_some_and(|v| !v.is_empty()))
    });
    let query = match any {
        false => Vec::new(),
        true => profile::with_runtime(profile, cancel, |rt| {
            rt.embed(Kind::Query, std::slice::from_ref(&req.query), cancel)
        })
        .map_err(|e| match e {
            Stop::Cancelled => FigError::new(ErrorKind::Cancelled, "search cancelled"),
            e => runtime_failed(e.to_string()),
        })?
        .remove(0),
    };
    // Semantic was used only if some vector could be compared.
    if any {
        used.push("semantic");
    }
    let mut semantic: Vec<((usize, u32), f32, u8, u32)> = Vec::new();
    let mut excluded = [0usize; 2];
    for (part, side) in sides.iter().enumerate() {
        let (order, out) = score(side.index, side.layer, &query, req, cancel)
            .map_err(|_| FigError::new(ErrorKind::Cancelled, "search cancelled"))?;
        excluded = [excluded[0] + out[0], excluded[1] + out[1]];
        semantic.extend(
            order
                .into_iter()
                .map(|(rid, s, f, c)| ((part, rid), s, f, c)),
        );
    }
    semantic.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    semantic.truncate(CANDIDATES);
    let fingerprint = short_hash(
        json!([
            key,
            req.query,
            q.phrase,
            q.terms,
            req.scope,
            req.types,
            req.include_hidden,
            req.include_internal,
            req.limit,
            mode.as_str(),
            profile.id,
            model::FUSION,
            SEMANTIC_RANKING,
            CANDIDATES,
            crate::workspace::model::POLICY
        ])
        .to_string(),
    );
    if q.fingerprint
        .as_ref()
        .is_some_and(|fp| *fp != fingerprint || !ready)
    {
        return Err(invalid(
            "search cursor belongs to another query, mode, profile, filter set, workspace search or model layer state; search again without cursor",
        ));
    }
    let lexical_keys: Vec<(usize, u32)> = lexical.iter().map(|h| (h.source, h.record)).collect();
    let semantic_keys: Vec<(usize, u32)> = semantic.iter().map(|s| s.0).collect();
    let order: Vec<(usize, u32)> = match mode {
        Mode::Hybrid => model::fuse(&[&lexical_keys, &semantic_keys])
            .into_iter()
            .map(|f| f.0)
            .collect(),
        _ => semantic_keys,
    };
    let more = order.len() > q.offset + req.limit;
    let lexical_by: HashMap<(usize, u32), &Hit> =
        lexical.iter().map(|h| ((h.source, h.record), h)).collect();
    let semantic_by: HashMap<(usize, u32), (u8, u32)> =
        semantic.iter().map(|s| (s.0, (s.2, s.3))).collect();
    let page: Vec<(usize, u32)> = order.into_iter().skip(q.offset).take(req.limit).collect();
    let hits = page
        .iter()
        .map(|k| match lexical_by.get(k) {
            Some(h) => Hit {
                source: h.source,
                record: h.record,
                tier: h.tier,
                score: h.score,
            },
            None => Hit {
                source: k.0,
                record: k.1,
                tier: 0,
                score: 0.0,
            },
        })
        .collect();
    let decor = Decor {
        rows: page.iter().map(|k| semantic_by.get(k).copied()).collect(),
        lexical: page.iter().map(|k| lexical_by.contains_key(k)).collect(),
        used,
        profile_id: Some(profile.id.clone()),
        layer,
        ready,
    };
    Ok((
        Ranked {
            hits,
            more,
            excluded,
            fingerprint,
        },
        decor,
    ))
}

impl Decor {
    /// Each ranked row's semantic evidence, as `decorate` writes it, for another fusion.
    pub(crate) fn evidence(&self) -> Vec<Option<Json>> {
        self.rows
            .iter()
            .map(|r| r.map(|(field, chunk)| json!({"field": field_name(field), "chunk": chunk, "basis": "semantic"})))
            .collect()
    }

    /// Whether every source's model layer was ready for this ranking.
    pub(crate) fn ready(&self) -> bool {
        self.ready
    }
}

/// Adds the semantic fields to a workspace page rendered from `workspace_rank`'s `Ranked`.
pub(crate) fn decorate(out: &mut Json, mode: Mode, decor: Decor) {
    for (i, row) in out["results"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let mut matches: Vec<Json> = Vec::new();
        if decor.lexical.get(i).copied().unwrap_or(false) {
            for m in row["matches"].as_array_mut().into_iter().flatten() {
                m["basis"] = json!("lexical");
                matches.push(m.take());
            }
        }
        if let Some(Some((field, chunk))) = decor.rows.get(i) {
            matches.push(json!({"field": field_name(*field), "chunk": chunk, "basis": "semantic"}));
        }
        row["matches"] = json!(matches);
    }
    out["query"]["mode"] = json!(mode.as_str());
    out["query"]["requestedMode"] = json!(mode.as_str());
    out["query"]["usedModes"] = json!(decor.used);
    out["profileId"] = json!(decor.profile_id);
    out["ranking"] = json!({"version": if mode == Mode::Hybrid { model::FUSION } else { SEMANTIC_RANKING },
        "channels": {"text": if mode == Mode::Hybrid { corpus_ranking() } else { Json::Null }, "semantic": SEMANTIC_RANKING},
        "candidatesPerChannel": CANDIDATES});
    out["coverage"]["layers"]["semantic"] = decor.layer;
    if !decor.ready {
        out["coverage"]["complete"] = json!(false);
        out["nextCursor"] = Json::Null;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(text: &str) -> Index {
        let bytes = std::fs::read("examples/first-success/design.fig").unwrap();
        let deadline = Instant::now() + DEADLINE;
        let mut index = index_bytes(bytes, "first-success", "0:0", deadline).unwrap();
        for r in &mut index.records {
            r.text = Some(text.to_owned());
        }
        index
    }

    fn bounds(work: usize, bytes: usize) -> Bounds {
        Bounds {
            work,
            bytes,
            depth: 0,
            deadline: Instant::now() + DEADLINE,
            stop: None,
        }
    }

    #[test]
    fn a_long_shared_field_is_chunked_once_and_stops_at_its_bound() {
        let long = "字".repeat(model::CHUNK_CHARS * 3);
        let idx = index(&long);
        let no = AtomicBool::new(false);
        let all = Layer::new(&idx, json!(1), 8, &mut bounds(LAYER_WORK, LAYER_BYTES), &no).unwrap();
        // Every record shares the same three text chunks; only names differ.
        assert!(all.stop.is_none());
        assert!(
            all.texts
                .iter()
                .filter(|t| t.chars().count() == model::CHUNK_CHARS)
                .count()
                == 1
        );
        let cut = Layer::new(&idx, json!(1), 8, &mut bounds(2, LAYER_BYTES), &no).unwrap();
        assert_eq!((cut.stop, cut.units.len()), (Some("work_limit"), 2));
        let tiny = Layer::new(&idx, json!(1), 8, &mut bounds(LAYER_WORK, 16), &no).unwrap();
        assert_eq!((tiny.stop, tiny.texts.len()), (Some("byte_limit"), 0));
    }

    #[test]
    fn cached_bytes_count_long_names_and_their_normalized_copies() {
        let mut idx = index("t");
        let short = Layer::new(
            &idx,
            json!(1),
            8,
            &mut bounds(LAYER_WORK, LAYER_BYTES),
            &AtomicBool::new(false),
        )
        .unwrap();
        let long = "n".repeat(4000);
        for r in &mut idx.records {
            r.name = long.clone();
            r.norm[0] = long.clone();
        }
        assert!(cached_bytes(&idx, &short) >= 2 * 4000 * idx.records.len());
    }

    #[test]
    fn extraction_stops_when_cancelled() {
        let idx = index("text");
        let got = Layer::new(
            &idx,
            json!(1),
            8,
            &mut bounds(LAYER_WORK, LAYER_BYTES),
            &AtomicBool::new(true),
        );
        assert!(matches!(got, Err(Stop::Cancelled)));
    }
}
