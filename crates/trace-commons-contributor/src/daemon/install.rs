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

use crate::config::ConfigStore;

const UNIT_FILE_NAME: &str = crate::channel::SYSTEMD_UNIT_NAME;

/// The systemd user unit text.
pub fn systemd_unit_text(exe: &str, config_dir: &str) -> String {
    let env = crate::channel::STATE_DIR_ENV;
    let app = crate::brand::APP_NAME;
    let channel = if crate::channel::near_ai_staging() {
        " (staging)"
    } else {
        ""
    };
    format!(
        "[Unit]\n\
         Description={app} background uploader{channel}\n\
         Documentation=https://github.com/zmanian/trace-commons-server\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} daemon run\n\
         Environment={env}={config_dir}\n\
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
    // An older config may still name the retired client-side NEAR AI second
    // scan. It no longer refuses an install: the setting is ignored and
    // uploads are scrubbed locally only.
    if !cfg!(target_os = "linux") {
        bail!(
            "autostart-not-supported-on-this-platform: on this platform the \
             {} application registers the daemon as a login item. \
             Run `daemon run` directly in the meantime.",
            crate::brand::APP_NAME
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
    uninstall_at(&unit_path()?, super::host_link::runtime_dir().as_deref())
}

/// [`uninstall`] with the unit file and the host link's runtime directory
/// given. Also removes the "host part is installed" marker the unit's daemon
/// left (`host_link`), so a Flatpak window started later in this login runs
/// its own daemon rather than waiting on a host daemon that is gone.
fn uninstall_at(path: &std::path::Path, runtime: Option<&std::path::Path>) -> Result<()> {
    if let Some(runtime) = runtime
        && super::host_link::forget_host_unit(runtime).is_err()
    {
        // DRAFT, NEEDS APPROVAL (2026-11-17).
        println!("could not remove the host link marker; it clears at logout");
    }
    match std::fs::remove_file(path) {
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

    #[test]
    fn uninstall_forgets_the_host_unit_marker() {
        let dir = tempfile::tempdir().unwrap();
        let unit = dir.path().join(UNIT_FILE_NAME);
        std::fs::write(&unit, b"[Unit]\n").unwrap();
        let runtime = dir.path().join("run");
        std::fs::create_dir(&runtime).unwrap();
        let marker = runtime.join(super::super::host_link::HOST_UNIT_MARKER_FILE);
        std::fs::write(&marker, b"").unwrap();
        uninstall_at(&unit, Some(&runtime)).unwrap();
        assert!(!unit.exists());
        assert!(!marker.exists());
        // Nothing installed and no runtime directory: still fine.
        uninstall_at(&unit, None).unwrap();
    }
    use crate::config::tests_support::temp_store;

    fn cfg() -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            inference_receipt_endpoint: None,
            consent_scopes_chosen: Some(true),
            witness_origin: None,
            account_identity: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example.ai".into(),
            ingest_url: "https://ingest.example.ai".into(),
            audience: "trace-commons-ingest".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "key-1".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
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
        assert!(unit.contains(&format!(
            "{}=/home/z/.config/trace-commons",
            crate::channel::STATE_DIR_ENV
        )));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    /// A staging daemon gets its own unit, so it runs beside a production
    /// one and installing either never overwrites the other.
    #[test]
    fn each_channel_installs_its_own_unit() {
        let unit = systemd_unit_text("/x", "/y");
        if crate::channel::near_ai_staging() {
            assert_eq!(UNIT_FILE_NAME, "trace-commons-contributor-staging.service");
            assert!(unit.contains("Description=Near AI Gateway background uploader (staging)\n"));
        } else {
            assert_eq!(UNIT_FILE_NAME, "trace-commons-contributor.service");
            assert!(unit.contains("Description=Near AI Gateway background uploader\n"));
        }
    }

    #[test]
    fn install_ignores_a_retired_pii_filter() {
        // An old config naming the retired NEAR AI scan, with no credential
        // and no notice marker: both used to refuse the install.
        let (_d, store) = temp_store();
        store.save_config(&cfg()).unwrap();
        let path = store.dir().join("contributor.json");
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        raw["pii_filter"] = serde_json::json!("near-ai");
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
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
