//! `sonojot-cli`: drive SonoJot from a terminal, a script or an agent.
//!
//! Every command goes to the running app over a local channel, so the CLI and
//! the window share one library, one set of models and one recorder. If the
//! app is not running, the CLI starts it in the background first.

use serde_json::{json, Value};
use sonojot_core::cli_protocol::{call, CallError, Request, PROTOCOL_VERSION};
use std::{
    process::ExitCode,
    time::{Duration, Instant},
};

const EXIT_FAILED: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_NOT_RUNNING: u8 = 3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut options = Options::default();
    let mut words = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--detach" => options.detach = true,
            "--no-launch" => options.no_launch = true,
            "--confirm" | "--yes" => options.confirm = true,
            "--cleaned" => options.cleaned = true,
            "--no-diarize" => options.no_diarize = true,
            "--latest-attempt" => options.latest_attempt = true,
            "--key-stdin" => options.key_stdin = true,
            "--stdin" => options.sync_stdin = true,
            "--no-browser" => options.no_browser = true,
            "--rekey" => options.rekey = Some(true),
            "--sign-out-only" => options.rekey = Some(false),
            "-h" | "--help" => words.insert(0, "help".into()),
            "--version" | "-V" => words.insert(0, "version".into()),
            flag if flag.starts_with("--") && flag.len() > 2 => {
                let (name, value) = match flag[2..].split_once('=') {
                    Some((name, value)) => (name.to_owned(), value.to_owned()),
                    None => match iter.next() {
                        Some(value) => (flag[2..].to_owned(), value),
                        None => return usage(&format!("{flag} needs a value.")),
                    },
                };
                options.values.insert(name, value);
            }
            _ => words.push(arg),
        }
    }
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match run(&words, &options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => usage(&message),
        Err(Failure::NotRunning(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(EXIT_NOT_RUNNING)
        }
        Err(Failure::Failed(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(EXIT_FAILED)
        }
    }
}

#[derive(Default)]
struct Options {
    detach: bool,
    no_launch: bool,
    confirm: bool,
    cleaned: bool,
    no_diarize: bool,
    latest_attempt: bool,
    key_stdin: bool,
    sync_stdin: bool,
    no_browser: bool,
    rekey: Option<bool>,
    values: std::collections::HashMap<String, String>,
}

impl Options {
    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

enum Failure {
    Usage(String),
    NotRunning(String),
    Failed(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Failed(message)
    }
}

type Outcome<T = ()> = Result<T, Failure>;

fn usage(message: &str) -> ExitCode {
    eprintln!("error: {message}\nRun `sonojot-cli help` for usage.");
    ExitCode::from(EXIT_USAGE)
}

fn run(words: &[&str], options: &Options) -> Outcome {
    if options.key_stdin && words != ["invoke", "connect_openrouter_key"] {
        return Err(Failure::Usage(
            "Use --key-stdin with `invoke connect_openrouter_key`, without JSON arguments.".into(),
        ));
    }
    let client = Client { options };
    match words {
        ["synctest-verify"] if sonojot_core::flavor::SYNC_TEST => {
            println!("{}", sonojot_core::flavor::verify()?);
            Ok(())
        }
        [] | ["help"] => {
            print!("{}", help());
            Ok(())
        }
        ["guide"] | ["help", "agents"] | ["agents"] => {
            print!("{}", guide());
            Ok(())
        }
        ["version"] => {
            println!(
                "sonojot-cli {} (protocol {PROTOCOL_VERSION})",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        ["mcp"] => mcp_stub(),
        ["agents", rest @ ..] => agents(rest),
        ["sync", rest @ ..] => sync_commands(&client, rest),
        ["ping"] => client.print("ping", json!({})),
        ["commands"] => client.print("commands", json!({})),
        ["invoke", "connect_openrouter_key"] if options.key_stdin => {
            use std::io::IsTerminal;
            let stdin = std::io::stdin();
            if stdin.is_terminal() {
                return Err(Failure::Usage(
                    "Pipe the OpenRouter key into standard input.".into(),
                ));
            }
            client.print(
                "connect_openrouter_key",
                json!({ "key": read_key_stdin(stdin.lock())? }),
            )
        }
        ["invoke", command] => client.print(command, json!({})),
        ["invoke", command, args] => {
            let args: Value = serde_json::from_str(args)
                .map_err(|e| Failure::Usage(format!("Arguments must be a JSON object: {e}")))?;
            if *command == "sync_command"
                && matches!(
                    args["operation"].as_str(),
                    Some("recovery_use" | "callback")
                )
            {
                return Err(Failure::Usage("Use sync recovery use --stdin or sync callback --stdin so codes stay out of arguments.".into()));
            }
            #[cfg(feature = "synctest")]
            if *command == "sync_command" && args["operation"] == "test_signin" {
                return Err(Failure::Usage("Use sync test-sign-in <subject> --stdin.".into()));
            }
            client.print(command, args)
        }
        ["status"] => {
            let snapshot = client.call("get_state", json!({}))?;
            let settings = &snapshot["settings"];
            emit(&json!({
                "version": snapshot["version"],
                "state": snapshot["state"],
                "meetingRecording": snapshot["meetingRecording"],
                "activeMeetingId": snapshot["activeMeetingId"],
                "error": snapshot["error"],
                "dictationModel": settings["asrModel"],
                "meetingModel": if settings["meetingAsrModel"].as_str().is_some_and(|m| !m.is_empty()) { &settings["meetingAsrModel"] } else { &settings["asrModel"] },
                "meetingSpeakerLabels": settings["meetingSpeakerLabels"],
                "notesModel": settings["notesModel"],
                "cli": std::env::current_exe().ok().map(|p| p.display().to_string()),
            }));
            Ok(())
        }
        ["snapshot"] => client.print("get_snapshot", json!({})),
        ["diagnostics"] => client.print("get_diagnostics", json!({})),
        ["quit"] => {
            let options = Options {
                no_launch: true,
                ..Default::default()
            };
            Client { options: &options }.print("quit_app", json!({}))
        }
        ["open"] => client.print("show_page", json!({ "page": "dictation" })),
        ["open", page] => client.print("show_page", json!({ "page": page })),
        ["settings", rest @ ..] => settings(&client, rest),
        ["dictation", rest @ ..] => dictation(&client, rest),
        ["meetings", rest @ ..] | ["meeting", rest @ ..] => meetings(&client, rest),
        ["vocabulary"] | ["vocabulary", "list"] => client.print("get_vocabulary", json!({})),
        ["models"] | ["models", "list"] => client.print("get_model_library", json!({})),
        ["ai"] | ["ai", "settings"] => client.print("get_ai_settings", json!({})),
        ["update", rest @ ..] | ["updates", rest @ ..] => update(&client, rest),
        _ => Err(Failure::Usage(format!(
            "Unknown command: {}",
            words.join(" ")
        ))),
    }
}

fn sync_commands(client: &Client<'_>, words: &[&str]) -> Outcome {
    let o = client.options;
    let send = |operation: &str, args: Value| {
        client.print("sync_command", json!({"operation":operation,"args":args}))
    };
    let confirm = || {
        if o.confirm {
            Ok(())
        } else {
            Err(Failure::Usage("This action needs --confirm.".into()))
        }
    };
    let file = || {
        o.values
            .get("file")
            .cloned()
            .ok_or_else(|| Failure::Usage("Choose --file <path>.".into()))
    };
    match words {
        [] | ["status"] => send("status", json!({})),
        #[cfg(all(target_os = "macos", feature = "synctest"))]
        ["test-credentials"] => send("test_credentials", json!({})),
        #[cfg(all(target_os = "macos", feature = "synctest"))]
        ["test-credentials", phase] => send("test_credentials", json!({"phase":phase})),
        #[cfg(feature = "synctest")]
        ["test-offline", mode] if matches!(*mode, "on" | "off") => {
            send("test_offline", json!({"offline": *mode == "on"}))
        }
        #[cfg(feature = "synctest")]
        ["test-sign-in", subject] => {
            use std::io::IsTerminal;
            let stdin = std::io::stdin();
            if !o.sync_stdin || stdin.is_terminal() {
                return Err(Failure::Usage("Pipe the test token through standard input with --stdin.".into()));
            }
            let value = zeroize::Zeroizing::new(read_key_stdin(stdin.lock())?);
            send("test_signin", json!({"subject":subject,"token":*value}))
        }
        ["signin"] => send(
            "signin",
            json!({"provider":o.values.get("provider").map(String::as_str).unwrap_or("authkit"),"server":o.values.get("server"),"noBrowser":o.no_browser}),
        ),
        ["status", "remote"] => send("remote_status",json!({})),
        ["now"] => send("now", json!({})),
        ["signout"] => send("signout", json!({"noBrowser":o.no_browser})),
        ["reset", "request"] => { confirm()?; send("reset_request",json!({"confirmed":true})) }
        ["reset", "cancel"] => send("reset_cancel",json!({})),
        ["journal", "acknowledge"] => { confirm()?; send("journal_ack",json!({"confirmed":true})) }
        ["setup"] | ["reset"] => {
            confirm()?;
            send(words[0], json!({"confirmed":true}))
        }
        ["link", provider] => send(
            "signin",
            json!({"provider":provider,"link":true,"noBrowser":o.no_browser}),
        ),
        ["conflicts"] => send("conflicts", json!({})),
        ["use-local", id] => {
            confirm()?;
            send("use_local", json!({"id":id,"confirmed":true}))
        }
        ["devices", "list"] => send("devices", json!({})),
        ["devices", "remove", device] => {
            confirm()?;
            if o.rekey == Some(false) {
                eprintln!("Sign out only: this device keeps its keys and can regain access by signing in again.");
            }
            send(
                "remove",
                json!({"device":device,"rekey":o.rekey,"confirmed":true}),
            )
        }
        ["devices", "add"] => send("pair_start", json!({})),
        ["devices", "requests"] => send("pair_list", json!({})),
        ["devices", step, id] if ["accept", "poll", "deny"].contains(step) => {
            send(&format!("pair_{step}"), json!({"id":id}))
        }
        ["devices", step, id] if ["approve", "confirm"].contains(step) => {
            confirm()?;
            send(&format!("pair_{step}"), json!({"id":id,"confirmed":true}))
        }
        ["recovery", "show"] => {
            confirm()?;
            send("recovery_show", json!({"confirmed":true}))
        }
        ["recovery", "new"] => {
            confirm()?;
            let path = file()?;
            client.call(
                "sync_command",
                json!({"operation":"recovery_new","args":{"confirmed":true}}),
            )?;
            send("recovery_download", json!({"path":path,"confirmed":true}))
        }
        ["recovery", "saved"] => {
            confirm()?;
            send("recovery_ack", json!({"confirmed":true}))
        }
        ["recovery", op] if ["copy", "print"].contains(op) => {
            confirm()?;
            send(&format!("recovery_{op}"), json!({"confirmed":true}))
        }
        ["recovery", "download"] => {
            confirm()?;
            send(
                "recovery_download",
                json!({"path":file()?,"confirmed":true}),
            )
        }
        ["recovery", "use"] | ["callback"] => {
            if !o.sync_stdin {
                return Err(Failure::Usage(
                    "Use --stdin and pipe the code or callback URL through standard input.".into(),
                ));
            }
            use std::io::IsTerminal;
            let stdin = std::io::stdin();
            if stdin.is_terminal() {
                return Err(Failure::Usage(
                    "Pipe the value through standard input.".into(),
                ));
            }
            let mut value = read_key_stdin(stdin.lock())?;
            let result = if words[0] == "callback" {
                send("callback", json!({"url":value}))
            } else {
                send("recovery_use", json!({"code":value}))
            };
            use zeroize::Zeroize;
            value.zeroize();
            result
        }
        ["account", "export"] => send("account_export_file", json!({"path":file()?})),
        ["account", "delete"] => {
            confirm()?;
            send("account_delete", json!({"confirmed":true}))
        }
        _ => Err(Failure::Usage(
            "Use sync signin|status|now|signout|setup|devices|recovery|account. See docs/CLI.md."
                .into(),
        )),
    }
}

fn read_key_stdin(input: impl std::io::Read) -> Outcome<String> {
    use std::io::Read;
    let mut value = String::new();
    input
        .take(4099)
        .read_to_string(&mut value)
        .map_err(|_| Failure::Usage("Could not read the key from standard input.".into()))?;
    let key = value.trim();
    if value.len() > 4098
        || key.is_empty()
        || key.len() > 4096
        || key.chars().any(char::is_whitespace)
    {
        return Err(Failure::Usage(
            "Pipe one key of at most 4,096 bytes into standard input.".into(),
        ));
    }
    Ok(key.to_owned())
}

#[cfg(test)]
mod key_stdin_tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "synctest"))]
    fn default_build_has_no_test_signin_command() {
        let options = Options { no_launch: true, ..Options::default() };
        assert!(matches!(run(&["sync", "test-sign-in", "synctest-fixture"], &options), Err(Failure::Usage(_))));
        assert!(matches!(run(&["sync", "test-offline", "on"], &options), Err(Failure::Usage(_))));
        assert!(matches!(run(&["sync", "test-credentials"], &options), Err(Failure::Usage(_))));
    }

    #[test]
    fn reads_a_piped_key_without_a_command_line_argument() {
        assert!(
            matches!(read_key_stdin(&b"sk-test-placeholder\r\n"[..]), Ok(key) if key == "sk-test-placeholder")
        );
        assert!(matches!(read_key_stdin(&b"\n"[..]), Err(Failure::Usage(_))));
        assert!(matches!(
            read_key_stdin(&b"one\ntwo"[..]),
            Err(Failure::Usage(_))
        ));
        assert!(matches!(
            read_key_stdin(&vec![b'x'; 5000][..]),
            Err(Failure::Usage(_))
        ));
    }

    #[test]
    fn stdin_errors_and_invalid_flag_usage_do_not_echo_input() {
        let options = Options {
            key_stdin: true,
            ..Options::default()
        };
        let result = run(
            &["invoke", "connect_openrouter_key", "secret-placeholder"],
            &options,
        );
        assert!(
            matches!(result, Err(Failure::Usage(message)) if !message.contains("secret-placeholder"))
        );
        assert!(
            matches!(read_key_stdin(&[0xff][..]), Err(Failure::Usage(message)) if message == "Could not read the key from standard input.")
        );
    }
}

