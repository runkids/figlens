//! One global rank fusion across the text, semantic, visual-hash and visual-embedding channels
//! of an image search. Each channel is ranked over the whole selected corpus first; their
//! candidate lists are fused once. Keys keep source identity: (part, record).

use super::{Distance, Hit as VisualHit};
use crate::search::{Hit, Part, Prepared, Ranked, Request, corpus_rank, short_hash};
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::AtomicBool;

pub(crate) type Key = (usize, u32);
/// Candidates each channel contributes, ranked across every selected source.
pub(crate) const CANDIDATES: usize = 200;
/// The version of how visual channels are fused; the cursor binds it.
pub(crate) const RANKING: &str = "visual-rrf-1";

/// Corpus-wide candidate lists, best first. `None` is a channel that did not run.
pub(crate) struct Channels {
    pub text: Option<Vec<Hit>>,
    /// Semantic keys and, per key, its field/chunk evidence.
    pub semantic: Option<(Vec<Key>, Vec<Option<Json>>)>,
    /// The perceptual-hash ranking: the visual signal when no vision layer is used.
    pub hash: Option<Vec<(Key, VisualHit)>>,
    /// Exact normalized-pixel copies only, beside a vision ranking.
    pub exact: Option<Vec<(Key, VisualHit)>>,
    pub vision: Option<Vec<(Key, f32)>>,
}

pub(crate) struct Decor {
    matches: Vec<Vec<Json>>,
    lexical: Vec<bool>,
    channels: Vec<&'static str>,
}

/// `q` as a first page with no cursor, for a channel's own corpus-wide candidate list.
pub(crate) fn first(q: &Prepared) -> Prepared {
    Prepared {
        offset: 0,
        fingerprint: None,
        gens: None,
        issued: None,
        phrase: q.phrase.clone(),
        terms: q.terms.clone(),
    }
}

/// The lexical candidates of `parts`: the text ranking's first `CANDIDATES` hits.
pub(crate) fn text(
    parts: &[Part],
    q: &Prepared,
    req: &Request,
    key: &Json,
    cancel: &AtomicBool,
) -> Result<Vec<Hit>, Box<dyn Error>> {
    let wide = Request {
        limit: CANDIDATES,
        cursor: None,
        ..req.clone()
    };
    Ok(corpus_rank(parts, &first(q), &wide, key, cancel)?.hits)
}

/// Fuses the channels with the versioned RRF of semantic search and pages the result. `key`
/// identifies the sources, query image and profiles; a cursor of another key, or any cursor
/// when `complete` is false, is refused.
pub(crate) fn rank(
    q: &Prepared,
    req: &Request,
    key: &Json,
    channels: Channels,
    complete: bool,
) -> Result<(Ranked, Decor), Box<dyn Error>> {
    let keys = |v: &mut dyn Iterator<Item = Key>| v.take(CANDIDATES).collect::<Vec<Key>>();
    let mut names = Vec::new();
    let mut lists: Vec<Vec<Key>> = Vec::new();
    if let Some(hits) = &channels.text {
        names.push("text");
        lists.push(keys(&mut hits.iter().map(|h| (h.source, h.record))));
    }
    if let Some((semantic, _)) = &channels.semantic {
        names.push("semantic");
        lists.push(keys(&mut semantic.iter().copied()));
    }
    if let Some(hash) = &channels.hash {
        names.push("visual_hash");
        lists.push(keys(&mut hash.iter().map(|h| h.0)));
    }
    if let Some(exact) = &channels.exact {
        names.push("visual_exact");
        lists.push(keys(&mut exact.iter().map(|h| h.0)));
    }
    if let Some(vision) = &channels.vision {
        names.push("visual_embedding");
        lists.push(keys(&mut vision.iter().map(|h| h.0)));
    }
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
            names,
            RANKING,
            crate::semantic::FUSION,
            CANDIDATES,
        ])
        .to_string(),
    );
    if q.fingerprint
        .as_ref()
        .is_some_and(|f| *f != fingerprint || !complete)
    {
        return Err(crate::FigError::new(
            crate::ErrorKind::CursorStale,
            "this search cursor belongs to another query image, mode, profile, source set or coverage; search again without cursor",
        )
        .into());
    }
    let refs: Vec<&[Key]> = lists.iter().map(Vec::as_slice).collect();
    let fused = crate::semantic::fuse(&refs);
    let more = fused.len() > q.offset + req.limit;
    let text_hits: HashMap<Key, &Hit> = channels
        .text
        .iter()
        .flatten()
        .map(|h| ((h.source, h.record), h))
        .collect();
    let mut evidence: HashMap<Key, Vec<Json>> = HashMap::new();
    if let Some((semantic, basis)) = &channels.semantic {
        for (k, b) in semantic.iter().zip(basis) {
            if let Some(b) = b {
                evidence.entry(*k).or_default().push(b.clone());
            }
        }
    }
    for (k, h) in channels.hash.iter().chain(&channels.exact).flatten() {
        evidence
            .entry(*k)
            .or_default()
            .push(hash_match(h.exact, h.distance));
    }
    for (k, cosine) in channels.vision.iter().flatten() {
        evidence
            .entry(*k)
            .or_default()
            .push(json!({"basis": "visual_embedding", "cosine": cosine}));
    }
    let mut hits = Vec::new();
    let mut matches = Vec::new();
    let mut lexical = Vec::new();
    for (k, score) in fused.into_iter().skip(q.offset).take(req.limit) {
        let text = text_hits.get(&k);
        hits.push(Hit {
            source: k.0,
            record: k.1,
            tier: text.map_or(0, |h| h.tier),
            score,
        });
        lexical.push(text.is_some());
        matches.push(evidence.remove(&k).unwrap_or_default());
    }
    Ok((
        Ranked {
            hits,
            more,
            excluded: [0, 0],
            fingerprint,
        },
        Decor {
            matches,
            lexical,
            channels: names,
        },
    ))
}

