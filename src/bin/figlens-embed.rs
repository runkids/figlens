//! `figlens-embed <model-dir>`: the local text model child of figlens semantic search (#94).
//! BGE-M3 dense: XLM-RoBERTa, CLS pooling, L2 normalization, no instruction, read from the
//! model's own `pytorch_model.bin` by candle's data-only pickle reader. It reads only that
//! directory, opens no network connection and speaks `figlens::semantic`'s stdio protocol.

use candle_core::{DType, Device, Tensor};
use candle_transformers::models::xlm_roberta::{Config, XLMRobertaModel};
use figlens::semantic::{self, Kind};
use serde_json::json;
use std::error::Error;
use std::path::PathBuf;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Tokens per text, special tokens included; a longer text is refused, never truncated.
const TOKENS: usize = 512;
/// Texts per forward pass, sorted by length so padding stays short; the mask keeps it exact.
const BATCH: usize = 16;
/// CPU threads unless RAYON_NUM_THREADS says otherwise: more oversubscribed an 18-CPU host.
const THREADS: usize = 8;

const USAGE: &str = "usage: figlens-embed <model-dir>\n\nThe local text model child of figlens semantic search. figlens starts it after\n`figlens models configure`; it reads only <model-dir>, opens no network connection\nand speaks figlens's stdio protocol.";

fn main() -> Result<()> {
    // Answered before any model file is read, so packaging can check the binary alone.
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("figlens-embed {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--help" | "-h") | None => {
            println!("{USAGE}");
            return Ok(());
        }
        _ => {}
    }
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: figlens-embed <model-dir>")?,
    );
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        // SAFETY: no other thread exists yet.
        unsafe { std::env::set_var("RAYON_NUM_THREADS", cpus.min(THREADS).to_string()) };
    }
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
    if config["model_type"] != "xlm-roberta" {
        return Err("config.json is not an xlm-roberta model".into());
    }
    let config: Config = serde_json::from_value(config)?;
    let vb =
        candle_nn::VarBuilder::from_pth(dir.join("pytorch_model.bin"), DType::F32, &Device::Cpu)?;
    let model = XLMRobertaModel::new(&config, vb)?;
    let mut tokenizer =
        tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
    tokenizer.with_truncation(None).map_err(|e| e.to_string())?;
    let pad_token = tokenizer
        .id_to_token(config.pad_token_id)
        .ok_or("the pad id has no token")?;
    tokenizer.with_padding(Some(tokenizers::PaddingParams {
        pad_id: config.pad_token_id,
        pad_token,
        ..Default::default()
    }));
    let hello = json!({
        "protocol": semantic::PROTOCOL, "dim": config.hidden_size, "model": "BAAI/bge-m3",
        "pooling": "cls", "normalize": true, "queryPrefix": "", "passagePrefix": "",
        "maxTokens": TOKENS, "runtime": format!("figlens-embed {} candle 0.11.0", env!("CARGO_PKG_VERSION")),
    });
    semantic::serve(&hello, |_: Kind, texts| embed(&model, &tokenizer, texts))
}

fn rows(
    enc: &[tokenizers::Encoding],
    f: impl Fn(&tokenizers::Encoding) -> &[u32],
) -> Result<Tensor> {
    let rows = enc
        .iter()
        .map(|e| Tensor::new(f(e), &Device::Cpu))
        .collect::<candle_core::Result<Vec<_>>>()?;
    Ok(Tensor::stack(&rows, 0)?)
}

fn embed(
    model: &XLMRobertaModel,
    tokenizer: &tokenizers::Tokenizer,
    texts: &[String],
) -> Result<Vec<Vec<f32>>> {
    let mut order: Vec<usize> = (0..texts.len()).collect();
    order.sort_by_key(|&i| texts[i].len());
    let mut out = vec![Vec::new(); texts.len()];
    for batch in order.chunks(BATCH) {
        let inputs: Vec<&str> = batch.iter().map(|&i| texts[i].as_str()).collect();
        let enc = tokenizer
            .encode_batch(inputs, true)
            .map_err(|e| e.to_string())?;
        if enc
            .iter()
            .any(|e| e.get_attention_mask().iter().sum::<u32>() as usize > TOKENS)
        {
            return Err(format!("a text is longer than {TOKENS} tokens").into());
        }
        let ids = rows(&enc, |e| e.get_ids())?;
        let mask = rows(&enc, |e| e.get_attention_mask())?;
        let hidden = model.forward(&ids, &mask, &ids.zeros_like()?, None, None, None)?;
        let cls = hidden.narrow(1, 0, 1)?.squeeze(1)?;
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
        for (v, &i) in cls
            .broadcast_div(&norm)?
            .to_vec2::<f32>()?
            .into_iter()
            .zip(batch)
        {
            out[i] = v;
        }
    }
    Ok(out)
}
