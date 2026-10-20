//! Recorded consent for subscription backends.
//!
//! `docs/TRUST.md` §2: a subscription backend stays off until the user says yes
//! once, to a specific prompt, in plain language. That answer is recorded with
//! the prompt version it answered, so that changing what we ask invalidates
//! consent given to the old wording rather than silently inheriting it.
//!
//! `ironclaw_llm` emits a `tracing::warn!` at this point. A warning in a log is
//! not consent; this is.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::login::{SubscriptionLogin, SubscriptionProduct};

/// Bumped whenever the consent prompt's *meaning* changes. A user who agreed
/// to v1 has not agreed to v2.
///
/// v2: `ironwire init` asks once for every subscription it found, rather than
/// once per backend, and the default moved from no to yes. Both change what a
/// keypress means, so consent given to v1 does not carry over.
///
/// Additional logins (a second Claude Code config dir, a second `CODEX_HOME`)
/// arrived without a bump, deliberately. Their prompt is new wording — it names
/// the directory — but no recorded answer can be to an older form of it: before
/// this there was no prompt for any id but `claude-sub` and `codex-sub`, and the
/// control API refused to record consent for one. So no existing answer is
/// being reinterpreted, and bumping would only withdraw every consent the
/// default logins hold for no reason. The same change fixed the Codex prompt
/// saying "Codex Code" — a misspelled name, not a different question.
///
/// Binding a record to the login it answered ([`ConsentRecord::location`])
/// needed no bump either: it narrows what an existing yes covers to what it was
/// actually given for, rather than reinterpreting it.
pub const CONSENT_PROMPT_VERSION: u32 = 2;

/// The question a user must answer before a subscription backend is used.
///
/// Data rather than `println!` calls, because there is now more than one surface
/// that has to ask it: `ironwire connect` in a terminal, and the menu bar app.
/// Two hand-written copies of a consent prompt is two prompts, and the one that
/// gets edited is never the one someone read — while `CONSENT_PROMPT_VERSION`
/// goes on claiming they answered the same question.
///
/// `docs/TRUST.md` §2 fixes the content. Changing what it *means* requires
/// bumping the version, which invalidates consent given to the old wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentPrompt {
    /// The version this wording is. Recorded with the answer.
    pub version: u32,
    /// Backend this grants, e.g. `claude-sub`.
    pub backend_id: String,
    /// Echoed with a grant so a restarted daemon cannot apply an old answer
    /// to another product or credential directory behind the same backend id.
    pub binding: String,
    /// What the user calls it, e.g. `Claude`.
    pub product: String,
    /// What IronWire will do, in one sentence.
    pub summary: String,
    /// What the user is taking on, one point at a time. Never softened, and
    /// never reordered so the cost reads last.
    pub points: Vec<String>,
    /// The question itself.
    pub question: String,
}

impl ConsentPrompt {
    /// The prompt for a default login's backend, or `None` where consent is
    /// not the gate.
    ///
    /// Shorthand for [`Self::for_login`] on `claude-sub` and `codex-sub`, for
    /// callers that have an id and no backend to ask. An unknown id yields
    /// `None` rather than a generic prompt: a consent screen that cannot name
    /// what it is about is not consent. That includes an additional login's id
    /// — its prompt names its directory, which only its backend knows.
    #[must_use]
    pub fn for_backend(backend_id: &str) -> Option<Self> {
        let product = SubscriptionProduct::for_default_backend_id(backend_id)?;
        Some(Self::for_login(
            &SubscriptionLogin::default_for(product),
            backend_id,
        ))
    }

