use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::managed::{
    ConnectionKind, Generation, ManagedError, Selection, ToolId, accounts::AccountStore,
};

#[test]
fn saved_accounts_survive_restart_and_tool_defaults_are_independent() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let personal = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
        .unwrap();
    let work = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
        .unwrap();
    let codex = accounts
        .add(ToolId::Codex, ConnectionKind::Subscription, "Work")
        .unwrap();
    for (tool, id) in [(ToolId::Claude, personal.id), (ToolId::Codex, codex.id)] {
        accounts
            .select(
                Selection {
                    tool,
                    connection: ConnectionKind::Subscription,
                    account_id: id,
                    generation: Generation(0),
                },
                Generation(0),
            )
            .unwrap();
    }
    accounts
        .select(
            Selection {
                tool: ToolId::Claude,
                connection: ConnectionKind::Subscription,
                account_id: work.id,
                generation: Generation(1),
            },
            Generation(1),
        )
        .unwrap();
    let reopened = AccountStore::open(&store).unwrap();
    assert_eq!(reopened.list().len(), 3);
    assert_eq!(
        reopened.selection(ToolId::Claude).unwrap().account_id,
        work.id
    );
    assert_eq!(
        reopened.selection(ToolId::Codex).unwrap().account_id,
        codex.id
    );
    assert_eq!(
        reopened.selection(ToolId::Claude).unwrap().generation,
        Generation(2)
    );
}

#[test]
fn stale_generation_and_wrong_tool_cannot_replace_default() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let personal = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
        .unwrap();
    let selection = Selection {
        tool: ToolId::Claude,
        connection: ConnectionKind::Subscription,
        account_id: personal.id,
        generation: Generation(0),
    };
    accounts.select(selection.clone(), Generation(0)).unwrap();
    assert_eq!(
        accounts.select(selection.clone(), Generation(0)),
        Err(ManagedError::Conflict)
    );
    let wrong = Selection {
        tool: ToolId::Codex,
        ..selection
    };
    assert_eq!(
        accounts.select(wrong, Generation(0)),
        Err(ManagedError::Conflict)
    );
    assert!(accounts.selection(ToolId::Codex).is_none());
}

#[test]
fn ambiguous_label_requires_id_and_account_views_never_serialize_secrets() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let first = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Work")
        .unwrap();
    accounts
        .add(ToolId::Claude, ConnectionKind::ApiKey, "Work")
        .unwrap();
    assert_eq!(
        accounts.resolve(ToolId::Claude, "Work"),
        Err(ManagedError::AmbiguousAccount)
    );
    assert_eq!(
        accounts.resolve(ToolId::Claude, &first.id.to_string()),
        Ok(first.id)
    );
    assert_eq!(
        accounts.resolve(ToolId::Codex, &first.id.to_string()),
        Err(ManagedError::NotFound)
    );
    let value = serde_json::to_value(accounts.list()).unwrap();
    let object = value[0].as_object().unwrap();
    assert_eq!(object.len(), 6);
    assert_eq!(object["auth_state"], "sign_in_required");
    assert!(!object.contains_key("profile"));
    assert!(!object.contains_key("secret"));
}

#[test]
fn invalid_metadata_is_not_silently_reset() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
    store
        .write_daemon_file("managed-accounts.json", b"not json")
        .unwrap();
    assert!(matches!(
        AccountStore::open(&store),
        Err(ManagedError::StorageUnavailable)
    ));
    assert_eq!(
        std::fs::read(home.path().join("managed-accounts.json")).unwrap(),
        b"not json"
    );
}

#[test]
fn rename_and_remove_preserve_other_accounts_and_clear_only_the_removed_default() {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let a = accounts
        .add(ToolId::Claude, ConnectionKind::Subscription, "Personal")
        .unwrap();
    let b = accounts
        .add(ToolId::Codex, ConnectionKind::Subscription, "Work")
        .unwrap();
    accounts.rename(a.id, "Private").unwrap();
    accounts
        .select(
            Selection {
                tool: ToolId::Claude,
                connection: ConnectionKind::Subscription,
                account_id: a.id,
                generation: Generation(0),
            },
            Generation(0),
        )
        .unwrap();
    accounts.remove(a.id).unwrap();
    let accounts = AccountStore::open(&store).unwrap();
    assert_eq!(accounts.list(), vec![b]);
    assert!(accounts.selection(ToolId::Claude).is_none());
}

