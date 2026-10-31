//! CPU-only diagnostic for the pinned visual candidate; output belongs in ignored samples/.
use std::error::Error;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use figlens::search::visual::{MAX_BYTES, QueryImage, model};
use serde_json::json;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .ok_or("usage: visual-preflight MODEL_DIR IMAGE.png ...")?;
    rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build_global()?;
    let started = Instant::now();
    let model = model::Model::load(Path::new(&dir))?;
    eprintln!("loadMs={:.1}", started.elapsed().as_secs_f64() * 1000.0);
    let mut batch = Vec::new();
    for path in args {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        let image = QueryImage::from_png(&bytes, None)?;
        let images = [image];
        let input = model::pixels(&images)?;
        let started = Instant::now();
        let vectors = model.embed(&images)?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let input: Vec<u8> = input.iter().flat_map(|v| v.to_le_bytes()).collect();
        println!(
            "{}",
            json!({"path":path,"ms":ms,"vectors":vectors,"inputF32le":figlens::render::base64(&input)})
        );
        if batch.len() < model::MAX_BATCH {
            batch.push(images.into_iter().next().unwrap());
        }
    }
    if !batch.is_empty() {
        let started = Instant::now();
        let vectors = model.embed(&batch)?;
        println!(
            "{}",
            json!({"batch": true, "ms": started.elapsed().as_secs_f64() * 1000.0, "vectors": vectors})
        );
    }
    Ok(())
}
