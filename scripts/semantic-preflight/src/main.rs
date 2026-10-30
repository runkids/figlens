//! Issue #94 preflight: a synthetic design file, exact-vector rankings with timings, and a
//! bounded stdio embedding child. Not product code.
use kiwi_schema::{DefKind, Def, Field, Schema, TYPE_STRING, TYPE_UINT, Value};
use candle_core::{DType, Device, Tensor};
use candle_transformers::models::bert::{BertModel, Config};
use candle_transformers::models::qwen3;
use candle_transformers::models::xlm_roberta::{Config as XlmConfig, XLMRobertaForSequenceClassification};
use model2vec_rs::model::StaticModel;
use serde_json::{Value as Json, json};
use std::error::Error;
use std::io::{BufRead, Read, Write};
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Texts one child request may carry, and bytes per text; larger requests are refused, never
/// truncated.
const BATCH: usize = 64;
const TEXT_BYTES: usize = 2048;
const LINE_BYTES: usize = 256 * 1024;
const TOP: usize = 50;

fn obj(name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
    Value::Object(name, fields.into_iter().collect())
}

fn guid(id: u32) -> Value<'static> {
    obj("GUID", vec![("sessionID", Value::UInt(0)), ("localID", Value::UInt(id))])
}

fn local(id: &str) -> u32 {
    id.trim_start_matches("0:").parse().expect("corpus ids are 0:<n>")
}

fn node(id: u32, parent: Option<u32>, kind: &'static str, name: &str) -> Vec<(&'static str, Value<'static>)> {
    let mut f = vec![
        ("guid", guid(id)),
        ("type", Value::Enum("NodeType", kind)),
        ("name", Value::String(name.into())),
    ];
    if let Some(p) = parent {
        let index = obj("ParentIndex", vec![("guid", guid(p)), ("position", Value::String(format!("{id:05}")))]);
        f.push(("parentIndex", index));
    }
    f
}

/// One canvas with a frame per corpus layer, holding that layer's text node.
fn fig(corpus: &Json, out: &str) -> Result<()> {
    let field = |name: &str, type_id, value, is_array| Field { name: name.into(), type_id, value, is_array };
    let schema = Schema::new(vec![
        Def::new("GUID".into(), DefKind::Struct, vec![field("sessionID", TYPE_UINT, 0, false), field("localID", TYPE_UINT, 0, false)]),
        Def::new("NodeType".into(), DefKind::Enum, ["DOCUMENT", "CANVAS", "FRAME", "TEXT"].iter().enumerate().map(|(i, n)| field(n, 0, i as u32, false)).collect()),
        Def::new("ParentIndex".into(), DefKind::Struct, vec![field("guid", 0, 0, false), field("position", TYPE_STRING, 0, false)]),
        Def::new("TextData".into(), DefKind::Message, vec![field("characters", TYPE_STRING, 1, false)]),
        Def::new("NodeChange".into(), DefKind::Message, vec![
            field("guid", 0, 1, false), field("type", 1, 2, false), field("name", TYPE_STRING, 3, false),
            field("parentIndex", 2, 4, false), field("textData", 3, 5, false),
        ]),
        Def::new("Message".into(), DefKind::Message, vec![field("nodeChanges", 4, 1, true)]),
    ]);
    let mut nodes = vec![obj("NodeChange", node(0, None, "DOCUMENT", "Document")), obj("NodeChange", node(1, Some(0), "CANVAS", "Screens"))];
    for (k, l) in corpus["layers"].as_array().ok_or("layers")?.iter().enumerate() {
        let text = l["text"].as_str().ok_or("text")?;
        let frame = local(l["frame"].as_str().ok_or("frame")?);
        nodes.push(obj("NodeChange", node(frame, Some(1), "FRAME", &format!("Frame {k}"))));
        let mut t = node(local(l["node"].as_str().ok_or("node")?), Some(frame), "TEXT", text);
        t.push(("textData", obj("TextData", vec![("characters", Value::String(text.into()))])));
        nodes.push(obj("NodeChange", t));
    }
    let message = obj("Message", vec![("nodeChanges", Value::Array(nodes))]);
    let mut canvas = b"fig-kiwi".to_vec();
    canvas.extend(106u32.to_le_bytes());
    for bytes in [schema.encode(), message.encode(&schema)] {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&bytes)?;
        let bytes = e.finish()?;
        canvas.extend((bytes.len() as u32).to_le_bytes());
        canvas.extend(bytes);
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("canvas.fig", zip::write::SimpleFileOptions::default())?;
    zip.write_all(&canvas)?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(out)?;
    file.write_all(&zip.finish()?.into_inner())?;
    Ok(())
}

