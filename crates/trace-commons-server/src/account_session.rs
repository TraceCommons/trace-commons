use std::collections::BTreeSet;

/// Durable pseudonymous account id (locally-owned UUID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountId(uuid::Uuid);
impl AccountId {
    pub fn from_uuid(u: uuid::Uuid) -> Self {
        Self(u)
    }
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

/// The ONLY ownership-bearing principal set for the account read surface.
/// Producible only within this module (and by `AccountCtx` later), never
/// convertible into the `&TenantAuth` shape the legacy
/// `visible_submission_records` helper wants.
#[derive(Debug, Clone)]
pub struct AccountPrincipalSet(BTreeSet<String>);
impl AccountPrincipalSet {
    // Crate-private constructor: only this module and the forthcoming
    // `AccountCtx` (a later slice task) may mint a principal set. Today only
    // the in-module tests call it, so under `-D warnings` the non-test bins
    // build sees it as dead code; the allow keeps the not-yet-wired
    // constructor in place for that task without widening visibility.
    #[allow(dead_code)]
    pub(crate) fn from_iter<I: IntoIterator<Item = String>>(it: I) -> Self {
        Self(it.into_iter().collect())
    }
    pub fn contains(&self, principal_ref: &str) -> bool {
        self.0.contains(principal_ref)
    }
    pub fn as_slice(&self) -> Vec<String> {
        self.0.iter().cloned().collect()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Actor/audit ref for the cookie path. Reserved-prefix literal, NOT hashed,
/// so it is structurally incapable of equalling any `principal_<sha>` ref.
pub fn account_actor_ref(account: &AccountId) -> String {
    format!("account-actor:{}", account.as_uuid())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_principal_set_membership() {
        let set = AccountPrincipalSet::from_iter(["principal_abc".to_string()]);
        assert!(set.contains("principal_abc"));
        assert!(!set.contains("principal_xyz"));
    }
    #[test]
    fn account_actor_ref_is_not_sha_shaped() {
        let actor = account_actor_ref(&AccountId::from_uuid(uuid::Uuid::nil()));
        assert!(actor.starts_with("account-actor:"));
        assert!(!actor.starts_with("principal_"));
    }
}