/// PATH and agent MCP registration. Runs here, without the app.
fn agents(words: &[&str]) -> Outcome {
    use sonojot_core::agent_integration::{ensure, forget, run, Mode};
    let cli = std::env::current_exe().map_err(|e| e.to_string())?;
    let data = sonojot_core::cli_protocol::data_dir();
    let items = match words {
        [] | ["status"] => run(Mode::Check, &cli, &data),
        ["install"] => run(Mode::Install, &cli, &data),
        ["remove"] => run(Mode::Remove, &cli, &data),
        // The installer: respects an earlier `agents remove`.
        ["ensure"] => ensure(&cli, &data).unwrap_or_default(),
        // The uninstaller: removes, and lets a later install register again.
        ["uninstall"] => {
            let items = run(Mode::Remove, &cli, &data);
            forget(&data);
            items
        }
        _ => return Err(Failure::Usage("Use `agents status|install|remove`.".into())),
    };
    emit(&serde_json::to_value(&items).map_err(|e| e.to_string())?);
    if items.iter().any(|item| item.state == "failed") {
        return Err(Failure::Failed(
            "Some targets could not be updated; see `detail`.".into(),
        ));
    }
    Ok(())
}

fn settings(client: &Client, words: &[&str]) -> Outcome {
    match words {
        [] | ["get"] => {
            emit(&client.call("get_state", json!({}))?["settings"]);
            Ok(())
        }
        ["get", key] => {
            emit(&client.call("get_state", json!({}))?["settings"][key]);
            Ok(())
        }
        ["set", pairs @ ..] if !pairs.is_empty() => {
            let mut settings = client.call("get_state", json!({}))?["settings"].take();
            let object = settings
                .as_object_mut()
                .ok_or("SonoJot returned no settings.".to_string())?;
            for pair in pairs {
                let (key, value) = pair
                    .split_once('=')
                    .ok_or_else(|| Failure::Usage(format!("Use key=value, not `{pair}`.")))?;
                if !object.contains_key(key) {
                    return Err(Failure::Usage(format!(
                        "Unknown setting `{key}`. Run `sonojot-cli settings get` to list them."
                    )));
                }
                object.insert(
                    key.into(),
                    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.into())),
                );
            }
            let saved = client.call("save_settings", json!({ "settings": settings }))?;
            emit(&saved["settings"]);
            Ok(())
        }
        _ => Err(Failure::Usage(
            "Use `settings get [key]` or `settings set key=value ...`.".into(),
        )),
    }
}

