//! Measure token export stages without workspace/index setup; use release builds.
use std::{error::Error, io::Write, path::Path, time::Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let [source, collection, mode, output] = <[String; 4]>::try_from(
        std::env::args().skip(1).collect::<Vec<_>>(),
    )
    .map_err(|_| "usage: token-probe <file.fig> <collection-id> <mode-id> <out.tokens.json>")?;
    let source = Path::new(&source).canonicalize()?;
    let start = Instant::now();
    let mut doc = figlens::Document::open(&source)?;
    let message = doc.take_message()?;
    let tree = figlens::Tree::new(&message);
    let parse_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let snapshot = figlens::variables::variables(&tree, None);
    let extraction_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let mut tokens = figlens::tokens::convert(&snapshot, &collection, &mode)?;
    tokens["$extensions"]["org.figlens"]["source"] = serde_json::to_value(&source)?;
    let conversion_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    figlens::output::write(&source, Path::new(&output), |file| {
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &tokens)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    })?;
    let writing_ms = start.elapsed().as_secs_f64() * 1000.0;
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&output)?)?;
    assert_eq!(written, tokens);
    println!(
        "{}",
        serde_json::json!({
            "sourceBytes": std::fs::metadata(&source)?.len(),
            "outputBytes": std::fs::metadata(&output)?.len(),
            "variables": snapshot["variables"].as_array().map_or(0, Vec::len),
            "parseMs": parse_ms, "extractionMs": extraction_ms,
            "conversionMs": conversion_ms, "writingMs": writing_ms,
            "modeId": mode,
        })
    );
    Ok(())
}
