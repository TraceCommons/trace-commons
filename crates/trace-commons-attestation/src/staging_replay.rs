//! The RTMR3 replay and the compose-manager check a staging build makes
//! before it trusts a NEAR AI model key (staging design, owner decision S16,
//! sections 5.2 and 5.5).
//!
//! A staging pin set pins `mrtd`, `rtmr0-2` and `mrconfigid`, never a static
//! `rtmr3`: dstack extends RTMR3 at boot with a per-boot random
//! `instance-id`, so a static value pins one instance and every restart is a
//! mismatch. What RTMR3 records instead is checked here, in two halves.
//!
//! # The RTMR3 replay
//!
//! The model entry's own `event_log` is replayed. Only events with `imr` 3
//! take part. For each, the digest is **recomputed**, never taken from the
//! log:
//!
//! ```text
//! digest = SHA-384(event_type as 4 bytes little-endian || ":" || event || ":" || payload)
//! ```
//!
//! where `payload` is the hex-decoded `event_payload`. NEAR AI ships these
//! events with an empty `digest`; one whose recorded digest is non-empty and
//! differs from the recomputed one is refused, or a log could carry the right
//! digests beside invented payloads. Then from 48 zero bytes, `rtmr3 =
//! SHA-384(rtmr3 || digest)` for each event in log order, and the result must
//! equal the **verified** quote's RTMR3. Only after that are the events read:
//! they must be exactly [`RTMR3_EVENTS`], each once, in that order, each with
//! `event_type` [`RTMR3_EVENT_TYPE`], and the `compose-hash` event must carry
//! the compose hash in the verified quote's MRCONFIGID (config-id v1: `01`,
//! then the 32-byte compose hash). [`replay_rtmr3`] does all of that and needs
//! no allowlist, so the re-sign tool's `observe` runs it too.
//! [`check_rtmr3_allowlist`] then checks the values a signed allowlist names:
//! app id, KMS, OS image, key provider and the four progress markers.
//! `instance-id` is accepted with any value: it is the one the replay exists
//! to absorb.
//!
//! # The compose-manager record
//!
//! Post-boot compose actions do not extend RTMR3. The model entry's
//! `compose_manager_attestation` records them under its own quote, and
//! [`check_compose_manager`] requires that quote to verify (through the
//! caller's verifier, with an up-to-date TCB), to carry registers identical to
//! the model quote's (the same TD and the same boot), our nonce at
//! `report_data[32..64]`, and at `report_data[0..32]` the SHA-256 of the
//! `actions` list serialised as JSON with sorted keys and compact separators
//! ([`compose_actions_hash`]). The record's own `actions_hash` must equal that
//! recomputed value. With an allowlist, every action (kind, container, image;
//! never the timestamp) must be listed in it.
//!
//! # Failing closed
//!
//! Every function here returns a [`ReplayRefusal`] on the first check that
//! fails, and a refusal earns no key. A refusal is a label: no event name,
//! payload, digest, register or nonce is carried, so it can go into a status
//! reply or a log line as it is. [`ReplayedEvent`] and [`ComposeAction`] do
//! not print their payloads under `{:?}`.

use std::fmt;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256, Sha384};

use crate::quote::{REQUIRED_TCB_STATUS, VerifiedQuote};

/// The `event_type` every RTMR3 event carries: dstack's runtime event type.
pub const RTMR3_EVENT_TYPE: u32 = 0x0800_0001;

/// The RTMR3 events a replayed log must hold: exactly these, each once, in
/// this order (the order seen live on 2026-10-22).
pub const RTMR3_EVENTS: [&str; 10] = [
    "system-preparing",
    "app-id",
    "compose-hash",
    "instance-id",
    "boot-mr-done",
    "mr-kms",
    "os-image-hash",
    "key-provider",
    "storage-fs",
    "system-ready",
];

/// The progress markers among [`RTMR3_EVENTS`], whose payloads the allowlist
/// lists one by one.
pub const MARKER_EVENTS: [&str; 4] = [
    "system-preparing",
    "boot-mr-done",
    "storage-fs",
    "system-ready",
];

/// The one RTMR3 event whose value is never checked and never printed.
pub const INSTANCE_ID_EVENT: &str = "instance-id";

/// The keys every compose action carries, and no others.
const ACTION_KEYS: [&str; 4] = ["action", "container", "image", "timestamp"];

/// Why an entry's RTMR3 log or compose-manager record was refused. Every
/// refusal earns no key for that entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayRefusal {
    /// The model entry carries no `event_log`.
    LogMissing,
    /// The log is not a list of events, an RTMR3 event lacks a field or has
    /// one of the wrong type, or a payload is not hex.
    LogMalformed,
    /// An RTMR3 event's recorded digest is non-empty and is not the digest of
    /// its own type, name and payload.
    EventDigestMismatch,
    /// The replayed RTMR3 is not the verified quote's.
    ReplayMismatch,
    /// An RTMR3 event's name is not one of [`RTMR3_EVENTS`].
    EventUnknown,
    /// The RTMR3 events are not [`RTMR3_EVENTS`] exactly: one is missing,
    /// repeated or extra, or they are out of order.
    EventSequence,
    /// An RTMR3 event's `event_type` is not [`RTMR3_EVENT_TYPE`].
    EventType,
    /// The verified quote's MRCONFIGID is not the config-id v1 layout, so no
    /// compose hash can be read out of it.
    MrConfigIdNotV1,
    /// The `compose-hash` event is not the compose hash in MRCONFIGID.
    ComposeHashMismatch,
    /// The `app-id` event is not the allowlisted app id.
    AppIdNotAllowed,
    /// The `mr-kms` event is not the allowlisted value.
    MrKmsNotAllowed,
    /// The `os-image-hash` event is not the allowlisted value.
    OsImageHashNotAllowed,
    /// The `key-provider` event is not the allowlisted KMS identity.
    KeyProviderNotAllowed,
    /// A progress marker's payload is not the allowlisted one.
    MarkerNotAllowed,
    /// The model entry carries no `compose_manager_attestation`.
    ComposeManagerMissing,
    /// The record is not the expected shape: a field missing or of the wrong
    /// type, a quote that is not hex, or an action that is not an object with
    /// exactly the keys `action`, `container`, `image` and `timestamp`.
    ComposeManagerMalformed,
    /// The record's quote did not verify against Intel collateral.
    ComposeManagerQuoteUnverified,
    /// The record's quote verified, on a platform whose TCB is not
    /// [`REQUIRED_TCB_STATUS`].
    ComposeManagerTcbNotUpToDate,
    /// The record's quote is from another TD, or another boot: a register
    /// differs from the model quote's.
    ComposeManagerOtherTd,
    /// The record's quote does not carry our nonce at `report_data[32..64]`.
    ComposeManagerNonceMismatch,
    /// The recomputed actions hash is not at the quote's `report_data[0..32]`,
    /// or the record's own `actions_hash` is not the recomputed one.
    ActionsHashMismatch,
    /// A compose action is not in the allowlist.
    ComposeActionNotAllowed,
    /// The quote's own registers could not be read as hex.
    QuoteUnreadable,
}

impl ReplayRefusal {
    /// A stable label for status replies and logs.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ReplayRefusal::LogMissing => "rtmr3_log_missing",
            ReplayRefusal::LogMalformed => "rtmr3_log_malformed",
            ReplayRefusal::EventDigestMismatch => "rtmr3_event_digest_mismatch",
            ReplayRefusal::ReplayMismatch => "rtmr3_replay_mismatch",
            ReplayRefusal::EventUnknown => "rtmr3_event_unknown",
            ReplayRefusal::EventSequence => "rtmr3_event_sequence",
            ReplayRefusal::EventType => "rtmr3_event_type",
            ReplayRefusal::MrConfigIdNotV1 => "rtmr3_mrconfigid_not_v1",
            ReplayRefusal::ComposeHashMismatch => "rtmr3_compose_hash_mismatch",
            ReplayRefusal::AppIdNotAllowed => "rtmr3_app_id_not_allowed",
            ReplayRefusal::MrKmsNotAllowed => "rtmr3_mr_kms_not_allowed",
            ReplayRefusal::OsImageHashNotAllowed => "rtmr3_os_image_hash_not_allowed",
            ReplayRefusal::KeyProviderNotAllowed => "rtmr3_key_provider_not_allowed",
            ReplayRefusal::MarkerNotAllowed => "rtmr3_marker_not_allowed",
            ReplayRefusal::ComposeManagerMissing => "compose_manager_missing",
            ReplayRefusal::ComposeManagerMalformed => "compose_manager_malformed",
            ReplayRefusal::ComposeManagerQuoteUnverified => "compose_manager_quote_unverified",
            ReplayRefusal::ComposeManagerTcbNotUpToDate => "compose_manager_tcb_not_up_to_date",
            ReplayRefusal::ComposeManagerOtherTd => "compose_manager_other_td",
            ReplayRefusal::ComposeManagerNonceMismatch => "compose_manager_nonce_mismatch",
            ReplayRefusal::ActionsHashMismatch => "compose_actions_hash_mismatch",
            ReplayRefusal::ComposeActionNotAllowed => "compose_action_not_allowed",
            ReplayRefusal::QuoteUnreadable => "quote_registers_unreadable",
        }
    }

    /// Every refusal, for tests that hold the labels stable.
    pub const ALL: [ReplayRefusal; 23] = [
        ReplayRefusal::LogMissing,
        ReplayRefusal::LogMalformed,
        ReplayRefusal::EventDigestMismatch,
        ReplayRefusal::ReplayMismatch,
        ReplayRefusal::EventUnknown,
        ReplayRefusal::EventSequence,
        ReplayRefusal::EventType,
        ReplayRefusal::MrConfigIdNotV1,
        ReplayRefusal::ComposeHashMismatch,
        ReplayRefusal::AppIdNotAllowed,
        ReplayRefusal::MrKmsNotAllowed,
        ReplayRefusal::OsImageHashNotAllowed,
        ReplayRefusal::KeyProviderNotAllowed,
        ReplayRefusal::MarkerNotAllowed,
        ReplayRefusal::ComposeManagerMissing,
        ReplayRefusal::ComposeManagerMalformed,
        ReplayRefusal::ComposeManagerQuoteUnverified,
        ReplayRefusal::ComposeManagerTcbNotUpToDate,
        ReplayRefusal::ComposeManagerOtherTd,
        ReplayRefusal::ComposeManagerNonceMismatch,
        ReplayRefusal::ActionsHashMismatch,
        ReplayRefusal::ComposeActionNotAllowed,
        ReplayRefusal::QuoteUnreadable,
    ];
}

