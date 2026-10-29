//! Wire contract for referral codes and applying one
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections
//! 2, 3 and 6).
//!
//! Five routes, all on the contributor's signed-in account session:
//!
//! - `POST /v1/account/referral-codes` makes a code and returns its text once
//!   ([`ReferralCodeCreated`]).
//! - `GET /v1/account/referral-codes` lists the account's codes with the
//!   number of people each referred and the bonus they earned
//!   ([`ReferralCodesList`]).
//! - `POST /v1/account/referral-codes/{code_id}/disable` turns one off
//!   ([`ReferralCodeDisabled`]). Existing referrals keep earning.
//! - `POST /v1/account/referral` applies a code ([`ApplyReferralRequest`],
//!   answered by [`ReferralApplied`]).
//! - `GET /v1/account/referral` says whether a code is applied
//!   ([`ReferralStatus`]).
//!
//! The NEAR AI and wallet provisioning finish requests also take an optional
//! `referral_code`; their answers then carry `referral`, a
//! [`ReferralOutcome`]. Provisioning never fails because of a code.
//!
//! **One outward refusal.** Every refused apply answers the same label,
//! [`REFERRAL_NOT_APPLIED`], whatever the reason, so a caller cannot probe
//! which codes exist or whose they are. The precise reason is recorded only
//! in the server's own audit rows.
//!
//! **Codes.** Ten random Crockford base32 symbols (50 bits), shown as
//! `TC-XXXXX-XXXXX`. [`normalize_referral_code`] reads what a person typed:
//! case, `-` and spaces do not matter, `I` and `L` read as `1`, `O` as `0`,
//! and the `TC` prefix is optional. Codes are server-made only.
//!
//! Amounts are nano-USD (the `near_ai` rail's atomic unit), as whole numbers.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The contract version every response carries.
pub const REFERRAL_CONTRACT_VERSION: &str = "referral-v1";

/// The prefix a code is shown with. Optional on input.
pub const REFERRAL_CODE_PREFIX: &str = "TC";

/// Symbols in a code, not counting the prefix or separators.
pub const REFERRAL_CODE_SYMBOLS: usize = 10;

/// Random bits in a code: five per Crockford symbol.
pub const REFERRAL_CODE_BITS: u32 = 50;

/// The Crockford base32 alphabet, in value order. No `I`, `L`, `O` or `U`.
pub const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The most live codes one account may hold (spec section 2, "Limits").
pub const MAX_LIVE_REFERRAL_CODES: u32 = 5;

/// The longest input [`normalize_referral_code`] reads. Anything longer is
/// not a code, and is refused before it is looked at.
pub const MAX_REFERRAL_CODE_INPUT_LEN: usize = 64;

/// The one label every refused apply answers with.
pub const REFERRAL_NOT_APPLIED: &str = "referral_not_applied";

/// Referral codes are not open: the program has not started, or has ended,
/// or this server does not run it. Creating a code answers this.
pub const REFERRAL_UNAVAILABLE: &str = "referral_unavailable";

/// The account already has [`MAX_LIVE_REFERRAL_CODES`] live codes.
pub const REFERRAL_CODE_LIMIT_REACHED: &str = "referral_code_limit_reached";

/// No code with that id belongs to the account.
pub const REFERRAL_CODE_NOT_FOUND: &str = "referral_code_not_found";

/// The account is closed.
pub const REFERRAL_ACCOUNT_INELIGIBLE: &str = "referral_account_ineligible";

/// The request body does not parse.
pub const INVALID_REFERRAL_REQUEST: &str = "invalid_referral_request";

/// The canonical form of a typed code: ten Crockford symbols, upper case,
/// no prefix and no separators. `None` when the input is not a code.
///
/// The rules (spec section 2): ASCII case is ignored; `-` and whitespace are
/// removed; `I` and `L` read as `1` and `O` as `0`; a leading `TC` is
/// dropped when twelve symbols remain. `U` and anything outside the alphabet
/// refuse the whole input.
pub fn normalize_referral_code(input: &str) -> Option<String> {
    if input.len() > MAX_REFERRAL_CODE_INPUT_LEN {
        return None;
    }
    let mut symbols = String::with_capacity(REFERRAL_CODE_SYMBOLS + 2);
    for character in input.chars() {
        if character == '-' || character.is_whitespace() {
            continue;
        }
        if !character.is_ascii() {
            return None;
        }
        symbols.push(character.to_ascii_uppercase());
    }
    // The prefix is two valid symbols, so only the length tells it apart
    // from the code itself.
    let body = if symbols.len() == REFERRAL_CODE_SYMBOLS + REFERRAL_CODE_PREFIX.len()
        && symbols.starts_with(REFERRAL_CODE_PREFIX)
    {
        &symbols[REFERRAL_CODE_PREFIX.len()..]
    } else {
        symbols.as_str()
    };
    if body.len() != REFERRAL_CODE_SYMBOLS {
        return None;
    }
    body.bytes()
        .map(|byte| match byte {
            b'I' | b'L' => Some('1'),
            b'O' => Some('0'),
            other if CROCKFORD_ALPHABET.contains(&other) => Some(other as char),
            _ => None,
        })
        .collect()
}

