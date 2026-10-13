//! Explicit, synthetic local-model smoke check; never opens the microphone,
//! clipboard, settings, meeting library, or dictation history.
use sonojot_core::{acceleration, cleanup, domain::TextAcceleration};
use std::{path::PathBuf, sync::atomic::AtomicBool, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("Pass the models directory"));
    let preference = match args.next().as_deref() {
        Some("cpu") => TextAcceleration::Cpu,
        Some("auto") => TextAcceleration::Auto,
        _ => panic!("Choose cpu or auto explicitly"),
    };
    acceleration::set_preference(preference);
    let start = Instant::now();
    let warm = cleanup::warm_blocking(&root, "qwen3-4b-instruct-2507");
    let load_ms = start.elapsed().as_millis();
    let generation = Instant::now();
    let result = cleanup::generate_local(
        &root,
        "qwen3-4b-instruct-2507",
        "Follow the instruction exactly.",
        "Reply with READY only.",
        8,
        &AtomicBool::new(false),
    );
    let generation_ms = generation.elapsed().as_millis();
    let status = acceleration::inspect(&root);
    println!(
        "{}",
        serde_json::json!({
            "preference": preference, "elapsedMs": start.elapsed().as_millis(),
            "loadMs": load_ms, "loadError": warm.err(), "generationMs": generation_ms,
            "completed": result.is_ok(), "outputBytes": result.as_ref().map(|s| s.len()).ok(),
            "error": result.err(), "acceleration": status,
        })
    );
    cleanup::shutdown();
}