    /// The prompt for one login, granted under `backend_id`.
    ///
    /// A login in its usual place gets the wording users have already
    /// answered. One with a directory gets the same four points, with the
    /// summary and question naming that directory — with two Claude logins,
    /// "enable the Claude subscription" does not say which account is being
    /// handed over.
    #[must_use]
    pub fn for_login(login: &SubscriptionLogin, backend_id: &str) -> Self {
        let product = login.product;
        let client = product.client();
        let host = product.host();
        let vendor = product.vendor();
        let alternative = match product {
            SubscriptionProduct::Claude => "an Anthropic API key",
            SubscriptionProduct::ChatGpt => "an OpenAI API key",
        };

        let (name, summary, question) = match &login.location {
            None => {
                // The name v2 shipped with. "ChatGPT" would read better, but
                // this is the text existing consent was given to.
                let name = match product {
                    SubscriptionProduct::Claude => "Claude",
                    SubscriptionProduct::ChatGpt => "Codex",
                };
                (
                    name,
                    format!(
                        "IronWire will read the OAuth token that {client} stores on this \
                         machine and send requests to {host} with it, from this computer only."
                    ),
                    format!("Enable the {name} subscription backend?"),
                )
            }
            Some(location) => {
                let name = product.name();
                (
                    name,
                    format!(
                        "IronWire will read the OAuth token that {client} stores for the \
                         login in {location} and send requests to {host} with it, from this \
                         computer only."
                    ),
                    format!("Enable the {name} subscription in {location}?"),
                )
            }
        };

        Self {
            version: CONSENT_PROMPT_VERSION,
            backend_id: backend_id.to_string(),
            binding: serde_json::json!([
                backend_id,
                CONSENT_PROMPT_VERSION,
                login.product,
                login.consent_location(),
                login.source_dir
            ])
            .to_string(),
            product: name.to_string(),
            summary,
            points: vec![
                format!(
                    "This uses a private authentication path. {vendor} does not document \
                     it and may change or block it at any time."
                ),
                format!(
                    "Using it from a third-party proxy may fall outside your subscription's \
                     intended use. If {vendor} objects, it is your account that is affected."
                ),
                format!("Your token is never sent anywhere except {host}."),
                format!("You can use {alternative} instead — fully supported, no ambiguity."),
            ],
            question,
        }
    }
}

/// One recorded consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// Prompt version the user answered.
    pub prompt_version: u32,
    /// When they answered.
    pub granted_at: DateTime<Utc>,
    /// The login it answered: [`SubscriptionLogin::consent_location`] at the
    /// time. `None` for a default login in its usual place.
    ///
    /// The backend id alone does not say which account was handed over —
    /// `config_dir` on `claude-sub` keeps the id and changes the account. So
    /// consent counts only while the backend still reads the login it was
    /// given for. A file written before this field existed answered for the
    /// unrelocated default, which is exactly what `None` means; and left out
    /// when `None`, such a record is written back byte for byte as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// All recorded consents, keyed by backend id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConsentLedger {
    entries: BTreeMap<String, ConsentRecord>,
}

impl ConsentLedger {
    /// Load from disk. A missing file is an empty ledger — the correct
    /// starting state, since it means nothing has been consented to.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Persist to disk.
    ///
    /// # Errors
    ///
    /// Propagates I/O and serialization failures; a consent we failed to record
    /// must not be treated as granted.
    /// Written atomically. [`Self::load`] fails closed on a corrupt file —
    /// correct, and it means a truncated write would silently withdraw *every*
    /// consent the user ever gave, not just fail to record the new one. Nothing
    /// about that symptom points at a crash (`ironwire_core::atomic`).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        ironwire_core::atomic::write(path, &text)
    }

    /// Whether `backend_id` has current consent for a login in its usual
    /// place — [`Self::is_granted_for`] with no location.
    ///
    /// For a backend that reads a login, ask [`Self::is_granted_for`] with
    /// [`SubscriptionLogin::consent_location`] instead. This does not know
    /// where that login is now, so for a relocated or additional one it
    /// answers about a login that backend does not read.
    #[must_use]
    pub fn is_granted(&self, backend_id: &str) -> bool {
        self.is_granted_for(backend_id, None)
    }

    /// Whether `backend_id` has current consent for the login at `location`
    /// ([`SubscriptionLogin::consent_location`]).
    ///
    /// Consent recorded against an older prompt version does not count, and
    /// nor does consent recorded for a login somewhere else: the user agreed
    /// to hand over the account in that directory, not whichever one the id
    /// reads now. The answer to a moved login is to ask again.
    #[must_use]
    pub fn is_granted_for(&self, backend_id: &str, location: Option<&str>) -> bool {
        self.entries
            .get(backend_id)
            .is_some_and(|r| answers(r, location))
    }

    /// Record consent for `backend_id`'s login in its usual place —
    /// [`Self::grant_for`] with no location.
    pub fn grant(&mut self, backend_id: &str, now: DateTime<Utc>) {
        self.grant_for(backend_id, None, now);
    }

    /// Record consent for `backend_id`, given to the login at `location`
    /// ([`SubscriptionLogin::consent_location`]).
    ///
    /// Replaces whatever was recorded for the id: one backend reads one login,
    /// so a consent for its old location has nothing left to answer.
    pub fn grant_for(&mut self, backend_id: &str, location: Option<&str>, now: DateTime<Utc>) {
        self.entries.insert(
            backend_id.to_string(),
            ConsentRecord {
                prompt_version: CONSENT_PROMPT_VERSION,
                granted_at: now,
                location: location.map(str::to_string),
            },
        );
    }

    /// Withdraw consent for `backend_id`, whichever login it was given for.
    ///
    /// Deliberately not location-bound: a user taking consent back must never
    /// be told the record was for some other directory and left in place.
    pub fn revoke(&mut self, backend_id: &str) {
        self.entries.remove(backend_id);
    }

    /// Backends with current consent for a login in its usual place, sorted —
    /// [`Self::granted_for`] with no location for any of them.
    #[must_use]
    pub fn granted(&self) -> Vec<&str> {
        self.granted_for(|_| None)
    }

    /// Backends with current consent, sorted, where `location_of` says where
    /// each backend's login is now ([`SubscriptionLogin::consent_location`]).
    #[must_use]
    pub fn granted_for(&self, location_of: impl Fn(&str) -> Option<String>) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(id, r)| answers(r, location_of(id).as_deref()))
            .map(|(id, _)| id.as_str())
            .collect()
    }
}