/// The most tokens any contract may allow: the BERT encoders' position limit, and this preflight's cap for Qwen3.
const TOKENS: usize = 512;

enum Model {
    Static(StaticModel),
    /// A BERT encoder with mean pooling over the attention mask and the model's own input contract.
    Bert(Box<BertModel>, Box<tokenizers::Tokenizer>, Contract),
    /// A Qwen3 decoder with last-token pooling, one text per forward pass: candle's CPU attention
    /// is causal only, so a padded batch would let real tokens attend to padding.
    Qwen3(Box<qwen3::Model>, Box<tokenizers::Tokenizer>, Contract),
}

/// The query and passage prefixes and the token limit a BERT or Qwen3 model card specifies, from
/// the `figlens-preflight.json` every such model directory holds: no model inherits another's prompts.
/// A text over the limit, prefix and special tokens included, is refused, never truncated.
struct Contract {
    query: String,
    passage: String,
    tokens: usize,
}

/// One row per padded encoding, stacked into a batch tensor.
fn rows(enc: &[tokenizers::Encoding], f: impl Fn(&tokenizers::Encoding) -> &[u32]) -> Result<Tensor> {
    let v: Vec<Tensor> = enc.iter().map(|e| Tensor::new(f(e), &Device::Cpu)).collect::<std::result::Result<_, _>>()?;
    Ok(Tensor::stack(&v, 0)?)
}

impl Model {
    fn embed(&self, texts: &[String], query: bool) -> Result<Vec<Vec<f32>>> {
        match self {
            Model::Static(m) => Ok(m.encode(texts)),
            Model::Bert(m, tokenizer, contract) => {
                let prefix = if query { &contract.query } else { &contract.passage };
                let inputs: Vec<String> = texts.iter().map(|t| format!("{prefix}{t}")).collect();
                let enc = tokenizer.encode_batch(inputs, true).map_err(|e| e.to_string())?;
                if enc.iter().any(|e| e.get_ids().len() > contract.tokens) {
                    return Err(format!("a text with its prefix is longer than {} tokens", contract.tokens).into());
                }
                let ids = rows(&enc, |e| e.get_ids())?;
                let mask = rows(&enc, |e| e.get_attention_mask())?;
                let hidden = m.forward(&ids, &ids.zeros_like()?, Some(&mask))?;
                let mask = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
                let pooled = hidden.broadcast_mul(&mask)?.sum(1)?.broadcast_div(&mask.sum(1)?)?;
                let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
                Ok(pooled.broadcast_div(&norm)?.to_vec2()?)
            }
            Model::Qwen3(m, tokenizer, contract) => {
                let prefix = if query { &contract.query } else { &contract.passage };
                texts.iter().map(|t| {
                    let enc = tokenizer.encode(format!("{prefix}{t}"), true).map_err(|e| e.to_string())?;
                    let ids = enc.get_ids();
                    if ids.len() > contract.tokens {
                        return Err(format!("a text with its prefix is longer than {} tokens", contract.tokens).into());
                    }
                    // A clone of the never-run model has an empty KV cache (its reset is private) and
                    // shares the weight tensors, so no weight is copied.
                    let hidden = (**m).clone().forward(&Tensor::new(ids, &Device::Cpu)?.unsqueeze(0)?, 0)?;
                    // The tokenizer appends <|endoftext|>; its hidden state is the embedding.
                    let last = hidden.get(0)?.get(ids.len() - 1)?;
                    Ok(last.broadcast_div(&last.sqr()?.sum_all()?.sqrt()?)?.to_vec1()?)
                }).collect()
            }
        }
    }
}

