fn checked_url(url: &str) -> Result<&str, String> {
    match url {
        "https://sonojot.com/privacy" | "https://sonojot.com/trademarks" => Ok(url),
        _ => Err("Unknown legal link.".into()),
    }
}

fn browser_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid web address.")?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Invalid web address.".into());
    }
    Ok(())
}

/// Hands a bundled HTTPS destination to the default browser; never a file or scheme handler.
pub(crate) fn open_https_blocking(url: &str, what: &str) -> Result<(), String> {
    browser_url(url)?;
    let program = if cfg!(windows) { "explorer.exe" }
        else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(program).arg(url).spawn()
        .map(|_| ()).map_err(|error| format!("Cannot open the {what}: {error}"))
}

pub(crate) async fn open_https(url: String, what: &'static str) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || open_https_blocking(&url, what))
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_legal_link(url: String) -> Result<(), String> {
    checked_url(&url)?;
    open_https(url, "legal page").await
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_bundled_legal_destinations_open() {
        for url in ["https://sonojot.com/privacy", "https://sonojot.com/trademarks"] {
            assert!(super::checked_url(url).is_ok());
        }
        for url in ["file:///C:/Windows", "--help", "javascript:alert(1)", "https://sonojot.com.evil/privacy", "http://sonojot.com/privacy"] {
            assert!(super::checked_url(url).is_err());
        }
    }

    #[test]
    fn the_shared_opener_accepts_only_plain_https_addresses() {
        assert!(super::browser_url("https://docs.nvidia.com/cuda/eula/index.html#a").is_ok());
        for url in ["http://example.com", "file:///C:/Windows/notepad.exe", "C:\\Windows",
            "--help", "https://user:pass@example.com", "ms-settings:privacy", "https://"] {
            assert!(super::browser_url(url).is_err(), "{url}");
        }
    }
}