#[test]
fn removing_a_default_never_reuses_its_generation() {
    let home = tempfile::tempdir().unwrap();
    let config = trace_commons_contributor::config::ConfigStore::open(home.path().into()).unwrap();
    let mut store = AccountStore::open(&config).unwrap();
    let a = store
        .add(ToolId::Claude, ConnectionKind::Subscription, "A")
        .unwrap();
    let b = store
        .add(ToolId::Claude, ConnectionKind::Subscription, "B")
        .unwrap();
    let selection = Selection {
        tool: ToolId::Claude,
        connection: ConnectionKind::Subscription,
        account_id: a.id,
        generation: Generation(0),
    };
    store.select(selection, Generation(0)).unwrap();
    store.remove(a.id).unwrap();
    let request = Selection {
        tool: ToolId::Claude,
        connection: ConnectionKind::Subscription,
        account_id: b.id,
        generation: Generation(0),
    };
    assert!(store.select(request.clone(), Generation(0)).is_err());
    assert!(store.select(request, Generation(1)).is_err());
}

#[test]
fn cli_requires_a_new_choice_after_default_removal() {
    use trace_commons_contributor::managed::cli::{Snapshot, resolve};
    let snapshot: Snapshot = serde_json::from_value(serde_json::json!({"accounts":[{"id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","label":"Remaining","auth_state":"ready","verified_at":null}],"defaults":[],"generations":{"claude":2}})).unwrap();
    assert!(resolve(&snapshot, ToolId::Claude, None, None).is_err());
    assert!(resolve(&snapshot, ToolId::Claude, Some("Remaining"), None).is_ok());
}

#[test]
fn implicit_launch_uses_near_ai_even_with_a_subscription_default() {
    use trace_commons_contributor::managed::cli::{Snapshot, resolve};
    let near = uuid::Uuid::new_v4();
    let personal = uuid::Uuid::new_v4();
    let snapshot: Snapshot = serde_json::from_value(serde_json::json!({
        "accounts": [
            {"id":personal,"tool":"claude","connection":"subscription","label":"Personal","auth_state":"ready","verified_at":null},
            {"id":near,"tool":"claude","connection":"near_ai","label":"NEAR AI","auth_state":"ready","verified_at":null}
        ],
        "defaults":[{"tool":"claude","connection":"subscription","account_id":personal,"generation":1}],
        "generations":{"claude":1}
    })).unwrap();
    assert_eq!(
        resolve(&snapshot, ToolId::Claude, None, None).unwrap().id.0,
        near
    );
    assert_eq!(
        resolve(&snapshot, ToolId::Claude, Some("Personal"), None)
            .unwrap()
            .id
            .0,
        personal
    );
    assert!(resolve(&snapshot, ToolId::Codex, None, None).is_err());
}

#[test]
fn near_ai_default_is_required_only_when_multiple_accounts_exist() {
    use trace_commons_contributor::managed::cli::{Snapshot, resolve};
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    let mut snapshot: Snapshot = serde_json::from_value(serde_json::json!({
        "accounts": [
            {"id":first,"tool":"codex","connection":"near_ai","label":"One","auth_state":"unavailable","verified_at":null},
            {"id":second,"tool":"codex","connection":"near_ai","label":"Two","auth_state":"ready","verified_at":null}
        ], "defaults":[], "generations":{}
    })).unwrap();
    assert_eq!(
        resolve(&snapshot, ToolId::Codex, None, None),
        Err(ManagedError::AmbiguousAccount)
    );
    snapshot.defaults.push(Selection {
        tool: ToolId::Codex,
        connection: ConnectionKind::NearAi,
        account_id: snapshot.accounts[0].id,
        generation: Generation(0),
    });
    // An unavailable chosen account must not silently route to another account.
    assert_eq!(
        resolve(&snapshot, ToolId::Codex, None, None).unwrap().id.0,
        first
    );
    assert_eq!(
        resolve(&snapshot, ToolId::Codex, Some("Two"), None)
            .unwrap()
            .id
            .0,
        second
    );
    snapshot.defaults.clear();
    snapshot.accounts.remove(1);
    assert_eq!(
        resolve(&snapshot, ToolId::Codex, None, None).unwrap().id.0,
        first
    );
}