/// Whether `record` is current consent to the login at `location`.
fn answers(record: &ConsentRecord, location: Option<&str>) -> bool {
    record.prompt_version >= CONSENT_PROMPT_VERSION && record.location.as_deref() == location
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
    }

    #[test]
    fn nothing_is_consented_by_default() {
        let ledger = ConsentLedger::default();
        assert!(!ledger.is_granted("claude-sub"));
        assert!(ledger.granted().is_empty());
    }

    #[test]
    fn a_missing_file_is_an_empty_ledger_not_a_crash() {
        let ledger = ConsentLedger::load(Path::new("/nonexistent/consent.json"));
        assert!(!ledger.is_granted("claude-sub"));
    }

    #[test]
    fn grant_and_revoke_round_trip_through_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("consent.json");

        let mut ledger = ConsentLedger::default();
        ledger.grant("claude-sub", now());
        ledger.save(&path).expect("saves");

        let reloaded = ConsentLedger::load(&path);
        assert!(reloaded.is_granted("claude-sub"));
        assert_eq!(reloaded.granted(), vec!["claude-sub"]);

        let mut reloaded = reloaded;
        reloaded.revoke("claude-sub");
        reloaded.save(&path).expect("saves");
        assert!(!ConsentLedger::load(&path).is_granted("claude-sub"));
    }

    #[test]
    fn consent_to_an_older_prompt_does_not_carry_forward() {
        // If we change what we're asking, the old yes doesn't answer it.
        let json = r#"{"claude-sub": {"prompt_version": 0, "granted_at": "2026-01-01T00:00:00Z"}}"#;
        let ledger: ConsentLedger = serde_json::from_str(json).expect("parses");
        assert!(!ledger.is_granted("claude-sub"));
    }
}

#[cfg(test)]
mod durability_tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn a_corrupt_ledger_grants_nothing() {
        // Fail-closed is the only acceptable direction here: a consent ledger
        // that fails *open* would use someone's subscription because a file got
        // truncated.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("consent.json");
        std::fs::write(&path, "{\"entries\": {\"claude-sub\"").expect("write");

