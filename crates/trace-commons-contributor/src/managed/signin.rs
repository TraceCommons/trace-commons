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
const OPENCODE: Uuid = Uuid::from_u128(0x6e65_6172_2d61_4900_8000_0000_0000_0003);

/// OpenCode starts on its session proxy's default model, as Claude Code and
/// Codex do; the daemon tells its launcher that model, and every model the
/// proxy serves, with the port.
pub const TOOLS: [ToolId; 3] = [ToolId::Claude, ToolId::Codex, ToolId::Opencode];

/// The stable id of a tool's sign-in entry. Stable so the tool's isolated
/// profile directory survives between launches. Optional so a tool added
/// later can have none.
pub fn account_id(tool: ToolId) -> Option<AccountId> {
    Some(AccountId(match tool {
        ToolId::Claude => CLAUDE,
        ToolId::Codex => CODEX,
        ToolId::Opencode => OPENCODE,
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

/// Every tool's entry while signed in; none otherwise.
pub fn entries(signed_in: bool) -> Vec<AccountView> {
    if signed_in {
        TOOLS.into_iter().filter_map(entry).collect()
    } else {
        vec![]
    }
}

/// The defaults a shell sees. A choice the person saved wins; a tool with
/// no saved choice defaults to its sign-in entry while signed in.
pub fn defaults(
    saved: Vec<Selection>,
    signed_in: bool,
    generation: impl Fn(ToolId) -> Generation,
) -> Vec<Selection> {
    let mut out = saved;
    if signed_in {
        for tool in TOOLS {
            let Some(account_id) = account_id(tool) else {
                continue;
            };
            if !out.iter().any(|s| s.tool == tool) {
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
        let present = entries(true);
        assert_eq!(present.len(), 3);
        assert_eq!(
            present.iter().map(|a| a.tool).collect::<Vec<_>>(),
            vec![ToolId::Claude, ToolId::Codex, ToolId::Opencode]
        );
        for (account, tool) in present.iter().zip(TOOLS) {
            assert_eq!(account.tool, tool);
            assert_eq!(account.connection, ConnectionKind::NearAiSignIn);
            assert_eq!(account.auth_state, AuthState::Ready);
            assert_eq!(tool_for(account.id), Some(tool));
        }
        assert_ne!(account_id(ToolId::Claude), account_id(ToolId::Codex));
        assert_ne!(account_id(ToolId::Claude), account_id(ToolId::Opencode));
        assert_ne!(account_id(ToolId::Codex), account_id(ToolId::Opencode));
        // Stable, so OpenCode's isolated profile survives between launches.
        assert_eq!(
            account_id(ToolId::Opencode).map(|id| id.0.to_string()),
            Some("6e656172-2d61-4900-8000-000000000003".to_owned())
        );
        assert_eq!(tool_for(AccountId(Uuid::new_v4())), None);
    }

    #[test]
    fn a_saved_default_wins_and_an_unset_one_falls_to_the_sign_in() {
        let saved = Selection {
            tool: ToolId::Claude,
            connection: ConnectionKind::Subscription,
            account_id: AccountId(Uuid::new_v4()),
            generation: Generation(3),
        };
        let out = defaults(vec![saved.clone()], true, |_| Generation(7));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], saved);
        assert_eq!(out[1].tool, ToolId::Codex);
        assert_eq!(Some(out[1].account_id), account_id(ToolId::Codex));
        assert_eq!(out[1].generation, Generation(7));
        // OpenCode with nothing saved defaults to the sign-in too.
        assert_eq!(out[2].tool, ToolId::Opencode);
        assert_eq!(out[2].connection, ConnectionKind::NearAiSignIn);
        assert_eq!(Some(out[2].account_id), account_id(ToolId::Opencode));
        assert_eq!(
            defaults(vec![saved.clone()], false, |_| Generation(7)),
            vec![saved]
        );
        assert!(defaults(vec![], false, |_| Generation(0)).is_empty());
    }
}
