//! Serves `sonojot-cli` from inside the running app.
//!
//! Requests call the same command functions as the window, through the same
//! runtime, so the GUI updates live and there is still one owner for capture,
//! models and the meeting library. See `cli_protocol` for the wire format.

use crate::{
    ai_commands, app::Runtime, cli_protocol::{Request, Response, MAX_MESSAGE_BYTES, PROTOCOL_VERSION},
    commands, download_consent, media_support, model_manager, permissions, setup_commands,
    system_audio_commands, updater,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// Every command the CLI can invoke, with its arguments.
pub const COMMANDS: &[(&str, &str)] = &[
    ("ping", ""),
    ("commands", ""),
    ("get_state", ""),
    ("get_snapshot", ""),
    ("save_settings", "settings"),
    ("get_diagnostics", ""),
    ("get_model_acceleration", ""),
    ("show_page", "page"),
    ("open_path", "kind"),
    ("start_dictation", ""),
    ("stop_dictation", ""),
    ("cancel_dictation", ""),
    ("stop_cleanup", ""),
    ("test_microphone", ""),
    ("set_screen_tab", "visible, expanded?, focus?, pinned?"),
    ("screen_tab_record", ""),
    ("screen_tab_meeting", ""),
    ("copy_last_dictation", ""),
    ("copy_text", "original, resultSessionId?"),
    ("copy_smart_original", "resultSessionId"),
    ("list_dictations", "limit"),
    ("count_dictations", ""),
    ("delete_dictation", "id, confirmed"),
    ("delete_all_dictations", "confirmed"),
    ("save_dictation_correction", "id, text, scope?"),
    ("copy_history", "id, original"),
    ("copy_history_pre_model", "id"),
    ("get_vocabulary", ""),
    ("save_vocabulary", "entries"),
    ("get_learning", ""),
    ("save_learning", "entries"),
    ("reset_learning", "confirmed"),
    ("learn_correction", "id, heard, written"),
    ("list_meetings", ""),
    ("get_meeting", "id"),
    ("start_meeting", "title, source"),
    ("stop_meeting", ""),
    ("import_media", "path"),
    ("join_meetings", "ids, title?"),
    ("transcribe_meeting", "id, model?"),
    ("cancel_meeting", "id"),
    ("save_meeting", "id, title, transcript, notes, cleanedTranscript?"),
    ("clean_meeting", "id"),
    ("set_meeting_speaker_labels", "id, enabled, confirmed"),
    ("meeting_speakers", "id"),
    ("rename_meeting_speaker", "id, from, to"),
    ("generate_notes", "id, preset, instructions"),
    ("export_meeting", "id, path, format, transcriptVersion?"),
    ("export_meeting_audio", "id, path"),
    ("delete_meeting", "id"),
    ("system_audio_status", ""),
    ("permission_status", "kind"),
    ("permission_assistant", "kind, action"),
    ("permission_reset", "kind"),
    ("get_setup", ""),
    ("get_hardware_profile", ""),
    ("apply_setup_profile", "profile"),
    ("complete_setup", ""),
    ("locate_models", "path"),
    ("locate_converter", "path"),
    ("install_media_support", ""),
    ("get_model_library", ""),
    ("start_model_install", "id"),
    ("cancel_model_install", "id"),
    ("remove_model", "id, confirmed"),
    ("get_download_requirements", "ids"),
    ("acknowledge_download_terms", "acknowledgments"),
    ("consent_install_asr_model", "id"),
    ("consent_install_text_model", "id"),
    ("get_ai_settings", ""),
    ("save_ai_settings", "settings"),
    ("connect_openrouter", ""),
    ("cancel_ai_connect", ""),
    ("open_openrouter_privacy", ""),
    ("disconnect_openrouter", ""),
    ("list_ai_models", ""),
    ("list_ai_catalog", ""),
    ("check_ai_model", "model"),
    ("suggest_vocabulary", ""),
    ("cancel_vocabulary_suggestions", ""),
    ("check_for_updates", ""),
    ("download_update", ""),
    ("restart_to_update", ""),
    ("dismiss_update_notice", ""),
    ("quit_app", ""),
];

/// Starts listening. A failure is logged and leaves the app running without the CLI.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = listen(app).await {
            crate::recognition_diagnostics::event("cli-server-stopped", &error.to_string(), std::process::id(), 0, 0, 0);
        }
    });
}

