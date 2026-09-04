//! Starting the daemon at login.
//!
//! Only the systemd user unit is written here, and deliberately so. A headless
//! Linux machine has no tray to own its own startup, so it genuinely needs
//! this. On macOS and Windows the native application is what registers as a
//! login item, and having the daemon fight it for ownership of that
//! registration would be worse than not offering it.
//!
//! Install refuses when a privacy filter is configured without persisted
//! credentials. A service manager passes no shell environment, so such a
//! daemon would start cleanly, refuse every single upload with
//! `pii-filter-unavailable`, and look like a bug rather than a
//! misconfiguration.

use anyhow::{Context, Result, bail};

use super::settings::DaemonSettings;
use crate::config::ConfigStore;

const UNIT_FILE_NAME: &str = "trace-commons-contributor.service";

/// The systemd user unit text.
pub fn systemd_unit_text(exe: &str, config_dir: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Trace Commons contributor background uploader\n\
         Documentation=https://github.com/zmanian/trace-commons-server\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} daemon run\n\
         Environment=TRACE_COMMONS_CONTRIBUTOR_DIR={config_dir}\n\
         Restart=on-failure\n\
         RestartSec=30\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

fn unit_path() -> Result<std::path::PathBuf> {
    let base = dirs::config_dir().context("could not determine a config directory")?;
    Ok(base.join("systemd/user").join(UNIT_FILE_NAME))
}

pub fn install(store: &ConfigStore) -> Result<()> {
    let cfg = store.load_config().context("loading contributor config")?;
    let settings = DaemonSettings::load(store)?;

    if let Some(cfg) = cfg.as_ref() {
        if cfg.pii_filter.as_deref() == Some("near-ai") && settings.near_ai.is_none() {
            bail!(
                "refusing to install: pii_filter is near-ai but no near_ai \
                 credentials are persisted in daemon settings. A service \
                 manager passes no shell environment, so every upload would \
                 refuse with pii-filter-unavailable. Set them with \
                 `daemon settings` first."
            );
        }
        if cfg.pii_filter.as_deref() == Some("near-ai") && !store.near_ai_notice_shown() {
            bail!(
                "refusing to install: the NEAR AI first-use notice has not \
                 been shown yet. Run an interactive `submit` once so the \
                 disclosure is actually delivered to you."
            );
        }
    }

    if !cfg!(target_os = "linux") {
        bail!(
            "autostart-not-supported-on-this-platform: on this platform the \
             Trace Commons application registers the daemon as a login item. \
             Run `daemon run` directly in the meantime."
        );
    }

    let exe = std::env::current_exe().context("finding this executable")?;
    let unit = systemd_unit_text(&exe.to_string_lossy(), &store.dir().to_string_lossy());
    let path = unit_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&path, unit).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    println!("enable it with: systemctl --user enable --now {UNIT_FILE_NAME}");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let path = unit_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => {
            println!("removed {}", path.display());
            println!("disable it with: systemctl --user disable --now {UNIT_FILE_NAME}");
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("no unit file installed");
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;
    use crate::envelope::NearAiSettings;

    fn cfg_with_filter(filter: Option<&str>) -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            inference_receipt_endpoint: None,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example.ai".into(),
            ingest_url: "https://ingest.example.ai".into(),
            audience: "trace-commons-ingest".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "key-1".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: filter.map(str::to_string),
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        }
    }

    #[test]
    fn the_unit_names_the_run_subcommand_and_the_state_directory() {
        let unit = systemd_unit_text(
            "/usr/local/bin/trace-commons-contributor",
            "/home/z/.config/trace-commons",
        );
        assert!(unit.contains("ExecStart=/usr/local/bin/trace-commons-contributor daemon run"));
        assert!(unit.contains("TRACE_COMMONS_CONTRIBUTOR_DIR=/home/z/.config/trace-commons"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn install_refuses_near_ai_without_persisted_credentials() {
        // Otherwise every upload refuses under systemd and it reads as a bug.
        let (_d, store) = temp_store();
        store
            .save_config(&cfg_with_filter(Some("near-ai")))
            .unwrap();
        let err = install(&store).unwrap_err();
        assert!(
            err.to_string().contains("near_ai"),
            "expected a credentials refusal, got: {err}"
        );
    }

    #[test]
    fn install_refuses_near_ai_until_the_notice_was_delivered() {
        let (_d, store) = temp_store();
        store
            .save_config(&cfg_with_filter(Some("near-ai")))
            .unwrap();
        let settings = DaemonSettings {
            near_ai: Some(NearAiSettings {
                api_key: "k".into(),
                base_url: None,
                model: None,
            }),
            ..Default::default()
        };
        settings.save(&store).unwrap();
        let err = install(&store).unwrap_err();
        assert!(
            err.to_string().contains("notice"),
            "expected a notice refusal, got: {err}"
        );
    }

    #[test]
    fn install_without_a_privacy_filter_gets_past_the_credential_gates() {
        let (_d, store) = temp_store();
        store.save_config(&cfg_with_filter(None)).unwrap();
        let result = install(&store);
        if cfg!(target_os = "linux") {
            assert!(result.is_ok(), "{result:?}");
        } else {
            // Elsewhere the application owns login-item registration.
            let err = result.unwrap_err();
            assert!(
                err.to_string()
                    .contains("autostart-not-supported-on-this-platform"),
                "got: {err}"
            );
        }
    }
}