fn dictation(client: &Client, words: &[&str]) -> Outcome {
    match words {
        ["start"] => client.print_state("start_dictation"),
        ["stop"] => client.print_state("stop_dictation"),
        ["cancel"] => client.print_state("cancel_dictation"),
        ["history"] => {
            let limit: usize = client
                .options
                .value("limit")
                .map(str::parse)
                .transpose()
                .map_err(|_| Failure::Usage("--limit must be a number.".into()))?
                .unwrap_or(20);
            client.print("list_dictations", json!({ "limit": limit }))
        }
        ["copy-last"] => client.print("copy_last_dictation", json!({})),
        _ => Err(Failure::Usage(
            "Use `dictation start|stop|cancel|history|copy-last`.".into(),
        )),
    }
}

fn meetings(client: &Client, words: &[&str]) -> Outcome {
    let o = client.options;
    if o.latest_attempt && o.cleaned {
        return Err(Failure::Usage("Latest attempt text has no aligned cleaned copy. Use --latest-attempt without --cleaned.".into()));
    }
    match words {
        [] | ["list"] => client.print("list_meetings", json!({})),
        ["show", id] => client.print("get_meeting", json!({ "id": client.meeting(id)? })),
        ["transcript", id] => {
            match o.value("format").unwrap_or("text") {
                "json" => return client.print("get_timed_transcript", json!({ "id": client.meeting(id)?, "latestAttempt": o.latest_attempt })),
                "text" | "txt" => {},
                other => return Err(Failure::Usage(format!("Unknown transcript format `{other}`. Use text or json."))),
            }
            if o.latest_attempt { return Err(Failure::Usage("--latest-attempt requires --format json for transcript output.".into())); }
            let meeting = client.call("get_meeting", json!({ "id": client.meeting(id)? }))?;
            let text = if o.cleaned { &meeting["cleanedTranscript"] } else { &meeting["transcript"] };
            if o.cleaned && text.is_null() {
                return Err(Failure::Failed("Clean this meeting before requesting its cleaned transcript.".into()));
            }
            println!("{}", text.as_str().unwrap_or_default());
            Ok(())
        }
        ["notes", id] => {
            println!("{}", client.call("get_meeting", json!({ "id": client.meeting(id)? }))?["notes"].as_str().unwrap_or_default());
            Ok(())
        }
        ["record", "start"] => client.print_meeting("start_meeting", json!({
            "title": o.value("title").unwrap_or(""), "source": o.value("source").unwrap_or("both") })),
        ["record", "stop"] => client.print_meeting("stop_meeting", json!({})),
        ["import", path] => client.print_meeting("import_media", json!({ "path": absolute(path)?, "speaker": o.value("speaker") })),
        ["join", ids @ ..] if ids.len() >= 2 => {
            let ids = ids.iter().map(|id| client.meeting(id)).collect::<Outcome<Vec<_>>>()?;
            client.print_meeting("join_meetings", json!({ "ids": ids, "title": o.value("title") }))
        }
        ["transcribe", id] => {
            let id = client.meeting(id)?;
            client.print_meeting("transcribe_meeting", json!({ "id": id, "model": o.value("model"),
                "speaker": o.value("speaker"), "noDiarize": o.no_diarize }))
        }
        ["wait", id] => wait(client, &client.meeting(id)?),
        ["cancel", id] => client.print("cancel_meeting", json!({ "id": client.meeting(id)? })),
        ["rename", id, title] => {
            let meeting = client.call("get_meeting", json!({ "id": client.meeting(id)? }))?;
            client.print_meeting("save_meeting", json!({ "id": meeting["id"], "title": title,
                "transcript": meeting["transcript"], "notes": meeting["notes"] }))
        }
        ["speakers", id] => client.print("meeting_speakers", json!({ "id": client.meeting(id)? })),
        ["rename-speaker", id, from, to] => client.print_meeting("rename_meeting_speaker",
            json!({ "id": client.meeting(id)?, "from": from, "to": to })),
        ["labels", id, state @ ("on" | "off")] => client.print_meeting("set_meeting_speaker_labels",
            json!({ "id": client.meeting(id)?, "enabled": *state == "on", "confirmed": o.confirm })),
        ["clean", id] => client.print_meeting("clean_meeting", json!({ "id": client.meeting(id)? })),
        ["generate-notes", id] => {
            let preset = o.value("preset").unwrap_or("summary");
            let preset = match preset {
                "formatted-transcript" | "formattedTranscript" => "formattedTranscript",
                "summary" | "tasks" | "speakers" | "brief" | "custom" => preset,
                _ => return Err(Failure::Usage("--preset is summary, tasks, speakers, brief, formatted-transcript or custom.".into())),
            };
            client.print_meeting("generate_notes", json!({ "id": client.meeting(id)?, "preset": preset,
                "instructions": o.value("instructions").unwrap_or("") }))
        }
        ["set-notes", id] => {
            let notes = text_input(o)?;
            let meeting = client.call("get_meeting", json!({ "id": client.meeting(id)? }))?;
            client.print_meeting("save_meeting", json!({ "id": meeting["id"], "title": meeting["title"],
                "transcript": meeting["transcript"], "notes": notes }))
        }
        ["set-transcript", id] => {
            let transcript = text_input(o)?;
            let meeting = client.call("get_meeting", json!({ "id": client.meeting(id)? }))?;
            client.print_meeting("save_meeting", json!({ "id": meeting["id"], "title": meeting["title"],
                "transcript": transcript, "notes": meeting["notes"] }))
        }
        ["export", id, path] => {
            let path = absolute(path)?;
            let format = match o.value("format").map(str::to_owned).unwrap_or_else(|| extension(&path)).as_str() {
                "md" | "markdown" => "markdown",
                "txt" | "text" => "text",
                "html" | "htm" => "html",
                "pdf" => "pdf",
                "wav" | "audio" => "wav",
                "json" => "json",
                "srt" => "srt",
                "vtt" => "vtt",
                other => return Err(Failure::Usage(format!("Unknown export format `{other}`. Use md, txt, html, pdf, wav, json, srt or vtt."))),
            };
            if o.latest_attempt && !matches!(format, "json" | "srt" | "vtt") {
                return Err(Failure::Usage("--latest-attempt supports JSON, SRT or VTT.".into()));
            }
            client.call("export_meeting", json!({ "id": client.meeting(id)?, "path": path, "format": format,
                "transcriptVersion": if o.latest_attempt { "latestAttempt" } else if o.cleaned { "cleaned" } else { "recognized" } }))?;
            emit(&json!({ "exported": path, "format": format }));
            Ok(())
        }
        ["export-audio", id, path] => {
            let path = absolute(path)?;
            client.call("export_meeting_audio", json!({ "id": client.meeting(id)?, "path": path }))?;
            emit(&json!({ "exported": path }));
            Ok(())
        }
        ["delete", id] => {
            if !o.confirm {
                return Err(Failure::Usage("Deleting a meeting removes its audio and transcript. Add --confirm.".into()));
            }
            client.print("delete_meeting", json!({ "id": client.meeting(id)? }))
        }
        _ => Err(Failure::Usage(format!("Unknown meetings command: {}", words.join(" ")))),
    }
}