#[cfg(windows)]
async fn listen(app: AppHandle) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = crate::cli_protocol::endpoint();
    // The first instance claims the name, so another process cannot squat on it.
    let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
    loop {
        server.connect().await?;
        let next = ServerOptions::new().reject_remote_clients(true).create(&name)?;
        let client = std::mem::replace(&mut server, next);
        let app = app.clone();
        tauri::async_runtime::spawn(async move { serve(client, app).await });
    }
}

#[cfg(unix)]
async fn listen(app: AppHandle) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = crate::cli_protocol::endpoint();
    // Single-instance startup means a leftover socket belongs to a previous run.
    if std::fs::symlink_metadata(&path).is_ok_and(|m| { use std::os::unix::fs::FileTypeExt; m.file_type().is_socket() }) {
        std::fs::remove_file(&path)?;
    }
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (stream, _) = listener.accept().await?;
        let app = app.clone();
        tauri::async_runtime::spawn(async move { serve(stream, app).await });
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(stream: S, app: AppHandle) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut line = String::new();
    let response = match BufReader::new(reader.take(MAX_MESSAGE_BYTES)).read_line(&mut line).await {
        Err(error) => Response::from_result(Err(format!("Cannot read the request: {error}"))),
        Ok(_) => match serde_json::from_str::<Request>(&line) {
            Err(error) => Response::from_result(Err(format!("The request is not valid JSON: {error}"))),
            Ok(request) if request.version != PROTOCOL_VERSION => Response::from_result(Err(format!(
                "This SonoJot speaks CLI protocol {PROTOCOL_VERSION}, but the CLI sent {}. Use the sonojot-cli installed with the app.",
                request.version))),
            Ok(request) if request.detach => {
                if !COMMANDS.iter().any(|(name, _)| *name == request.command) {
                    Response::from_result(Err(unknown(&request.command)))
                } else {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = dispatch(app, &request.command, request.args).await;
                    });
                    Response::from_result(Ok(json!({ "started": true })))
                }
            }
            Ok(request) => Response::from_result(dispatch(app, &request.command, request.args).await),
        },
    };
    let mut reply = serde_json::to_vec(&response).unwrap_or_else(|_| br#"{"ok":false,"error":"Cannot encode the reply."}"#.to_vec());
    reply.push(b'\n');
    let _ = writer.write_all(&reply).await;
    let _ = writer.flush().await;
}

fn unknown(command: &str) -> String {
    format!("Unknown command `{command}`. Run `sonojot-cli commands` for the list.")
}

fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|error| format!("Argument `{name}`: {error}"))
}

fn out<T: Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result.and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string()))
}