        let ledger = ConsentLedger::load(&path);
        assert!(!ledger.is_granted("claude-sub"));
    }

    #[test]
    fn a_missing_ledger_grants_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = ConsentLedger::load(&dir.path().join("absent.json"));
        assert!(!ledger.is_granted("claude-sub"));
    }

    #[test]
    fn saving_leaves_no_partial_file_and_no_temp_file() {
        // The pairing that makes atomicity matter: `load` fails closed, so a
        // truncated write does not fail — it silently withdraws every consent
        // the user ever gave, and nothing about that points at a crash.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("consent.json");

        let mut ledger = ConsentLedger::default();
        ledger.grant("claude-sub", Utc::now());
        ledger.grant("codex-sub", Utc::now());
        ledger.save(&path).expect("saves");

        let reloaded = ConsentLedger::load(&path);
        assert!(reloaded.is_granted("claude-sub"));
        assert!(reloaded.is_granted("codex-sub"));

        let entries: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["consent.json".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn the_ledger_is_not_writable_by_other_local_users() {
        // Another local user granting a consent on this user's behalf would be
        // a straightforward way around `docs/TRUST.md` §2.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("consent.json");
        ConsentLedger::default().save(&path).expect("saves");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o077, 0, "group or other can access it: {mode:o}");
    }

    /// Both surfaces that ask for consent read the same wording from here. A
    /// second copy would drift, and the recorded version would go on claiming
    /// both users answered the same question.
    #[test]
    fn every_backend_that_requires_consent_has_a_prompt() {
        for backend in ["claude-sub", "codex-sub"] {
            let prompt = ConsentPrompt::for_backend(backend).expect("has a prompt");
            assert_eq!(prompt.backend_id, backend);
            assert_eq!(prompt.version, CONSENT_PROMPT_VERSION);
            assert!(!prompt.summary.is_empty());
            assert!(!prompt.question.is_empty());
        }
    }

    /// A consent screen that cannot name what it is about is not consent, so an
    /// unknown backend gets nothing rather than a generic form of words.
    #[test]
    fn an_unknown_backend_has_no_prompt_rather_than_a_generic_one() {
        assert!(ConsentPrompt::for_backend("nearai").is_none());
        assert!(ConsentPrompt::for_backend("").is_none());
    }

    /// `docs/TRUST.md` §2 fixes what has to be said. These are the four things a
    /// user is agreeing to, and none of them may quietly go missing.
    #[test]
    fn the_prompt_states_the_cost_and_not_only_the_benefit() {
        let prompt = ConsentPrompt::for_backend("claude-sub").expect("has a prompt");
        let all = prompt.points.join(" ");
        assert!(
            all.contains("does not document"),
            "the path being private: {all}"
        );
        assert!(
            all.contains("your account that is affected"),
            "who bears the risk: {all}"
        );
        assert!(
            all.contains("never sent anywhere except"),
            "where the token goes: {all}"
        );
        assert!(all.contains("instead"), "the alternative: {all}");
        assert!(
            prompt.summary.contains("this computer only"),
            "the scope: {}",
            prompt.summary
        );
    }

    /// The prompt names the product and the host it will talk to, because
    /// "enable the subscription backend" on its own does not say what happens.
    #[test]
    fn the_prompt_names_the_product_and_the_host() {
        let claude = ConsentPrompt::for_backend("claude-sub").expect("has a prompt");
        assert!(
            claude.summary.contains("api.anthropic.com"),
            "{}",
            claude.summary
        );
        assert!(claude.question.contains("Claude"), "{}", claude.question);

        let codex = ConsentPrompt::for_backend("codex-sub").expect("has a prompt");
        assert!(codex.summary.contains("chatgpt.com"), "{}", codex.summary);
        assert!(codex.question.contains("Codex"), "{}", codex.question);
    }

    /// The prompt is stored as sentences, not pre-broken lines: a terminal wraps
    /// it to its width and a menu lays it out in a view. Embedded newlines would
    /// make one of those two look wrong.
    #[test]
    fn the_prompt_carries_no_layout_of_its_own() {
        let additional = ConsentPrompt::for_login(&work_login(), "claude-work");
        for prompt in [
            ConsentPrompt::for_backend("claude-sub").expect("has a prompt"),
            additional,
        ] {
            for text in std::iter::once(&prompt.summary).chain(&prompt.points) {
                assert!(!text.contains('\n'), "{text}");
                assert!(
                    !text.contains("  "),
                    "double spaces suggest hand-wrapping: {text}"
                );
            }
        }
    }

    fn work_login() -> SubscriptionLogin {
        SubscriptionLogin {
            product: SubscriptionProduct::Claude,
            location: Some("~/.claude-work".into()),
            config_dir: Some("/Users/alice/.claude-work".into()),
            source_dir: None,
            default: false,
        }
    }

    /// The default logins' wording is what existing consent was given to under
    /// this version. Pinned word for word so a change to it has to be a
    /// decision, made next to `CONSENT_PROMPT_VERSION`.
    #[test]
    fn the_default_claude_prompt_is_the_question_users_already_answered() {
        let prompt = ConsentPrompt::for_backend("claude-sub").expect("has a prompt");
        assert_eq!(prompt.product, "Claude");
        assert_eq!(
            prompt.summary,
            "IronWire will read the OAuth token that Claude Code stores on this machine \
             and send requests to api.anthropic.com with it, from this computer only."
        );
        assert_eq!(prompt.question, "Enable the Claude subscription backend?");
        assert_eq!(
            prompt,
            ConsentPrompt::for_login(
                &SubscriptionLogin::default_for(SubscriptionProduct::Claude),
                "claude-sub"
            )
        );
    }

    /// v2 rendered "the OAuth token that Codex Code stores". There is no such
    /// product, and a consent screen that misnames what it reads is a worse
    /// consent screen.
    #[test]
    fn the_codex_prompt_names_codex_and_not_a_product_that_does_not_exist() {
        let prompt = ConsentPrompt::for_backend("codex-sub").expect("has a prompt");
        assert!(!prompt.summary.contains("Codex Code"), "{}", prompt.summary);
        assert_eq!(
            prompt.summary,
            "IronWire will read the OAuth token that Codex stores on this machine \
             and send requests to chatgpt.com with it, from this computer only."
        );
        assert_eq!(prompt.question, "Enable the Codex subscription backend?");
    }

    /// With two Claude logins, "enable the Claude subscription" does not say
    /// which account is being handed over. The prompt for one names it.
    #[test]
    fn consent_for_an_additional_login_names_its_directory() {
        let prompt = ConsentPrompt::for_login(&work_login(), "claude-work");
        assert_eq!(prompt.backend_id, "claude-work");
        assert_eq!(prompt.version, CONSENT_PROMPT_VERSION);
        assert_eq!(
            prompt.summary,
            "IronWire will read the OAuth token that Claude Code stores for the login in \
             ~/.claude-work and send requests to api.anthropic.com with it, from this \
             computer only."
        );
        assert_eq!(
            prompt.question,
            "Enable the Claude subscription in ~/.claude-work?"
        );

        let codex = ConsentPrompt::for_login(
            &SubscriptionLogin {
                product: SubscriptionProduct::ChatGpt,
                location: Some("~/.codex-work".into()),
                config_dir: Some("/Users/alice/.codex-work".into()),
                source_dir: None,
                default: false,
            },
            "codex-work",
        );
        assert!(
            codex
                .summary
                .contains("that Codex stores for the login in ~/.codex-work"),
            "{}",
            codex.summary
        );
        assert!(codex.summary.contains("chatgpt.com"), "{}", codex.summary);
        assert_eq!(
            codex.question,
            "Enable the ChatGPT subscription in ~/.codex-work?"
        );
    }

    /// Naming the directory must not cost a point: an additional login's user
    /// is agreeing to exactly what a default login's user agrees to.
    #[test]
    fn an_additional_login_is_told_the_same_four_things() {
        let default = ConsentPrompt::for_backend("claude-sub").expect("has a prompt");
        let additional = ConsentPrompt::for_login(&work_login(), "claude-work");
        assert_eq!(additional.points, default.points);
    }

    /// An additional login's id has no shorthand prompt: without its backend
    /// there is no directory to name, and a prompt that cannot say which login
    /// it is about is not consent to that login.
    #[test]
    fn an_additional_logins_id_alone_has_no_prompt() {
        assert!(ConsentPrompt::for_backend("claude-work").is_none());
        assert!(ConsentPrompt::for_backend("codex-work").is_none());
    }
}

