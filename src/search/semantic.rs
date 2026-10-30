//! Semantic and hybrid search over one stored scope (#94). The model layer embeds each
//! record's name and effective text, chunked without truncation, once per index generation
//! and profile; identical chunk texts share one vector. Retrieval is an exact scan, and
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

/// One index's model layer under one profile.
struct Layer {
    generation: String,
    profile: String,
    /// `(record, field, chunk, text)` for every chunk, in record order.
    units: Vec<(u32, u8, u32, usize)>,
    texts: Vec<String>,
    /// One per text: `None` until embedded, then the vector (empty when the child refused it).
    vectors: Vec<Option<Vec<f32>>>,
    /// Chunks left out because the layer reached `MAX_TEXTS`.
    over: usize,
}

impl Layer {
    fn new(index: &Index, profile: &str) -> Layer {
        let mut slot: HashMap<String, usize> = HashMap::new();
        let mut layer = Layer {
            generation: index.generation.clone(),
            profile: profile.to_owned(),
            units: Vec::new(),
            texts: Vec::new(),
            vectors: Vec::new(),
            over: 0,
        };
        for (rid, r) in index.records.iter().enumerate() {
            for (field, text) in [(NAME, Some(r.name.as_str())), (TEXT, r.text.as_deref())] {
                let field_name = if field == NAME { "name" } else { "text" };
                for c in model::chunks(field_name, text.unwrap_or_default()) {
                    let next = layer.texts.len();
                    let at = match slot.get(&c.text) {
                        Some(&at) => at,
                        None if next == MAX_TEXTS => {
                            layer.over += 1;
                            continue;
                        }
                        None => {
                            slot.insert(c.text.clone(), next);
                            layer.texts.push(c.text);
                            next
                        }
                    };
                    layer.units.push((rid as u32, field, c.index, at));
                }
            }
        }
        layer.vectors = vec![None; layer.texts.len()];
        layer
    }

    fn embedded(&self) -> usize {
        self.vectors.iter().filter(|v| v.is_some()).count()
    }

