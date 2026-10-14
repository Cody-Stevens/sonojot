pub mod acceleration;
mod permissions;
mod ai;
mod ai_catalog;
mod ai_commands;
mod app;
pub mod asr;
mod asr_filter;
pub mod audio;
mod audio_backlog;
pub mod cleanup;
mod commands;
mod download_consent;
mod diarization;
mod diarization_model;
pub mod diarization_process;
pub mod domain;
mod environment;
mod granite_model;
mod granite_speech;
pub mod granite_worker;
mod llama_asr;
mod media_support;
pub mod meeting_ai;
pub mod meeting_audio_export;
mod meeting_speech;
pub mod meetings;
mod model_manager;
mod model_licenses;
mod legal_links;
mod model_verification;
mod mac_speech;
#[cfg(target_os = "macos")]
mod mac_paste;
mod optional_asr;
mod optional_llm;
pub mod recognition;
mod recognition_diagnostics;
#[cfg(feature = "fidelity-eval")]
pub mod resource_manager;
#[cfg(not(feature = "fidelity-eval"))]
mod resource_manager;
mod whisper_catalog;
// GPU-ASR worker delivered gpu_asr.rs + asr.rs routing (concurrent land).
// Parent-owned declaration only; implementation stays GPU-worker owned.
mod gpu_asr;
// Compatibility alias for the resource coordinator, which was written against
// a top-level `cleanup_resource` path. The canonical implementation lives in
// `cleanup::resource` (cleanup-owned, single SERVER instance). This re-export
// keeps one instance; do NOT add a duplicate `mod cleanup_resource;`.
pub use crate::cleanup::resource as cleanup_resource;
pub mod platform;
mod profiles;
mod onboarding;
pub mod punctuation;
pub mod written_form;
mod punctuation_worker;
mod punctuation_model;
mod qwen_guard;
mod qwen_window;
pub mod recording;
pub mod setup;
mod setup_commands;
mod sherpa;
pub mod speech;
pub mod store;
pub mod structured_clean;
pub mod system_audio;
pub mod system_audio_commands;
mod menu_panel;
mod windows;
mod updater;

use app::Runtime;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

fn tray_icon() -> Result<tauri::image::Image<'static>, tauri::Error> {
    #[cfg(windows)]
    let light = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .and_then(|key| key.get_value::<u32, _>("SystemUsesLightTheme"))
        .unwrap_or(0)
        != 0;
    #[cfg(not(windows))]
    let light = true;
    let bytes: &[u8] = if light {
        include_bytes!("../../src/brand/marks/tray-black.png")
    } else {
        include_bytes!("../../src/brand/marks/tray-white.png")
    };
    tauri::image::Image::from_bytes(bytes)
}