#[test]
fn account_list_exposes_all_choices_and_unambiguous_commands_without_label_execution() {
    use trace_commons_contributor::managed::cli::{Snapshot, format_accounts};
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    let snapshot: Snapshot = serde_json::from_value(serde_json::json!({
        "accounts":[
            {"id":first,"tool":"claude","connection":"near_ai","label":"Shared $(touch /tmp/never)","auth_state":"ready","verified_at":null},
            {"id":second,"tool":"claude","connection":"subscription","label":"Shared $(touch /tmp/never)","auth_state":"sign_in_required","verified_at":null}
        ],"defaults":[],"generations":{}
    })).unwrap();
    let output = format_accounts(&snapshot, false).unwrap();
    assert!(output.contains("NEAR AI | Ready | yes"));
    assert!(output.contains("Subscription | Needs sign-in"));
    let commands: Vec<_> = output
        .lines()
        .filter(|line| line.starts_with("  near-ai launch"))
        .collect();
    assert_eq!(
        commands,
        vec![
            format!("  near-ai launch claude --account {first}"),
            format!("  near-ai launch claude --account {second}")
        ]
    );
    let rows: serde_json::Value =
        serde_json::from_str(&format_accounts(&snapshot, true).unwrap()).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["launch_default"], true);
    assert_eq!(rows[1]["launch_default"], false);
    assert_eq!(rows[0]["id"], first.to_string());
}

#[test]
fn no_near_ai_account_never_uses_a_saved_subscription_default() {
    use trace_commons_contributor::managed::cli::{Snapshot, resolve};
    let id = uuid::Uuid::new_v4();
    let snapshot: Snapshot = serde_json::from_value(serde_json::json!({
        "accounts":[{"id":id,"tool":"claude","connection":"subscription","label":"Personal","auth_state":"ready","verified_at":null}],
        "defaults":[{"tool":"claude","connection":"subscription","account_id":id,"generation":0}], "generations":{}
    })).unwrap();
    assert_eq!(
        resolve(&snapshot, ToolId::Claude, None, None),
        Err(ManagedError::NotFound)
    );
    assert_eq!(
        resolve(
            &snapshot,
            ToolId::Claude,
            Some("Personal"),
            Some(ConnectionKind::NearAi)
        ),
        Err(ManagedError::NotFound)
    );
    assert_eq!(
        resolve(&snapshot, ToolId::Claude, Some(&id.to_string()), None)
            .unwrap()
            .id
            .0,
        id
    );
    assert_eq!(
        resolve(&snapshot, ToolId::Codex, Some(&id.to_string()), None),
        Err(ManagedError::NotFound)
    );
}

const CLAUDE_SIGN_IN: &str = "6e656172-2d61-4900-8000-000000000001";
const CODEX_SIGN_IN: &str = "6e656172-2d61-4900-8000-000000000002";

fn signed_in(
    accounts: serde_json::Value,
    defaults: serde_json::Value,
) -> trace_commons_contributor::managed::cli::Snapshot {
    serde_json::from_value(serde_json::json!({
        "accounts": accounts,
        "sign_in_accounts": [
            {"id":CLAUDE_SIGN_IN,"tool":"claude","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready","verified_at":null},
            {"id":CODEX_SIGN_IN,"tool":"codex","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready","verified_at":null}
        ],
        "defaults": defaults,
        "generations": {}
    }))
    .unwrap()
}

