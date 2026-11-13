//! Build-time identities for the isolated staging trial.
pub const SYNC_TEST: bool = cfg!(feature = "synctest");

/// Release-channel preferences cannot enable sync in an installed stable build.
pub fn sync_enabled() -> bool {
    require_sync().is_ok()
}

fn sync_enabled_for(version: &str, sync_test: bool) -> bool {
    sync_test || semver::Version::parse(version).is_ok_and(|v| !v.pre.is_empty())
}

pub fn require_sync() -> Result<(), String> {
    require_sync_for(env!("CARGO_PKG_VERSION"), SYNC_TEST, crate::sync_client::STAGING)
}

fn require_sync_for(version: &str, sync_test: bool, origin: &str) -> Result<(), String> {
    if !sync_enabled_for(version, sync_test) {
        return Err("Sync is available in prerelease and Sync Test builds only.".into());
    }
    crate::sync_client::validate_staging_origin(origin)
}
pub const NAME: &str = if SYNC_TEST {
    "SonoJot Sync Test"
} else {
    "SonoJot"
};
pub const SLUG: &str = if SYNC_TEST {
    "sonojot-synctest"
} else {
    "sonojot"
};
pub const IDENTIFIER: &str = if SYNC_TEST {
    "studio.codystevens.sonojot.synctest"
} else {
    "studio.codystevens.eavesdropper.next"
};
pub const SYNC_KEYRING: &str = if SYNC_TEST {
    "SonoJot Sync Test sync"
} else {
    "SonoJot sync"
};
pub const SYNC_KEYRING_APP: &str = if SYNC_TEST {
    "sonojot-synctest-sync"
} else {
    "sonojot-sync"
};
pub const AI_KEYRING: &str = if SYNC_TEST {
    "SonoJot Sync Test OpenRouter connection"
} else {
    "SonoJot OpenRouter connection"
};

/// A bounded startup probe, with no desktop, network, capture or integration work.
/// Also performs the same one-shot vocabulary seed as normal first launch.
pub fn verify() -> Result<serde_json::Value, String> {
    if !SYNC_TEST {
        return Err("This command requires the Sync Test build.".into());
    }
    let store = crate::store::Store::open(None)?;
    let settings = store.load_settings()?;
    let seed: serde_json::Value = store.read(&store.root().join("synctest-seed.json"))?;
    let value = serde_json::json!({
        "name": NAME, "identifier": IDENTIFIER, "dataDirectory": store.root(),
        "cliEndpoint": crate::cli_protocol::endpoint(), "server": crate::sync_client::STAGING,
        "syncEnabled": sync_enabled(), "syncUnavailableReason": require_sync().err(),
        "vocabularyCount": store.get_vocabulary()?.len(), "seed": seed,
        "autostart": settings.launch_at_login, "globalShortcut": false,
        "updater": false, "agentIntegration": false, "deepLinkRegistration": false,
        "signInReturn": "loopback"
    });
    store.write(&store.root().join("synctest-verification.json"), &value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_requires_a_prerelease_or_the_isolated_flavor() {
        for version in ["0.28.9", "1.0.0", "0.28.9+nightly", "invalid-nightly"] {
            assert!(!sync_enabled_for(version, false), "{version}");
            assert!(sync_enabled_for(version, true), "{version}");
        }
        for version in ["0.28.10-nightly.202611131500", "1.0.0-rc.1", "1.0.0-beta+build"] {
            assert!(sync_enabled_for(version, false), "{version}");
        }
        assert_eq!(sync_enabled(), require_sync().is_ok());
    }

    #[test]
    fn sync_requires_a_configured_staging_origin_in_every_flavor() {
        let staging = "https://sonojot-sync-staging.example.workers.dev";
        for (version, sync_test) in [("1.0.0", true), ("1.0.0-nightly.202611131500", false)] {
            assert!(require_sync_for(version, sync_test, "").unwrap_err().contains("not configured"));
            assert!(require_sync_for(version, sync_test, staging).is_ok());
            assert!(require_sync_for(version, sync_test, "https://sync.sonojot.com").is_err());
        }
        assert!(require_sync_for("1.0.0", false, staging).unwrap_err().contains("prerelease"));
    }
    #[test]
    fn synctest_identities_and_safe_defaults_match_flavor() {
        let settings = crate::domain::Settings::default();
        assert!(!settings.launch_at_login);
        if SYNC_TEST {
            assert_eq!(NAME, "SonoJot Sync Test");
            assert_eq!(IDENTIFIER, "studio.codystevens.sonojot.synctest");
            assert_ne!(SYNC_KEYRING, "SonoJot sync");
            assert!(settings.start_tab_hidden);
            assert!(crate::cli_protocol::data_dir().ends_with(NAME));
            #[cfg(windows)]
            assert!(crate::cli_protocol::endpoint()
                .to_string_lossy()
                .contains("Sync Test"));
        } else {
            assert_eq!(NAME, "SonoJot");
            assert_eq!(SLUG, "sonojot");
            assert!(!settings.start_tab_hidden);
        }
    }

    #[test]
    fn synctest_overlay_cannot_claim_daily_install_or_protocol() {
        let overlay: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.synctest.conf.json")).unwrap();
        assert_eq!(overlay["identifier"], "studio.codystevens.sonojot.synctest");
        assert_eq!(overlay["productName"], "SonoJot Sync Test");
        assert_eq!(overlay["mainBinaryName"], "SonoJot-SyncTest");
        assert_eq!(
            overlay["build"]["features"],
            serde_json::json!(["synctest"])
        );
        assert_eq!(
            overlay["plugins"]["deep-link"]["desktop"]["schemes"],
            serde_json::json!([])
        );
        assert_eq!(
            overlay["plugins"]["updater"]["endpoints"],
            serde_json::json!([])
        );
        assert_eq!(overlay["bundle"]["createUpdaterArtifacts"], false);
        assert!(overlay["bundle"]["windows"]["nsis"]["template"].is_null());
        assert!(overlay["bundle"]["windows"]["nsis"]["installerHooks"].is_null());
    }

    #[test]
    #[cfg(feature = "synctest")]
    fn synctest_integrations_are_inert() {
        let missing = std::path::Path::new("not-a-real-install");
        assert!(crate::agent_integration::ensure(missing, missing).is_none());
        for mode in [
            crate::agent_integration::Mode::Check,
            crate::agent_integration::Mode::Install,
            crate::agent_integration::Mode::Remove,
        ] {
            let items = crate::agent_integration::run(mode, missing, missing);
            assert!(items.iter().all(|item| item.state == "skipped"));
        }
        assert!(crate::sync_client::SyncClient::new("https://example.com", "").is_err());
    }
}
