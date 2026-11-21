//! Writes `manifest.json` for a model directory, hashing every listed file.
//!
//! ```sh
//! cargo run --release --example make_pack -- <pack-dir> \
//!     --id whisper-base-multilingual-int8 --family whisper --languages '*' \
//!     encoder=base-encoder.int8.onnx decoder=base-decoder.int8.onnx tokens=base-tokens.txt \
//!     other=LICENSE
//! ```
//!
//! Options: `--version <v>` (default 1), `--model-type transducer|nemo_transducer`,
//! `--language <code|auto>`, `--feature-dim <n>`, `--max-segment-ms <ms>`, `--no-itn`,
//! `--name <display name>`, `--license <spdx>`, `--source <url>`.

use sonojot_mobile::{
    ModelFamily, ModelOptions, ModelPackManifest, PackFile, RuntimeRequirement, MANIFEST_FILE, MANIFEST_SCHEMA,
    SHERPA_ONNX_ABI,
};
use std::path::PathBuf;

fn main() {
    if let Err(message) = run() {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("usage: make_pack <pack-dir> --id ID --family F --languages a,b role=path…")?);
    let (mut id, mut family, mut languages, mut version) = (None, None, None, "1".to_string());
    let (mut options, mut display_name, mut license, mut source) = (ModelOptions::default(), None, None, None);
    let mut roles = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--id" => id = Some(value()?),
            "--family" => {
                family = Some(
                    serde_json::from_value::<ModelFamily>(serde_json::Value::String(value()?))
                        .map_err(|_| "family must be whisper, sense_voice, transducer or moonshine")?,
                )
            }
            "--languages" => languages = Some(value()?.split(',').map(str::to_owned).collect::<Vec<_>>()),
            "--version" => version = value()?,
            "--model-type" => options.model_type = Some(value()?),
            "--language" => options.language = Some(value()?),
            "--feature-dim" => options.feature_dim = Some(value()?.parse().map_err(|_| "bad --feature-dim")?),
            "--max-segment-ms" => options.max_segment_ms = Some(value()?.parse().map_err(|_| "bad --max-segment-ms")?),
            "--no-itn" => options.use_itn = Some(false),
            "--name" => display_name = Some(value()?),
            "--license" => license = Some(value()?),
            "--source" => source = Some(value()?),
            role_path if role_path.contains('=') => {
                let (role, path) = role_path.split_once('=').expect("contains =");
                roles.push((role.to_owned(), path.to_owned()));
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let files = roles
        .iter()
        .map(|(role, path)| PackFile::from_disk(&dir, role, path).map_err(|e| format!("{path}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let manifest = ModelPackManifest {
        schema: MANIFEST_SCHEMA,
        id: id.ok_or("--id is required")?,
        version,
        family: family.ok_or("--family is required")?,
        languages: languages.ok_or("--languages is required")?,
        runtime: RuntimeRequirement { kind: "sherpa-onnx".into(), min_version: SHERPA_ONNX_ABI.into() },
        options,
        files,
        display_name,
        license,
        source,
    };
    manifest.validate().map_err(|e| e.to_string())?;
    let path = dir.join(MANIFEST_FILE);
    std::fs::write(&path, manifest.to_json() + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {} ({} files, {} bytes)", path.display(), manifest.files.len(), manifest.files.iter().map(|f| f.bytes).sum::<u64>());
    Ok(())
}
