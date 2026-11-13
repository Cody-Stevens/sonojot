//! Token IDs are not exposed by fidelity_eval or punctuation_probe.
//! This reads only explicitly supplied synthetic strings and a tokenizer file.
use std::{fs, io::Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("Use text_tokenizer_goldens TOKENIZER.json INPUT.json OUTPUT.json".into());
    }
    let mut tokenizer = tokenizers::Tokenizer::from_file(&args[0])
        .map_err(|error| error.to_string())?;
    // Rust punctuation.rs::Engine::load / Engine::predict: no specials, padding or truncation.
    tokenizer.with_truncation(None).map_err(|error| error.to_string())?;
    tokenizer.with_padding(None);
    let inputs: Vec<String> = serde_json::from_slice(&fs::read(&args[1])?)?;
    let mut rows = Vec::new();
    for text in inputs {
        let encoded = tokenizer.encode(text.as_str(), false).map_err(|error| error.to_string())?;
        rows.push(serde_json::json!({"text": text, "ids": encoded.get_ids()}));
    }
    let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&args[2])?;
    serde_json::to_writer_pretty(&mut output, &rows)?;
    output.write_all(b"\n")?;
    Ok(())
}