impl fmt::Display for ReplayRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for ReplayRefusal {}

// ---------------------------------------------------------------------------
// The allowlist
// ---------------------------------------------------------------------------

/// Why an allowlist did not parse. A label, like every refusal here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowlistMalformed;

impl fmt::Display for AllowlistMalformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("allowlist_malformed")
    }
}

impl std::error::Error for AllowlistMalformed {}

/// A progress marker's allowlisted payload.
#[derive(Clone, PartialEq, Eq)]
pub struct Marker {
    /// One of [`MARKER_EVENTS`].
    pub name: String,
    /// The payload bytes the event must carry.
    pub payload: Vec<u8>,
}

impl fmt::Debug for Marker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Marker")
            .field("name", &self.name)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// What a replayed RTMR3 log's values must be. Every value is the event's
/// payload bytes, compared exactly; `key_provider` too, as the event carries
/// it (its inner structure is not parsed). An empty `app_id`, `mr_kms`,
/// `os_image_hash` or `key_provider` matches no event, not even one with an
/// empty payload; a marker's payload may be empty and is compared as it is.
#[derive(Clone, PartialEq, Eq)]
pub struct Rtmr3Allowlist {
    pub app_id: Vec<u8>,
    pub mr_kms: Vec<u8>,
    pub os_image_hash: Vec<u8>,
    pub key_provider: Vec<u8>,
    /// Each of [`MARKER_EVENTS`] at most once. A marker not listed matches no
    /// event, so a log carrying it is refused.
    pub markers: Vec<Marker>,
}

impl fmt::Debug for Rtmr3Allowlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rtmr3Allowlist")
            .field("app_id_len", &self.app_id.len())
            .field("mr_kms_len", &self.mr_kms.len())
            .field("os_image_hash_len", &self.os_image_hash.len())
            .field("key_provider_len", &self.key_provider.len())
            .field("markers", &self.markers)
            .finish()
    }
}

/// One compose action: its kind, container and image reference. The
/// timestamp is never compared. `container` and `image` are `None` where the
/// record carries JSON `null`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ComposeAction {
    pub action: String,
    pub container: Option<String>,
    pub image: Option<String>,
}

impl fmt::Debug for ComposeAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComposeAction")
            .field("action_len", &self.action.len())
            .field("container_len", &self.container.as_ref().map(String::len))
            .field("image_len", &self.image.as_ref().map(String::len))
            .finish()
    }
}

/// The allowlist a v2 pin payload carries next to its sets (spec 5.2).
///
/// An allowlist whose scalars are empty and whose lists are empty parses: it
/// is the shape-valid placeholder that matches nothing, so every entry
/// checked against it is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowlist {
    pub rtmr3: Rtmr3Allowlist,
    pub compose_actions: Vec<ComposeAction>,
}

impl Allowlist {
    /// Parse an allowlist from its JSON value. Every field is required and no
    /// other is allowed; hex is lower-case and of even length (empty is
    /// allowed); a marker names one of [`MARKER_EVENTS`], at most once.
    pub fn from_json(value: &Value) -> Result<Self, AllowlistMalformed> {
        let object = value.as_object().ok_or(AllowlistMalformed)?;
        exact_keys(object, &["rtmr3", "compose_actions"]).ok_or(AllowlistMalformed)?;
        let rtmr3 = object
            .get("rtmr3")
            .and_then(Value::as_object)
            .ok_or(AllowlistMalformed)?;
        exact_keys(
            rtmr3,
            &[
                "app_id",
                "mr_kms",
                "os_image_hash",
                "key_provider",
                "markers",
            ],
        )
        .ok_or(AllowlistMalformed)?;
        let hex_field = |name: &str| -> Result<Vec<u8>, AllowlistMalformed> {
            lower_hex(
                rtmr3
                    .get(name)
                    .and_then(Value::as_str)
                    .ok_or(AllowlistMalformed)?,
            )
            .ok_or(AllowlistMalformed)
        };
        let mut markers: Vec<Marker> = Vec::new();
        for marker in rtmr3
            .get("markers")
            .and_then(Value::as_array)
            .ok_or(AllowlistMalformed)?
        {
            let marker = marker.as_object().ok_or(AllowlistMalformed)?;
            exact_keys(marker, &["name", "payload"]).ok_or(AllowlistMalformed)?;
            let name = marker
                .get("name")
                .and_then(Value::as_str)
                .ok_or(AllowlistMalformed)?;
            if !MARKER_EVENTS.contains(&name) || markers.iter().any(|m| m.name == name) {
                return Err(AllowlistMalformed);
            }
            let payload = lower_hex(
                marker
                    .get("payload")
                    .and_then(Value::as_str)
                    .ok_or(AllowlistMalformed)?,
            )
            .ok_or(AllowlistMalformed)?;
            markers.push(Marker {
                name: name.to_string(),
                payload,
            });
        }
        let mut compose_actions: Vec<ComposeAction> = Vec::new();
        for action in object
            .get("compose_actions")
            .and_then(Value::as_array)
            .ok_or(AllowlistMalformed)?
        {
            let action = action.as_object().ok_or(AllowlistMalformed)?;
            exact_keys(action, &["action", "container", "image"]).ok_or(AllowlistMalformed)?;
            let parsed = compose_action_fields(action).ok_or(AllowlistMalformed)?;
            if compose_actions.contains(&parsed) {
                return Err(AllowlistMalformed);
            }
            compose_actions.push(parsed);
        }
        Ok(Allowlist {
            rtmr3: Rtmr3Allowlist {
                app_id: hex_field("app_id")?,
                mr_kms: hex_field("mr_kms")?,
                os_image_hash: hex_field("os_image_hash")?,
                key_provider: hex_field("key_provider")?,
                markers,
            },
            compose_actions,
        })
    }

    /// The allowlist as the JSON value [`Allowlist::from_json`] reads, fields
    /// in a fixed order, so the signing tool writes it field for field.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let optional =
            |v: &Option<String>| v.as_ref().map_or(Value::Null, |s| Value::from(s.as_str()));
        let mut rtmr3 = Map::new();
        rtmr3.insert("app_id".into(), hex::encode(&self.rtmr3.app_id).into());
        rtmr3.insert("mr_kms".into(), hex::encode(&self.rtmr3.mr_kms).into());
        rtmr3.insert(
            "os_image_hash".into(),
            hex::encode(&self.rtmr3.os_image_hash).into(),
        );
        rtmr3.insert(
            "key_provider".into(),
            hex::encode(&self.rtmr3.key_provider).into(),
        );
        rtmr3.insert(
            "markers".into(),
            Value::Array(
                self.rtmr3
                    .markers
                    .iter()
                    .map(|m| {
                        let mut o = Map::new();
                        o.insert("name".into(), m.name.clone().into());
                        o.insert("payload".into(), hex::encode(&m.payload).into());
                        Value::Object(o)
                    })
                    .collect(),
            ),
        );
        let mut out = Map::new();
        out.insert("rtmr3".into(), Value::Object(rtmr3));
        out.insert(
            "compose_actions".into(),
            Value::Array(
                self.compose_actions
                    .iter()
                    .map(|a| {
                        let mut o = Map::new();
                        o.insert("action".into(), a.action.clone().into());
                        o.insert("container".into(), optional(&a.container));
                        o.insert("image".into(), optional(&a.image));
                        Value::Object(o)
                    })
                    .collect(),
            ),
        );
        Value::Object(out)
    }
}