/// Loads only from an existing local directory holding the three files (and a BERT or Qwen3 model's
/// contract): no code here can download (model2vec is built `local-only`, candle reads the
/// safetensors file).
fn load(dir: &str) -> Result<(Model, f64)> {
    let path = std::path::Path::new(dir);
    for file in ["config.json", "model.safetensors", "tokenizer.json"] {
        if !path.join(file).is_file() {
            return Err(format!("{dir} has no {file}; nothing is downloaded").into());
        }
    }
    let t = Instant::now();
    let config: Json = serde_json::from_str(&std::fs::read_to_string(path.join("config.json"))?)?;
    let model = if config["model_type"] == "model2vec" {
        Model::Static(StaticModel::from_pretrained(dir, None, None, None)?)
    } else if config["model_type"] == "qwen3" {
        let config: qwen3::Config = serde_json::from_value(config)?;
        // SAFETY: the pinned weights file is only read while this process runs.
        let st = unsafe { candle_core::safetensors::MmapedSafetensors::new(path.join("model.safetensors"))? };
        let mut weights = std::collections::HashMap::new();
        for (name, _) in st.tensors() {
            // The card's checkpoint is the bare decoder; candle names it under `model.`.
            weights.insert(format!("model.{name}"), st.load(&name, &Device::Cpu)?.to_dtype(DType::F32)?);
        }
        let mut tokenizer = tokenizers::Tokenizer::from_file(path.join("tokenizer.json")).map_err(|e| e.to_string())?;
        tokenizer.with_truncation(None).map_err(|e| e.to_string())?;
        tokenizer.with_padding(None);
        let vb = candle_nn::VarBuilder::from_tensors(weights, DType::F32, &Device::Cpu);
        Model::Qwen3(Box::new(qwen3::Model::new(&config, vb)?), Box::new(tokenizer), contract(dir)?)
    } else {
        let config: Config = serde_json::from_value(config)?;
        // SAFETY: the pinned weights file is only read while this process runs.
        let vb = unsafe {
            candle_nn::VarBuilder::from_mmaped_safetensors(&[path.join("model.safetensors")], DType::F32, &Device::Cpu)?
        };
        let mut tokenizer = tokenizers::Tokenizer::from_file(path.join("tokenizer.json")).map_err(|e| e.to_string())?;
        tokenizer.with_truncation(None).map_err(|e| e.to_string())?;
        tokenizer.with_padding(Some(tokenizers::PaddingParams::default()));
        let contract = contract(dir)?;
        Model::Bert(Box::new(BertModel::load(vb, &config)?), Box::new(tokenizer), contract)
    };
    Ok((model, t.elapsed().as_secs_f64() * 1e3))
}

fn contract(dir: &str) -> Result<Contract> {
    let contract = std::fs::read_to_string(std::path::Path::new(dir).join("figlens-preflight.json"))
        .map_err(|e| format!("{dir} has no readable figlens-preflight.json with its card's contract: {e}"))?;
    let c: Json = serde_json::from_str(&contract)?;
    let text = |k: &str| c[k].as_str().map(str::to_owned).ok_or(format!("figlens-preflight.json has no {k}"));
    let tokens = c["maxTokens"].as_u64().filter(|&n| n > 0 && n as usize <= TOKENS).ok_or("figlens-preflight.json maxTokens must be 1-512")?;
    Ok(Contract { query: text("queryPrefix")?, passage: text("passagePrefix")?, tokens: tokens as usize })
}

/// Pairs one rerank forward pass scores at a time.
const RERANK_BATCH: usize = 32;

