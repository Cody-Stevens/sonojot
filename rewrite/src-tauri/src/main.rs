#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// `dev` comes from tauri-build and reflects the dependency's actual mode,
// including the custom-protocol feature enabled directly by the Tauri CLI.
#[cfg(all(not(debug_assertions), dev))]
compile_error!("Release apps must embed the frontend. Build with `npm run tauri build` or enable `--features custom-protocol`.");

fn main() {
    sonojot_core::granite_worker::dispatch_worker();
    sonojot_core::recognition::dispatch_worker();
    sonojot_core::diarization_process::dispatch_worker();
    sonojot_core::run()
}