// ---------------------------------------------------------------------------
// The RTMR3 replay
// ---------------------------------------------------------------------------

/// One RTMR3 event out of a log that replayed to its quote.
#[derive(Clone, PartialEq, Eq)]
pub struct ReplayedEvent {
    /// The event name (dstack's `event` field).
    pub name: String,
    pub event_type: u32,
    /// The hex-decoded `event_payload`.
    pub payload: Vec<u8>,
}

impl fmt::Debug for ReplayedEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplayedEvent")
            .field("name_len", &self.name.len())
            .field("event_type", &self.event_type)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// The digest dstack extends RTMR3 with for one event:
/// `SHA-384(event_type LE32 || ":" || name || ":" || payload)`.
#[must_use]
pub fn event_digest(event_type: u32, name: &str, payload: &[u8]) -> [u8; 48] {
    let mut hasher = Sha384::new();
    hasher.update(event_type.to_le_bytes());
    hasher.update(b":");
    hasher.update(name.as_bytes());
    hasher.update(b":");
    hasher.update(payload);
    hasher.finalize().into()
}

/// RTMR3 after extending 48 zero bytes with each digest in order.
#[must_use]
pub fn replay_digests<'a>(digests: impl IntoIterator<Item = &'a [u8; 48]>) -> [u8; 48] {
    let mut register = [0u8; 48];
    for digest in digests {
        let mut hasher = Sha384::new();
        hasher.update(register);
        hasher.update(digest);
        register = hasher.finalize().into();
    }
    register
}

/// Replay an entry's RTMR3 events against its **verified** quote and check
/// the log's structure (see the module documentation). Returns the ten
/// events, in order, once every check has passed.
///
/// `event_log` is the model entry's top-level `event_log`, `None` when the
/// entry carries none.
///
/// # Errors
///
/// [`ReplayRefusal::LogMissing`], [`ReplayRefusal::LogMalformed`],
/// [`ReplayRefusal::EventDigestMismatch`], [`ReplayRefusal::ReplayMismatch`],
/// [`ReplayRefusal::EventUnknown`], [`ReplayRefusal::EventSequence`],
/// [`ReplayRefusal::EventType`], [`ReplayRefusal::MrConfigIdNotV1`],
/// [`ReplayRefusal::ComposeHashMismatch`], checked in that order, or
/// [`ReplayRefusal::QuoteUnreadable`].
pub fn replay_rtmr3(
    event_log: Option<&Value>,
    quote: &VerifiedQuote,
) -> Result<Vec<ReplayedEvent>, ReplayRefusal> {
    let log = event_log
        .ok_or(ReplayRefusal::LogMissing)?
        .as_array()
        .ok_or(ReplayRefusal::LogMalformed)?;

    let mut events = Vec::new();
    let mut digests = Vec::new();
    for event in log {
        let event = event.as_object().ok_or(ReplayRefusal::LogMalformed)?;
        let imr = event
            .get("imr")
            .and_then(Value::as_u64)
            .ok_or(ReplayRefusal::LogMalformed)?;
        if imr > 3 {
            return Err(ReplayRefusal::LogMalformed);
        }
        if imr != 3 {
            // RTMR0-2 are pinned directly; their events take no part here.
            continue;
        }
        let event_type = event
            .get("event_type")
            .and_then(Value::as_u64)
            .and_then(|t| u32::try_from(t).ok())
            .ok_or(ReplayRefusal::LogMalformed)?;
        let name = event
            .get("event")
            .and_then(Value::as_str)
            .ok_or(ReplayRefusal::LogMalformed)?;
        let payload = event
            .get("event_payload")
            .and_then(Value::as_str)
            .and_then(any_case_hex)
            .ok_or(ReplayRefusal::LogMalformed)?;
        let recorded = event
            .get("digest")
            .and_then(Value::as_str)
            .ok_or(ReplayRefusal::LogMalformed)?;
        let digest = event_digest(event_type, name, &payload);
        if !recorded.is_empty() && any_case_hex(recorded).as_deref() != Some(&digest[..]) {
            return Err(ReplayRefusal::EventDigestMismatch);
        }
        digests.push(digest);
        events.push(ReplayedEvent {
            name: name.to_string(),
            event_type,
            payload,
        });
    }

    let quoted = register_bytes(&quote.rtmr[3])?;
    if replay_digests(&digests)[..] != quoted[..] {
        return Err(ReplayRefusal::ReplayMismatch);
    }

    // Only now, with the log proven to be the one the quote measured, are
    // its events read.
    if events
        .iter()
        .any(|event| !RTMR3_EVENTS.contains(&event.name.as_str()))
    {
        return Err(ReplayRefusal::EventUnknown);
    }
    if events.len() != RTMR3_EVENTS.len()
        || events
            .iter()
            .zip(RTMR3_EVENTS)
            .any(|(event, expected)| event.name != expected)
    {
        return Err(ReplayRefusal::EventSequence);
    }
    if events
        .iter()
        .any(|event| event.event_type != RTMR3_EVENT_TYPE)
    {
        return Err(ReplayRefusal::EventType);
    }
    let compose_hash = mr_config_id_compose_hash(quote)?;
    if event_payload(&events, "compose-hash") != compose_hash.as_slice() {
        return Err(ReplayRefusal::ComposeHashMismatch);
    }
    Ok(events)
}

/// Check a replayed log's values against the allowlist. `events` is what
/// [`replay_rtmr3`] returned.
///
/// # Errors
///
/// [`ReplayRefusal::AppIdNotAllowed`], [`ReplayRefusal::MrKmsNotAllowed`],
/// [`ReplayRefusal::OsImageHashNotAllowed`],
/// [`ReplayRefusal::KeyProviderNotAllowed`] and
/// [`ReplayRefusal::MarkerNotAllowed`], checked in that order; and
/// [`ReplayRefusal::EventSequence`] for a list that is not the ten events.
pub fn check_rtmr3_allowlist(
    events: &[ReplayedEvent],
    allowlist: &Rtmr3Allowlist,
) -> Result<(), ReplayRefusal> {
    // Defensive: a caller that skipped the replay gets no pass.
    if events.len() != RTMR3_EVENTS.len()
        || events
            .iter()
            .zip(RTMR3_EVENTS)
            .any(|(event, expected)| event.name != expected)
    {
        return Err(ReplayRefusal::EventSequence);
    }
    for (name, expected, refusal) in [
        ("app-id", &allowlist.app_id, ReplayRefusal::AppIdNotAllowed),
        ("mr-kms", &allowlist.mr_kms, ReplayRefusal::MrKmsNotAllowed),
        (
            "os-image-hash",
            &allowlist.os_image_hash,
            ReplayRefusal::OsImageHashNotAllowed,
        ),
        (
            "key-provider",
            &allowlist.key_provider,
            ReplayRefusal::KeyProviderNotAllowed,
        ),
    ] {
        // An empty listed value is one not filled in: it matches nothing,
        // not an event whose payload happens to be empty.
        if expected.is_empty() || event_payload(events, name) != expected.as_slice() {
            return Err(refusal);
        }
    }
    for name in MARKER_EVENTS {
        let listed = allowlist
            .markers
            .iter()
            .find(|marker| marker.name == name)
            .ok_or(ReplayRefusal::MarkerNotAllowed)?;
        if event_payload(events, name) != listed.payload.as_slice() {
            return Err(ReplayRefusal::MarkerNotAllowed);
        }
    }
    Ok(())
}

/// The payload of the event named `name`; empty when there is none, which
/// [`replay_rtmr3`]'s structure check has already ruled out.
fn event_payload<'a>(events: &'a [ReplayedEvent], name: &str) -> &'a [u8] {
    events
        .iter()
        .find(|event| event.name == name)
        .map_or(&[][..], |event| event.payload.as_slice())
}

/// The compose hash in a config-id v1 MRCONFIGID: the 32 bytes after the
/// leading `01`.
fn mr_config_id_compose_hash(quote: &VerifiedQuote) -> Result<Vec<u8>, ReplayRefusal> {
    let bytes = register_bytes(&quote.mr_config_id)?;
    if bytes.len() < 33 || bytes[0] != 0x01 {
        return Err(ReplayRefusal::MrConfigIdNotV1);
    }
    Ok(bytes[1..33].to_vec())
}

fn register_bytes(hex_register: &str) -> Result<Vec<u8>, ReplayRefusal> {
    any_case_hex(hex_register).ok_or(ReplayRefusal::QuoteUnreadable)
}

// ---------------------------------------------------------------------------
// The compose-manager record
// ---------------------------------------------------------------------------

/// A model entry's `compose_manager_attestation`, read but not yet checked.
#[derive(Clone, PartialEq)]
pub struct ComposeManagerRecord {
    /// The record's own TDX quote.
    pub quote: Vec<u8>,
    /// The `actions` list exactly as the record carries it: it is hashed as
    /// it stands.
    pub actions: Vec<Value>,
    /// The record's own claim of the actions hash, lower-cased. Not trusted:
    /// it must equal the recomputed value.
    pub actions_hash: String,
}

