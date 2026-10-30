//! Test child for `src/semantic.rs`: speaks the embedding protocol with deterministic
//! bag-of-characters vectors, no model. An argument `mode=<m>` selects a misbehaviour: `nan`,
//! `dim`, `hang` (never answers) or `grandchild=<file>` (starts a sleeper, writes its pid).

use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).unwrap_or_default();
    let mode = arg.strip_prefix("mode=").unwrap_or_default().to_owned();
    let dim = 8;
    if let Some(file) = mode.strip_prefix("grandchild=") {
        let sleeper = std::process::Command::new("sleep").arg("60").spawn()?;
        std::fs::write(file, sleeper.id().to_string())?;
    }
    let hello = json!({"protocol": figlens::semantic::PROTOCOL, "dim": dim, "model": "fake", "pooling": "none"});
    figlens::semantic::serve(&hello, |_, texts| {
        if mode == "hang" {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; if mode == "dim" { dim + 1 } else { dim }];
                for c in t.chars() {
                    v[c as usize % dim] += 1.0;
                }
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                v.iter_mut()
                    .for_each(|x| *x = if mode == "nan" { f32::NAN } else { *x / n });
                v
            })
            .collect())
    })
}
