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
//! ([`compose_actions_hash`]), over the raw actions exactly as received. The
//! record's own `actions_hash` must equal that recomputed value.
//!
//! Each action is parsed by kind ([`ComposeAction`]) with exactly that
//! kind's keys; an unknown kind, a missing or extra key or a wrong type
//! refuses. With an allowlist, every action must be allowed by it (owner
//! decision S17, [`ComposeAllowlist`]): the compose manager's image by
//! digest, a `compose_up`'s compose file by path, SHA-256 and commit, a
//! `compose_down`'s by path and commit. Services, tag and timestamp are not
//! compared.
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

/// The container a `compose_manager_started` action must name.
pub const COMPOSE_MANAGER_CONTAINER: &str = "compose-manager";

/// The keys of each compose-action kind, and no others (the live record of
/// 2026-10-22). Every action carries `action` and a string `timestamp`.
const MANAGER_STARTED_KEYS: [&str; 4] = ["action", "container", "image", "timestamp"];
const COMPOSE_UP_KEYS: [&str; 7] = [
    "action",
    "tag",
    "commit",
    "file",
    "file_sha256",
    "services",
    "timestamp",
];
const COMPOSE_DOWN_KEYS: [&str; 6] = ["action", "tag", "commit", "file", "services", "timestamp"];

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
    /// a string `action` and exactly its kind's keys, each of its type (see
    /// [`ComposeAction`]).
    ComposeManagerMalformed,
    /// An action's kind is none of `compose_manager_started`, `compose_up`
    /// and `compose_down`.
    ComposeActionUnknown,
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
    /// A compose action is not allowed: a `compose_manager_started` whose
    /// container is not [`COMPOSE_MANAGER_CONTAINER`] (refused with or
    /// without an allowlist) or whose image is not listed, a `compose_up`
    /// whose file, file hash and commit are not listed together, or a
    /// `compose_down` whose file and commit are not.
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
            ReplayRefusal::ComposeActionUnknown => "compose_action_unknown",
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
    pub const ALL: [ReplayRefusal; 24] = [
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
        ReplayRefusal::ComposeActionUnknown,
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

/// One compose action, parsed by kind (the live record of 2026-10-22).
/// The `timestamp` every kind carries is read as a string and not kept: it
/// is never compared. It still counts in the actions hash, which is taken
/// over the raw record, never over these.
#[derive(Clone, PartialEq, Eq)]
pub enum ComposeAction {
    /// `compose_manager_started`: the compose manager's own container and
    /// image reference.
    ManagerStarted { container: String, image: String },
    /// `compose_up`: a compose file from NEAR AI's repository, named by
    /// repository path, commit and the file's SHA-256, brought up with the
    /// named services.
    Up {
        tag: String,
        commit: String,
        file: String,
        file_sha256: String,
        services: Vec<String>,
    },
    /// `compose_down`: services of a compose file stopped. It carries no
    /// file hash.
    Down {
        tag: String,
        commit: String,
        file: String,
        services: Vec<String>,
    },
}

impl ComposeAction {
    /// The record's name for this kind.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            ComposeAction::ManagerStarted { .. } => "compose_manager_started",
            ComposeAction::Up { .. } => "compose_up",
            ComposeAction::Down { .. } => "compose_down",
        }
    }

    /// Parse one raw action: a string `action` naming a known kind, exactly
    /// that kind's keys, each a string (`services` a list of strings).
    fn parse(value: &Value) -> Result<Self, ReplayRefusal> {
        let malformed = ReplayRefusal::ComposeManagerMalformed;
        let object = value.as_object().ok_or(malformed)?;
        let kind = object
            .get("action")
            .and_then(Value::as_str)
            .ok_or(malformed)?;
        let keys: &[&str] = match kind {
            "compose_manager_started" => &MANAGER_STARTED_KEYS,
            "compose_up" => &COMPOSE_UP_KEYS,
            "compose_down" => &COMPOSE_DOWN_KEYS,
            _ => return Err(ReplayRefusal::ComposeActionUnknown),
        };
        exact_keys(object, keys).ok_or(malformed)?;
        let text = |name: &str| -> Result<String, ReplayRefusal> {
            object
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or(malformed)
        };
        // Read for its type only.
        text("timestamp")?;
        let services = || -> Result<Vec<String>, ReplayRefusal> {
            object
                .get("services")
                .and_then(Value::as_array)
                .ok_or(malformed)?
                .iter()
                .map(|service| service.as_str().map(str::to_string).ok_or(malformed))
                .collect()
        };
        Ok(match kind {
            "compose_manager_started" => ComposeAction::ManagerStarted {
                container: text("container")?,
                image: text("image")?,
            },
            "compose_up" => ComposeAction::Up {
                tag: text("tag")?,
                commit: text("commit")?,
                file: text("file")?,
                file_sha256: text("file_sha256")?,
                services: services()?,
            },
            _ => ComposeAction::Down {
                tag: text("tag")?,
                commit: text("commit")?,
                file: text("file")?,
                services: services()?,
            },
        })
    }
}