/// Scores `{"pairs": [[query, text], ...]}` with a one-label XLM-R cross-encoder: the card's pair
/// encoding (`<s> query </s></s> text </s>`), padded with the model's own pad id (its position ids
/// skip pad tokens), and the raw logit as the score.
fn rerank(dir: &str, pairs: &Json) -> Result<Json> {
    let path = std::path::Path::new(dir);
    let t = Instant::now();
    let raw: Json = serde_json::from_str(&std::fs::read_to_string(path.join("config.json"))?)?;
    if raw["model_type"] != "xlm-roberta" || raw["id2label"].as_object().map(|m| m.len()) != Some(1) {
        return Err(format!("{dir} is not a one-label xlm-roberta cross-encoder").into());
    }
    let config: XlmConfig = serde_json::from_value(raw)?;
    let contract = contract(dir)?;
    // SAFETY: the pinned weights file is only read while this process runs.
    let vb = unsafe {
        candle_nn::VarBuilder::from_mmaped_safetensors(&[path.join("model.safetensors")], DType::F32, &Device::Cpu)?
    };
    let model = XLMRobertaForSequenceClassification::new(1, &config, vb)?;
    let mut tokenizer = tokenizers::Tokenizer::from_file(path.join("tokenizer.json")).map_err(|e| e.to_string())?;
    tokenizer.with_truncation(None).map_err(|e| e.to_string())?;
    let pad_token = tokenizer.id_to_token(config.pad_token_id).ok_or("the pad id has no token")?;
    tokenizer.with_padding(Some(tokenizers::PaddingParams { pad_id: config.pad_token_id, pad_token, ..Default::default() }));
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    let pairs = pairs["pairs"].as_array().ok_or("pairs")?;
    let t = Instant::now();
    let mut scores = Vec::with_capacity(pairs.len());
    for chunk in pairs.chunks(RERANK_BATCH) {
        let inputs: Vec<tokenizers::EncodeInput> = chunk
            .iter()
            .map(|p| Ok((format!("{}{}", contract.query, p[0].as_str().ok_or("pair query")?), format!("{}{}", contract.passage, p[1].as_str().ok_or("pair text")?)).into()))
            .collect::<Result<_>>()?;
        let enc = tokenizer.encode_batch(inputs, true).map_err(|e| e.to_string())?;
        if enc.iter().any(|e| e.get_ids().len() > contract.tokens) {
            return Err(format!("a pair is longer than {} tokens", contract.tokens).into());
        }
        let ids = rows(&enc, |e| e.get_ids())?;
        let mask = rows(&enc, |e| e.get_attention_mask())?;
        let logits = model.forward(&ids, &mask, &ids.zeros_like()?)?.squeeze(1)?.to_vec1::<f32>()?;
        if !logits.iter().all(|x| x.is_finite()) {
            return Err("a score is not finite".into());
        }
        scores.extend(logits);
    }
    Ok(json!({"loadMs": load_ms, "scoreMs": t.elapsed().as_secs_f64() * 1e3, "pairs": pairs.len(), "peakRssMiB": hwm(), "scores": scores}))
}

/// Peak resident set of this process from /proc, in MiB (Linux only).
fn hwm() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let l = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    Some(l.split_whitespace().nth(1)?.parse::<u64>().ok()? / 1024)
}

fn finite(v: &[f32], dim: usize) -> Result<()> {
    if v.len() != dim || !v.iter().all(|x| x.is_finite()) {
        return Err("vector has the wrong dimension or a non-finite value".into());
    }
    Ok(())
}

fn quantiles(mut xs: Vec<f64>) -> Json {
    if xs.is_empty() {
        return json!({"n": 0});
    }
    xs.sort_by(|a, b| a.total_cmp(b));
    let at = |p: usize| xs[(xs.len() * p).div_ceil(100).max(1) - 1];
    json!({"n": xs.len(), "p50": at(50), "p95": at(95), "max": xs[xs.len() - 1]})
}

