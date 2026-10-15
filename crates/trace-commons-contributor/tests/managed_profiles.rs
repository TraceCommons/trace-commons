use trace_commons_contributor::{
    config::ConfigStore,
    managed::{
        ConnectionKind, ToolId,
        accounts::AccountStore,
        profiles::{NativeAction, NativeProfiles},
    },
};

#[cfg(unix)]
#[test]
fn escaped_codex_provider_override_is_refused_but_comments_are_allowed() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Codex, ConnectionKind::Subscription, "Work")
        .unwrap();
    let profiles = NativeProfiles::new(&store);
    let project = home.path().join("project");
    std::fs::create_dir_all(project.join(".codex")).unwrap();
    let config = project.join(".codex/config.toml");
    std::fs::write(&config, "\"\\u006dodel_provider\" = \"other\"\n").unwrap();
    assert!(
        profiles
            .command(
                &account,
                std::path::Path::new("/usr/bin/true"),
                NativeAction::Run,
                &project
            )
            .is_err()
    );
    std::fs::write(
        &config,
        "# auth is handled by the native tool\nmodel = \"example\"\n",
    )
    .unwrap();
    assert!(
        profiles
            .command(
                &account,
                std::path::Path::new("/usr/bin/true"),
                NativeAction::Run,
                &project
            )
            .is_ok()
    );
}

#[cfg(unix)]
#[test]
fn native_status_is_required_and_api_auth_cannot_masquerade_as_a_subscription() {
    use std::os::unix::fs::PermissionsExt;
    use trace_commons_contributor::managed::AuthState;
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
        .unwrap();
    let profiles = NativeProfiles::new(&store);
    let native = home.path().join("claude");
    std::fs::write(&native, "#!/bin/sh\nif [ \"$1\" = '--version' ]; then echo '2.1.289 (Claude Code)'; else echo '{\"loggedIn\":true,\"authMethod\":\"api_key\"}'; fi\n").unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        profiles.inspect(&account, &native).unwrap(),
        AuthState::SignInRequired
    );
    std::fs::write(&native, "#!/bin/sh\nif [ \"$1\" = '--version' ]; then echo '2.1.289 (Claude Code)'; else echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; fi\n").unwrap();
    assert_eq!(
        profiles.inspect(&account, &native).unwrap(),
        AuthState::Ready
    );
}

#[test]
fn managed_codex_does_not_consult_standard_configuration_above_project_root() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Codex, ConnectionKind::Subscription, "Work")
        .unwrap();
    let project = home.path().join("repo");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(home.path().join(".codex")).unwrap();
    std::fs::write(
        home.path().join(".codex/config.toml"),
        "model_provider = \"standard_provider\"\n",
    )
    .unwrap();
    let profiles = NativeProfiles::new(&store);
    assert!(
        profiles
            .command(
                &account,
                &std::env::current_exe().unwrap(),
                NativeAction::Run,
                &project
            )
            .is_ok()
    );
}

#[cfg(unix)]
#[test]
fn managed_profiles_coexist_without_modifying_standard_settings_or_inheriting_keys() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().join("state")).unwrap();
    let project = home.path().join("project with spaces");
    std::fs::create_dir(&project).unwrap();
    let ordinary = home.path().join("ordinary-settings.json");
    std::fs::write(&ordinary, b"standard session settings").unwrap();
    let native = home.path().join("native-fixture");
    std::fs::write(&native, b"#!/bin/sh\nprintf '%s\\n%s\\n%s\\n' \"$CLAUDE_CONFIG_DIR\" \"$PWD\" \"${ANTHROPIC_API_KEY-unset}\"\n").unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let personal = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
        .unwrap();
    let work = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
        .unwrap();
    let profiles = NativeProfiles::new(&store);
    let a = profiles
        .command(&personal, &native, NativeAction::Run, &project)
        .unwrap()
        .output()
        .unwrap();
    let b = profiles
        .command(&work, &native, NativeAction::Run, &project)
        .unwrap()
        .output()
        .unwrap();
    assert!(a.status.success() && b.status.success());
    let a = String::from_utf8(a.stdout).unwrap();
    let b = String::from_utf8(b.stdout).unwrap();
    let a: Vec<_> = a.lines().collect();
    let b: Vec<_> = b.lines().collect();
    assert_ne!(a[0], b[0]);
    assert!(a[0].contains(&personal.id.to_string()));
    assert!(b[0].contains(&work.id.to_string()));
    assert_eq!(
        a[1],
        std::fs::canonicalize(&project).unwrap().to_str().unwrap()
    );
    assert_eq!(a[2], "unset");
    assert_eq!(
        std::fs::read(ordinary).unwrap(),
        b"standard session settings"
    );
    assert_eq!(
        std::fs::metadata(a[0]).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn symlink_profile_and_project_provider_overrides_are_refused() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
        .unwrap();
    let profiles = NativeProfiles::new(&store);
    let profile = profiles.profile_root(account.id).unwrap();
    std::fs::remove_dir(&profile).unwrap();
    std::os::unix::fs::symlink(home.path(), &profile).unwrap();
    assert!(profiles.profile_root(account.id).is_err());
    std::fs::remove_file(&profile).unwrap();
    let project = home.path().join("project");
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    std::fs::write(
        project.join(".claude/settings.json"),
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://different.example"}}"#,
    )
    .unwrap();
    assert!(
        profiles
            .command(
                &account,
                std::path::Path::new("/usr/bin/true"),
                NativeAction::Run,
                &project
            )
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn unsupported_native_logout_cannot_touch_credentials() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
        .unwrap();
    let profiles = NativeProfiles::new(&store);
    let profile = profiles.profile_root(account.id).unwrap();
    std::fs::write(profile.join("auth-fixture"), b"keep").unwrap();
    let native = home.path().join("old-claude");
    std::fs::write(
        &native,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '1.0.0'; else rm auth-fixture; fi\n",
    )
    .unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(profiles.remove(&account, &native).is_err());
    assert_eq!(
        std::fs::read(profile.join("auth-fixture")).unwrap(),
        b"keep"
    );
}
