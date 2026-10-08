//! Metadata only. Native credentials are owned by isolated native profiles.

use super::{
    AccountId, AccountView, AuthState, ConnectionKind, Generation, ManagedError, Selection, ToolId,
};
use crate::config::ConfigStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

const FILE: &str = "managed-accounts.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    schema_version: u32,
    accounts: Vec<AccountView>,
    defaults: BTreeMap<ToolId, Selection>,
    #[serde(default)]
    generations: BTreeMap<ToolId, Generation>,
}

/// The daemon serializes mutations. Failed persistence never updates the live view.
pub struct AccountStore {
    store: ConfigStore,
    data: Data,
}

impl AccountStore {
    pub fn open(store: &ConfigStore) -> Result<Self, ManagedError> {
        let data = match store
            .read_daemon_file(FILE)
            .map_err(|_| ManagedError::StorageUnavailable)?
        {
            Some(bytes) => {
                let data: Data =
                    serde_json::from_slice(&bytes).map_err(|_| ManagedError::StorageUnavailable)?;
                if data.schema_version != 1
                    || data
                        .accounts
                        .iter()
                        .enumerate()
                        .any(|(i, a)| data.accounts[..i].iter().any(|b| b.id == a.id))
                {
                    return Err(ManagedError::StorageUnavailable);
                }
                for (tool, selection) in &data.defaults {
                    if *tool != selection.tool
                        || !data.accounts.iter().any(|a| {
                            a.id == selection.account_id
                                && a.tool == *tool
                                && a.connection == selection.connection
                        })
                    {
                        return Err(ManagedError::StorageUnavailable);
                    }
                }
                data
            }
            None => Data {
                schema_version: 1,
                accounts: vec![],
                defaults: BTreeMap::new(),
                generations: BTreeMap::new(),
            },
        };
        Ok(Self {
            store: ConfigStore::open(store.dir().to_path_buf())
                .map_err(|_| ManagedError::StorageUnavailable)?,
            data,
        })
    }

    pub fn list(&self) -> Vec<AccountView> {
        self.data.accounts.clone()
    }

    pub fn selection(&self, tool: ToolId) -> Option<Selection> {
        self.data.defaults.get(&tool).cloned()
    }

    pub fn generation(&self, tool: ToolId) -> Generation {
        self.data
            .generations
            .get(&tool)
            .copied()
            .unwrap_or_else(|| {
                self.selection(tool)
                    .map(|s| s.generation)
                    .unwrap_or_default()
            })
    }

    pub fn update_auth(
        &mut self,
        id: AccountId,
        state: AuthState,
    ) -> Result<AccountView, ManagedError> {
        let mut next = self.data.clone();
        let account = next
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(ManagedError::NotFound)?;
        account.auth_state = state;
        account.verified_at = Some(chrono::Utc::now());
        let view = account.clone();
        self.commit(next)?;
        Ok(view)
    }

    pub fn rename(&mut self, id: AccountId, label: &str) -> Result<AccountView, ManagedError> {
        let label = label.trim();
        if label.is_empty() || label.len() > 128 || label.chars().any(char::is_control) {
            return Err(ManagedError::InvalidRequest);
        }
        let mut next = self.data.clone();
        let account = next
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(ManagedError::NotFound)?;
        account.label = label.into();
        let view = account.clone();
        self.commit(next)?;
        Ok(view)
    }

    /// Caller must first ensure no session or native login holds this profile.
    pub fn remove(&mut self, id: AccountId) -> Result<(), ManagedError> {
        if !self.data.accounts.iter().any(|a| a.id == id) {
            return Err(ManagedError::NotFound);
        }
        let mut next = self.data.clone();
        next.accounts.retain(|a| a.id != id);
        for (tool, selection) in &self.data.defaults {
            if selection.account_id == id {
                next.generations.insert(
                    *tool,
                    Generation(
                        self.generation(*tool)
                            .0
                            .checked_add(1)
                            .ok_or(ManagedError::Conflict)?,
                    ),
                );
            }
        }
        next.defaults.retain(|_, s| s.account_id != id);
        self.commit(next)
    }

    pub fn add(
        &mut self,
        tool: ToolId,
        connection: ConnectionKind,
        label: &str,
    ) -> Result<AccountView, ManagedError> {
        let label = label.trim();
        if label.is_empty() || label.len() > 128 || label.chars().any(char::is_control) {
            return Err(ManagedError::InvalidRequest);
        }
        let account = AccountView {
            id: AccountId(Uuid::new_v4()),
            tool,
            connection,
            label: label.into(),
            auth_state: AuthState::SignInRequired,
            verified_at: None,
        };
        let mut next = self.data.clone();
        next.accounts.push(account.clone());
        self.commit(next)?;
        Ok(account)
    }

    pub fn resolve(&self, tool: ToolId, label_or_id: &str) -> Result<AccountId, ManagedError> {
        if let Ok(id) = Uuid::parse_str(label_or_id) {
            return self
                .data
                .accounts
                .iter()
                .find(|a| a.id.0 == id && a.tool == tool)
                .map(|a| a.id)
                .ok_or(ManagedError::NotFound);
        }
        let mut matches = self
            .data
            .accounts
            .iter()
            .filter(|a| a.tool == tool && a.label == label_or_id);
        let first = matches.next().ok_or(ManagedError::NotFound)?;
        if matches.next().is_some() {
            return Err(ManagedError::AmbiguousAccount);
        }
        Ok(first.id)
    }

    pub fn select(
        &mut self,
        mut selection: Selection,
        expected: Generation,
    ) -> Result<Selection, ManagedError> {
        let current = self.generation(selection.tool);
        if current != expected
            || !self.data.accounts.iter().any(|a| {
                a.id == selection.account_id
                    && a.tool == selection.tool
                    && a.connection == selection.connection
            })
        {
            return Err(ManagedError::Conflict);
        }
        selection.generation = Generation(current.0.checked_add(1).ok_or(ManagedError::Conflict)?);
        let mut next = self.data.clone();
        next.generations
            .insert(selection.tool, selection.generation);
        next.defaults.insert(selection.tool, selection.clone());
        self.commit(next)?;
        Ok(selection)
    }

    fn commit(&mut self, next: Data) -> Result<(), ManagedError> {
        let bytes =
            serde_json::to_vec_pretty(&next).map_err(|_| ManagedError::StorageUnavailable)?;
        self.store
            .write_daemon_file(FILE, &bytes)
            .map_err(|_| ManagedError::StorageUnavailable)?;
        self.data = next;
        Ok(())
    }
}