/// Updates go through the running app, which checks each package's signature
/// before it installs it. Installing restarts SonoJot, so it needs --confirm.
fn update(client: &Client, words: &[&str]) -> Outcome {
    match words {
        [] | ["status"] => {
            emit(&update_summary(&client.call("get_state", json!({}))?));
            Ok(())
        }
        ["check"] => {
            emit(&update_summary(&check_updates(client, false)?));
            Ok(())
        }
        ["install"] => install_update(client, false),
        ["repair"] => install_update(client, true),
        _ => Err(Failure::Usage(
            "Use `update status|check|install|repair`.".into(),
        )),
    }
}

fn update_state(state: &Value) -> &str {
    state["update"]["status"]["state"]
        .as_str()
        .unwrap_or_default()
}

fn update_summary(state: &Value) -> Value {
    let update = &state["update"];
    let cli = installed_cli_version();
    let app = state["version"].as_str().unwrap_or_default();
    // The app reports this after its next check; apps before 0.28.9 never do.
    let problem = match (update["installProblem"].as_str(), cli.as_deref()) {
        (Some(problem), _) => json!(problem),
        (None, Some(cli)) if !app.is_empty() && cli != app => json!(format!(
            "sonojot-cli is {cli} but SonoJot is {app}: the last update did not finish. \
Run `sonojot-cli update repair --confirm`."
        )),
        (None, None) if cfg!(windows) => json!("sonojot-cli is missing from the SonoJot folder."),
        _ => Value::Null,
    };
    json!({
        "currentVersion": app,
        "status": update["status"],
        "installing": update["installing"],
        "restartBlockedReason": update["restartBlockedReason"],
        "installProblem": problem,
        "installedCli": cli,
    })
}

