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