/// Exact cosine top `TOP` concepts per query (vectors are normalized), with timings.
fn rank(dir: &str, corpus: &Json, queries: &Json, warm: usize) -> Result<Json> {
    let (model, load_ms) = load(dir)?;
    let dim = model.embed(&["dim".to_owned()], true)?[0].len();
    let layers = corpus["layers"].as_array().ok_or("layers")?;
    let texts: Vec<String> = layers.iter().map(|l| l["text"].as_str().unwrap_or("").to_owned()).collect();
    let t = Instant::now();
    let docs = model.embed(&texts, false)?;
    let corpus_ms = t.elapsed().as_secs_f64() * 1e3;
    for d in &docs {
        finite(d, dim)?;
    }
    let qs = queries["queries"].as_array().ok_or("queries")?;
    let (mut embed, mut search, mut out) = (Vec::new(), Vec::new(), Vec::new());
    for round in 0..=warm {
        for q in qs {
            let text = q["query"].as_str().ok_or("query")?;
            let t = Instant::now();
            let v = model.embed(&[text.to_owned()], true)?.remove(0);
            let e = t.elapsed().as_secs_f64() * 1e3;
            finite(&v, dim)?;
            let t = Instant::now();
            let mut scored: Vec<(f32, usize)> = docs.iter().enumerate().map(|(i, d)| (d.iter().zip(&v).map(|(a, b)| a * b).sum(), i)).collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            scored.truncate(TOP);
            let s = t.elapsed().as_secs_f64() * 1e3;
            if round == 0 {
                let ranked: Vec<Json> = scored.iter().map(|&(score, i)| json!({"concept": layers[i]["concept"], "score": score})).collect();
                out.push(json!({"query": text, "ranked": ranked}));
            } else {
                embed.push(e);
                search.push(s);
            }
        }
    }
    Ok(json!({
        "dim": dim, "layers": layers.len(), "loadMs": load_ms, "corpusEmbedMs": corpus_ms,
        "warmQueryEmbedMs": quantiles(embed), "exactSearchMs": quantiles(search),
        "peakRssMiB": hwm(), "rankings": out,
    }))
}

/// Answers `{"id", "texts"}` lines with `{"id", "dim", "vectors"}` until stdin closes; an
/// oversized or malformed request gets an error line and the child keeps serving.
fn child(dir: &str) -> Result<()> {
    let (model, _) = load(dir)?;
    let dim = model.embed(&["dim".to_owned()], true)?[0].len();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", json!({"ready": true, "dim": dim}))?;
    out.flush()?;
    let mut line = String::new();
    let mut input = stdin.lock();
    loop {
        line.clear();
        let n = (&mut input).take(LINE_BYTES as u64 + 1).read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        let reply = match serde_json::from_str::<Json>(&line) {
            _ if n > LINE_BYTES => json!({"error": "request line too long"}),
            Ok(r) => match r["texts"].as_array() {
                Some(t) if !t.is_empty() && t.len() <= BATCH && t.iter().all(|s| s.as_str().is_some_and(|s| s.len() <= TEXT_BYTES)) => {
                    let texts: Vec<String> = t.iter().map(|s| s.as_str().unwrap().to_owned()).collect();
                    match model.embed(&texts, true).and_then(|v| v.iter().try_for_each(|x| finite(x, dim)).map(|()| v)) {
                        Ok(vectors) => json!({"id": r["id"], "dim": dim, "vectors": vectors}),
                        Err(e) => json!({"id": r["id"], "error": e.to_string()}),
                    }
                }
                _ => json!({"id": r["id"], "error": "texts must be 1-64 strings of at most 2048 bytes"}),
            },
            Err(_) => json!({"error": "malformed request"}),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let read = |p: &str| -> Result<Json> { Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?) };
    match args.get(1).map(String::as_str) {
        Some("fig") if args.len() == 4 => fig(&read(&args[2])?, &args[3]),
        Some("rank") if args.len() == 6 => {
            println!("{}", rank(&args[2], &read(&args[3])?, &read(&args[4])?, args[5].parse()?)?);
            Ok(())
        }
        Some("child") if args.len() == 3 => child(&args[2]),
        Some("rerank") if args.len() == 4 => {
            println!("{}", rerank(&args[2], &read(&args[3])?)?);
            Ok(())
        }
        _ => Err("usage: semantic-preflight fig <corpus.json> <new.fig> | rank <model-dir> <corpus.json> <queries.json> <warm-rounds> | child <model-dir> | rerank <model-dir> <pairs.json>".into()),
    }
}
