//! Minting placeholders, and the map that reverses them.
//!
//! The design decision this module exists to enforce is in `docs/PRIVACY.md`
//! §4: **the map is derived fresh from each request and never persisted.** A
//! stored plaintext-to-placeholder map would be a purpose-built PII database
//! created in the course of trying not to expose PII, and it would outlive
//! every `--clear` a user thought had cleared it.
//!
//! That works because coding agents are stateless over HTTP and resend their
//! whole history every turn, so the same plaintext is visible again and the
//! same map is rebuilt. For that to hold, minting must be **deterministic**
//! within a conversation and **different** across conversations — hence a
//! per-conversation salt, held in memory, never written down.

use std::collections::HashMap;

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The class of value a placeholder stands in for.
///
/// Carried in the token so a person reading a diff can see *what* was replaced
/// without being able to recover it, and so format-preserving surrogates can be
/// chosen per class (`docs/PRIVACY.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// An API key, token, or private key.
    Secret,
    /// A value the user nominated by name.
    Named,
    /// An email address.
    Email,
    /// An IP address.
    IpAddress,
    /// A phone number.
    Phone,
    /// Something a classifier flagged that has no narrower class.
    Personal,
}

impl Class {
    /// Short, stable slug used inside a placeholder.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Named => "named",
            Self::Email => "email",
            Self::IpAddress => "ip",
            Self::Phone => "phone",
            Self::Personal => "personal",
        }
    }
}

/// Delimiters chosen so a placeholder cannot be produced by ordinary text and
/// survives JSON encoding without escaping.
///
/// Not `<<...>>` or `{{...}}`: both appear in template languages and in code a
/// coding agent is routinely editing, and a false match on the way back would
/// corrupt a user's file. `⟦⟧` (U+27E6/U+27E7) appears in essentially no
/// source code, needs no JSON escaping, and is visually obvious in a diff.
pub const OPEN: &str = "⟦";
/// Closing delimiter. See [`OPEN`].
pub const CLOSE: &str = "⟧";

/// How many hex characters of the digest to keep.
///
/// 12 hex characters is 48 bits. Long enough that a collision within one
/// conversation is not a practical concern — a collision would map two
/// different values to one token and reverse them both to whichever was seen
/// last, which is the one silent-corruption failure this module can have. Short
/// enough that a placeholder does not dominate the text around it, which
/// matters because the model has to reason about that text.
const DIGEST_HEX: usize = 12;

/// The longest a placeholder can be, for the streaming reverser's buffer bound.
///
/// `⟦` + longest slug + `.` + digest + `⟧`, with the delimiters at 3 bytes each
/// in UTF-8.
#[must_use]
pub fn max_placeholder_len() -> usize {
    let longest_slug = ["secret", "named", "email", "ip", "phone", "personal"]
        .iter()
        .map(|s| s.len())
        .max()
        .unwrap_or(8);
    OPEN.len() + longest_slug + 1 + DIGEST_HEX + CLOSE.len()
}

/// A per-conversation minting key.
///
/// Random, memory-only, and dropped with the conversation. Two consequences,
/// both wanted: the same email in two conversations gets different
/// placeholders, and a token carries nothing an offline attacker could use to
/// recover its plaintext.
#[derive(Clone)]
pub struct Salt([u8; 32]);

impl std::fmt::Debug for Salt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Printing it would put the one secret this module has into a log.
        f.write_str("Salt(<redacted>)")
    }
}

impl Salt {
    /// A fresh random salt from the OS CSPRNG.
    ///
    /// # Panics
    ///
    /// If the OS random source is unavailable. Deriving one from the clock
    /// instead would make placeholders predictable across conversations, which
    /// is exactly the property the salt exists to provide — so failing loudly
    /// is the only correct behaviour.
    #[must_use]
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the OS random source is available");
        Self(bytes)
    }

    /// A fixed salt, for tests that need reproducibility.
    #[must_use]
    pub fn fixed(seed: u8) -> Self {
        Self([seed; 32])
    }
}