    fn refused(&self) -> usize {
        self.vectors
            .iter()
            .filter(|v| v.as_ref().is_some_and(Vec::is_empty))
            .count()
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

/// The last layer built in this process; a new generation or profile replaces it.
static LAYER: Mutex<Option<Layer>> = Mutex::new(None);

fn unavailable(message: impl Into<String>) -> Box<dyn Error> {
    FigError::new(ErrorKind::ModeUnavailable, message)
        .with_recovery(
            json!([{"action": "models_status", "tool": "models", "arguments": {"action": "status"},
            "requires": "none"}]),
        )
        .into()
}

/// Searches `scope` of the `.fig` at `path` in `mode`. Text mode is exactly `search::search`.
/// Semantic mode needs a configured profile; hybrid falls back to text and says why.
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
    let profile = match profile::active(storage)? {
        Some(p) => Some(p),
        None if mode == Mode::Semantic => {
            return Err(unavailable(
                "semantic search needs a configured local model; run models status for setup",
            ));
        }
        None => None,
    };
    let (index, source) = loaded(path, &req.scope, false)?;
    reply_with(&index, &q, req, mode, profile.as_ref(), source, cancel)
}

fn reply_with(
    index: &Index,
    q: &Prepared,
    req: &Request,
    mode: Mode,
    profile: Option<&Profile>,
    source: Json,
    cancel: &AtomicBool,
) -> Result<Json, Box<dyn Error>> {
    let fingerprint = short_hash(
        json!([
            req.fingerprint(q, &source, index),
            mode.as_str(),
            profile.map(|p| &p.id),
            model::FUSION,
            SEMANTIC_RANKING,
            CANDIDATES
        ])
        .to_string(),
    );
    if q.gens.is_some() || q.fingerprint.as_ref().is_some_and(|fp| *fp != fingerprint) {
        return Err(invalid(
            "search cursor belongs to another query, mode, profile, filter set, source, snapshot or index generation; search again without cursor",
        ));
    }
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
    let (lexical, _, excluded) = match mode {
        Mode::Hybrid => retrieve(&[index], &first, &wide, cancel),
        _ => (Vec::new(), false, [0, 0]),
    };
    let mut layer_json = json!({"state": "unavailable", "reason": "no_profile"});
    let mut semantic: Vec<Best> = Vec::new();
    let mut used = Vec::new();
    if mode == Mode::Hybrid {
        used.push("text");
    }
    if let Some(p) = profile {
        match ranked(index, req, p, cancel) {
            Ok((ranked, layer)) => {
                semantic = ranked;
                layer_json = layer;
                used.push("semantic");
            }
            Err(Stop::Cancelled) => {
                return Err(FigError::new(ErrorKind::Cancelled, "search cancelled").into());
            }
            Err(e) if mode == Mode::Semantic => {
                return Err(FigError::new(ErrorKind::OperationFailed, e.to_string()).into());
            }
            Err(e) => {
                layer_json = json!({"state": "unavailable", "reason": "runtime_failed", "message": e.to_string()})
            }
        }
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
    let results: Vec<Json> = page
        .iter()
        .enumerate()
        .map(|(i, &rid)| {
            let rank = q.offset + i + 1;
            let mut matches = Vec::new();
            let mut out = match lexical.iter().find(|h| h.record == rid) {
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
            if let Some(&(_, field, chunk)) = semantic.iter().find(|s| s.0 == rid) {
                matches.push(json!({"field": if field == NAME { "name" } else { "text" }, "chunk": chunk, "basis": "semantic"}));
            }
            out["matches"] = json!(matches);
            out
        })
        .collect();
    let mut coverage = coverage(index, req, excluded);
    coverage["layers"]["semantic"] = layer_json;
    if coverage["layers"]["semantic"]["state"] != "ready" {
        coverage["complete"] = json!(false);
    }
    let scope = index.scope();
    Ok(json!({
        "query": {"text": req.query, "normalized": q.phrase, "mode": mode.as_str(), "requestedMode": mode.as_str(),
            "usedModes": used, "terms": q.terms},
        "source": source,
        "scope": {"nodeId": req.scope, "name": scope.map(|r| r.name.as_str())},
        "profileId": profile.map(|p| &p.id),
        "ranking": {"version": if mode == Mode::Hybrid { model::FUSION } else { SEMANTIC_RANKING },
            "channels": {"text": if mode == Mode::Hybrid { json!(RANKING) } else { Json::Null }, "semantic": SEMANTIC_RANKING},
            "candidatesPerChannel": CANDIDATES},
        "results": results,
        "coverage": coverage,
        "hasMore": more,
        "nextCursor": (more && q.offset + req.limit <= MAX_OFFSET).then(|| format!("s1.{fingerprint}.{}", q.offset + req.limit)),
    }))
}

/// Records ranked by their best chunk's similarity to the query (filters applied first), each
/// with that chunk, and the layer's coverage.
fn ranked(
    index: &Index,
    req: &Request,
    profile: &Profile,
    cancel: &AtomicBool,
) -> Result<(Vec<Best>, Json), Stop> {
    let mut guard = model::lock(&LAYER, cancel)?;
    if !guard
        .as_ref()
        .is_some_and(|l| l.generation == index.generation && l.profile == profile.id)
    {
        *guard = Some(Layer::new(index, &profile.id));
    }
    let layer = guard.as_mut().expect("set above");
    layer.fill(profile, cancel)?;
    let query = profile::with_runtime(profile, cancel, |rt| {
        rt.embed(Kind::Query, std::slice::from_ref(&req.query), cancel)
    })?
    .remove(0);
    let vectors = layer.vectors.iter().enumerate().filter_map(|(i, v)| {
        v.as_ref()
            .filter(|v| !v.is_empty())
            .map(|v| (i, v.as_slice()))
    });
    let scores: HashMap<usize, f32> = model::nearest(&query, vectors, layer.texts.len())
        .map_err(|e| Stop::Failed(e.to_string()))?
        .into_iter()
        .collect();
    let mut best: BTreeMap<u32, (f32, u8, u32)> = BTreeMap::new();
    for &(rid, field, chunk, text) in &layer.units {
        let Some(&score) = scores.get(&text) else {
            continue;
        };
        let r = index.record(rid);
        let internal = r.page.is_some_and(|p| index.pages[p as usize].internal);
        if (!req.types.is_empty() && !req.types.contains(&r.kind))
            || (r.hidden && !req.include_hidden)
            || (internal && !req.include_internal)
        {
            continue;
        }
        let e = best.entry(rid).or_insert((f32::NEG_INFINITY, field, chunk));
        if score > e.0 {
            *e = (score, field, chunk);
        }
    }
    let mut order: Vec<(u32, (f32, u8, u32))> = best.into_iter().collect();
    order.sort_by(|a, b| b.1.0.total_cmp(&a.1.0).then(a.0.cmp(&b.0)));
    order.truncate(CANDIDATES);
    let (embedded, refused) = (layer.embedded(), layer.refused());
    let complete = embedded == layer.texts.len() && refused == 0 && layer.over == 0;
    let mut reasons = Vec::new();
    if embedded < layer.texts.len() {
        reasons.push(json!({"code": "semantic_layer_building", "counts": {"embeddedTexts": embedded, "texts": layer.texts.len()}}));
    }
    if refused > 0 {
        reasons.push(json!({"code": "chunk_refused", "counts": {"texts": refused}}));
    }
    if layer.over > 0 {
        reasons.push(json!({"code": "semantic_capacity", "counts": {"chunks": layer.over, "maxTexts": MAX_TEXTS}}));
    }
    let json = json!({"state": if complete { "ready" } else { "partial" }, "profileId": profile.id,
        "chunks": layer.units.len(), "texts": layer.texts.len(), "embeddedTexts": embedded, "reasons": reasons});
    Ok((
        order
            .into_iter()
            .map(|(rid, (_, field, chunk))| (rid, field, chunk))
            .collect(),
        json,
    ))
}