/// How a canonical code is shown: `TC-XXXXX-XXXXX`. `None` for anything
/// that is not already canonical.
pub fn format_referral_code(normalized: &str) -> Option<String> {
    if normalized.len() != REFERRAL_CODE_SYMBOLS
        || !normalized
            .bytes()
            .all(|byte| CROCKFORD_ALPHABET.contains(&byte))
    {
        return None;
    }
    let (first, second) = normalized.split_at(REFERRAL_CODE_SYMBOLS / 2);
    Some(format!("{REFERRAL_CODE_PREFIX}-{first}-{second}"))
}

/// The canonical code for the low [`REFERRAL_CODE_BITS`] bits of `bits`,
/// most significant symbol first. Higher bits are ignored. The server draws
/// `bits` from the operating system's random source.
pub fn referral_code_from_bits(bits: u64) -> String {
    (0..REFERRAL_CODE_SYMBOLS)
        .rev()
        .map(|position| {
            let value = (bits >> (position * 5)) & 0x1f;
            CROCKFORD_ALPHABET[value as usize] as char
        })
        .collect()
}

/// A code's state as its owner sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferralCodeState {
    /// Accepts new referrals.
    Live,
    /// Its owner turned it off.
    Disabled,
    /// Paused by the abuse check; an operator may resume it.
    Suspended,
    /// Closed by an operator.
    Voided,
}

impl ReferralCodeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Disabled => "disabled",
            Self::Suspended => "suspended",
            Self::Voided => "voided",
        }
    }
}

/// The answer to creating a code. The only time the server sends a code it
/// has not been asked to list.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferralCodeCreated {
    pub contract_version: String,
    pub code_id: Uuid,
    /// `TC-XXXXX-XXXXX`.
    pub code: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for ReferralCodeCreated {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReferralCodeCreated")
            .field("code_id", &self.code_id)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

/// One of the account's codes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferralCodeSummary {
    pub code_id: Uuid,
    /// `TC-XXXXX-XXXXX`. `None` when the server cannot open the stored code
    /// (its key was rotated away); the code still works for anyone who has
    /// it.
    #[serde(default)]
    pub code: Option<String>,
    pub state: ReferralCodeState,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// People who joined with this code and whose referral stands.
    pub referee_count: u64,
    /// The bonus those people earned the owner, in nano-USD, net of
    /// reversals; never below zero.
    pub bonus_owed_nano_usd: u64,
}

impl std::fmt::Debug for ReferralCodeSummary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReferralCodeSummary")
            .field("code_id", &self.code_id)
            .field("state", &self.state)
            .field("referee_count", &self.referee_count)
            .finish_non_exhaustive()
    }
}

/// The answer to listing codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferralCodesList {
    pub contract_version: String,
    /// Oldest first.
    pub codes: Vec<ReferralCodeSummary>,
    /// [`MAX_LIVE_REFERRAL_CODES`].
    pub max_live_codes: u32,
    /// Every referral bonus the account earned, in nano-USD; never below
    /// zero.
    pub bonus_owed_nano_usd: u64,
    /// The part of it NEAR AI confirmed sending, in nano-USD.
    pub bonus_sent_nano_usd: u64,
}

/// The answer to turning a code off. Also the answer when it was already
/// off; `state` then says how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferralCodeDisabled {
    pub contract_version: String,
    pub code_id: Uuid,
    pub state: ReferralCodeState,
}

/// The body of `POST /v1/account/referral`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyReferralRequest {
    /// As typed; see [`normalize_referral_code`].
    pub code: String,
}

impl std::fmt::Debug for ApplyReferralRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplyReferralRequest")
            .finish_non_exhaustive()
    }
}

