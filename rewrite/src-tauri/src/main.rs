#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// `dev` comes from tauri-build and reflects the dependency's actual mode,
// including the custom-protocol feature enabled directly by the Tauri CLI.
#[cfg(all(not(debug_assertions), dev))]
compile_error!("Release apps must embed the frontend. Build with `npm run tauri build` or enable `--features custom-protocol`.");

fn main() {
    #[cfg(target_os = "linux")]
    {
        if matches!(std::env::args().nth(1).as_deref(), Some("--version" | "-V")) {
            println!("SonoJot {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        // Before single-instance handling or any app-data/config access.
        if std::env::args().nth(1).as_deref() == Some("--cli") {
            use std::os::unix::process::CommandExt;
            let cli = std::env::current_exe().expect("Cannot locate the bundled CLI")
                .with_file_name("sonojot-cli");
            let error = std::process::Command::new(cli).args(std::env::args_os().skip(2)).exec();
            eprintln!("Cannot start sonojot-cli: {error}");
            std::process::exit(1);
        }
    }
    #[cfg(feature = "synctest")]
    if std::env::args().any(|arg| arg == "--synctest-verify") {
        match sonojot_core::flavor::verify() {
            Ok(value) => println!("{value}"),
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    sonojot_core::granite_worker::dispatch_worker();
    sonojot_core::recognition::dispatch_worker();
    sonojot_core::diarization_process::dispatch_worker();
    sonojot_core::run()
}