impl fmt::Debug for ComposeManagerRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComposeManagerRecord")
            .field("quote_len", &self.quote.len())
            .field("action_count", &self.actions.len())
            .finish_non_exhaustive()
    }
}

impl ComposeManagerRecord {
    /// Read the record out of a model entry. `None` when the entry has no
    /// record is [`ReplayRefusal::ComposeManagerMissing`].
    ///
    /// # Errors
    ///
    /// [`ReplayRefusal::ComposeManagerMissing`] or
    /// [`ReplayRefusal::ComposeManagerMalformed`].
    pub fn from_entry(entry: &Value) -> Result<Self, ReplayRefusal> {
        let record = entry
            .get("compose_manager_attestation")
            .ok_or(ReplayRefusal::ComposeManagerMissing)?
            .as_object()
            .ok_or(ReplayRefusal::ComposeManagerMalformed)?;
        // `quote`, `actions` and `actions_hash` are required, each read
        // below. Other keys the record may carry (`nonce`, `nonce_source`,
        // `event_log`, `report_data`, `vm_config`) are not read: the quote
        // is the authority for what they claim.
        let quote = record
            .get("quote")
            .and_then(Value::as_str)
            .and_then(any_case_hex)
            .ok_or(ReplayRefusal::ComposeManagerMalformed)?;
        let actions = record
            .get("actions")
            .and_then(Value::as_array)
            .ok_or(ReplayRefusal::ComposeManagerMalformed)?
            .clone();
        let actions_hash = record
            .get("actions_hash")
            .and_then(Value::as_str)
            .ok_or(ReplayRefusal::ComposeManagerMalformed)?
            .to_ascii_lowercase();
        Ok(ComposeManagerRecord {
            quote,
            actions,
            actions_hash,
        })
    }
}

/// Check a compose-manager record against the model quote it sits beside.
///
/// `verify` is the caller's quote verifier, applied to the record's quote: it
/// returns the verified quote, or `None` when the quote did not verify (the
/// caller fetches collateral for [`ComposeManagerRecord::quote`] first). It
/// is the same verifier the caller applies to the model quote. `allowlist` is
/// `None` only in the re-sign tool's `observe`, which has no allowlist yet;
/// every other caller passes the signed one.
///
/// Returns the actions, parsed, once every check has passed.
///
/// # Errors
///
/// [`ReplayRefusal::ComposeManagerMalformed`],
/// [`ReplayRefusal::ComposeManagerQuoteUnverified`],
/// [`ReplayRefusal::ComposeManagerTcbNotUpToDate`],
/// [`ReplayRefusal::ComposeManagerOtherTd`],
/// [`ReplayRefusal::ComposeManagerNonceMismatch`],
/// [`ReplayRefusal::ActionsHashMismatch`] and
/// [`ReplayRefusal::ComposeActionNotAllowed`], checked in that order.
pub fn check_compose_manager(
    record: &ComposeManagerRecord,
    verify: impl FnOnce(&[u8]) -> Option<VerifiedQuote>,
    model_quote: &VerifiedQuote,
    nonce_hex: &str,
    allowlist: Option<&[ComposeAction]>,
) -> Result<Vec<ComposeAction>, ReplayRefusal> {
    // The actions' shape first: nothing below is worth a quote verification
    // if they cannot be read.
    let actions = record
        .actions
        .iter()
        .map(|action| {
            let object = action.as_object()?;
            exact_keys(object, &ACTION_KEYS)?;
            compose_action_fields(object)
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(ReplayRefusal::ComposeManagerMalformed)?;

    let quote = verify(&record.quote).ok_or(ReplayRefusal::ComposeManagerQuoteUnverified)?;
    if quote.tcb_status != REQUIRED_TCB_STATUS {
        return Err(ReplayRefusal::ComposeManagerTcbNotUpToDate);
    }
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    if !same(&quote.mrtd, &model_quote.mrtd)
        || !same(&quote.mr_config_id, &model_quote.mr_config_id)
        || quote
            .rtmr
            .iter()
            .zip(&model_quote.rtmr)
            .any(|(a, b)| !same(a, b))
    {
        return Err(ReplayRefusal::ComposeManagerOtherTd);
    }
    let nonce = any_case_hex(nonce_hex)
        .filter(|nonce| nonce.len() == 32)
        .ok_or(ReplayRefusal::ComposeManagerNonceMismatch)?;
    if quote.report_data.get(32..64) != Some(nonce.as_slice()) {
        return Err(ReplayRefusal::ComposeManagerNonceMismatch);
    }
    let recomputed = compose_actions_hash(&record.actions)?;
    if quote.report_data.get(0..32) != Some(&recomputed[..])
        || record.actions_hash != hex::encode(recomputed)
    {
        return Err(ReplayRefusal::ActionsHashMismatch);
    }
    if let Some(allowed) = allowlist {
        if actions.iter().any(|action| !allowed.contains(action)) {
            return Err(ReplayRefusal::ComposeActionNotAllowed);
        }
    }
    Ok(actions)
}

/// SHA-256 of `actions` serialised as Python's `json.dumps(actions,
/// separators=(",", ":"), sort_keys=True)` does: keys sorted, no whitespace,
/// non-ASCII and control characters escaped (`ensure_ascii`). That is how
/// NEAR AI computes `actions_hash`; other serialisations do not match.
///
/// # Errors
///
/// [`ReplayRefusal::ComposeManagerMalformed`] for a number Python would print
/// in a form this does not reproduce (non-finite).
pub fn compose_actions_hash(actions: &[Value]) -> Result<[u8; 32], ReplayRefusal> {
    let mut text = String::new();
    write_python_json(&Value::Array(actions.to_vec()), &mut text)
        .ok_or(ReplayRefusal::ComposeManagerMalformed)?;
    Ok(Sha256::digest(text.as_bytes()).into())
}

/// `json.dumps(value, separators=(",", ":"), sort_keys=True)`, appended to
/// `out`. `None` for a value it cannot reproduce exactly.
///
/// Written out rather than taken from `serde_json`: in this crate's build
/// graph `serde_json` keeps insertion order (`dcap-qvl` turns on
/// `preserve_order`), and its string escaping and float form are not
/// Python's.
fn write_python_json(value: &Value, out: &mut String) -> Option<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                out.push_str(&python_float(n.as_f64()?)?);
            }
        }
        Value::String(s) => python_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_python_json(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Python sorts by code point; UTF-8 byte order is the same order.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                python_string(key, out);
                out.push(':');
                write_python_json(&map[key], out)?;
            }
            out.push('}');
        }
    }
    Some(())
}

/// A string as Python's `json` writes it with `ensure_ascii`: `"` and `\`
/// escaped, `\b \f \n \r \t` short, every other character outside
/// space..`~` as `\uXXXX` (lower-case hex; UTF-16 surrogate pairs above the
/// BMP).
fn python_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// A float as Python's `repr` writes it: the shortest digits that round-trip,
/// positional from 1e-4 up to 1e16 (with `.0` when integral), scientific with
/// a signed two-digit exponent outside that. `None` for NaN or infinity,
/// which `json.dumps` would write as non-JSON.
fn python_float(x: f64) -> Option<String> {
    if !x.is_finite() {
        return None;
    }
    if x == 0.0 {
        return Some(if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_string());
    }
    let magnitude = x.abs();
    if (1e-4..1e16).contains(&magnitude) {
        let text = format!("{x}");
        return Some(if text.contains('.') {
            text
        } else {
            format!("{text}.0")
        });
    }
    let text = format!("{x:e}");
    let (mantissa, exponent) = text.split_once('e')?;
    let exponent: i32 = exponent.parse().ok()?;
    let sign = if exponent < 0 { '-' } else { '+' };
    Some(format!("{mantissa}e{sign}{:02}", exponent.abs()))
}

// ---------------------------------------------------------------------------
// A report's model entries
// ---------------------------------------------------------------------------

/// One model entry of an attestation report, with everything a staging
/// check needs from it: its own quote, its own log and its own
/// compose-manager record. Read from one entry, so the log and record that
/// are checked are always those beside the quote that is checked.
#[derive(Clone)]
pub struct EntryEvidence {
    /// The entry's `intel_quote`.
    pub quote: Vec<u8>,
    /// The entry's top-level `event_log`, if any.
    pub event_log: Option<Value>,
    /// The entry itself, for [`ComposeManagerRecord::from_entry`].
    pub entry: Value,
}

impl fmt::Debug for EntryEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntryEvidence")
            .field("quote_len", &self.quote.len())
            .field("has_event_log", &self.event_log.is_some())
            .finish_non_exhaustive()
    }
}