fn hash_match(exact: bool, distance: Distance) -> Json {
    json!({"basis": if exact { "normalized_pixels" } else { "perceptual_hash" },
        "exactPixels": exact, "distance": {"hamming": distance.hamming,
            "color": distance.color, "aspect": distance.aspect}})
}

/// Adds every channel's evidence to a page rendered from `rank`'s `Ranked`: real lexical spans
/// only for lexical hits, the others as their own basis. A partial page offers no continuation.
pub(crate) fn decorate(out: &mut Json, mode: &str, decor: Decor, complete: bool) {
    for (i, row) in out["results"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let mut matches = Vec::new();
        if decor.lexical.get(i).copied().unwrap_or(false) {
            for m in row["matches"].as_array_mut().into_iter().flatten() {
                m["basis"] = json!("lexical");
                matches.push(m.take());
            }
        }
        matches.extend(decor.matches.get(i).cloned().unwrap_or_default());
        row["matches"] = json!(matches);
    }
    out["query"]["mode"] = json!(mode);
    out["query"]["requestedMode"] = json!(mode);
    out["query"]["usedModes"] = json!(decor.channels);
    out["ranking"] = json!({"version": RANKING, "fusion": crate::semantic::FUSION,
        "channels": decor.channels, "candidatesPerChannel": CANDIDATES});
    if !complete {
        out["coverage"]["complete"] = json!(false);
        out["nextCursor"] = Json::Null;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            query: String::new(),
            scope: "0:0".into(),
            types: vec![],
            include_hidden: false,
            include_internal: false,
            limit: 20,
            cursor: None,
        }
    }

    fn hit(rid: u32, exact: bool) -> VisualHit {
        VisualHit {
            rid,
            exact,
            distance: Distance {
                hamming: 0,
                color: 0,
                aspect: 0,
            },
        }
    }

    #[test]
    fn fusion_keeps_sources_distinct_and_merges_all_reasons_for_a_node() {
        let req = request();
        let q = req.validate_image().unwrap();
        let channels = Channels {
            text: None,
            semantic: None,
            hash: Some(vec![((0, 7), hit(7, true)), ((1, 7), hit(7, false))]),
            exact: None,
            vision: Some(vec![((1, 7), 0.95), ((0, 7), 0.9)]),
        };
        let (ranked, decor) =
            rank(&q, &req, &json!(["image", "generations"]), channels, true).unwrap();
        assert_eq!(ranked.shown().collect::<Vec<_>>(), [(0, 7), (1, 7)]);
        let mut out =
            json!({"results": [{"matches": [{"span": "fake"}]}, {"matches": []}], "coverage": {}});
        decorate(&mut out, "visual", decor, true);
        let first = out["results"][0]["matches"].as_array().unwrap();
        assert_eq!(first.len(), 2);
        assert!(first.iter().all(|m| m.get("span").is_none()));
        assert_eq!(first[0]["basis"], "normalized_pixels");
        assert_eq!(out["results"][1]["matches"][0]["basis"], "perceptual_hash");
        assert_eq!(
            out["query"]["usedModes"],
            json!(["visual_hash", "visual_embedding"])
        );
    }

    #[test]
    fn cursor_binds_the_query_image_and_partial_coverage_cannot_resume() {
        let mut req = request();
        req.limit = 1;
        let make = || Channels {
            text: None,
            semantic: None,
            hash: Some(vec![((0, 1), hit(1, true)), ((0, 2), hit(2, false))]),
            exact: None,
            vision: None,
        };
        let (first, decor) = rank(
            &req.validate_image().unwrap(),
            &req,
            &json!("image-a"),
            make(),
            true,
        )
        .unwrap();
        assert!(first.more);
        // A partial first page names no continuation.
        let mut out = json!({"results": [{}], "coverage": {}, "nextCursor": "w1.x"});
        decorate(&mut out, "visual", decor, false);
        assert_eq!(
            (
                out["nextCursor"].clone(),
                out["coverage"]["complete"].clone()
            ),
            (Json::Null, json!(false))
        );
        req.cursor = Some(format!("w1.{}.1.1.0", first.fingerprint));
        let q = req.validate_image().unwrap();
        let next = rank(&q, &req, &json!("image-a"), make(), true).unwrap().0;
        assert_eq!(next.shown().collect::<Vec<_>>(), [(0, 2)]);
        assert!(rank(&q, &req, &json!("image-b"), make(), true).is_err());
        assert!(rank(&q, &req, &json!("image-a"), make(), false).is_err());
    }
}