pub fn run() {
    recognition::dispatch_worker();
    punctuation::dispatch_worker();
    diarization_process::dispatch_worker();
    granite_worker::dispatch_worker();
    recognition_diagnostics::initialize();
    let desktop_started = std::time::Instant::now();
    recognition_diagnostics::event("desktop-start", "", std::process::id(), 0, 0, 0);
    if std::env::args().any(|arg| arg == "--register-startup") {
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| platform::set_startup_enabled(true, &exe));
        std::process::exit(if result == Ok(true) { 0 } else { 1 });
    }
    std::env::set_var("MOONSHINE_ORT_SINGLE_THREAD", "1");
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = windows::show_settings(&app);
            });
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            onboarding::begin_onboarding,
            onboarding::confirm_onboarding_profile,
            onboarding::queue_onboarding_download,
            onboarding::finish_onboarding,
            onboarding::onboarding_microphone,
            onboarding::open_permission_settings,
            onboarding::onboarding_paste_permission,
            updater::check_for_updates,
            updater::download_update,
            updater::restart_to_update,
            updater::dismiss_update_notice,
            commands::get_model_acceleration,
            ai_commands::get_ai_settings,
            ai_commands::save_ai_settings,
            ai_commands::connect_openrouter,
            ai_commands::cancel_ai_connect,
            ai_commands::disconnect_openrouter,
            ai_commands::list_ai_models,
            ai_commands::list_ai_catalog,
            ai_commands::check_ai_model,
            ai_commands::suggest_vocabulary,
            ai_commands::cancel_vocabulary_suggestions,
            setup_commands::get_setup,
            setup_commands::get_hardware_profile,
            setup_commands::apply_setup_profile,
            setup_commands::locate_models,
            setup_commands::locate_converter,
            setup_commands::complete_setup,
            media_support::install_media_support,
            download_consent::consent_install_asr_model,
            // Retired: Setup deletes only via the guarded `model_manager::remove_model
            // { confirmed:true }` path, which re-reads current selections inside
            // the shared guard. The legacy unconfirmed uninstalls fail closed
            // and are no longer exposed to the UI.
            download_consent::consent_install_text_model,
            download_consent::get_download_requirements,
            download_consent::acknowledge_download_terms,
            download_consent::open_download_license,
            model_manager::get_model_library,
            model_licenses::open_model_license,
            legal_links::open_legal_link,
            model_manager::start_model_install,
            model_manager::cancel_model_install,
            model_manager::remove_model,
            commands::get_snapshot,
            commands::save_settings,
            commands::start_dictation,
            commands::test_microphone,
            commands::stop_dictation,
            commands::cancel_dictation,
            commands::stop_cleanup,
            commands::set_screen_tab,
            commands::screen_tab_record,
            commands::copy_last_dictation,
            commands::copy_smart_original,
            commands::copy_history_pre_model,
            commands::screen_tab_meeting,
            system_audio_commands::system_audio_status,
            commands::show_page,
            commands::copy_text,
            commands::list_meetings,
            commands::get_meeting,
            commands::start_meeting,
            commands::stop_meeting,
            commands::import_media,
            commands::transcribe_meeting,
            windows::drag_screen_tab,
            commands::cancel_meeting,
            commands::save_meeting,
            commands::clean_meeting,
            commands::set_meeting_speaker_labels,
            commands::delete_meeting,
            commands::generate_notes,
            commands::export_meeting,
            commands::export_meeting_audio,
            commands::get_vocabulary,
            commands::save_vocabulary,
            commands::list_dictations,
            commands::count_dictations,
            commands::delete_dictation,
            commands::delete_all_dictations,
            commands::save_dictation_correction,
            commands::get_learning,
            commands::save_learning,
            commands::reset_learning,
            commands::learn_correction,
            commands::copy_history,
            commands::open_path,
            commands::open_punctuation_model_page,
            commands::get_diagnostics,
            permissions::permission_status,
            permissions::permission_assistant,
            permissions::permission_drag
        ])
        .setup(move |app| {
            let runtime = Runtime::create(app.handle()).map_err(std::io::Error::other)?;
            // Settings lifecycle: keep the shared resource coordinator on the
            // current overlap/retention choices from startup. Dictation starts
            // re-apply it via speech::StartOptions; commands::save_settings
            // must also call resource_manager::configure (hook in result doc)
            // so later jobs see the new mode without a restart.
            if let Ok(snapshot) = runtime.snapshot() {
                resource_manager::configure(&snapshot.settings);
            }
            if let Err(error) = runtime.meetings.recover_interrupted() {
                runtime.report(app.handle(), error);
            }
            app.manage(runtime.clone());
            // macOS: remember the app in use before a menu bar click activates SonoJot.
            platform::observe_app_activations();
            meeting_ai::start_automatic_titles(&runtime, app.handle());
            updater::start_scheduler(&runtime, app.handle());
            let verification_runtime = Arc::downgrade(&runtime);
            let verification_app = app.handle().clone();
            model_verification::on_completion(move || {
                if let Some(runtime) = verification_runtime.upgrade() {
                    // Catalog checks only enqueue work. No snapshot/store lock
                    // is held across a hash, and completion reuses app-state.
                    let models = app::model_catalog(&runtime.models_root);
                    if let Ok(mut snapshot) = runtime.snapshot.lock() {
                        snapshot.models = models;
                    }
                    runtime.emit(&verification_app);
                }
            });
            let prewarm_runtime = runtime.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if !onboarding::needs_setup(&prewarm_runtime)
                    && prewarm_runtime.snapshot().is_ok_and(|s| onboarding::terms_accepted(&s.settings)) {
                    prewarm_runtime.prewarm_selected_models();
                }
            });
            let shortcut = runtime
                .snapshot()
                .map_err(std::io::Error::other)?
                .settings
                .hotkey;
            match app::register_shortcut(app.handle(), &shortcut) {
                Ok(()) => {
                    if let Ok(mut snapshot) = runtime.snapshot.lock() {
                        snapshot.capabilities.global_shortcut = true;
                    }
                }
                Err(error) => runtime.report(app.handle(), error),
            }
            let settings = MenuItem::with_id(app, "settings", "Open SonoJot", true, None::<&str>)?;
            let dictation = MenuItem::with_id(
                app,
                "dictation",
                "Start / stop dictation",
                true,
                None::<&str>,
            )?;
            let copy = MenuItem::with_id(
                app,
                "original",
                "Copy original dictation",
                true,
                None::<&str>,
            )?;
            let separator = PredefinedMenuItem::separator(app)?;
            let screen_tab = MenuItem::with_id(
                app,
                "screen-tab",
                if cfg!(target_os = "macos") {
                    "Dictation controls"
                } else {
                    "Show screen tab"
                },
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit SonoJot", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&settings, &screen_tab, &dictation, &copy, &separator, &quit],
            )?;
            // AppKit opens a status item's attached menu on the click itself.
            // The paper sheet needs that click, so the menu is shown only on right click.
            #[cfg(target_os = "macos")]
            let status_menu = menu.clone();
            TrayIconBuilder::with_id("eavesdropper")
                .icon(tray_icon()?)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("SonoJot · local dictation")
                .menu(&menu)
                .show_menu_on_left_click(cfg!(not(target_os = "macos")))
                .on_menu_event(|app, event| {
                    let runtime = app.state::<Arc<Runtime>>().inner().clone();
                    match event.id.as_ref() {
                        "screen-tab" => {
                            let app = app.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                                if let Err(error) = runtime.set_tab(&app, true, Some(true), true) {
                                    runtime.report(&app, error);
                                }
                            });
                        }
                        "settings" => {
                            let app = app.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                                if let Err(error) = windows::show_settings(&app) {
                                    runtime.report(&app, error);
                                }
                            });
                        }
                        "dictation" => {
                            let app = app.clone();
                            let action = runtime.snapshot().map(|s| s.state);
                            let generation = runtime
                                .command_generation
                                .load(std::sync::atomic::Ordering::Acquire);
                            let target = platform::capture_paste_target();
                            let ticket = if matches!(
                                action,
                                Ok(domain::AppState::Listening | domain::AppState::Loading)
                            ) {
                                runtime.stop_ticket().ok()
                            } else {
                                None
                            };
                            tauri::async_runtime::spawn_blocking(move || {
                                let result = match action {
                                    Ok(domain::AppState::Listening | domain::AppState::Loading) => {
                                        let Some(ticket) = ticket else { return };
                                        runtime.stop_at(&app, ticket)
                                    }
                                    Ok(
                                        domain::AppState::Finishing | domain::AppState::Cleaning,
                                    ) => return,
                                    _ => runtime.start_at_generation(&app, target, generation),
                                };
                                if let Err(error) = result {
                                    runtime.report(&app, error);
                                }
                            });
                        }
                        "original" => {
                            let app = app.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Ok(snapshot) = runtime.snapshot() {
                                    let result = if snapshot.recognition_completeness
                                        == Some(domain::RecognitionCompleteness::Incomplete)
                                    {
                                        Err("Open Dictation to review incomplete recognition before copying it.".into())
                                    } else {
                                        commands::copy_current_text(
                                            &runtime,
                                            true,
                                            snapshot.result_session_id,
                                        )
                                        .await
                                    };
                                    if let Err(error) = result {
                                        runtime.report(&app, error);
                                    }
                                }
                            });
                        }
                        "quit" => {
                            recognition_diagnostics::event("tray-quit-requested", "", std::process::id(), 0, 0, 0);
                            let app = app.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(error) = onboarding::close_early(&app, &runtime).await {
                                    runtime.report(&app, error);
                                    return;
                                }
                                runtime.engine.cancel();
                                runtime.meetings.cancel_all();
                                crate::ai_commands::cancel_optional_jobs();
                                let r = runtime.clone();
                                let a = app.clone();
                                let _ = tauri::async_runtime::spawn_blocking(move || {
                                    let _ = r.engine.shutdown(std::time::Duration::from_secs(5));
                                    if r.snapshot().is_ok_and(|s| s.meeting_recording) {
                                        let _ = commands::finish_recording(&a, &r);
                                    }
                                })
                                .await;
                                cleanup::shutdown();
                                resource_manager::release_all_for_exit();
                                app.exit(0);
                            });
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(move |tray, event| {
                    #[cfg(target_os = "macos")]
                    {
                        menu_panel::observe_tray(tray.app_handle(), &event);
                        if let TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Right,
                            button_state: tauri::tray::MouseButtonState::Down,
                            ..
                        } = &event
                        {
                            let app = tray.app_handle().clone();
                            let menu = status_menu.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                                if let Some(runtime) = app.try_state::<Arc<Runtime>>() {
                                    let runtime = runtime.inner().clone();
                                    let _ = runtime.set_tab(&app, false, Some(false), false);
                                }
                                if let Some(window) = menu_panel::menu_host(&app) {
                                    let _ = window.popup_menu(&menu);
                                }
                            });
                            return;
                        }
                        if let TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Left,
                            button_state: tauri::tray::MouseButtonState::Up,
                            ..
                        } = &event
                        {
                            if menu_panel::toggle_allowed() {
                                let app = tray.app_handle().clone();
                                tauri::async_runtime::spawn_blocking(move || {
                                    if let Some(runtime) = app.try_state::<Arc<Runtime>>() {
                                        let runtime = runtime.inner().clone();
                                        let open = runtime
                                            .snapshot()
                                            .map(|snapshot| snapshot.tab_visible)
                                            .unwrap_or(false);
                                        if let Err(error) =
                                            runtime.set_tab(&app, !open, Some(true), false)
                                        {
                                            runtime.report(&app, error);
                                        }
                                    }
                                });
                            }
                        }
                        return;
                    }
                    #[cfg(not(target_os = "macos"))]
                    if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                        let app = tray.app_handle().clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            let _ = windows::show_settings(&app);
                        });
                    }
                })
                .build(app)?;
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Some(tray) = handle.tray_by_id("eavesdropper") {
                        let _ = tray.set_menu(None::<Menu<tauri::Wry>>);
                    }
                });
            }
            runtime
                .refresh_tab(app.handle())
                .map_err(std::io::Error::other)?;
            runtime.emit(app.handle());
            let setup_app = app.handle().clone();
            let setup_runtime = runtime.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = onboarding::recover(&setup_app, &setup_runtime).await {
                    setup_runtime.report(&setup_app, error);
                }
                let result = if setup_commands::needs_first_setup(&setup_runtime) {
                    windows::show_settings_page(&setup_app, "setup")
                } else if !std::env::args().any(|arg| arg == "--startup") {
                    windows::show_settings(&setup_app)
                } else { Ok(()) };
                if let Err(error) = result { setup_runtime.report(&setup_app, error); }
            });
            recognition_diagnostics::event("desktop-windows-created", "", std::process::id(), 0, desktop_started.elapsed().as_millis() as u64, 0);
            let handle = app.handle().clone();
            let weak = Arc::downgrade(&runtime);
            tauri::async_runtime::spawn(async move {
                let mut shown_problem: Option<recording::SystemProblem> = None;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let Some(runtime) = weak.upgrade() else { break };
                    let recording = runtime.recorder.lock().ok().and_then(|r| {
                        r.as_ref().map(|(id, capture)| {
                            (
                                id.clone(),
                                capture.error(),
                                capture.level(),
                                capture.stem_levels(),
                                capture.source_status(),
                                capture.system_problem(),
                            )
                        })
                    });
                    if recording.is_none() {
                        shown_problem = None;
                    }
                    if let Some((_, Some(_), _, _, _, _)) = recording {
                        let r = runtime.clone();
                        let a = handle.clone();
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = commands::finish_recording(&a, &r) {
                                r.report(&a, error);
                            }
                        })
                        .await;
                    } else if let Some((id, None, level, stems, source_status, problem)) = recording {
                        if let Ok(mut snapshot) = runtime.snapshot.lock() {
                            if !snapshot.meeting_recording
                                || snapshot.active_meeting_id.as_deref() != Some(id.as_str())
                            {
                                continue;
                            }
                            snapshot.audio_level = level;
                            snapshot.meeting_source_status = source_status;
                            snapshot.meeting_system_audio =
                                problem.map(|problem| problem.as_str().to_owned());
                            snapshot.meeting_levels = stems.map(|(microphone, system)| {
                                domain::MeetingLevels {
                                    microphone: microphone.clamp(0.0, 1.0),
                                    system: system.clamp(0.0, 1.0),
                                }
                            });
                        }
                        // The other side going missing is never a quiet status
                        // line: reveal the screen tab or menu bar sheet at once.
                        if problem.is_some() && problem != shown_problem {
                            let r = runtime.clone();
                            let a = handle.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                                let _ = r.apply_tab(&a, true, Some(true), false, true);
                            });
                        }
                        shown_problem = problem;
                        runtime.emit(&handle);
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::Focused(true) = event {
                    updater::window_focused(window.app_handle());
                }
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let window = window.clone();
                    let app = window.app_handle().clone();
                    let runtime = app.state::<Arc<Runtime>>().inner().clone();
                    tauri::async_runtime::spawn(async move {
                        match onboarding::close_early(&app, &runtime).await {
                            Ok(()) => { let _ = window.destroy(); }
                            Err(error) => runtime.report(&app, error),
                        }
                    });
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("SonoJot could not start");
    application.run(|_app, event| {
        #[cfg(target_os = "macos")]
        if matches!(event, tauri::RunEvent::Reopen { .. }) {
            let app = _app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = windows::show_settings(&app);
            });
        }
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            } else {
                // Includes native Quit paths, not only our tray item.
                static SETUP_EXIT_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !SETUP_EXIT_READY.load(std::sync::atomic::Ordering::Acquire) {
                    api.prevent_exit();
                    let app = _app.clone();
                    let runtime = app.state::<Arc<Runtime>>().inner().clone();
                    tauri::async_runtime::spawn(async move {
                        match onboarding::close_early(&app, &runtime).await {
                            Ok(()) => {
                                SETUP_EXIT_READY.store(true, std::sync::atomic::Ordering::Release);
                                app.exit(code.unwrap_or(0));
                            }
                            Err(error) => runtime.report(&app, error),
                        }
                    });
                }
            }
        }
    });
}
