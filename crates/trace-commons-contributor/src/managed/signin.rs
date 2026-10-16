//! Sessions that run on the app's NEAR AI sign-in.
//!
//! These entries are derived, never saved: one per tool, present exactly
//! while the daemon holds a NEAR AI sign-in. Nothing here is written to the
//! managed accounts store, and nothing here sees the credential itself; the
//! daemon reads it at launch time and hands it only to the session's proxy.

use super::{AccountId, AccountView, AuthState, ConnectionKind, Generation, Selection, ToolId};
use uuid::Uuid;

const CLAUDE: Uuid = Uuid::from_u128(0x6e65_6172_2d61_4900_8000_0000_0000_0001);
const CODEX: Uuid = Uuid::from_u128(0x6e65_6172_2d61_4900_8000_0000_0000_0002);

pub const TOOLS: [ToolId; 2] = [ToolId::Claude, ToolId::Codex];

/// The stable id of a tool's sign-in entry. Stable so the tool's isolated
/// profile directory survives between launches. OpenCode has no sign-in entry.
pub fn account_id(tool: ToolId) -> Option<AccountId> {
    Some(AccountId(match tool {
        ToolId::Claude => CLAUDE,
        ToolId::Codex => CODEX,
        ToolId::Opencode => return None,
    }))
}

/// The tool whose sign-in entry this id is, if it is one.
pub fn tool_for(id: AccountId) -> Option<ToolId> {
    TOOLS.into_iter().find(|tool| account_id(*tool) == Some(id))
}

pub fn entry(tool: ToolId) -> Option<AccountView> {
    Some(AccountView {
        id: account_id(tool)?,
        tool,
        connection: ConnectionKind::NearAiSignIn,
        label: super::copy::copy()["near_ai"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        auth_state: AuthState::Ready,
        verified_at: None,
    })
}

/// Both tools' entries while signed in; none otherwise.
pub fn entries(signed_in: bool) -> Vec<AccountView> {
    if signed_in {
        TOOLS.into_iter().filter_map(entry).collect()
    } else {
        vec![]
    }
}

/// The defaults a shell sees: every saved choice, plus a tool's sign-in
/// entry where it is the tool's implicit NEAR AI launch, which is while
/// signed in with no saved NEAR AI default and no saved NEAR AI key for that
/// tool. A saved subscription or API-key default stays listed, for a launch
/// that names that backend, but it does not keep the sign-in out.
pub fn defaults(
    saved: Vec<Selection>,
    accounts: &[AccountView],
    signed_in: bool,
    generation: impl Fn(ToolId) -> Generation,
) -> Vec<Selection> {
    let mut out = saved;
    if signed_in {
        for tool in TOOLS {
            let Some(account_id) = account_id(tool) else {
                continue;
            };
            let near_ai = |connection: ConnectionKind| connection == ConnectionKind::NearAi;
            if !out.iter().any(|s| s.tool == tool && near_ai(s.connection))
                && !accounts
                    .iter()
                    .any(|a| a.tool == tool && near_ai(a.connection))
            {
                out.push(Selection {
                    tool,
                    connection: ConnectionKind::NearAiSignIn,
                    account_id,
                    generation: generation(tool),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_follow_the_sign_in() {
        assert!(entries(false).is_empty());
        assert_eq!(account_id(ToolId::Opencode), None);
        assert_eq!(entry(ToolId::Opencode), None);
        let present = entries(true);
        assert_eq!(present.len(), 2);
        for (account, tool) in present.iter().zip(TOOLS) {
            assert_eq!(account.tool, tool);
            assert_eq!(account.connection, ConnectionKind::NearAiSignIn);
            assert_eq!(account.auth_state, AuthState::Ready);
            assert_eq!(tool_for(account.id), Some(tool));
        }
        assert_ne!(account_id(ToolId::Claude), account_id(ToolId::Codex));
        assert_eq!(tool_for(AccountId(Uuid::new_v4())), None);
    }

    #[test]
    fn the_sign_in_fills_the_near_ai_slot_beside_a_legacy_default() {
        let saved = Selection {
            tool: ToolId::Claude,
            connection: ConnectionKind::Subscription,
            account_id: AccountId(Uuid::new_v4()),
            generation: Generation(3),
        };
        // A legacy subscription default stays listed, and the sign-in is
        // still the NEAR AI default for both tools.
        let out = defaults(vec![saved.clone()], &[], true, |_| Generation(7));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], saved);
        assert_eq!(out[1].tool, ToolId::Claude);
        assert_eq!(out[1].connection, ConnectionKind::NearAiSignIn);
        assert_eq!(out[2].tool, ToolId::Codex);
        assert_eq!(Some(out[2].account_id), account_id(ToolId::Codex));
        assert_eq!(out[2].generation, Generation(7));
        assert_eq!(
            defaults(vec![saved.clone()], &[], false, |_| Generation(7)),
            vec![saved]
        );
        assert!(defaults(vec![], &[], false, |_| Generation(0)).is_empty());
    }

    #[test]
    fn a_saved_near_ai_key_keeps_the_sign_in_out() {
        let key = AccountView {
            id: AccountId(Uuid::new_v4()),
            tool: ToolId::Claude,
            connection: ConnectionKind::NearAi,
            label: "Work".into(),
            auth_state: AuthState::Ready,
            verified_at: None,
        };
        let out = defaults(vec![], &[key], true, |_| Generation(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tool, ToolId::Codex);
        assert_eq!(out[0].connection, ConnectionKind::NearAiSignIn);
    }
}
