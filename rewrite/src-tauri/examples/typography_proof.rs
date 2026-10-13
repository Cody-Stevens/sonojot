//! Isolated proof host using this app's Tauri/WebView2 dependencies and CSP.
//! No capture runtime, tray, global shortcut, production data or registration.
#[tauri::command]
fn proof_zoom(window: tauri::WebviewWindow, scale: f64) -> Result<(), String> {
    if ![1.0, 1.25, 1.5, 2.0].contains(&scale) {
        return Err("Unsupported proof scale".into());
    }
    window.set_zoom(scale).map_err(|error| error.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![proof_zoom])
        .setup(|app| {
            let data =
                std::env::current_dir()?.join("../../artifacts/typography-relief/webview-data");
            let surface = if std::env::var("SONOJOT_PROOF_SURFACE").as_deref() == Ok("material") {
                "src/tests/typography-fixture/index.html"
            } else {
                "src/tests/typography-relief/index.html"
            };
            tauri::WebviewWindowBuilder::new(
                app,
                "typography-proof",
                tauri::WebviewUrl::App(surface.into()),
            )
            .title("SonoJot — typography relief proof")
            .data_directory(data)
            .inner_size(1180.0, 1000.0)
            .min_inner_size(360.0, 540.0)
            .center()
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Typography proof host failed");
}