/// The version of the `sonojot-cli` installed beside this one. During an
/// update this process may be a renamed older copy, so ask the file itself.
fn installed_cli_version() -> Option<String> {
    let cli = sonojot_core::agent_integration::cli_beside(&std::env::current_exe().ok()?);
    let output = std::process::Command::new(cli)
        .arg("version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
}

/// Asks the app to check now and waits for the answer. `reinstall` also
/// accepts the installed version, which repairs an incomplete update.
fn check_updates(client: &Client, reinstall: bool) -> Outcome<Value> {
    let mut state = client.call("check_for_updates", json!({ "reinstall": reinstall }))?;
    let started = Instant::now();
    while update_state(&state) == "checking" {
        if started.elapsed() > Duration::from_secs(120) {
            return Err(Failure::Failed(
                "The update check did not finish within 2 minutes.".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
        state = client.call("get_state", json!({}))?;
    }
    Ok(state)
}

fn install_update(client: &Client, repair: bool) -> Outcome {
    if !client.options.confirm {
        return Err(Failure::Usage(
            "Installing an update restarts SonoJot. Add --confirm.".into(),
        ));
    }
    let mut state = client.call("get_state", json!({}))?;
    // An offer already in progress is used as it is. A fresh check repairs on
    // its own when the app finds the installation incomplete.
    if !matches!(update_state(&state), "available" | "downloading" | "ready") {
        state = check_updates(client, repair)?;
    }
    match update_state(&state) {
        "idle" if repair => {
            return Err(Failure::Failed("No release was found to reinstall.".into()))
        }
        "idle" => {
            emit(&json!({ "upToDate": true, "currentVersion": state["version"] }));
            return Ok(());
        }
        "error" => {
            return Err(Failure::Failed(
                state["update"]["status"]["message"]
                    .as_str()
                    .unwrap_or("The update failed.")
                    .into(),
            ))
        }
        _ => {}
    }
    let version = state["update"]["status"]["version"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if update_state(&state) == "available" {
        state = client.call("download_update", json!({}))?;
    }
    let started = Instant::now();
    let mut shown = None;
    while update_state(&state) == "downloading" {
        let percent = state["update"]["status"]["percent"].as_u64();
        if percent != shown && percent.is_some() {
            eprintln!(
                "Downloading SonoJot {version}: {}%",
                percent.unwrap_or_default()
            );
            shown = percent;
        }
        if started.elapsed() > Duration::from_secs(30 * 60) {
            return Err(Failure::Failed(
                "The download did not finish within 30 minutes.".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
        state = client.call("get_state", json!({}))?;
    }
    if update_state(&state) != "ready" {
        return Err(Failure::Failed(
            state["update"]["status"]["message"]
                .as_str()
                .unwrap_or("The update could not be downloaded.")
                .into(),
        ));
    }
    let version = state["update"]["status"]["version"]
        .as_str()
        .unwrap_or(&version)
        .to_owned();
    if let Some(reason) = state["update"]["restartBlockedReason"].as_str() {
        return Err(Failure::Failed(format!(
            "SonoJot {version} is downloaded, but SonoJot can't restart yet: {reason} \
Run `sonojot-cli update install --confirm` again afterwards."
        )));
    }
    eprintln!("Installing SonoJot {version}. SonoJot will restart.");
    client.call("restart_to_update", json!({}))?;
    wait_for_restart(&version)
}

/// Waits for the installer to replace SonoJot and start it again. Never starts
/// the app itself: that would race the installer.
fn wait_for_restart(version: &str) -> Outcome {
    let options = Options {
        no_launch: true,
        ..Default::default()
    };
    let client = Client { options: &options };
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10 * 60) {
        std::thread::sleep(Duration::from_secs(1));
        let Ok(state) = client.call("get_state", json!({})) else {
            continue;
        };
        let update = &state["update"];
        if update["installing"].as_bool() == Some(true) {
            continue;
        }
        if state["version"].as_str() == Some(version) {
            let cli = installed_cli_version();
            let mut summary = update_summary(&state);
            summary["installed"] = json!(version);
            emit(&summary);
            return if cli.as_deref() == Some(version) {
                Ok(())
            } else {
                Err(Failure::Failed(format!(
                    "SonoJot {version} is running, but sonojot-cli is {}. \
Run `sonojot-cli update repair --confirm`.",
                    cli.as_deref().unwrap_or("missing")
                )))
            };
        }
        if update_state(&state) == "error" {
            return Err(Failure::Failed(
                update["status"]["message"]
                    .as_str()
                    .unwrap_or("The update could not be installed.")
                    .into(),
            ));
        }
    }
    Err(Failure::Failed(format!("SonoJot did not come back on {version} within 10 minutes. \
If the installer is showing a message, answer it; otherwise open SonoJot, then run `sonojot-cli update status`.")))
}

/// Polls until the meeting has no running operation, then prints it.
fn wait(client: &Client, id: &str) -> Outcome {
    let timeout = client
        .options
        .value("timeout")
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| Failure::Usage("--timeout is a number of seconds.".into()))?
        .unwrap_or(4 * 3600);
    let started = Instant::now();
    loop {
        let meeting = client.call("get_meeting", json!({ "id": id }))?;
        let status = meeting["status"].as_str().unwrap_or_default();
        if !matches!(
            status,
            "recording"
                | "finalizing"
                | "transcribing"
                | "processingNotes"
                | "generatingTitle"
                | "importing"
        ) {
            emit(&brief(&meeting));
            return if status == "failed" {
                Err(Failure::Failed(
                    meeting["error"]
                        .as_str()
                        .unwrap_or("The operation failed.")
                        .into(),
                ))
            } else {
                Ok(())
            };
        }
        if started.elapsed().as_secs() >= timeout {
            return Err(Failure::Failed(format!(
                "Still {status} after {timeout} s."
            )));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn text_input(options: &Options) -> Outcome<String> {
    use std::io::Read;
    match (options.value("text"), options.value("file")) {
        (Some(text), None) => Ok(text.into()),
        (None, Some("-")) => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            Ok(text)
        }
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| Failure::Failed(format!("Cannot read {path}: {e}"))),
        _ => Err(Failure::Usage(
            "Pass --text \"...\" or --file <path> (use - for stdin).".into(),
        )),
    }
}

fn absolute(path: &str) -> Outcome<String> {
    let path = std::path::Path::new(path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    Ok(path.display().to_string())
}

fn extension(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("md")
        .to_ascii_lowercase()
}

/// The fields of a meeting that matter after an operation, without its text.
fn brief(meeting: &Value) -> Value {
    let chars = |key: &str| {
        meeting[key]
            .as_str()
            .map(|text| text.chars().count())
            .unwrap_or(0)
    };
    json!({
        "id": meeting["id"], "title": meeting["title"], "status": meeting["status"],
        "createdAt": meeting["createdAt"], "durationSeconds": meeting["durationSeconds"],
        "source": meeting["source"], "transcriptionModel": meeting["transcriptionModel"],
        "speakerLabels": meeting["speakerLabels"], "speakerCount": meeting["speakerCount"],
        "transcriptChars": chars("transcript"), "notesChars": chars("notes"),
        "recognitionCompleteness": meeting["recognitionCompleteness"],
        "recordingWarnings": meeting["recordingWarnings"], "recognitionWarnings": meeting["recognitionWarnings"],
        "error": meeting["error"],
    })
}

fn emit(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

struct Client<'a> {
    options: &'a Options,
}

impl Client<'_> {
    /// Lookups and reads always wait for their answer.
    fn call(&self, command: &str, args: Value) -> Outcome<Value> {
        self.send(command, args, false)
    }
    /// The command a user asked for; `--detach` applies here only.
    fn act(&self, command: &str, args: Value) -> Outcome<Value> {
        self.send(command, args, self.options.detach)
    }
    fn send(&self, command: &str, args: Value, detach: bool) -> Outcome<Value> {
        let request = Request {
            version: PROTOCOL_VERSION,
            command: command.into(),
            args,
            detach,
        };
        match call(&request) {
            Ok(value) => Ok(value),
            Err(CallError::Failed(message)) => Err(Failure::Failed(message)),
            Err(CallError::NotRunning(_)) if !self.options.no_launch => {
                launch_app()?;
                call(&request).map_err(|error| match error {
                    CallError::Failed(message) => Failure::Failed(message),
                    CallError::NotRunning(message) => Failure::NotRunning(format!(
                        "SonoJot started but is not answering: {message}"
                    )),
                })
            }
            Err(CallError::NotRunning(message)) => Err(Failure::NotRunning(format!(
                "SonoJot is not running ({message})."
            ))),
        }
    }
    fn print(&self, command: &str, args: Value) -> Outcome {
        emit(&self.act(command, args)?);
        Ok(())
    }
    fn print_meeting(&self, command: &str, args: Value) -> Outcome {
        let result = self.act(command, args)?;
        let mut output = if result.get("transcript").is_some() {
            brief(&result)
        } else {
            result.clone()
        };
        if command == "transcribe_meeting" && result.get("transcript").is_some() {
            // Coverage is advisory; the transcription itself already succeeded.
            match self.call("get_timed_transcript", json!({"id":result["id"]})) {
                Ok(timed) => {
                    output["publishedTranscriptUsesLatestAttempt"] =
                        timed["publishedTranscriptUsesLatestAttempt"].clone();
                    output["publishedTranscriptCoverage"] = timed["coverage"].clone();
                    if !timed["latestAttempt"].is_null() {
                        output["latestAttemptCoverage"] =
                            timed["latestAttempt"]["coverage"].clone();
                        output["attemptNotice"] = timed["attemptNotice"].clone();
                    }
                }
                Err(
                    Failure::Failed(message)
                    | Failure::Usage(message)
                    | Failure::NotRunning(message),
                ) => {
                    output["coverageError"] = json!(message);
                }
            }
        }
        emit(&output);
        Ok(())
    }
    fn print_state(&self, command: &str) -> Outcome {
        let snapshot = self.act(command, json!({}))?;
        emit(
            &json!({ "state": snapshot["state"], "error": snapshot["error"], "outputText": snapshot["outputText"] }),
        );
        Ok(())
    }
    /// Accepts a full ID, a unique ID prefix, or `latest`.
    fn meeting(&self, id: &str) -> Outcome<String> {
        if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(id.into());
        }
        let list = self.call("list_meetings", json!({}))?;
        let rows = list.as_array().cloned().unwrap_or_default();
        if id == "latest" {
            return rows
                .first()
                .and_then(|row| row["id"].as_str())
                .map(str::to_owned)
                .ok_or_else(|| Failure::Failed("There are no meetings yet.".into()));
        }
        let matches: Vec<&str> = rows
            .iter()
            .filter_map(|row| row["id"].as_str())
            .filter(|known| known.starts_with(id))
            .collect();
        match matches.as_slice() {
            [one] => Ok((*one).into()),
            [] => Err(Failure::Failed(format!(
                "No meeting ID starts with `{id}`. Run `sonojot-cli meetings list`."
            ))),
            _ => Err(Failure::Failed(format!(
                "`{id}` matches {} meetings; use more characters.",
                matches.len()
            ))),
        }
    }
}

/// Starts the app beside this executable, without opening its window, and
/// waits for it to answer.
fn launch_app() -> Outcome {
    let here = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = here
        .parent()
        .ok_or("Cannot find the SonoJot folder.".to_string())?;
    let names: &[&str] = if sonojot_core::flavor::SYNC_TEST {
        if cfg!(windows) { &["SonoJot-SyncTest.exe"] } else { &["SonoJot-SyncTest"] }
    } else if cfg!(windows) {
        &["SonoJot.exe", "sonojot.exe"]
    } else {
        &["SonoJot", "sonojot"]
    };
    let app = names
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            Failure::NotRunning(format!(
                "SonoJot is not running, and the app was not found beside {}.",
                here.display()
            ))
        })?;
    let mut command = std::process::Command::new(&app);
    command
        .arg("--startup")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    let mut child = {
        use std::os::windows::process::CommandExt;
        keep_console_from_child();
        const DETACHED_PROCESS: u32 = 0x8;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // Leave the caller's console and job so the app outlives this command.
        command.creation_flags(
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
        );
        match command.spawn() {
            Ok(child) => child,
            Err(_) => {
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
                command
                    .spawn()
                    .map_err(|e| format!("Cannot start {}: {e}", app.display()))?
            }
        }
    };
    #[cfg(not(windows))]
    let mut child = {
        use std::os::unix::process::CommandExt;
        // Its own process group, so closing this terminal doesn't stop the app.
        command.process_group(0);
        command
            .spawn()
            .map_err(|e| format!("Cannot start {}: {e}", app.display()))?
    };
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(90) {
        std::thread::sleep(Duration::from_millis(500));
        let ping = Request {
            version: PROTOCOL_VERSION,
            command: "ping".into(),
            args: json!({}),
            detach: false,
        };
        if call(&ping).is_ok() {
            return Ok(());
        }
        // A second instance hands off to the first and exits at once. If the
        // first never answers, it predates the CLI.
        if child.try_wait().ok().flatten().is_some() && call(&ping).is_err() {
            return Err(Failure::NotRunning(
                "SonoJot is running but does not accept CLI commands. \
Update SonoJot, or quit and reopen it, then try again."
                    .into(),
            ));
        }
    }
    Err(Failure::NotRunning(
        "SonoJot did not start within 90 seconds.".into(),
    ))
}

/// Windows hands every inheritable handle to a child process. The caller's
/// stdout is often an inheritable pipe, and an app holding it open would keep
/// `sonojot-cli status | jq` waiting until the app quits.
#[cfg(windows)]
fn keep_console_from_child() {
    extern "system" {
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 1;
    const STD_HANDLES: [u32; 3] = [-10i32 as u32, -11i32 as u32, -12i32 as u32];
    for which in STD_HANDLES {
        // SAFETY: plain Win32 calls on this process's own standard handles.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() && handle as isize != -1 {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// A stdio MCP server with no real tools. Agents that look for SonoJot over
/// MCP learn to use this CLI instead.
fn mcp_stub() -> Outcome {
    use std::io::{BufRead, Write};
    let text = guide();
    let pointer = format!(
        "SonoJot is controlled through its command-line tool, not MCP. Run `\"{}\" guide` for the full reference; \
every GUI action is available, for example `meetings list`, `meetings transcribe <id>` and `invoke <command> <json>`.",
        cli_path());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        }; // notifications need no reply
        let method = message["method"].as_str().unwrap_or_default();
        let result = match method {
            "initialize" => Some(json!({
                "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "sonojot", "version": env!("CARGO_PKG_VERSION") },
                "instructions": pointer,
            })),
            "ping" => Some(json!({})),
            "tools/list" => Some(json!({ "tools": [{
                "name": "sonojot_use_cli",
                "description": pointer,
                "inputSchema": { "type": "object", "properties": {} },
            }] })),
            "tools/call" => {
                Some(json!({ "content": [{ "type": "text", "text": text }], "isError": false }))
            }
            "resources/list" => Some(json!({ "resources": [] })),
            "prompts/list" => Some(json!({ "prompts": [] })),
            _ => None,
        };
        let reply = match result {
            Some(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            None => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": pointer } })
            }
        };
        if writeln!(stdout, "{reply}")
            .and_then(|_| stdout.flush())
            .is_err()
        {
            break;
        }
    }
    Ok(())
}

fn cli_path() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "sonojot-cli".into())
}

fn help() -> String {
    format!(
        r#"sonojot-cli {version}: control SonoJot from a terminal, script or agent.

Usage: sonojot-cli <command> [options]

The CLI talks to the running SonoJot app and starts it in the background if
needed (--no-launch to skip). Results print as JSON; errors go to stderr.
Exit codes: 0 ok, 1 failed, 2 usage, 3 SonoJot not running.

App
  status                         Version, state and selected models
  quit                           Quit SonoJot (saves an active recording first)
  open [page]                    Open a window: dictation, meetings, setup, appearance, microphone, shortcuts
  settings get [key]             Show settings
  settings set key=value ...     Change settings (values are JSON where they parse)
  models list                    Installed and available models
  ai                             Cloud AI settings (Notes, titles)
  vocabulary list                Custom words
  diagnostics                    Diagnostic report

Updates  (on the channel chosen in About; the app checks each package's signature)
  update status                  Version, update state and installation health
  update check                   Check for a newer version now. After
                                 `settings set updateChannel=stable` (or
                                 `=nightly`)
                                 this offers that channel's newest build,
                                 even an older one
  update install --confirm       Download and install it; SonoJot restarts
  update repair --confirm        Reinstall the current release, for an update
                                 that stopped partway

Dictation
  dictation start|stop|cancel
  dictation history [--limit N]
  dictation copy-last

Meetings  (<id> is a full ID, a unique prefix, or `latest` for the most recently started)
  meetings list
  meetings show <id>             Full record as JSON
  meetings transcript <id> [--format text|json] [--cleaned] [--latest-attempt]
  meetings notes <id>
  meetings record start [--title T] [--source both|microphone|system]
  meetings record stop
  meetings import <file> [--speaker "Name"]  Known speaker, diarization off
  meetings join <id> <id> ... [--title T]    New meeting from recordings in order
  meetings transcribe <id> [--model M] [--speaker "Name"] [--no-diarize] [--detach]
  meetings wait <id> [--timeout S]           Wait for a detached job to finish
  meetings cancel <id>
  meetings rename <id> <title>
  meetings speakers <id>                     Speaker labels in the transcript
  meetings rename-speaker <id> <from> <to>   e.g. "Speaker 2" "Rachel Afant"
  meetings labels <id> on|off [--confirm]    Numbered voices on or off
  meetings clean <id>                        Local transcript cleanup
  meetings generate-notes <id> [--preset summary|tasks|speakers|brief|formatted-transcript|custom] [--instructions T]
  meetings set-notes <id> --file F | --text T
  meetings set-transcript <id> --file F | --text T
  meetings export <id> <path> [--format md|txt|html|pdf|wav|json|srt|vtt] [--cleaned] [--latest-attempt]
  meetings export-audio <id> <path>
  meetings delete <id> --confirm

Everything else
  commands                       Every app command and its arguments
  invoke <command> [json]        Call any app command, e.g. invoke get_meeting '{{"id":"..."}}'
  invoke connect_openrouter_key --key-stdin   Read a key from piped stdin, never an argument
  guide                          How agents should use SonoJot
  agents status|install|remove   sonojot-cli on PATH, and the MCP pointer in
                                 Claude Code, Codex, Gemini, Cursor, Windsurf,
                                 Claude Desktop and opencode
  mcp                            MCP stub for agents: it points them to this CLI
  version
"#,
        version = env!("CARGO_PKG_VERSION")
    )
}

fn guide() -> String {
    format!(
        r#"# Using SonoJot from an agent

SonoJot has no MCP tools. Use this CLI instead: it reaches everything the app
window can do, and it runs in the background without the window.

CLI: {path}

## Basics
- Every command talks to the running app (one owner for recording, models and
  the meeting library). If the app is closed, the CLI starts it hidden.
- Metadata is JSON on stdout; transcripts default to plain text. Errors go to stderr. Exit codes: 0 ok,
  1 failed, 2 usage, 3 not running.
- Meeting IDs accept a unique prefix or `latest`.
- `sonojot-cli commands` lists every app command; `sonojot-cli invoke <command>
  '<json args>'` calls one directly with the same camelCase arguments the
  window uses.

## Long jobs
`meetings transcribe` waits until recognition and speaker labeling finish.
That can take several minutes for an hour of audio, so allow a long timeout,
or add `--detach` and then run `meetings wait <id>`.

## Typical meeting workflow
    sonojot-cli meetings list
    sonojot-cli meetings join <first> <second> --title "Client call"
    sonojot-cli meetings transcribe <new-id>
    sonojot-cli meetings speakers <new-id>
    sonojot-cli meetings rename-speaker <new-id> "You" "Cody Stevens"
    sonojot-cli meetings rename-speaker <new-id> "Other speakers" "Jane Doe"
    sonojot-cli meetings transcript <new-id>
    sonojot-cli meetings set-notes <new-id> --file notes.md
    sonojot-cli meetings export <new-id> call.md

## Video editing with aligned single-speaker tracks
Import each Riverside track separately. Keep each file's leading silence so all
exports share the recording timeline; times are seconds from imported media zero.

    sonojot-cli meetings import "cody.wav" --speaker "Cody Stevens"
    sonojot-cli meetings transcribe <id> --model qwen3-asr-1.7b-q8-cuda --no-diarize
    sonojot-cli meetings clean <id>
    sonojot-cli meetings transcript <id> --format json
    sonojot-cli meetings export <id> cody.json --format json
    sonojot-cli meetings export <id> cody.srt
    sonojot-cli meetings export <id> cody.vtt

Repeat for each speaker, then sort the exported segments by start in your editor.
`meetings join` concatenates recordings; it does not align simultaneous tracks.
`--speaker "Name"` on import persists a single-speaker preference for subsequent
transcriptions. On transcribe it overrides the name for that job and disables
diarization. `--no-diarize` skips diarization without requiring a known name.

JSON preserves raw timed segments and includes the cleaned transcript separately.
`--cleaned` selects cleaned text for plain text/document exports; it never rewrites
JSON timing. Cleaned subtitle export is refused because cleanup is not aligned.
SRT/VTT use raw segment boundaries. New timed exports refuse existing files.
Check timing metadata before choosing cuts: Qwen provides audio-window boundaries,
not word alignment. Word timestamps are explicitly unavailable when the recognizer
does not retain them; no word timings are estimated from text or punctuation.
Older recognition without segment ends cannot produce subtitles; transcribe again.

## Detect missing speech and recover an incomplete retry
An incomplete retry preserves an existing document, even when the retry decoded
more audio. A status of `transcribed` is not proof that all speech is present.
Check `recognitionCompleteness`, warnings, and timed JSON before editing.

    sonojot-cli meetings transcript <id> --format json
    sonojot-cli meetings transcript <id> --format json --latest-attempt
    sonojot-cli meetings export <id> latest.srt --latest-attempt

When `publishedTranscriptUsesLatestAttempt` is false, default JSON includes the
separate `latestAttempt` segments and an explanation. `--latest-attempt` exports
those raw timed segments without replacing the saved document or cleaned copy.
It supports JSON/SRT/VTT; it cannot be combined with `--cleaned`.

`coverage.uncoveredRanges` gives second-based intervals without a complete saved
transcription window, including any uncovered tail. These may include silence;
they are review ranges, not a measured duration of missing speech.
`coverage.knownOmittedRanges` and `knownOmittedSeconds` identify empty incomplete
decode windows. Untimed older attempts report unknown ends rather than inferred
coverage. The meeting record's `missingSeconds` measures capture loss only;
`captureMissingSeconds` makes that scope explicit in timed JSON.
Use your media editor or FFmpeg to extract a review range into a new file, import
it as a new meeting and transcribe it. Add the extraction offset to its exported
times when placing it on the original timeline. Automatic resume and a CLI range
transcription option are not available.

`meetings clean` runs the same conservative local cleanup and punctuation pipeline
as the app. `settings get sonojotPunctuation` controls punctuation;
`settings set sonojotPunctuation=true` enables it when the model is installed.
`writtenForm` controls written-form normalization. Cleanup saves a separate copy;
it is explicit, not part of `transcribe` or the `--cleaned` output selector.

## Care
- `generate-notes`, automatic titles and some vocabulary features use the AI
  provider set in SonoJot, which may be a paid cloud service. Check
  `sonojot-cli ai` and ask the user before using one.
- Destructive commands need `--confirm`. Don't pass it without the user's go-ahead.
- `update install` and `update repair` restart SonoJot and need `--confirm`.
  If `update status` shows an `installProblem`, an earlier update stopped
  partway; `update repair --confirm` reinstalls it.
- Don't accept model licenses (`acknowledge_download_terms`) for the user.
- Recording uses the microphone and system audio; start one only when asked.
"#,
        path = cli_path()
    )
}