/// Mint the placeholder for one value.
///
/// Deterministic within a conversation: the same plaintext always yields the
/// same token, which is what makes the map re-derivable next turn.
#[must_use]
pub fn placeholder(salt: &Salt, class: Class, plaintext: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(&salt.0).expect("HMAC accepts any key length");
    // Domain-separate by class, so the same string flagged as two different
    // classes does not collapse into one token.
    mac.update(class.slug().as_bytes());
    mac.update(b"\x00");
    mac.update(plaintext.as_bytes());
    let digest = mac.finalize().into_bytes();

    let mut hex = String::with_capacity(DIGEST_HEX);
    for byte in digest.iter().take(DIGEST_HEX.div_ceil(2)) {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex.truncate(DIGEST_HEX);

    format!("{OPEN}{}.{hex}{CLOSE}", class.slug())
}

/// The substitutions made for one request.
///
/// Lives for the lifetime of the request and is dropped with it. There is
/// deliberately no `save`, no `load`, and no `Serialize`.
#[derive(Debug, Default)]
pub struct Map {
    /// placeholder → plaintext
    reverse: HashMap<String, String>,
    /// plaintext → placeholder, so repeated values reuse one token
    forward: HashMap<String, String>,
}

impl Map {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a substitution, returning the placeholder to use.
    ///
    /// Idempotent: the same plaintext always gets the same token back, so a
    /// value appearing forty times in a replayed history costs one entry.
    pub fn insert(&mut self, salt: &Salt, class: Class, plaintext: &str) -> String {
        if let Some(existing) = self.forward.get(plaintext) {
            return existing.clone();
        }
        let token = placeholder(salt, class, plaintext);
        self.forward.insert(plaintext.to_string(), token.clone());
        self.reverse.insert(token.clone(), plaintext.to_string());
        token
    }

    /// The plaintext behind a placeholder **this request minted**.
    ///
    /// `None` for anything else, including a placeholder-shaped string the
    /// model invented and a stale token from a previous salt. We never map a
    /// token we did not issue (`docs/PRIVACY.md` §4).
    #[must_use]
    pub fn plaintext(&self, placeholder: &str) -> Option<&str> {
        self.reverse.get(placeholder).map(String::as_str)
    }

    /// How many distinct values were substituted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.forward.len()
    }

    /// Whether anything was substituted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }

    /// Every placeholder this request minted.
    pub fn placeholders(&self) -> impl Iterator<Item = &str> {
        self.reverse.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minting_is_deterministic_within_a_conversation() {
        // The property the whole per-request-map design rests on: next turn we
        // see the same plaintext and must produce the same token.
        let salt = Salt::fixed(7);
        let a = placeholder(&salt, Class::Email, "alice@corp.com");
        let b = placeholder(&salt, Class::Email, "alice@corp.com");
        assert_eq!(a, b);
    }

    #[test]
    fn two_conversations_never_share_a_placeholder() {
        // So a token leaked from one session says nothing about another, and
        // so an offline attacker cannot build a rainbow table.
        let a = placeholder(&Salt::fixed(1), Class::Email, "alice@corp.com");
        let b = placeholder(&Salt::fixed(2), Class::Email, "alice@corp.com");
        assert_ne!(a, b);
    }

    #[test]
    fn the_class_is_part_of_the_identity() {
        let salt = Salt::fixed(3);
        assert_ne!(
            placeholder(&salt, Class::Email, "10.0.0.1"),
            placeholder(&salt, Class::IpAddress, "10.0.0.1")
        );
    }

    #[test]
    fn a_placeholder_says_its_class_and_nothing_else() {
        let token = placeholder(&Salt::fixed(4), Class::Email, "alice@corp.com");
        assert!(token.starts_with(OPEN));
        assert!(token.ends_with(CLOSE));
        assert!(token.contains("email"));
        assert!(
            !token.contains("alice") && !token.contains("corp"),
            "the token leaked its plaintext: {token}"
        );
    }

    #[test]
    fn the_delimiters_do_not_occur_in_code_a_coding_agent_edits() {
        // `<<...>>` and `{{...}}` both appear in template languages and in
        // source a coding agent routinely rewrites; a false match on the way
        // back would corrupt a user's file.
        let token = placeholder(&Salt::fixed(5), Class::Secret, "sk-x");
        for hostile in ["<<", ">>", "{{", "}}", "${", "%(", "#{"] {
            assert!(!token.contains(hostile), "{token} contains {hostile}");
        }
        // And it needs no JSON escaping.
        let encoded = serde_json::to_string(&token).expect("encodes");
        assert!(!encoded.contains('\\'), "{encoded} needed escaping");
    }

    #[test]
    fn every_placeholder_fits_the_declared_bound() {
        // The streaming reverser sizes its buffer from this. A placeholder
        // longer than the bound would be split and never reassembled.
        let salt = Salt::fixed(6);
        for class in [
            Class::Secret,
            Class::Named,
            Class::Email,
            Class::IpAddress,
            Class::Phone,
            Class::Personal,
        ] {
            let token = placeholder(&salt, class, "some value");
            assert!(
                token.len() <= max_placeholder_len(),
                "{token} is {} bytes, bound is {}",
                token.len(),
                max_placeholder_len()
            );
        }
    }

    #[test]
    fn a_map_reuses_one_token_for_a_repeated_value() {
        // A value appearing forty times in a replayed history is one entry.
        let salt = Salt::fixed(8);
        let mut map = Map::new();
        let a = map.insert(&salt, Class::Email, "alice@corp.com");
        let b = map.insert(&salt, Class::Email, "alice@corp.com");
        assert_eq!(a, b);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn a_map_never_reverses_a_token_it_did_not_mint() {
        // The model can invent placeholder-shaped strings, and a stale token
        // from a previous salt can arrive in replayed history. Reversing
        // either to a real value would be the worst bug this module could have.
        let salt = Salt::fixed(9);
        let mut map = Map::new();
        map.insert(&salt, Class::Email, "alice@corp.com");

        assert_eq!(map.plaintext("⟦email.deadbeefcafe⟧"), None);
        assert_eq!(map.plaintext("⟦secret.000000000000⟧"), None);
        assert_eq!(map.plaintext("not a placeholder"), None);
    }

    #[test]
    fn a_salt_never_prints_itself() {
        let salt = Salt::fixed(0xAB);
        let rendered = format!("{salt:?}");
        assert!(rendered.contains("redacted"));
        assert!(!rendered.contains("ab"), "got {rendered}");
    }

    #[test]
    fn a_random_salt_differs_from_the_next_one() {
        let a = placeholder(&Salt::random(), Class::Email, "x@y.z");
        let b = placeholder(&Salt::random(), Class::Email, "x@y.z");
        assert_ne!(a, b);
    }
}