async fn dispatch(app: AppHandle, command: &str, args: Value) -> Result<Value, String> {
    if app.try_state::<Arc<Runtime>>().is_none() {
        return Err("SonoJot is still starting. Try again in a moment.".into());
    }
    let a = &args;
    let s = || app.state::<Arc<Runtime>>();
    let h = || app.clone();
    match command {
        "ping" => Ok(json!({ "version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL_VERSION, "pid": std::process::id() })),
        "commands" => Ok(Value::Array(COMMANDS.iter().map(|(name, args)| json!({ "command": name, "args": args })).collect())),
        // The last published state, without get_snapshot's device and model
        // refresh, which can take minutes while large models are verified.
        "get_state" => out(s().snapshot()),
        "get_snapshot" => out(commands::get_snapshot(h(), s()).await),
        "save_settings" => out(commands::save_settings(h(), s(), arg(a, "settings")?).await),
        "get_diagnostics" => out(commands::get_diagnostics(s())),
        "get_model_acceleration" => out(commands::get_model_acceleration(s()).await),
        "show_page" => out(commands::show_page(h(), arg(a, "page")?).await),
        "open_path" => out(commands::open_path(s(), arg(a, "kind")?)),
        "start_dictation" => out(commands::start_dictation(h(), s()).await),
        "stop_dictation" => out(commands::stop_dictation(h(), s()).await),
        "cancel_dictation" => out(commands::cancel_dictation(h(), s()).await),
        "stop_cleanup" => out(commands::stop_cleanup(s()).await),
        "test_microphone" => out(commands::test_microphone(h(), s()).await),
        "set_screen_tab" => out(commands::set_screen_tab(h(), s(), arg(a, "visible")?, arg(a, "expanded")?, arg(a, "focus")?, arg(a, "pinned")?).await),
        "screen_tab_record" => out(commands::screen_tab_record(h(), s()).await),
        "screen_tab_meeting" => out(commands::screen_tab_meeting(h(), s()).await),
        "copy_last_dictation" => out(commands::copy_last_dictation(s()).await),
        "copy_text" => out(commands::copy_text(s(), arg(a, "original")?, arg(a, "resultSessionId")?).await),
        "copy_smart_original" => out(commands::copy_smart_original(s(), arg(a, "resultSessionId")?).await),
        "list_dictations" => out(commands::list_dictations(s(), arg::<Option<usize>>(a, "limit")?.unwrap_or(50))),
        "count_dictations" => out(commands::count_dictations(s())),
        "delete_dictation" => out(commands::delete_dictation(h(), s(), arg(a, "id")?, arg(a, "confirmed")?)),
        "delete_all_dictations" => out(commands::delete_all_dictations(h(), s(), arg(a, "confirmed")?)),
        "save_dictation_correction" => out(commands::save_dictation_correction(h(), s(), arg(a, "id")?, arg(a, "text")?, arg(a, "scope")?)),
        "copy_history" => out(commands::copy_history(s(), arg(a, "id")?, arg(a, "original")?).await),
        "copy_history_pre_model" => out(commands::copy_history_pre_model(s(), arg(a, "id")?).await),
        "get_vocabulary" => out(commands::get_vocabulary(s())),
        "save_vocabulary" => out(commands::save_vocabulary(s(), arg(a, "entries")?)),
        "get_learning" => out(commands::get_learning(s())),
        "save_learning" => out(commands::save_learning(s(), arg(a, "entries")?)),
        "reset_learning" => out(commands::reset_learning(s(), arg(a, "confirmed")?)),
        "learn_correction" => out(commands::learn_correction(s(), arg(a, "id")?, arg(a, "heard")?, arg(a, "written")?)),
        "list_meetings" => out(commands::list_meetings(s())),
        "get_meeting" => out(commands::get_meeting(s(), arg(a, "id")?)),
        "start_meeting" => out(commands::start_meeting(h(), s(), arg::<Option<String>>(a, "title")?.unwrap_or_default(),
            arg::<Option<String>>(a, "source")?.unwrap_or_else(|| "both".into())).await),
        "stop_meeting" => out(commands::stop_meeting(h(), s()).await),
        "import_media" => out(commands::import_media(h(), s(), arg(a, "path")?).await),
        "join_meetings" => out(commands::join_meetings(h(), s(), arg(a, "ids")?, arg(a, "title")?).await),
        "transcribe_meeting" => out(commands::transcribe_meeting(h(), s(), arg(a, "id")?, arg(a, "model")?).await),
        "cancel_meeting" => out(commands::cancel_meeting(h(), s(), arg(a, "id")?)),
        "save_meeting" => out(commands::save_meeting(h(), s(), arg(a, "id")?, arg(a, "title")?, arg(a, "transcript")?,
            arg(a, "notes")?, arg(a, "cleanedTranscript")?)),
        "clean_meeting" => out(commands::clean_meeting(h(), s(), arg(a, "id")?).await),
        "set_meeting_speaker_labels" => out(commands::set_meeting_speaker_labels(h(), s(), arg(a, "id")?, arg(a, "enabled")?,
            arg::<Option<bool>>(a, "confirmed")?.unwrap_or(false))),
        "meeting_speakers" => out(commands::meeting_speakers(s(), arg(a, "id")?)),
        "rename_meeting_speaker" => out(commands::rename_meeting_speaker(h(), s(), arg(a, "id")?, arg(a, "from")?, arg(a, "to")?)),
        "generate_notes" => out(commands::generate_notes(h(), s(), arg(a, "id")?, arg(a, "preset")?,
            arg::<Option<String>>(a, "instructions")?.unwrap_or_default()).await),
        "export_meeting" => out(commands::export_meeting(s(), arg(a, "id")?, arg(a, "path")?, arg(a, "format")?, arg(a, "transcriptVersion")?).await),
        "export_meeting_audio" => out(commands::export_meeting_audio(s(), arg(a, "id")?, arg(a, "path")?).await),
        "delete_meeting" => out(commands::delete_meeting(h(), s(), arg(a, "id")?)),
        "system_audio_status" => out(system_audio_commands::system_audio_status(h(), s()).await),
        "permission_status" => out(permissions::permission_status(h(), arg(a, "kind")?)),
        "permission_assistant" => out(permissions::permission_assistant(h(), arg(a, "kind")?, arg(a, "action")?).await),
        "permission_reset" => out(permissions::permission_reset(h(), arg(a, "kind")?).await),
        "get_setup" => out(setup_commands::get_setup(s()).await),
        "get_hardware_profile" => out(setup_commands::get_hardware_profile(s()).await),
        "apply_setup_profile" => out(setup_commands::apply_setup_profile(h(), s(), arg(a, "profile")?).await),
        "complete_setup" => out(setup_commands::complete_setup(s())),
        "locate_models" => out(setup_commands::locate_models(h(), s(), arg(a, "path")?).await),
        "locate_converter" => out(setup_commands::locate_converter(s(), arg(a, "path")?).await),
        "install_media_support" => out(media_support::install_media_support(s()).await),
        "get_model_library" => out(model_manager::get_model_library(s()).await),
        "start_model_install" => out(model_manager::start_model_install(s(), arg(a, "id")?).await),
        "cancel_model_install" => out(model_manager::cancel_model_install(s(), arg(a, "id")?).await),
        "remove_model" => out(model_manager::remove_model(s(), arg(a, "id")?, arg(a, "confirmed")?).await),
        "get_download_requirements" => out(download_consent::get_download_requirements(s(), arg(a, "ids")?).await),
        "acknowledge_download_terms" => out(download_consent::acknowledge_download_terms(h(), s(), arg(a, "acknowledgments")?).await),
        "consent_install_asr_model" => out(download_consent::consent_install_asr_model(h(), s(), arg(a, "id")?).await),
        "consent_install_text_model" => out(download_consent::consent_install_text_model(h(), s(), arg(a, "id")?).await),
        "get_ai_settings" => out(ai_commands::get_ai_settings(s())),
        "save_ai_settings" => out(ai_commands::save_ai_settings(s(), arg(a, "settings")?)),
        "connect_openrouter" => out(ai_commands::connect_openrouter(s()).await),
        "cancel_ai_connect" => { ai_commands::cancel_ai_connect(); Ok(Value::Null) }
        "open_openrouter_privacy" => out(ai_commands::open_openrouter_privacy().await),
        "disconnect_openrouter" => out(ai_commands::disconnect_openrouter(s())),
        "list_ai_models" => out(ai_commands::list_ai_models(s()).await),
        "list_ai_catalog" => out(ai_commands::list_ai_catalog(s()).await),
        "check_ai_model" => out(ai_commands::check_ai_model(arg(a, "model")?).await),
        "suggest_vocabulary" => out(ai_commands::suggest_vocabulary(s()).await),
        "cancel_vocabulary_suggestions" => { ai_commands::cancel_vocabulary_suggestions(); Ok(Value::Null) }
        "check_for_updates" => out(updater::check_for_updates(h(), s())),
        "download_update" => out(updater::download_update(h(), s())),
        "restart_to_update" => out(updater::restart_to_update(h(), s())),
        "dismiss_update_notice" => out(updater::dismiss_update_notice(h(), s())),
        "quit_app" => {
            crate::recognition_diagnostics::event("cli-quit-requested", "", std::process::id(), 0, 0, 0);
            crate::quit_app(h(), s().inner().clone());
            Ok(json!({ "quitting": true }))
        }
        _ => Err(unknown(command)),
    }
}