/// Whether a code was applied. The `referral` field of a provisioning
/// answer, and the `outcome` of [`ReferralApplied`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferralOutcome {
    Applied,
    NotApplied,
}

impl ReferralOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NotApplied => "not_applied",
        }
    }
}

/// How a code came to be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferralAppliedVia {
    /// With the provisioning request that created the account.
    Signup,
    /// Through `POST /v1/account/referral`.
    Later,
}

impl ReferralAppliedVia {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signup => "signup",
            Self::Later => "later",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "signup" => Some(Self::Signup),
            "later" => Some(Self::Later),
            _ => None,
        }
    }
}

/// The answer to a successful apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferralApplied {
    pub contract_version: String,
    /// Always `applied`.
    pub outcome: ReferralOutcome,
    pub applied_at: chrono::DateTime<chrono::Utc>,
    /// The end of the period in which the referrer earns from this account.
    pub earn_until: chrono::DateTime<chrono::Utc>,
}

/// The answer to `GET /v1/account/referral`. The referee sees that a code
/// was applied, never whose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferralStatus {
    pub contract_version: String,
    pub applied: bool,
    #[serde(default)]
    pub applied_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub applied_via: Option<ReferralAppliedVia>,
    /// Set while a referral is applied and stands.
    #[serde(default)]
    pub earn_until: Option<chrono::DateTime<chrono::Utc>>,
    /// While no code is applied and the program is open: the last moment
    /// one may be. `None` once that has passed.
    #[serde(default)]
    pub apply_until: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_codes_normalize_to_ten_crockford_symbols() {
        let canonical = "7K3M9QX2RD";
        for typed in [
            "TC-7K3M9-QX2RD",
            "tc-7k3m9-qx2rd",
            "7K3M9QX2RD",
            "7k3m9 qx2rd",
            " TC 7K3M9 QX2RD ",
            "tc7k3m9qx2rd",
            "7K3M9-QX2RD",
            "7-K-3-M-9-Q-X-2-R-D",
        ] {
            assert_eq!(
                normalize_referral_code(typed).as_deref(),
                Some(canonical),
                "{typed:?}"
            );
        }
        // I and L read as 1, O as 0.
        assert_eq!(
            normalize_referral_code("TC-IL0O1-ABCDE").as_deref(),
            Some("11001ABCDE")
        );
        assert_eq!(
            normalize_referral_code("ioliOLabcd").as_deref(),
            Some("101101ABCD")
        );
    }

    #[test]
    fn anything_that_is_not_a_code_is_refused() {
        for typed in [
            "",
            "TC",
            "TC-",
            "7K3M9QX2R",       // nine symbols
            "7K3M9QX2RDD",     // eleven
            "TC-7K3M9-QX2RDD", // twelve after the prefix
            "7K3M9QX2RU",      // U is not Crockford
            "7K3M9QX2R!",
            "7K3M9QX2R\u{00e9}",
            "TX-7K3M9-QX2RD", // twelve symbols, not the prefix
        ] {
            assert_eq!(normalize_referral_code(typed), None, "{typed:?}");
        }
        let long = format!("{}7K3M9QX2RD", " ".repeat(MAX_REFERRAL_CODE_INPUT_LEN));
        assert_eq!(normalize_referral_code(&long), None, "an over-long input");
    }

    #[test]
    fn a_ten_symbol_code_that_starts_with_tc_keeps_its_prefix_symbols() {
        assert_eq!(
            normalize_referral_code("TCABCDEFGH").as_deref(),
            Some("TCABCDEFGH")
        );
        assert_eq!(
            normalize_referral_code("TC-TCABC-DEFGH").as_deref(),
            Some("TCABCDEFGH")
        );
    }

    #[test]
    fn codes_are_shown_with_the_prefix_and_two_groups() {
        assert_eq!(
            format_referral_code("7K3M9QX2RD").as_deref(),
            Some("TC-7K3M9-QX2RD")
        );
        assert_eq!(format_referral_code("7k3m9qx2rd"), None, "not canonical");
        assert_eq!(format_referral_code("7K3M9QX2R"), None);
        assert_eq!(format_referral_code("7K3M9QX2RU"), None);
        let shown = format_referral_code("7K3M9QX2RD").unwrap();
        assert_eq!(
            normalize_referral_code(&shown).as_deref(),
            Some("7K3M9QX2RD"),
            "a shown code reads back as itself"
        );
    }

    #[test]
    fn fifty_bits_map_to_ten_symbols_and_back() {
        assert_eq!(referral_code_from_bits(0), "0000000000");
        assert_eq!(referral_code_from_bits((1 << 50) - 1), "ZZZZZZZZZZ");
        assert_eq!(
            referral_code_from_bits(u64::MAX),
            "ZZZZZZZZZZ",
            "bits above the fiftieth are ignored"
        );
        assert_eq!(referral_code_from_bits(1), "0000000001");
        assert_eq!(referral_code_from_bits(32), "0000000010");
        let bits: u64 = 0x2_3456_789a_bcde & ((1 << 50) - 1);
        let code = referral_code_from_bits(bits);
        assert_eq!(code.len(), REFERRAL_CODE_SYMBOLS);
        let decoded = code.bytes().fold(0u64, |acc, byte| {
            let value = CROCKFORD_ALPHABET.iter().position(|b| *b == byte).unwrap();
            (acc << 5) | value as u64
        });
        assert_eq!(decoded, bits);
        assert_eq!(
            normalize_referral_code(&code).as_deref(),
            Some(code.as_str())
        );
    }

    #[test]
    fn the_alphabet_is_crockford() {
        assert_eq!(CROCKFORD_ALPHABET.len(), 32);
        for excluded in [b'I', b'L', b'O', b'U'] {
            assert!(!CROCKFORD_ALPHABET.contains(&excluded));
        }
        let mut sorted = CROCKFORD_ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 32);
    }

    #[test]
    fn apply_requests_refuse_unknown_fields_and_never_print_the_code() {
        let parsed: ApplyReferralRequest =
            serde_json::from_value(serde_json::json!({"code": "TC-7K3M9-QX2RD"})).unwrap();
        assert_eq!(parsed.code, "TC-7K3M9-QX2RD");
        assert!(!format!("{parsed:?}").contains("7K3M9"));
        assert!(
            serde_json::from_value::<ApplyReferralRequest>(
                serde_json::json!({"code": "TC-7K3M9-QX2RD", "referrer": "x"})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<ApplyReferralRequest>(serde_json::json!({})).is_err());
    }

    #[test]
    fn created_and_listed_codes_are_not_printed() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let created = ReferralCodeCreated {
            contract_version: REFERRAL_CONTRACT_VERSION.into(),
            code_id: Uuid::nil(),
            code: "TC-7K3M9-QX2RD".into(),
            created_at: at,
        };
        assert!(!format!("{created:?}").contains("7K3M9"));
        let summary = ReferralCodeSummary {
            code_id: Uuid::nil(),
            code: Some("TC-7K3M9-QX2RD".into()),
            state: ReferralCodeState::Live,
            created_at: at,
            referee_count: 2,
            bonus_owed_nano_usd: 10,
        };
        assert!(!format!("{summary:?}").contains("7K3M9"));
    }

    #[test]
    fn wire_labels_match_their_spelling() {
        for (outcome, label) in [
            (ReferralOutcome::Applied, "applied"),
            (ReferralOutcome::NotApplied, "not_applied"),
        ] {
            assert_eq!(outcome.as_str(), label);
            assert_eq!(serde_json::to_value(outcome).unwrap(), label);
        }
        for state in [
            ReferralCodeState::Live,
            ReferralCodeState::Disabled,
            ReferralCodeState::Suspended,
            ReferralCodeState::Voided,
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), state.as_str());
        }
        for via in [ReferralAppliedVia::Signup, ReferralAppliedVia::Later] {
            assert_eq!(serde_json::to_value(via).unwrap(), via.as_str());
            assert_eq!(ReferralAppliedVia::parse(via.as_str()), Some(via));
        }
        assert_eq!(ReferralAppliedVia::parse("invite"), None);
        for label in [
            REFERRAL_NOT_APPLIED,
            REFERRAL_UNAVAILABLE,
            REFERRAL_CODE_LIMIT_REACHED,
            REFERRAL_CODE_NOT_FOUND,
            REFERRAL_ACCOUNT_INELIGIBLE,
            INVALID_REFERRAL_REQUEST,
        ] {
            assert!(
                label.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{label}"
            );
        }
    }

    #[test]
    fn status_reads_with_optional_fields_absent() {
        let status: ReferralStatus = serde_json::from_value(serde_json::json!({
            "contract_version": REFERRAL_CONTRACT_VERSION,
            "applied": false,
        }))
        .unwrap();
        assert!(!status.applied);
        assert_eq!(status.earn_until, None);
        assert_eq!(status.apply_until, None);
    }
}