#[test]
fn signed_in_with_no_saved_accounts_launches_on_the_sign_in() {
    use trace_commons_contributor::managed::cli::resolve;
    // The daemon lists the derived sign-in selections in `defaults`.
    let snapshot = signed_in(
        serde_json::json!([]),
        serde_json::json!([
            {"tool":"claude","connection":"near_ai_signin","account_id":CLAUDE_SIGN_IN,"generation":0},
            {"tool":"codex","connection":"near_ai_signin","account_id":CODEX_SIGN_IN,"generation":0}
        ]),
    );
    for (tool, id) in [
        (ToolId::Claude, CLAUDE_SIGN_IN),
        (ToolId::Codex, CODEX_SIGN_IN),
    ] {
        let account = resolve(&snapshot, tool, None, None).unwrap();
        assert_eq!(account.id.to_string(), id);
        assert_eq!(account.connection, ConnectionKind::NearAiSignIn);
        // --account reaches the sign-in entry by its ID.
        assert_eq!(
            resolve(&snapshot, tool, Some(id), None)
                .unwrap()
                .id
                .to_string(),
            id
        );
    }
    // Naming the key backend does not fall back to the sign-in.
    assert_eq!(
        resolve(
            &snapshot,
            ToolId::Claude,
            None,
            Some(ConnectionKind::NearAi)
        ),
        Err(ManagedError::NotFound)
    );
    // OpenCode has no sign-in entry.
    assert_eq!(
        resolve(&snapshot, ToolId::Opencode, None, None),
        Err(ManagedError::NotFound)
    );
}

#[test]
fn signed_in_with_a_legacy_subscription_default_launches_on_the_sign_in() {
    use trace_commons_contributor::managed::cli::resolve;
    let personal = uuid::Uuid::new_v4();
    let snapshot = signed_in(
        serde_json::json!([
            {"id":personal,"tool":"claude","connection":"subscription","label":"Personal","auth_state":"ready","verified_at":null}
        ]),
        serde_json::json!([
            {"tool":"claude","connection":"subscription","account_id":personal,"generation":1},
            {"tool":"claude","connection":"near_ai_signin","account_id":CLAUDE_SIGN_IN,"generation":1},
            {"tool":"codex","connection":"near_ai_signin","account_id":CODEX_SIGN_IN,"generation":0}
        ]),
    );
    let implicit = resolve(&snapshot, ToolId::Claude, None, None).unwrap();
    assert_eq!(implicit.id.to_string(), CLAUDE_SIGN_IN);
    // An explicit --connection subscription still honours the legacy default.
    assert_eq!(
        resolve(
            &snapshot,
            ToolId::Claude,
            None,
            Some(ConnectionKind::Subscription)
        )
        .unwrap()
        .id
        .0,
        personal
    );
}

#[test]
fn a_saved_near_ai_key_wins_over_the_sign_in_and_several_need_a_choice() {
    use trace_commons_contributor::managed::cli::{format_accounts, resolve};
    let one = uuid::Uuid::new_v4();
    let two = uuid::Uuid::new_v4();
    let key = |id: uuid::Uuid, label: &str| serde_json::json!({"id":id,"tool":"claude","connection":"near_ai","label":label,"auth_state":"ready","verified_at":null});
    // A sole key wins, even though the sign-in is present.
    let sole = signed_in(serde_json::json!([key(one, "One")]), serde_json::json!([]));
    assert_eq!(
        resolve(&sole, ToolId::Claude, None, None).unwrap().id.0,
        one
    );
    // Several keys with no default are ambiguous, never the sign-in.
    let mut several = signed_in(
        serde_json::json!([key(one, "One"), key(two, "Two")]),
        serde_json::json!([]),
    );
    assert_eq!(
        resolve(&several, ToolId::Claude, None, None),
        Err(ManagedError::AmbiguousAccount)
    );
    several.defaults.push(Selection {
        tool: ToolId::Claude,
        connection: ConnectionKind::NearAi,
        account_id: several.accounts[1].id,
        generation: Generation(1),
    });
    assert_eq!(
        resolve(&several, ToolId::Claude, None, None).unwrap().id.0,
        two
    );
    // The listing shows the sign-in rows and marks one default per tool.
    let text = format_accounts(&several, false).unwrap();
    assert!(
        text.contains("NEAR AI | claude | NEAR AI sign-in | Ready | —"),
        "{text}"
    );
    assert!(
        text.contains("NEAR AI | codex | NEAR AI sign-in | Ready | yes"),
        "{text}"
    );
    assert!(text.contains(&format!("  near-ai launch codex --account {CODEX_SIGN_IN}")));
    let rows: serde_json::Value =
        serde_json::from_str(&format_accounts(&several, true).unwrap()).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let defaults: Vec<_> = rows
        .iter()
        .filter(|r| r["launch_default"] == true)
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(defaults, vec![two.to_string(), CODEX_SIGN_IN.to_owned()]);
}