impl fmt::Debug for ComposeAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("ComposeAction");
        out.field("kind", &self.kind());
        match self {
            ComposeAction::ManagerStarted { container, image } => out
                .field("container_len", &container.len())
                .field("image_len", &image.len()),
            ComposeAction::Up {
                tag,
                commit,
                file,
                file_sha256,
                services,
            } => out
                .field("tag_len", &tag.len())
                .field("commit_len", &commit.len())
                .field("file_len", &file.len())
                .field("file_sha256_len", &file_sha256.len())
                .field("service_count", &services.len()),
            ComposeAction::Down {
                tag,
                commit,
                file,
                services,
            } => out
                .field("tag_len", &tag.len())
                .field("commit_len", &commit.len())
                .field("file_len", &file.len())
                .field("service_count", &services.len()),
        };
        out.finish()
    }
}

/// One compose file the allowlist lets the compose manager bring up: its
/// path in NEAR AI's repository, the commit it is taken at and the SHA-256
/// of its contents there (owner decision S17).
#[derive(Clone, PartialEq, Eq)]
pub struct ComposeFile {
    /// The repository path, as the action carries it. Not empty.
    pub file: String,
    /// 64 lower-case hex characters.
    pub file_sha256: String,
    /// 40 lower-case hex characters.
    pub commit: String,
}

impl fmt::Debug for ComposeFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComposeFile")
            .field("file_len", &self.file.len())
            .field("file_sha256_len", &self.file_sha256.len())
            .field("commit_len", &self.commit.len())
            .finish()
    }
}

/// What post-boot compose actions are allowed (owner decision S17, "Pin
/// compose file hashes"). Its JSON form, the allowlist's `compose_actions`:
///
/// ```json
/// {
///   "compose_manager_images": ["<name>@sha256:<64 hex>"],
///   "compose_files": [{"file": "<repository path>", "file_sha256": "<64 hex>", "commit": "<40 hex>"}]
/// }
/// ```
///
/// - `compose_manager_started` is allowed when its container is
///   [`COMPOSE_MANAGER_CONTAINER`] and its image is listed in
///   `compose_manager_images`, exactly; an image is listed by digest.
/// - `compose_up` is allowed when its `file`, `file_sha256` and `commit`
///   are one listed entry. Which `services` it starts is free: the pinned
///   file defines them all.
/// - `compose_down` is allowed when its `file` and `commit` are a listed
///   entry's: stopping containers adds no code. `services` is free.
/// - The `tag` is not matched: a tag is a movable name, the commit is the
///   immutable one, and the file's hash is taken at that commit.
/// - Any other kind refuses ([`ReplayRefusal::ComposeActionUnknown`]).
///
/// Format strictness sits here, not in the record parser: a listed value
/// is lower-case hex of its length, so a record value spelled any other way
/// matches nothing and is refused. Empty lists allow nothing.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ComposeAllowlist {
    /// Distinct digest references, `<name>@sha256:<64 lower-case hex>`.
    pub compose_manager_images: Vec<String>,
    /// At most one entry per (`file`, `commit`).
    pub compose_files: Vec<ComposeFile>,
}

impl fmt::Debug for ComposeAllowlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComposeAllowlist")
            .field(
                "compose_manager_image_count",
                &self.compose_manager_images.len(),
            )
            .field("compose_files", &self.compose_files)
            .finish()
    }
}