/// Consent answers one login, not one backend id (`docs/TRUST.md` §2).
#[cfg(test)]
mod location_tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{ClaudeLogin, CodexLogin};

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
    }

    fn alice() -> Option<PathBuf> {
        Some(PathBuf::from("/Users/alice"))
    }

    // Whether the login is behind the default id plays no part in which
    // login a consent answers — a relocated `claude-sub` is exactly the case
    // where the id stays and the login changes — so these never ask.
    fn claude(config_dir: Option<&str>) -> SubscriptionLogin {
        ClaudeLogin::with_home(config_dir.map(str::to_string), alice()).describe(false)
    }

    fn codex(codex_home: Option<&str>) -> SubscriptionLogin {
        CodexLogin::with_home(codex_home.map(PathBuf::from), alice()).describe(false)
    }

    #[test]
    fn moving_claude_sub_to_another_directory_withdraws_its_consent() {
        let usual = claude(None);
        let moved = claude(Some("~/.claude-main"));
        let mut ledger = ConsentLedger::default();
        ledger.grant_for("claude-sub", usual.consent_location(), now());

        assert!(ledger.is_granted_for("claude-sub", usual.consent_location()));
        assert!(
            !ledger.is_granted_for("claude-sub", moved.consent_location()),
            "a yes to the login in ~/.claude was carried to the account in ~/.claude-main"
        );

        // And back: a yes given to the moved login is not one for ~/.claude.
        ledger.grant_for("claude-sub", moved.consent_location(), now());
        assert!(ledger.is_granted_for("claude-sub", moved.consent_location()));
        assert!(!ledger.is_granted_for("claude-sub", usual.consent_location()));
        assert!(!ledger.is_granted("claude-sub"));
    }

    #[test]
    fn moving_codex_sub_to_another_home_withdraws_its_consent() {
        let usual = codex(None);
        let moved = codex(Some("~/.codex-main"));
        let mut ledger = ConsentLedger::default();
        ledger.grant_for("codex-sub", usual.consent_location(), now());

        assert!(ledger.is_granted_for("codex-sub", usual.consent_location()));
        assert!(!ledger.is_granted_for("codex-sub", moved.consent_location()));
    }

    #[test]
    fn an_additional_logins_consent_does_not_survive_its_config_dir_changing() {
        let work = claude(Some("~/.claude-work"));
        let mut ledger = ConsentLedger::default();
        ledger.grant_for("claude-work", work.consent_location(), now());
        assert!(ledger.is_granted_for("claude-work", work.consent_location()));

        for elsewhere in ["~/.claude-work2", "/srv/claude-work"] {
            let moved = claude(Some(elsewhere));
            assert!(
                !ledger.is_granted_for("claude-work", moved.consent_location()),
                "consent to ~/.claude-work survived the entry moving to {elsewhere}"
            );
        }

        // A trailing slash is a different Keychain item, so a different login
        // as far as Claude Code is concerned — and so as far as consent is.
        let slashed = claude(Some("~/.claude-work/"));
        assert!(!ledger.is_granted_for("claude-work", slashed.consent_location()));

        // Spelling the same directory another way is the same login: what
        // counts is the string the client sees, after the shell expands `~`.
        let spelled_out = claude(Some("/Users/alice/.claude-work"));
        assert!(ledger.is_granted_for("claude-work", spelled_out.consent_location()));

        let codex_work = codex(Some("~/.codex-work"));
        ledger.grant_for("codex-work", codex_work.consent_location(), now());
        assert!(!ledger.is_granted_for(
            "codex-work",
            codex(Some("~/.codex-other")).consent_location()
        ));
    }

    /// The other half of binding consent to a login: an id-only grant — what
    /// every caller recorded before — is not consent to a login with a
    /// directory, so nothing written the old way can stand in for one.
    #[test]
    fn a_consent_recorded_without_a_location_does_not_answer_for_a_login_with_one() {
        let work = claude(Some("~/.claude-work"));
        let mut ledger = ConsentLedger::default();
        ledger.grant("claude-work", now());
        assert!(!ledger.is_granted_for("claude-work", work.consent_location()));
    }

    #[test]
    fn an_old_consent_file_still_grants_the_unrelocated_default() {
        // Exactly what v2 wrote, before consent named a location.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("consent.json");
        std::fs::write(
            &path,
            r#"{
  "claude-sub": {
    "prompt_version": 2,
    "granted_at": "2026-01-01T00:00:00Z"
  },
  "codex-sub": {
    "prompt_version": 2,
    "granted_at": "2026-01-01T00:00:00Z"
  }
}"#,
        )
        .expect("write");
        let ledger = ConsentLedger::load(&path);

        assert!(ledger.is_granted("claude-sub"));
        assert!(ledger.is_granted_for("claude-sub", claude(None).consent_location()));
        assert!(ledger.is_granted_for("codex-sub", codex(None).consent_location()));
        assert_eq!(ledger.granted(), vec!["claude-sub", "codex-sub"]);

        // What it answered was the login in the usual place, and only that.
        assert!(!ledger.is_granted_for(
            "claude-sub",
            claude(Some("~/.claude-main")).consent_location()
        ));
        assert!(
            !ledger.is_granted_for("codex-sub", codex(Some("~/.codex-main")).consent_location())
        );
    }

    #[test]
    fn a_consents_location_round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("consent.json");
        let work = claude(Some("~/.claude-work"));

        let mut ledger = ConsentLedger::default();
        ledger.grant_for("claude-work", work.consent_location(), now());
        ledger.grant("claude-sub", now());
        ledger.save(&path).expect("saves");

        let text = std::fs::read_to_string(&path).expect("read");
        assert!(
            text.contains(r#""location": "/Users/alice/.claude-work""#),
            "{text}"
        );

        let reloaded = ConsentLedger::load(&path);
        assert_eq!(reloaded, ledger);
        assert!(reloaded.is_granted_for("claude-work", work.consent_location()));
        assert!(reloaded.is_granted("claude-sub"));
    }

    /// A user who goes back to a build from before this field must still find
    /// their default logins' consent where they left it. Stricter than that
    /// build actually is — it ignored unknown fields — so this pins the
    /// stronger property: for a login in its usual place the file is
    /// unchanged.
    #[test]
    fn a_ledger_this_version_writes_is_still_readable_by_the_old_schema() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct OldRecord {
            prompt_version: u32,
            granted_at: DateTime<Utc>,
        }

        let mut ledger = ConsentLedger::default();
        ledger.grant("claude-sub", now());
        ledger.grant_for("codex-sub", codex(None).consent_location(), now());
        let text = serde_json::to_string_pretty(&ledger).expect("serializes");
        assert!(!text.contains("location"), "{text}");

        let old: BTreeMap<String, OldRecord> =
            serde_json::from_str(&text).expect("the old schema reads it");
        assert_eq!(old.len(), 2);
        for record in old.values() {
            assert_eq!(record.prompt_version, CONSENT_PROMPT_VERSION);
            assert_eq!(record.granted_at, now());
        }
    }

    #[test]
    fn withdrawing_consent_removes_it_whichever_login_it_was_given_for() {
        let moved = claude(Some("~/.claude-main"));
        let mut ledger = ConsentLedger::default();
        ledger.grant_for("claude-sub", moved.consent_location(), now());

        ledger.revoke("claude-sub");
        assert!(!ledger.is_granted_for("claude-sub", moved.consent_location()));
        assert_eq!(ledger, ConsentLedger::default(), "a record was left behind");
    }

    #[test]
    fn only_backends_whose_consent_answers_their_login_now_are_listed_as_granted() {
        let work = claude(Some("~/.claude-work"));
        let mut ledger = ConsentLedger::default();
        ledger.grant("claude-sub", now());
        ledger.grant_for("claude-work", work.consent_location(), now());
        ledger.grant_for("claude-play", Some("/Users/alice/.claude-play"), now());

        let now_at = |id: &str| match id {
            "claude-work" => work.config_dir.clone(),
            // Moved since it was consented to.
            "claude-play" => Some("/Users/alice/.claude-games".to_string()),
            _ => None,
        };
        assert_eq!(
            ledger.granted_for(now_at),
            vec!["claude-sub", "claude-work"]
        );
        assert_eq!(ledger.granted(), vec!["claude-sub"]);
    }

    /// Withdrawn consent is asked for again, and the question has to say
    /// which account it is now about.
    #[test]
    fn a_moved_default_login_is_asked_about_by_its_new_directory() {
        let prompt = ConsentPrompt::for_login(&claude(Some("~/.claude-main")), "claude-sub");
        assert_eq!(
            prompt.question,
            "Enable the Claude subscription in ~/.claude-main?"
        );
    }
}

#[cfg(test)]
mod grant_binding_tests {
    use super::*;
    #[test]
    fn default_grants_bind_the_actual_source_even_when_legacy_ledger_identity_is_unbound() {
        let mut first = SubscriptionLogin::default_for(SubscriptionProduct::ChatGpt);
        first.source_dir = Some("/codex/first".into());
        let mut second = first.clone();
        second.source_dir = Some("/codex/second".into());
        assert_eq!(first.consent_location(), None);
        assert_eq!(second.consent_location(), None);
        assert_ne!(
            ConsentPrompt::for_login(&first, "codex-sub").binding,
            ConsentPrompt::for_login(&second, "codex-sub").binding
        );
    }
}