/// Every model-attestation entry naming `model`, in report order: the same
/// entries, in the same order, as
/// [`crate::receipt::model_entry_quotes`] reads. Never the gateway's
/// attestation.
///
/// # Errors
///
/// [`crate::receipt::AttestedKeyError::Malformed`] for a report that is not
/// JSON, has no model-attestation container, or has an entry for this model
/// without a hex `intel_quote`;
/// [`crate::receipt::AttestedKeyError::ModelNotAttested`] when no entry names
/// the model.
#[cfg(feature = "receipt")]
pub fn model_entry_evidence(
    report_json: &str,
    model: &str,
) -> Result<Vec<EntryEvidence>, crate::receipt::AttestedKeyError> {
    use crate::receipt::{AttestedKeyError, model_attestation_entries};
    let document: Value =
        serde_json::from_str(report_json).map_err(|_| AttestedKeyError::Malformed)?;
    let entries = model_attestation_entries(&document).ok_or(AttestedKeyError::Malformed)?;
    let mut evidence = Vec::new();
    for entry in entries {
        if entry.get("model_name").and_then(Value::as_str) != Some(model) {
            continue;
        }
        let quote = entry
            .get("intel_quote")
            .and_then(Value::as_str)
            .and_then(|q| hex::decode(q).ok())
            .ok_or(AttestedKeyError::Malformed)?;
        evidence.push(EntryEvidence {
            quote,
            event_log: entry.get("event_log").cloned(),
            entry: entry.clone(),
        });
    }
    if evidence.is_empty() {
        return Err(AttestedKeyError::ModelNotAttested);
    }
    Ok(evidence)
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn exact_keys(object: &Map<String, Value>, keys: &[&str]) -> Option<()> {
    (object.len() == keys.len() && keys.iter().all(|k| object.contains_key(*k))).then_some(())
}

/// `action` a string; `container` and `image` a string or `null`.
fn compose_action_fields(object: &Map<String, Value>) -> Option<ComposeAction> {
    let optional = |name: &str| match object.get(name)? {
        Value::Null => Some(None),
        Value::String(s) => Some(Some(s.clone())),
        _ => None,
    };
    Some(ComposeAction {
        action: object.get("action")?.as_str()?.to_string(),
        container: optional("container")?,
        image: optional("image")?,
    })
}

/// Hex in either case, as a log or quote carries it.
fn any_case_hex(text: &str) -> Option<Vec<u8>> {
    hex::decode(text).ok()
}

/// Lower-case hex of even length, as an allowlist must carry it, so each
/// value has one spelling.
fn lower_hex(text: &str) -> Option<Vec<u8>> {
    if text.bytes().any(|b| b.is_ascii_uppercase()) {
        return None;
    }
    hex::decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Synthetic payloads: none of these is a real measurement.
    const APP_ID: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
    const COMPOSE_HASH: &str = "c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
    const INSTANCE_ID: &str = "1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d";
    const MR_KMS: &str = "3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";
    const OS_IMAGE_HASH: &str =
        "05050505050505050505050505050505050505050505050505050505050505050505";
    /// `{"name":"kms","id":"<synthetic>"}`, hex: the key provider as the
    /// event carries it.
    const KEY_PROVIDER: &str = "7b226e616d65223a226b6d73222c226964223a2273796e746865746963227d";
    const NONCE: &str = "9f3c1d2e4b5a60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";

    fn marker_payload(name: &str) -> String {
        hex::encode(format!("synthetic {name}"))
    }

    /// The ten events, in order, with synthetic payloads.
    fn ten_events() -> Vec<(String, String)> {
        RTMR3_EVENTS
            .iter()
            .map(|name| {
                let payload = match *name {
                    "app-id" => APP_ID.to_string(),
                    "compose-hash" => COMPOSE_HASH.to_string(),
                    "instance-id" => INSTANCE_ID.to_string(),
                    "mr-kms" => MR_KMS.to_string(),
                    "os-image-hash" => OS_IMAGE_HASH.to_string(),
                    "key-provider" => KEY_PROVIDER.to_string(),
                    marker => marker_payload(marker),
                };
                (name.to_string(), payload)
            })
            .collect()
    }

    /// A log as NEAR AI ships it: a few RTMR0-2 events with digests (which
    /// take no part), then the RTMR3 events with empty digests.
    fn log_of(events: &[(String, String)]) -> Value {
        let mut log = vec![
            json!({"imr": 0, "event_type": 1, "digest": "ee".repeat(48), "event": "", "event_payload": "00"}),
            json!({"imr": 1, "event_type": 2, "digest": "dd".repeat(48), "event": "", "event_payload": ""}),
        ];
        for (name, payload) in events {
            log.push(json!({
                "imr": 3,
                "event_type": RTMR3_EVENT_TYPE,
                "digest": "",
                "event": name,
                "event_payload": payload,
            }));
        }
        Value::Array(log)
    }

    /// RTMR3 as the replay rule computes it over `events`.
    fn replayed(events: &[(String, String)]) -> String {
        let digests: Vec<[u8; 48]> = events
            .iter()
            .map(|(name, payload)| {
                event_digest(RTMR3_EVENT_TYPE, name, &hex::decode(payload).unwrap())
            })
            .collect();
        hex::encode(replay_digests(&digests))
    }

    /// A verified quote whose RTMR3 is what `events` replay to, and whose
    /// MRCONFIGID is config-id v1 over the synthetic compose hash.
    fn quote_replaying(events: &[(String, String)]) -> VerifiedQuote {
        let mut report_data = vec![0u8; 64];
        report_data[32..64].copy_from_slice(&hex::decode(NONCE).unwrap());
        VerifiedQuote {
            report_data,
            mrtd: "11".repeat(48),
            mr_config_id: format!("01{COMPOSE_HASH}{}", "00".repeat(15)),
            rtmr: [
                "20".repeat(48),
                "21".repeat(48),
                "22".repeat(48),
                replayed(events),
            ],
            tcb_status: REQUIRED_TCB_STATUS.to_string(),
            advisory_ids: Vec::new(),
        }
    }

    fn allowlist() -> Allowlist {
        Allowlist::from_json(&json!({
            "rtmr3": {
                "app_id": APP_ID,
                "mr_kms": MR_KMS,
                "os_image_hash": OS_IMAGE_HASH,
                "key_provider": KEY_PROVIDER,
                "markers": MARKER_EVENTS
                    .iter()
                    .map(|name| json!({"name": name, "payload": marker_payload(name)}))
                    .collect::<Vec<_>>(),
            },
            "compose_actions": [
                {"action": "compose_manager_started", "container": null, "image": null},
                {"action": "compose_up", "container": "app", "image": "registry.example/app@sha256:ab"},
            ],
        }))
        .expect("the synthetic allowlist parses")
    }

    /// Replay and allowlist together, as the app runs them.
    fn check(events: &[(String, String)], quote: &VerifiedQuote) -> Result<(), ReplayRefusal> {
        let replayed = replay_rtmr3(Some(&log_of(events)), quote)?;
        check_rtmr3_allowlist(&replayed, &allowlist().rtmr3)
    }

    fn with_payload(name: &str, payload: &str) -> Vec<(String, String)> {
        ten_events()
            .into_iter()
            .map(|(n, p)| {
                if n == name {
                    (n, payload.to_string())
                } else {
                    (n, p)
                }
            })
            .collect()
    }

    #[test]
    fn replay_matching_the_quote_is_accepted() {
        let events = ten_events();
        let quote = quote_replaying(&events);
        let replayed = replay_rtmr3(Some(&log_of(&events)), &quote).expect("replays");
        assert_eq!(
            replayed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            RTMR3_EVENTS
        );
        assert_eq!(check(&events, &quote), Ok(()));
        // Upper-case register hex is the same register.
        let mut upper = quote.clone();
        upper.rtmr[3] = upper.rtmr[3].to_ascii_uppercase();
        assert_eq!(check(&events, &upper), Ok(()));
    }

    #[test]
    fn replay_mismatch_is_refused() {
        let events = ten_events();
        let quote = quote_replaying(&events);
        // One event's payload altered.
        let altered = with_payload("storage-fs", "00");
        assert_eq!(check(&altered, &quote), Err(ReplayRefusal::ReplayMismatch));
        // One dropped.
        let mut dropped = events.clone();
        dropped.remove(4);
        assert_eq!(check(&dropped, &quote), Err(ReplayRefusal::ReplayMismatch));
        // Two swapped.
        let mut reordered = events.clone();
        reordered.swap(5, 6);
        assert_eq!(
            check(&reordered, &quote),
            Err(ReplayRefusal::ReplayMismatch)
        );
        // An event type changed is a different digest, so a different
        // register.
        let mut log = log_of(&events);
        log[3]["event_type"] = json!(RTMR3_EVENT_TYPE + 1);
        assert_eq!(
            replay_rtmr3(Some(&log), &quote).unwrap_err(),
            ReplayRefusal::ReplayMismatch
        );
        // No log at all, and a log that is not a list.
        assert_eq!(
            replay_rtmr3(None, &quote).unwrap_err(),
            ReplayRefusal::LogMissing
        );
        assert_eq!(
            replay_rtmr3(Some(&json!("not a list")), &quote).unwrap_err(),
            ReplayRefusal::LogMalformed
        );
        // An RTMR3 event without a name, or with a payload that is not hex.
        let mut nameless = log_of(&events);
        nameless[2].as_object_mut().unwrap().remove("event");
        assert_eq!(
            replay_rtmr3(Some(&nameless), &quote).unwrap_err(),
            ReplayRefusal::LogMalformed
        );
        let mut not_hex = log_of(&events);
        not_hex[2]["event_payload"] = json!("zz");
        assert_eq!(
            replay_rtmr3(Some(&not_hex), &quote).unwrap_err(),
            ReplayRefusal::LogMalformed
        );
    }

    #[test]
    fn the_event_digest_is_recomputed_from_type_name_and_payload() {
        // Known answer, computed independently with Python's hashlib:
        //   sha384(struct.pack("<I", 134217729) + b":" + b"app-id" + b":"
        //          + bytes(range(20)))
        let payload: Vec<u8> = (0u8..20).collect();
        assert_eq!(
            hex::encode(event_digest(0x0800_0001, "app-id", &payload)),
            "8afd336904cd928ad9bd019a89b31b387673ea9b07e82bb30fb9a65c80bb358548eb47ed2c57b8044a6e1f1533af243f"
        );
        // And one extension from 48 zero bytes:
        //   sha384(bytes(48) + digest)
        let digest = event_digest(0x0800_0001, "app-id", &payload);
        assert_eq!(
            hex::encode(replay_digests([&digest])),
            "51d5915256ccf3b07fc2fc3ef00095b382936793dbba8aacc682b87a1bc51d69068f64d9eb6ea479474b5719bf80d93f"
        );
        // The empty recorded digest replays: `log_of` records every RTMR3
        // digest as "". The correct digest recorded replays too.
        let events = ten_events();
        let quote = quote_replaying(&events);
        let mut recorded = log_of(&events);
        for event in recorded.as_array_mut().unwrap().iter_mut().skip(2) {
            let name = event["event"].as_str().unwrap().to_string();
            let payload = hex::decode(event["event_payload"].as_str().unwrap()).unwrap();
            event["digest"] = json!(hex::encode(event_digest(RTMR3_EVENT_TYPE, &name, &payload)));
        }
        assert!(replay_rtmr3(Some(&recorded), &quote).is_ok());
        // The RTMR0-2 events' recorded digests are not read.
        assert!(replay_rtmr3(Some(&log_of(&events)), &quote).is_ok());
    }

    #[test]
    fn an_event_whose_recorded_digest_differs_from_its_payload_is_refused() {
        let events = ten_events();
        let quote = quote_replaying(&events);
        // A log that carries the digest of the true payload beside an
        // invented one: the recorded digest would replay, the payload does
        // not match it.
        let mut log = log_of(&events);
        let true_digest = event_digest(
            RTMR3_EVENT_TYPE,
            "key-provider",
            &hex::decode(KEY_PROVIDER).unwrap(),
        );
        let index = 2 + RTMR3_EVENTS
            .iter()
            .position(|n| *n == "key-provider")
            .unwrap();
        log[index]["digest"] = json!(hex::encode(true_digest));
        log[index]["event_payload"] = json!(hex::encode("another kms"));
        assert_eq!(
            replay_rtmr3(Some(&log), &quote).unwrap_err(),
            ReplayRefusal::EventDigestMismatch
        );
        // A recorded digest that is not hex is not a match either.
        let mut garbled = log_of(&events);
        garbled[2]["digest"] = json!("not hex");
        assert_eq!(
            replay_rtmr3(Some(&garbled), &quote).unwrap_err(),
            ReplayRefusal::EventDigestMismatch
        );
    }

    #[test]
    fn an_event_above_rtmr3_is_refused() {
        // TDX has RTMR0-3 only; an `imr` above 3 is no register, and the
        // log carrying it is malformed rather than skipped.
        let events = ten_events();
        let mut log = log_of(&events);
        log.as_array_mut().unwrap().push(json!({
            "imr": 4, "event_type": RTMR3_EVENT_TYPE, "digest": "",
            "event": "app-id", "event_payload": APP_ID,
        }));
        assert_eq!(
            replay_rtmr3(Some(&log), &quote_replaying(&events)),
            Err(ReplayRefusal::LogMalformed)
        );
    }

    #[test]
    fn an_unknown_event_is_refused() {
        // A log the quote really measured, with an eleventh event of a name
        // the allowlist does not know: it replays, and is still refused.
        let mut events = ten_events();
        events.insert(8, ("add-container".to_string(), "00".to_string()));
        let quote = quote_replaying(&events);
        assert_eq!(check(&events, &quote), Err(ReplayRefusal::EventUnknown));
        // A known event renamed.
        let mut renamed = ten_events();
        renamed[7].0 = "key-provider-v2".to_string();
        assert_eq!(
            check(&renamed, &quote_replaying(&renamed)),
            Err(ReplayRefusal::EventUnknown)
        );
    }

    #[test]
    fn an_extra_missing_or_out_of_order_event_is_refused() {
        // Each log below replays to its own quote; what refuses it is the
        // structure, not the register.
        let mut missing = ten_events();
        missing.retain(|(name, _)| name != "key-provider");
        assert_eq!(
            check(&missing, &quote_replaying(&missing)),
            Err(ReplayRefusal::EventSequence),
            "a log without its key-provider event would skip the KMS check"
        );
        let mut repeated = ten_events();
        repeated.push(repeated[9].clone());
        assert_eq!(
            check(&repeated, &quote_replaying(&repeated)),
            Err(ReplayRefusal::EventSequence)
        );
        let mut reordered = ten_events();
        reordered.swap(5, 6);
        assert_eq!(
            check(&reordered, &quote_replaying(&reordered)),
            Err(ReplayRefusal::EventSequence)
        );
        // The replay alone refuses the reordered log, before any allowlist:
        // `observe` runs only the replay.
        assert_eq!(
            replay_rtmr3(Some(&log_of(&reordered)), &quote_replaying(&reordered)),
            Err(ReplayRefusal::EventSequence)
        );
        assert_eq!(
            replay_rtmr3(Some(&log_of(&repeated)), &quote_replaying(&repeated)),
            Err(ReplayRefusal::EventSequence)
        );
        let empty: Vec<(String, String)> = Vec::new();
        assert_eq!(
            check(&empty, &quote_replaying(&empty)),
            Err(ReplayRefusal::EventSequence)
        );
        // The right names under another event type: the log replays to its
        // quote and is refused on the type.
        let events = ten_events();
        let mut log = log_of(&events);
        let digests: Vec<[u8; 48]> = events
            .iter()
            .map(|(n, p)| event_digest(7, n, &hex::decode(p).unwrap()))
            .collect();
        for event in log.as_array_mut().unwrap().iter_mut().skip(2) {
            event["event_type"] = json!(7);
        }
        let mut quote = quote_replaying(&events);
        quote.rtmr[3] = hex::encode(replay_digests(&digests));
        assert_eq!(
            replay_rtmr3(Some(&log), &quote).unwrap_err(),
            ReplayRefusal::EventType
        );
        // And the allowlist check alone refuses a list that is not the ten.
        assert_eq!(
            check_rtmr3_allowlist(&[], &allowlist().rtmr3),
            Err(ReplayRefusal::EventSequence)
        );
    }

    #[test]
    fn a_different_mr_kms_or_os_image_hash_is_refused() {
        for (name, refusal) in [
            ("mr-kms", ReplayRefusal::MrKmsNotAllowed),
            ("os-image-hash", ReplayRefusal::OsImageHashNotAllowed),
        ] {
            let events = with_payload(name, &"99".repeat(48));
            assert_eq!(
                check(&events, &quote_replaying(&events)),
                Err(refusal),
                "{name}"
            );
        }
        // A marker with another payload, and an allowlist that lists no
        // payload for a marker the log carries.
        let events = with_payload("storage-fs", "00");
        assert_eq!(
            check(&events, &quote_replaying(&events)),
            Err(ReplayRefusal::MarkerNotAllowed)
        );
        let events = ten_events();
        let replayed = replay_rtmr3(Some(&log_of(&events)), &quote_replaying(&events)).unwrap();
        let mut unlisted = allowlist().rtmr3;
        unlisted.markers.retain(|m| m.name != "system-ready");
        assert_eq!(
            check_rtmr3_allowlist(&replayed, &unlisted),
            Err(ReplayRefusal::MarkerNotAllowed)
        );
    }

    #[test]
    fn a_different_app_id_is_refused() {
        let events = with_payload("app-id", &"b2".repeat(20));
        assert_eq!(
            check(&events, &quote_replaying(&events)),
            Err(ReplayRefusal::AppIdNotAllowed)
        );
    }

    #[test]
    fn a_different_key_provider_is_refused() {
        // Another KMS identity, and no key provider at all (empty payload).
        for payload in [
            hex::encode(r#"{"name":"kms","id":"another"}"#),
            String::new(),
        ] {
            let events = with_payload("key-provider", &payload);
            assert_eq!(
                check(&events, &quote_replaying(&events)),
                Err(ReplayRefusal::KeyProviderNotAllowed)
            );
        }
    }

    #[test]
    fn an_empty_allowlisted_value_matches_nothing() {
        // An allowlist that leaves a value empty by mistake does not accept
        // a log whose event carries an empty payload: empty is "not filled
        // in", never a value to compare with.
        for (name, refusal) in [
            ("app-id", ReplayRefusal::AppIdNotAllowed),
            ("mr-kms", ReplayRefusal::MrKmsNotAllowed),
            ("os-image-hash", ReplayRefusal::OsImageHashNotAllowed),
            ("key-provider", ReplayRefusal::KeyProviderNotAllowed),
        ] {
            let events = with_payload(name, "");
            let replayed = replay_rtmr3(Some(&log_of(&events)), &quote_replaying(&events)).unwrap();
            let mut list = allowlist().rtmr3;
            match name {
                "app-id" => list.app_id.clear(),
                "mr-kms" => list.mr_kms.clear(),
                "os-image-hash" => list.os_image_hash.clear(),
                _ => list.key_provider.clear(),
            }
            assert_eq!(
                check_rtmr3_allowlist(&replayed, &list),
                Err(refusal),
                "{name}"
            );
        }
        // A progress marker's payload may be empty, and an empty listed
        // payload matches it.
        let events = with_payload("system-ready", "");
        let replayed = replay_rtmr3(Some(&log_of(&events)), &quote_replaying(&events)).unwrap();
        let mut list = allowlist().rtmr3;
        for marker in &mut list.markers {
            if marker.name == "system-ready" {
                marker.payload.clear();
            }
        }
        assert_eq!(check_rtmr3_allowlist(&replayed, &list), Ok(()));
    }

    #[test]
    fn the_instance_id_is_accepted_with_any_value() {
        for payload in ["", "00", &"ff".repeat(64)] {
            let events = with_payload("instance-id", payload);
            assert_eq!(
                check(&events, &quote_replaying(&events)),
                Ok(()),
                "{payload}"
            );
        }
        // And the allowlist has no field for it at all.
        let value = allowlist().to_json();
        assert!(!value.to_string().contains("instance"));
    }

    #[test]
    fn a_compose_hash_event_other_than_mrconfigids_is_refused() {
        let events = with_payload("compose-hash", &"c1".repeat(32));
        assert_eq!(
            check(&events, &quote_replaying(&events)),
            Err(ReplayRefusal::ComposeHashMismatch)
        );
        // An MRCONFIGID that is not the v1 layout yields no compose hash to
        // compare, and is refused rather than read at the wrong offset.
        let events = ten_events();
        let mut quote = quote_replaying(&events);
        quote.mr_config_id = format!("02{COMPOSE_HASH}{}", "00".repeat(15));
        assert_eq!(check(&events, &quote), Err(ReplayRefusal::MrConfigIdNotV1));
    }

    // -- the compose-manager record ----------------------------------------

    fn actions() -> Vec<Value> {
        // Keys deliberately out of order, as a provider may send them.
        vec![
            json!({"timestamp": 1_700_000_000u64, "image": "registry.example/app@sha256:ab", "container": "app", "action": "compose_up"}),
            json!({"action": "compose_manager_started", "container": null, "image": null, "timestamp": 1_699_999_999.5}),
        ]
    }

    /// A record whose quote is the model quote's TD, bound to our nonce and
    /// to `actions`.
    fn record_over(
        actions: Vec<Value>,
        model: &VerifiedQuote,
    ) -> (ComposeManagerRecord, VerifiedQuote) {
        let hash = compose_actions_hash(&actions).unwrap();
        let mut quote = model.clone();
        quote.report_data[0..32].copy_from_slice(&hash);
        let record = ComposeManagerRecord::from_entry(&json!({
            "compose_manager_attestation": {
                "actions": actions,
                "actions_hash": hex::encode(hash),
                "nonce": NONCE,
                "nonce_source": "client",
                "quote": "0badc0de",
            }
        }))
        .expect("the record reads");
        (record, quote)
    }

    fn model_quote() -> VerifiedQuote {
        quote_replaying(&ten_events())
    }

    #[test]
    fn the_compose_manager_quote_is_bound_to_the_model_quote_and_nonce() {
        let model = model_quote();
        let (record, quote) = record_over(actions(), &model);
        let allowed = allowlist().compose_actions;
        let checked = check_compose_manager(
            &record,
            |bytes| {
                assert_eq!(
                    bytes,
                    [0x0b, 0xad, 0xc0, 0xde],
                    "the record's own quote is verified"
                );
                Some(quote.clone())
            },
            &model,
            NONCE,
            Some(&allowed),
        )
        .expect("bound to the model's TD, our nonce and its actions");
        assert_eq!(checked.len(), 2);

        // Another nonce at [32..64].
        let mut stale = quote.clone();
        stale.report_data[32..64].fill(0x5a);
        assert_eq!(
            check_compose_manager(
                &record,
                |_| Some(stale.clone()),
                &model,
                NONCE,
                Some(&allowed)
            ),
            Err(ReplayRefusal::ComposeManagerNonceMismatch)
        );
        // Our nonce asked for, a malformed one passed.
        assert_eq!(
            check_compose_manager(
                &record,
                |_| Some(quote.clone()),
                &model,
                "zz",
                Some(&allowed)
            ),
            Err(ReplayRefusal::ComposeManagerNonceMismatch)
        );
        // A platform behind on Intel's fixes.
        let mut old = quote.clone();
        old.tcb_status = "OutOfDate".to_string();
        assert_eq!(
            check_compose_manager(
                &record,
                |_| Some(old.clone()),
                &model,
                NONCE,
                Some(&allowed)
            ),
            Err(ReplayRefusal::ComposeManagerTcbNotUpToDate)
        );
        // The record is required.
        assert_eq!(
            ComposeManagerRecord::from_entry(&json!({"model_name": "m"})).unwrap_err(),
            ReplayRefusal::ComposeManagerMissing
        );
        for malformed in [
            json!({"compose_manager_attestation": "x"}),
            json!({"compose_manager_attestation": {"actions": [], "actions_hash": ""}}),
            json!({"compose_manager_attestation": {"actions": [], "actions_hash": "", "quote": "zz"}}),
            json!({"compose_manager_attestation": {"actions": {}, "actions_hash": "", "quote": "00"}}),
        ] {
            assert_eq!(
                ComposeManagerRecord::from_entry(&malformed).unwrap_err(),
                ReplayRefusal::ComposeManagerMalformed
            );
        }
    }

    #[test]
    fn a_compose_manager_quote_from_another_td_is_refused() {
        let model = model_quote();
        let (record, quote) = record_over(actions(), &model);
        let allowed = allowlist().compose_actions;
        let alterations: [fn(&mut VerifiedQuote); 6] = [
            |q| q.mrtd = "12".repeat(48),
            |q| q.mr_config_id = format!("01{}{}", "c1".repeat(32), "00".repeat(15)),
            |q| q.rtmr[0] = "30".repeat(48),
            |q| q.rtmr[1] = "31".repeat(48),
            |q| q.rtmr[2] = "32".repeat(48),
            // Same TD, another boot: RTMR3 differs.
            |q| q.rtmr[3] = "33".repeat(48),
        ];
        for alter in alterations {
            let mut other = quote.clone();
            alter(&mut other);
            assert_eq!(
                check_compose_manager(
                    &record,
                    |_| Some(other.clone()),
                    &model,
                    NONCE,
                    Some(&allowed)
                ),
                Err(ReplayRefusal::ComposeManagerOtherTd)
            );
        }
    }

    #[test]
    fn a_compose_manager_quote_that_does_not_verify_is_refused() {
        // A real, genuine quote that the checked-in collateral (for another
        // platform) does not verify, through the real verifier.
        const ED25519_REPORT: &str =
            include_str!("../tests/fixtures/near_ai_model_attestation_report_ed25519.json");
        const COLLATERAL: &str =
            include_str!("../tests/fixtures/near_ai_attestation_collateral.json");
        let report: Value = serde_json::from_str(ED25519_REPORT).unwrap();
        let quote_hex = report["model_attestations"][0]["intel_quote"]
            .as_str()
            .unwrap();
        let entry = json!({
            "compose_manager_attestation": {
                "actions": actions(),
                "actions_hash": hex::encode(compose_actions_hash(&actions()).unwrap()),
                "quote": quote_hex,
            }
        });
        let record = ComposeManagerRecord::from_entry(&entry).unwrap();
        let collateral = crate::quote::parse_collateral(COLLATERAL).unwrap();
        let model = model_quote();
        assert_eq!(
            check_compose_manager(
                &record,
                |bytes| crate::quote::verify_quote(bytes, &collateral, 1_788_264_000).ok(),
                &model,
                NONCE,
                Some(&allowlist().compose_actions),
            ),
            Err(ReplayRefusal::ComposeManagerQuoteUnverified)
        );
    }

    #[test]
    fn actions_hash_is_recomputed_as_sorted_compact_json() {
        // Known answer, computed with Python:
        //   hashlib.sha256(json.dumps(actions, separators=(",", ":"),
        //                             sort_keys=True).encode()).hexdigest()
        let sorted = "fa6e8d7482ede9f28ee5edf39549432ebd338af6664e8c2b49c1fec70fef9c83";
        assert_eq!(
            hex::encode(compose_actions_hash(&actions()).unwrap()),
            sorted
        );
        // The same actions unsorted, and with Python's default separators,
        // hash to other values (also from Python), which is why neither is
        // used.
        let unsorted = "83adc550fa06cf100365eaddaa76bf10843d251d72f41fe21df9c6a91041731b";
        let default = "b5004acd996f94bdb37367a628fef7cea52e3e31f612912a8d0a0ed80b881d32";
        assert_ne!(sorted, unsorted);
        assert_ne!(sorted, default);
        // serde_json's own compact form keeps this crate's insertion order,
        // so it is the unsorted one, not the sorted one.
        let serde_form = serde_json::to_vec(&Value::Array(actions())).unwrap();
        assert_ne!(hex::encode(Sha256::digest(&serde_form)), sorted);

        // Python's escaping and float forms, character for character:
        //   json.dumps(["caf\u00e9\U0001F600\x01\x7f/", 1e16, 1.5e-5, 0.0,
        //               -2.0, 123.25], separators=(",", ":"))
        let mut text = String::new();
        write_python_json(
            &json!([
                "caf\u{e9}\u{1F600}\u{1}\u{7f}/",
                1e16,
                1.5e-5,
                0.0,
                -2.0,
                123.25
            ]),
            &mut text,
        )
        .unwrap();
        assert_eq!(
            text,
            "[\"caf\\u00e9\\ud83d\\ude00\\u0001\\u007f/\",1e+16,1.5e-05,0.0,-2.0,123.25]"
        );

        // And it must sit at report_data[0..32].
        let model = model_quote();
        let (record, quote) = record_over(actions(), &model);
        assert_eq!(&quote.report_data[0..32], &hex::decode(sorted).unwrap()[..]);
        assert!(
            check_compose_manager(&record, |_| Some(quote.clone()), &model, NONCE, None).is_ok()
        );
    }

    #[test]
    fn an_actions_list_not_matching_report_data_is_refused() {
        let model = model_quote();
        let (record, quote) = record_over(actions(), &model);
        // An action added after the quote was made: the record's list no
        // longer hashes to what the quote binds, whatever its own
        // `actions_hash` field claims.
        let mut grown = record.clone();
        grown.actions.push(json!({"action": "compose_up", "container": "logger", "image": "registry.example/logger", "timestamp": 1}));
        grown.actions_hash = hex::encode(compose_actions_hash(&grown.actions).unwrap());
        assert_eq!(
            check_compose_manager(&grown, |_| Some(quote.clone()), &model, NONCE, None),
            Err(ReplayRefusal::ActionsHashMismatch)
        );
        // The list intact, the record's own claim of its hash wrong.
        let mut claimed = record.clone();
        claimed.actions_hash = "00".repeat(32);
        assert_eq!(
            check_compose_manager(&claimed, |_| Some(quote.clone()), &model, NONCE, None),
            Err(ReplayRefusal::ActionsHashMismatch)
        );
        // An action with a key the format does not define is not read.
        let mut extra = record.clone();
        extra.actions[0]["command"] = json!("sh");
        assert_eq!(
            check_compose_manager(&extra, |_| Some(quote.clone()), &model, NONCE, None),
            Err(ReplayRefusal::ComposeManagerMalformed)
        );
    }

    #[test]
    fn an_allowlisted_compose_action_is_accepted() {
        let model = model_quote();
        let (record, quote) = record_over(actions(), &model);
        let checked = check_compose_manager(
            &record,
            |_| Some(quote.clone()),
            &model,
            NONCE,
            Some(&allowlist().compose_actions),
        )
        .expect("every action is listed");
        assert_eq!(
            checked[0],
            ComposeAction {
                action: "compose_up".into(),
                container: Some("app".into()),
                image: Some("registry.example/app@sha256:ab".into()),
            }
        );
        // The timestamp is not compared: the same action at another time.
        let mut later = actions();
        later[0]["timestamp"] = json!(1_800_000_000u64);
        let (record, quote) = record_over(later, &model);
        assert!(
            check_compose_manager(
                &record,
                |_| Some(quote.clone()),
                &model,
                NONCE,
                Some(&allowlist().compose_actions)
            )
            .is_ok()
        );
    }

    #[test]
    fn a_compose_action_with_a_disallowed_image_is_refused() {
        let model = model_quote();
        let mut swapped = actions();
        swapped[0]["image"] = json!("registry.example/app-that-logs@sha256:cd");
        let (record, quote) = record_over(swapped, &model);
        assert_eq!(
            check_compose_manager(
                &record,
                |_| Some(quote.clone()),
                &model,
                NONCE,
                Some(&allowlist().compose_actions)
            ),
            Err(ReplayRefusal::ComposeActionNotAllowed)
        );
    }

    #[test]
    fn a_compose_action_of_an_unlisted_kind_or_container_is_refused() {
        let model = model_quote();
        for (field, value) in [
            ("action", json!("compose_down")),
            ("container", json!("sidecar")),
            ("container", Value::Null),
        ] {
            let mut altered = actions();
            altered[0][field] = value;
            let (record, quote) = record_over(altered, &model);
            assert_eq!(
                check_compose_manager(
                    &record,
                    |_| Some(quote.clone()),
                    &model,
                    NONCE,
                    Some(&allowlist().compose_actions)
                ),
                Err(ReplayRefusal::ComposeActionNotAllowed),
                "{field}"
            );
        }
        // The placeholder allowlist, which lists nothing, refuses every
        // action.
        let (record, quote) = record_over(actions(), &model);
        assert_eq!(
            check_compose_manager(&record, |_| Some(quote.clone()), &model, NONCE, Some(&[])),
            Err(ReplayRefusal::ComposeActionNotAllowed)
        );
    }

    // -- the allowlist ------------------------------------------------------

    #[test]
    fn the_placeholder_allowlist_parses_and_matches_nothing() {
        let placeholder = Allowlist::from_json(&json!({
            "rtmr3": {"app_id": "", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []},
            "compose_actions": [],
        }))
        .expect("the shape-valid placeholder parses");
        let events = ten_events();
        let replayed = replay_rtmr3(Some(&log_of(&events)), &quote_replaying(&events)).unwrap();
        assert!(check_rtmr3_allowlist(&replayed, &placeholder.rtmr3).is_err());
        assert_eq!(
            Allowlist::from_json(&placeholder.to_json()),
            Ok(placeholder)
        );
    }

    #[test]
    fn an_allowlist_round_trips_and_refuses_a_bad_shape() {
        let list = allowlist();
        assert_eq!(Allowlist::from_json(&list.to_json()), Ok(list.clone()));
        let good = list.to_json();
        let mut cases: Vec<Value> = Vec::new();
        let mut extra = good.clone();
        extra["comment"] = json!("x");
        cases.push(extra);
        let mut upper = good.clone();
        upper["rtmr3"]["app_id"] = json!("AB");
        cases.push(upper);
        let mut odd = good.clone();
        odd["rtmr3"]["mr_kms"] = json!("abc");
        cases.push(odd);
        let mut unknown_marker = good.clone();
        unknown_marker["rtmr3"]["markers"][0]["name"] = json!("instance-id");
        cases.push(unknown_marker);
        let mut repeated_marker = good.clone();
        let first = repeated_marker["rtmr3"]["markers"][0].clone();
        repeated_marker["rtmr3"]["markers"]
            .as_array_mut()
            .unwrap()
            .push(first);
        cases.push(repeated_marker);
        let mut timestamped = good.clone();
        timestamped["compose_actions"][0]["timestamp"] = json!(1);
        cases.push(timestamped);
        let mut repeated_action = good.clone();
        let first = repeated_action["compose_actions"][0].clone();
        repeated_action["compose_actions"]
            .as_array_mut()
            .unwrap()
            .push(first);
        cases.push(repeated_action);
        let mut missing = good.clone();
        missing["rtmr3"]
            .as_object_mut()
            .unwrap()
            .remove("key_provider");
        cases.push(missing);
        for case in cases {
            assert_eq!(
                Allowlist::from_json(&case),
                Err(AllowlistMalformed),
                "{case}"
            );
        }
    }

    #[test]
    fn refusal_labels_are_stable_and_carry_nothing() {
        let labels: Vec<&str> = ReplayRefusal::ALL.iter().map(|r| r.label()).collect();
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len(), "labels are distinct");
        for label in labels {
            assert!(
                label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{label}"
            );
        }
        // Debug of what a check reads carries no payload.
        let events = ten_events();
        let replayed = replay_rtmr3(Some(&log_of(&events)), &quote_replaying(&events)).unwrap();
        let shown = format!("{replayed:?} {:?}", allowlist());
        for value in [
            APP_ID,
            INSTANCE_ID,
            MR_KMS,
            OS_IMAGE_HASH,
            KEY_PROVIDER,
            COMPOSE_HASH,
        ] {
            assert!(!shown.contains(value), "Debug carries a payload");
        }
        assert!(!format!("{:?}", allowlist().compose_actions).contains("registry.example"));
    }
}