impl ComposeAllowlist {
    /// Whether both lists are empty: the placeholder, which allows nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.compose_manager_images.is_empty() && self.compose_files.is_empty()
    }

    /// Whether this allowlist allows `action` (rules above).
    #[must_use]
    pub fn allows(&self, action: &ComposeAction) -> bool {
        match action {
            ComposeAction::ManagerStarted { container, image } => {
                container == COMPOSE_MANAGER_CONTAINER
                    && self.compose_manager_images.iter().any(|i| i == image)
            }
            ComposeAction::Up {
                commit,
                file,
                file_sha256,
                ..
            } => self.compose_files.iter().any(|listed| {
                listed.file == *file
                    && listed.file_sha256 == *file_sha256
                    && listed.commit == *commit
            }),
            ComposeAction::Down { commit, file, .. } => self
                .compose_files
                .iter()
                .any(|listed| listed.file == *file && listed.commit == *commit),
        }
    }

    /// Parse the allowlist's `compose_actions` value.
    pub fn from_json(value: &Value) -> Result<Self, AllowlistMalformed> {
        let object = value.as_object().ok_or(AllowlistMalformed)?;
        exact_keys(object, &["compose_manager_images", "compose_files"])
            .ok_or(AllowlistMalformed)?;
        let mut images: Vec<String> = Vec::new();
        for image in object
            .get("compose_manager_images")
            .and_then(Value::as_array)
            .ok_or(AllowlistMalformed)?
        {
            let image = image.as_str().ok_or(AllowlistMalformed)?;
            if !is_digest_reference(image) || images.iter().any(|i| i == image) {
                return Err(AllowlistMalformed);
            }
            images.push(image.to_string());
        }
        let mut files: Vec<ComposeFile> = Vec::new();
        for entry in object
            .get("compose_files")
            .and_then(Value::as_array)
            .ok_or(AllowlistMalformed)?
        {
            let entry = entry.as_object().ok_or(AllowlistMalformed)?;
            exact_keys(entry, &["file", "file_sha256", "commit"]).ok_or(AllowlistMalformed)?;
            let text = |name: &str| {
                entry
                    .get(name)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or(AllowlistMalformed)
            };
            let listed = ComposeFile {
                file: text("file")?,
                file_sha256: text("file_sha256")?,
                commit: text("commit")?,
            };
            if listed.file.is_empty()
                || !is_lower_hex_of(&listed.file_sha256, 64)
                || !is_lower_hex_of(&listed.commit, 40)
                || files
                    .iter()
                    .any(|f| f.file == listed.file && f.commit == listed.commit)
            {
                return Err(AllowlistMalformed);
            }
            files.push(listed);
        }
        Ok(ComposeAllowlist {
            compose_manager_images: images,
            compose_files: files,
        })
    }

    /// The JSON value [`ComposeAllowlist::from_json`] reads, fields in a
    /// fixed order.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert(
            "compose_manager_images".into(),
            Value::Array(
                self.compose_manager_images
                    .iter()
                    .map(|image| Value::from(image.as_str()))
                    .collect(),
            ),
        );
        out.insert(
            "compose_files".into(),
            Value::Array(
                self.compose_files
                    .iter()
                    .map(|f| {
                        let mut o = Map::new();
                        o.insert("file".into(), f.file.clone().into());
                        o.insert("file_sha256".into(), f.file_sha256.clone().into());
                        o.insert("commit".into(), f.commit.clone().into());
                        Value::Object(o)
                    })
                    .collect(),
            ),
        );
        Value::Object(out)
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
    pub compose_actions: ComposeAllowlist,
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
        let compose_actions =
            ComposeAllowlist::from_json(object.get("compose_actions").ok_or(AllowlistMalformed)?)?;
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
        out.insert("compose_actions".into(), self.compose_actions.to_json());
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
        // is the authority for what they claim. So the record's
        // `event_log`, a JSON-encoded string in the live record (the model
        // entry's is a list, and that one is replayed), reads in any form.
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
/// [`ReplayRefusal::ComposeManagerMalformed`] or
/// [`ReplayRefusal::ComposeActionUnknown`] (an action's shape),
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
    allowlist: Option<&ComposeAllowlist>,
) -> Result<Vec<ComposeAction>, ReplayRefusal> {
    // The actions' shape first: nothing below is worth a quote verification
    // if they cannot be read.
    let actions = record
        .actions
        .iter()
        .map(ComposeAction::parse)
        .collect::<Result<Vec<_>, _>>()?;

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
    // Over the raw actions as received, every field included: never over
    // the parsed ones, which drop the timestamp.
    let recomputed = compose_actions_hash(&record.actions)?;
    if quote.report_data.get(0..32) != Some(&recomputed[..])
        || record.actions_hash != hex::encode(recomputed)
    {
        return Err(ReplayRefusal::ActionsHashMismatch);
    }
    // The compose manager runs in its own named container, allowlist or not.
    if actions.iter().any(|action| {
        matches!(action, ComposeAction::ManagerStarted { container, .. }
            if container != COMPOSE_MANAGER_CONTAINER)
    }) {
        return Err(ReplayRefusal::ComposeActionNotAllowed);
    }
    if let Some(allowed) = allowlist {
        if actions.iter().any(|action| !allowed.allows(action)) {
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

/// `text` is `len` lower-case hex characters.
fn is_lower_hex_of(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `<name>@sha256:<64 lower-case hex>`, the name not empty: an image named
/// by its content, never by a movable tag.
fn is_digest_reference(image: &str) -> bool {
    image
        .rsplit_once("@sha256:")
        .is_some_and(|(name, digest)| !name.is_empty() && is_lower_hex_of(digest, 64))
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
    /// Synthetic compose-action values, in the live record's formats.
    const COMPOSE_FILE: &str = "example/models.yaml";
    const FILE_SHA256: &str = "f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1";
    const COMMIT: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";

    fn manager_image() -> String {
        format!(
            "registry.example/compose-manager@sha256:{}",
            "ab".repeat(32)
        )
    }

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
            "compose_actions": {
                "compose_manager_images": [manager_image()],
                "compose_files": [
                    {"file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "commit": COMMIT},
                ],
            },
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

    /// One action of each kind, structurally exact to the live record:
    /// string timestamps, a digest-pinned compose-manager image, and a
    /// compose file named by path, commit and SHA-256. Keys are deliberately
    /// out of order, as a provider may send them. Every value is synthetic.
    fn actions() -> Vec<Value> {
        vec![
            json!({"timestamp": "2026-01-01T00:00:00Z", "image": manager_image(), "container": "compose-manager", "action": "compose_manager_started"}),
            json!({"action": "compose_up", "tag": "v0.0.1", "commit": COMMIT, "file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "services": ["model-a", "proxy"], "timestamp": "2026-01-01T00:00:01Z"}),
            json!({"services": ["proxy"], "file": COMPOSE_FILE, "commit": COMMIT, "tag": "v0.0.1", "action": "compose_down", "timestamp": "2026-01-01T00:00:02Z"}),
        ]
    }

    /// A record whose quote is the model quote's TD, bound to our nonce and
    /// to `actions`, carrying every key the live record carries: its
    /// `event_log` a JSON-encoded **string** (unlike the model entry's list)
    /// and `vm_config` a string.
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
                "event_log": "[{\"imr\":3,\"event\":\"app-id\"}]",
                "nonce": NONCE,
                "nonce_source": "client",
                "quote": "0badc0de",
                "report_data": hex::encode(&quote.report_data),
                "vm_config": "{\"cpu_count\":1}",
            }
        }))
        .expect("the record reads");
        (record, quote)
    }

    fn model_quote() -> VerifiedQuote {
        quote_replaying(&ten_events())
    }

    /// The record's check, against the synthetic allowlist.
    fn check_listed(actions: Vec<Value>) -> Result<Vec<ComposeAction>, ReplayRefusal> {
        let model = model_quote();
        let (record, quote) = record_over(actions, &model);
        check_compose_manager(
            &record,
            |_| Some(quote.clone()),
            &model,
            NONCE,
            Some(&allowlist().compose_actions),
        )
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
        assert_eq!(checked.len(), 3);

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
    fn the_record_reads_with_its_event_log_as_a_json_string_or_a_list() {
        // The live record's own `event_log` is a JSON-encoded string, and
        // `vm_config` a string; `record_over` builds it that way, and it
        // reads and passes. The record's copy of the log is never replayed
        // (the model entry's is), so a list there reads too.
        let model = model_quote();
        let (record, quote) = record_over(actions(), &model);
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
        let hash = hex::encode(compose_actions_hash(&actions()).unwrap());
        let listed = ComposeManagerRecord::from_entry(&json!({
            "compose_manager_attestation": {
                "actions": actions(),
                "actions_hash": hash,
                "event_log": [{"imr": 3, "event": "app-id"}],
                "quote": "0badc0de",
            }
        }))
        .expect("a list-form log reads too");
        assert_eq!(listed.actions, record.actions);
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
    fn actions_hash_is_recomputed_as_sorted_compact_json_over_the_raw_actions() {
        // Known answer over the three heterogeneous actions of `actions()`,
        // computed with Python 3 over the same list (keys in any order):
        //   img = "registry.example/compose-manager@sha256:" + "ab" * 32
        //   actions = [
        //     {"action": "compose_manager_started", "container": "compose-manager",
        //      "image": img, "timestamp": "2026-01-01T00:00:00Z"},
        //     {"action": "compose_up", "tag": "v0.0.1", "commit": "c1" * 20,
        //      "file": "example/models.yaml", "file_sha256": "f1" * 32,
        //      "services": ["model-a", "proxy"], "timestamp": "2026-01-01T00:00:01Z"},
        //     {"action": "compose_down", "tag": "v0.0.1", "commit": "c1" * 20,
        //      "file": "example/models.yaml", "services": ["proxy"],
        //      "timestamp": "2026-01-01T00:00:02Z"}]
        //   text = json.dumps(actions, separators=(",", ":"), sort_keys=True)
        //   hashlib.sha256(text.encode()).hexdigest()
        let text = format!(
            concat!(
                "[{{\"action\":\"compose_manager_started\",\"container\":\"compose-manager\",",
                "\"image\":\"registry.example/compose-manager@sha256:{ab}\",",
                "\"timestamp\":\"2026-01-01T00:00:00Z\"}},",
                "{{\"action\":\"compose_up\",\"commit\":\"{c1}\",\"file\":\"example/models.yaml\",",
                "\"file_sha256\":\"{f1}\",\"services\":[\"model-a\",\"proxy\"],\"tag\":\"v0.0.1\",",
                "\"timestamp\":\"2026-01-01T00:00:01Z\"}},",
                "{{\"action\":\"compose_down\",\"commit\":\"{c1}\",\"file\":\"example/models.yaml\",",
                "\"services\":[\"proxy\"],\"tag\":\"v0.0.1\",\"timestamp\":\"2026-01-01T00:00:02Z\"}}]"
            ),
            ab = "ab".repeat(32),
            c1 = COMMIT,
            f1 = FILE_SHA256,
        );
        let mut written = String::new();
        write_python_json(&Value::Array(actions()), &mut written).unwrap();
        assert_eq!(
            written, text,
            "Python's sort_keys compact form, byte for byte"
        );
        let sorted = "9cf4b48c50f1923664a8fdd332c5ac40760ff53c92f6e3482eed369d4d77242b";
        assert_eq!(
            hex::encode(compose_actions_hash(&actions()).unwrap()),
            sorted
        );
        // The same actions unsorted, and with Python's default separators,
        // hash to other values (also from Python), which is why neither is
        // used.
        let unsorted = "9dd5d2628f58795cd0259b017a17d52f468627c167ebcee0375bbbd8d42e30b4";
        let default = "ff1ecfaeb38190b43227b699b1b5d30fbbe64dee1aafadc6ec0418841474831b";
        assert_ne!(sorted, unsorted);
        assert_ne!(sorted, default);
        // serde_json's own compact form keeps this crate's insertion order,
        // so it is not the sorted one.
        let serde_form = serde_json::to_vec(&Value::Array(actions())).unwrap();
        assert_ne!(hex::encode(Sha256::digest(&serde_form)), sorted);

        // Python's escaping and float forms, character for character:
        //   json.dumps(["café\U0001F600\x01\x7f/", 1e16, 1.5e-5, 0.0,
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
        grown.actions.push(actions()[1].clone());
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
        // A field the parsed action does not keep (the timestamp) still
        // counts: the hash is over the raw action, not a re-serialised one.
        let mut retimed = record.clone();
        retimed.actions[0]["timestamp"] = json!("2026-01-01T00:00:09Z");
        assert_eq!(
            check_compose_manager(&retimed, |_| Some(quote.clone()), &model, NONCE, None),
            Err(ReplayRefusal::ActionsHashMismatch)
        );
    }

    #[test]
    fn each_kind_is_accepted_when_listed() {
        let checked = check_listed(actions()).expect("every action is listed");
        assert_eq!(
            checked,
            vec![
                ComposeAction::ManagerStarted {
                    container: "compose-manager".into(),
                    image: manager_image(),
                },
                ComposeAction::Up {
                    tag: "v0.0.1".into(),
                    commit: COMMIT.into(),
                    file: COMPOSE_FILE.into(),
                    file_sha256: FILE_SHA256.into(),
                    services: vec!["model-a".into(), "proxy".into()],
                },
                ComposeAction::Down {
                    tag: "v0.0.1".into(),
                    commit: COMMIT.into(),
                    file: COMPOSE_FILE.into(),
                    services: vec!["proxy".into()],
                },
            ]
        );
        // Each kind alone.
        for action in actions() {
            assert!(check_listed(vec![action.clone()]).is_ok(), "{action}");
        }
        // An empty list of actions has nothing to refuse.
        assert_eq!(check_listed(Vec::new()), Ok(Vec::new()));
    }

    #[test]
    fn an_unknown_action_kind_is_refused() {
        for kind in ["compose_restart", "Compose_up", ""] {
            let mut unknown = actions();
            unknown[1]["action"] = json!(kind);
            assert_eq!(
                check_listed(unknown),
                Err(ReplayRefusal::ComposeActionUnknown),
                "{kind}"
            );
        }
        // An action with no kind, or a kind that is not a string, is not an
        // unknown kind but a malformed action.
        let mut kindless = actions();
        kindless[0].as_object_mut().unwrap().remove("action");
        assert_eq!(
            check_listed(kindless),
            Err(ReplayRefusal::ComposeManagerMalformed)
        );
        let mut numbered = actions();
        numbered[0]["action"] = json!(1);
        assert_eq!(
            check_listed(numbered),
            Err(ReplayRefusal::ComposeManagerMalformed)
        );
        let mut not_object = actions();
        not_object[0] = json!("compose_up");
        assert_eq!(
            check_listed(not_object),
            Err(ReplayRefusal::ComposeManagerMalformed)
        );
    }

    #[test]
    fn an_extra_or_missing_key_is_refused_per_kind() {
        for (index, action) in actions().iter().enumerate() {
            let object = action.as_object().unwrap();
            // Every key but `action` dropped in turn.
            for key in object.keys().filter(|k| *k != "action") {
                let mut missing = actions();
                missing[index].as_object_mut().unwrap().remove(key);
                assert_eq!(
                    check_listed(missing),
                    Err(ReplayRefusal::ComposeManagerMalformed),
                    "{index} without {key}"
                );
            }
            // A key the kind does not define, including another kind's.
            for key in ["command", "file_sha256", "container", "image", "tag"] {
                if object.contains_key(key) {
                    continue;
                }
                let mut extra = actions();
                extra[index][key] = json!("x");
                assert_eq!(
                    check_listed(extra),
                    Err(ReplayRefusal::ComposeManagerMalformed),
                    "{index} with {key}"
                );
            }
        }
    }

    #[test]
    fn a_key_of_the_wrong_type_is_refused() {
        let cases: [(usize, &str, Value); 9] = [
            // A timestamp is a string in the live record; a number is not.
            (0, "timestamp", json!(1_700_000_000u64)),
            (0, "image", Value::Null),
            (0, "container", Value::Null),
            (1, "file_sha256", json!(1)),
            (1, "services", json!("model-a")),
            (1, "services", json!(["model-a", 1])),
            (1, "commit", Value::Null),
            (2, "tag", json!(["v0.0.1"])),
            (2, "file", json!({})),
        ];
        for (index, key, value) in cases {
            let mut wrong = actions();
            wrong[index][key] = value.clone();
            assert_eq!(
                check_listed(wrong),
                Err(ReplayRefusal::ComposeManagerMalformed),
                "{index} {key} = {value}"
            );
        }
    }

    #[test]
    fn a_compose_manager_image_not_listed_is_refused() {
        let mut swapped = actions();
        swapped[0]["image"] = json!(format!(
            "registry.example/compose-manager@sha256:{}",
            "cd".repeat(32)
        ));
        assert_eq!(
            check_listed(swapped),
            Err(ReplayRefusal::ComposeActionNotAllowed)
        );
        // The listed image under a tag rather than its digest is not it.
        let mut tagged = actions();
        tagged[0]["image"] = json!("registry.example/compose-manager:latest");
        assert_eq!(
            check_listed(tagged),
            Err(ReplayRefusal::ComposeActionNotAllowed)
        );
        // The listed image in another container is not the compose manager,
        // with or without an allowlist.
        let mut renamed = actions();
        renamed[0]["container"] = json!("sidecar");
        assert_eq!(
            check_listed(renamed.clone()),
            Err(ReplayRefusal::ComposeActionNotAllowed)
        );
        let model = model_quote();
        let (record, quote) = record_over(renamed, &model);
        assert_eq!(
            check_compose_manager(&record, |_| Some(quote.clone()), &model, NONCE, None),
            Err(ReplayRefusal::ComposeActionNotAllowed)
        );
    }

    #[test]
    fn compose_up_with_an_unlisted_file_hash_commit_or_path_is_refused() {
        for (key, value) in [
            ("file_sha256", "f2".repeat(32)),
            ("commit", "c2".repeat(20)),
            ("file", "example/other.yaml".to_string()),
        ] {
            let mut altered = actions();
            altered[1][key] = json!(value);
            assert_eq!(
                check_listed(vec![altered[1].clone()]),
                Err(ReplayRefusal::ComposeActionNotAllowed),
                "{key}"
            );
        }
    }

    #[test]
    fn compose_down_for_an_unlisted_file_or_commit_is_refused() {
        for (key, value) in [
            ("commit", "c2".repeat(20)),
            ("file", "example/other.yaml".to_string()),
        ] {
            let mut altered = actions();
            altered[2][key] = json!(value);
            assert_eq!(
                check_listed(vec![altered[2].clone()]),
                Err(ReplayRefusal::ComposeActionNotAllowed),
                "{key}"
            );
        }
    }

    #[test]
    fn services_tag_and_timestamp_are_free() {
        // Which services a listed file starts or stops is free: the pinned
        // file defines them all. The tag is informational, the commit is
        // what is pinned. The timestamp is never compared.
        let mut free = actions();
        free[1]["services"] = json!(["model-b", "logger", "proxy"]);
        free[2]["services"] = json!([]);
        free[1]["tag"] = json!("v9.9.9");
        free[2]["tag"] = json!("");
        free[0]["timestamp"] = json!("2027-06-01T12:00:00Z");
        free[1]["timestamp"] = json!("not a time at all");
        assert!(check_listed(free).is_ok());
    }

    #[test]
    fn the_placeholder_allowlist_refuses_every_action() {
        let placeholder = ComposeAllowlist::default();
        let model = model_quote();
        for action in actions() {
            let (record, quote) = record_over(vec![action.clone()], &model);
            assert_eq!(
                check_compose_manager(
                    &record,
                    |_| Some(quote.clone()),
                    &model,
                    NONCE,
                    Some(&placeholder)
                ),
                Err(ReplayRefusal::ComposeActionNotAllowed),
                "{action}"
            );
        }
    }

    // -- the allowlist ------------------------------------------------------

    #[test]
    fn the_placeholder_allowlist_parses_and_matches_nothing() {
        let placeholder = Allowlist::from_json(&json!({
            "rtmr3": {"app_id": "", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []},
            "compose_actions": {"compose_manager_images": [], "compose_files": []},
        }))
        .expect("the shape-valid placeholder parses");
        let events = ten_events();
        let replayed = replay_rtmr3(Some(&log_of(&events)), &quote_replaying(&events)).unwrap();
        assert!(check_rtmr3_allowlist(&replayed, &placeholder.rtmr3).is_err());
        assert!(placeholder.compose_actions.is_empty());
        assert_eq!(
            Allowlist::from_json(&placeholder.to_json()),
            Ok(placeholder)
        );
    }

    /// The repository's committed allowlist is the reviewed one (#1578), not
    /// the placeholder: it parses, every boot value is filled in, all four
    /// markers are listed, and it allows at least one compose-manager image
    /// and one compose file.
    #[test]
    fn the_committed_allowlist_is_complete() {
        let committed: Value = serde_json::from_str(include_str!(
            "../../../deploy/staging-gcp/near-ai-rtmr3-allowlist.json"
        ))
        .unwrap();
        let committed = Allowlist::from_json(&committed).expect("the committed allowlist parses");
        assert!(!committed.rtmr3.app_id.is_empty());
        assert!(!committed.rtmr3.mr_kms.is_empty());
        assert!(!committed.rtmr3.os_image_hash.is_empty());
        assert!(!committed.rtmr3.key_provider.is_empty());
        assert_eq!(committed.rtmr3.markers.len(), 4);
        assert!(!committed.compose_actions.is_empty());
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
        let mut missing = good.clone();
        missing["rtmr3"]
            .as_object_mut()
            .unwrap()
            .remove("key_provider");
        cases.push(missing);
        // The #1575 list form of `compose_actions` is not this schema.
        let mut old_form = good.clone();
        old_form["compose_actions"] =
            json!([{"action": "compose_up", "container": "app", "image": "registry.example/app"}]);
        cases.push(old_form);
        let mut compose_extra = good.clone();
        compose_extra["compose_actions"]["compose_services"] = json!([]);
        cases.push(compose_extra);
        let mut compose_missing = good.clone();
        compose_missing["compose_actions"]
            .as_object_mut()
            .unwrap()
            .remove("compose_files");
        cases.push(compose_missing);
        // An image listed by tag, not by digest; a digest of the wrong
        // length or case; no name before the digest.
        for image in [
            "registry.example/compose-manager:latest".to_string(),
            format!(
                "registry.example/compose-manager@sha256:{}",
                "ab".repeat(31)
            ),
            format!(
                "registry.example/compose-manager@sha256:{}",
                "AB".repeat(32)
            ),
            format!("@sha256:{}", "ab".repeat(32)),
        ] {
            let mut case = good.clone();
            case["compose_actions"]["compose_manager_images"] = json!([image]);
            cases.push(case);
        }
        let mut repeated_image = good.clone();
        repeated_image["compose_actions"]["compose_manager_images"] =
            json!([manager_image(), manager_image()]);
        cases.push(repeated_image);
        // A compose file whose hash or commit is not lower-case hex of its
        // length, whose path is empty, with a key missing or extra.
        for (key, value) in [
            ("file_sha256", json!("f1".repeat(31))),
            ("file_sha256", json!("F1".repeat(32))),
            ("commit", json!("c1".repeat(32))),
            ("commit", json!("C1".repeat(20))),
            ("file", json!("")),
            ("file", json!(1)),
            ("tag", json!("v0.0.1")),
        ] {
            let mut case = good.clone();
            case["compose_actions"]["compose_files"][0][key] = value;
            cases.push(case);
        }
        let mut file_missing = good.clone();
        file_missing["compose_actions"]["compose_files"][0]
            .as_object_mut()
            .unwrap()
            .remove("commit");
        cases.push(file_missing);
        // Two entries for one (file, commit): compose_down matches on that
        // pair, so two hashes for it are a contradiction, not a choice.
        let mut contradictory = good.clone();
        let mut second = contradictory["compose_actions"]["compose_files"][0].clone();
        second["file_sha256"] = json!("f2".repeat(32));
        contradictory["compose_actions"]["compose_files"]
            .as_array_mut()
            .unwrap()
            .push(second);
        cases.push(contradictory);
        for case in cases {
            assert_eq!(
                Allowlist::from_json(&case),
                Err(AllowlistMalformed),
                "{case}"
            );
        }
        // The same file at another commit is another entry.
        let mut two_commits = good.clone();
        let mut second = two_commits["compose_actions"]["compose_files"][0].clone();
        second["commit"] = json!("c2".repeat(20));
        two_commits["compose_actions"]["compose_files"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(Allowlist::from_json(&two_commits).is_ok());
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
        let checked = check_listed(actions()).unwrap();
        let shown = format!("{replayed:?} {:?} {checked:?}", allowlist());
        for value in [
            APP_ID,
            INSTANCE_ID,
            MR_KMS,
            OS_IMAGE_HASH,
            KEY_PROVIDER,
            COMPOSE_HASH,
            COMPOSE_FILE,
            FILE_SHA256,
            COMMIT,
            "registry.example",
            "model-a",
            "v0.0.1",
        ] {
            assert!(!shown.contains(value), "Debug carries {value}");
        }
    }
}
