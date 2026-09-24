// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Versioned contracts for the four-phase Trace Commons pipeline.
//!
//! The contracts in this module intentionally contain no persistence or
//! production policy implementation. Implementations consume these types
//! through trait objects so a policy backend can be replaced without changing
//! a runner.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const PIPELINE_OUTCOME_SCHEMA_ID: &str = "trace_commons.pipeline_outcome";
pub const PIPELINE_OUTCOME_SCHEMA_VERSION: u32 = 1;
pub const BUNDLE_MANIFEST_FORMAT_VERSION: u32 = 1;
pub const MICROCREDITS_PER_CREDIT: u64 = 1_000_000;
pub const MAX_INSTRUMENT_ID_LEN: usize = 64;
pub const TRACE_CREDIT_INSTRUMENT_ID: &str = "trace_credit";
/// Largest Trace Credit award, in microcredits. The credit ledger stores
/// amounts as signed 64-bit integers (`BIGINT`).
pub const MAX_TRACE_CREDIT_MICROCREDITS: u64 = i64::MAX as u64;
/// Trace Credit's pinned NEP-141 `decimals`. One atomic unit is then one
/// microcredit, so the microcredit ledger needs no scale conversion.
pub const TRACE_CREDIT_DECIMALS: u8 = 6;
/// Largest pinned `decimals`: one whole token, 10^38, still fits in `u128`.
pub const MAX_INSTRUMENT_DECIMALS: u8 = 38;
pub const INDEX_COMMAND_SCHEMA: &str = "trace_commons.pipeline_index_command.v1";

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// A bounded label: 1 to 64 lowercase ASCII letters, digits, `_`, `-`, or `.`.
fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_INSTRUMENT_ID_LEN
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}

/// A NEAR account id, as `near-account-id` checks it: 2 to 64 lowercase
/// letters and digits, with single `-`, `_`, or `.` separators between them.
fn is_near_account_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    let separator = |byte: &u8| matches!(byte, b'-' | b'_' | b'.');
    (2..=64).contains(&bytes.len())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || separator(byte))
        && !bytes.first().is_some_and(separator)
        && !bytes.last().is_some_and(separator)
        && !bytes
            .windows(2)
            .any(|pair| separator(&pair[0]) && separator(&pair[1]))
}

/// An EIP-155 chain id in canonical decimal: nonzero, with no leading zero.
fn is_evm_chain_id(value: &str) -> bool {
    !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

/// A lowercase `0x`-prefixed EVM address. Lowercase keeps one spelling per
/// contract, so the bundle identifier does not depend on EIP-55 casing.
fn is_evm_address(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .is_some_and(|hex| is_lower_hex(hex, 40))
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| is_lower_hex(hex, 64))
}

/// Every present value must be a lowercase SHA-256 reference.
fn require_sha256<'a>(
    values: impl IntoIterator<Item = Option<&'a str>>,
) -> Result<(), ContractError> {
    if values.into_iter().flatten().all(is_sha256) {
        Ok(())
    } else {
        Err(ContractError::MalformedHash)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Admission,
    Review,
    Score,
    Settle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(try_from = "String", into = "String")]
pub struct ReasonCode(String);

impl TryFrom<String> for ReasonCode {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ReasonCode> for String {
    fn from(code: ReasonCode) -> Self {
        code.0
    }
}

impl ReasonCode {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ContractError::InvalidReasonCode);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A tenant's derived storage reference: `tenant_sha256:` and 32 lowercase
/// hex digits, the shape the server derives from the tenant identifier. Every
/// index and storage seam is keyed by it, so a policy that reads an index uses
/// this value and never sees the raw tenant identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantStorageRef(String);

impl TenantStorageRef {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if !value
            .strip_prefix("tenant_sha256:")
            .is_some_and(|hex| is_lower_hex(hex, 32))
        {
            return Err(ContractError::InvalidTenantStorageRef);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(try_from = "String", into = "String")]
pub struct InstrumentId(String);

impl TryFrom<String> for InstrumentId {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<InstrumentId> for String {
    fn from(id: InstrumentId) -> Self {
        id.0
    }
}

impl InstrumentId {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if !is_safe_identifier(&value) {
            return Err(ContractError::InvalidInstrumentId);
        }
        Ok(Self(value))
    }

    pub fn trace_credit() -> Self {
        Self(TRACE_CREDIT_INSTRUMENT_ID.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An amount in an instrument's smallest unit. `u128` holds a NEP-141
/// balance.
///
/// The wire form is a canonical decimal string, as NEAR's `U128` uses: a JSON
/// number above 2^53 loses precision in JavaScript and in any decoder that
/// reads numbers as `f64`. Loading accepts ASCII digits only, with no sign, no
/// leading zero, and no value above `u128::MAX`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(try_from = "String", into = "String")]
pub struct AtomicUnits(u128);

impl TryFrom<String> for AtomicUnits {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<AtomicUnits> for String {
    fn from(units: AtomicUnits) -> Self {
        units.0.to_string()
    }
}

impl std::str::FromStr for AtomicUnits {
    type Err = ContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let canonical = !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && (value == "0" || !value.starts_with('0'));
        if !canonical {
            return Err(ContractError::NonCanonicalAtomicUnits);
        }
        value
            .parse::<u128>()
            .map(Self)
            .map_err(|_| ContractError::AtomicUnitOverflow)
    }
}

impl fmt::Display for AtomicUnits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AtomicUnits {
    pub const ZERO: Self = Self(0);

    pub const fn from_raw(value: u128) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u128 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, ContractError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(ContractError::AtomicUnitOverflow)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Microcredits(u64);

impl Microcredits {
    pub const ZERO: Self = Self(0);

    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// Parse a decimal credit amount without passing through floating point.
    pub fn from_credit_decimal(value: &str) -> Result<Self, MicrocreditConversionError> {
        if value.is_empty() || value.starts_with('-') || value.starts_with('+') {
            return Err(MicrocreditConversionError::Invalid);
        }
        let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fractional.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(MicrocreditConversionError::Invalid);
        }
        if fractional.len() > 6 {
            return Err(MicrocreditConversionError::ExcessPrecision);
        }
        let whole = whole
            .parse::<u64>()
            .map_err(|_| MicrocreditConversionError::Overflow)?;
        let whole = whole
            .checked_mul(MICROCREDITS_PER_CREDIT)
            .ok_or(MicrocreditConversionError::Overflow)?;
        let mut fractional_micros = if fractional.is_empty() {
            0
        } else {
            fractional
                .parse::<u64>()
                .map_err(|_| MicrocreditConversionError::Overflow)?
        };
        for _ in fractional.len()..6 {
            fractional_micros = fractional_micros
                .checked_mul(10)
                .ok_or(MicrocreditConversionError::Overflow)?;
        }
        whole
            .checked_add(fractional_micros)
            .map(Self)
            .ok_or(MicrocreditConversionError::Overflow)
    }

    pub fn to_credit_decimal(self) -> String {
        let whole = self.0 / MICROCREDITS_PER_CREDIT;
        let fractional = self.0 % MICROCREDITS_PER_CREDIT;
        if fractional == 0 {
            return whole.to_string();
        }
        format!("{whole}.{fractional:06}")
            .trim_end_matches('0')
            .to_string()
    }

    /// Convert Trace Credit microcredits to the generic settlement unit.
    ///
    /// This is the Trace Credit adapter boundary. Trace Credit pins
    /// `TRACE_CREDIT_DECIMALS`, so one atomic unit is one microcredit and the
    /// conversion is exact.
    pub const fn into_atomic_units(self) -> AtomicUnits {
        AtomicUnits::from_raw(self.0 as u128)
    }

    /// Private: units carry no instrument. Convert through
    /// `InstrumentAward::trace_credit_microcredits`, which checks it.
    fn from_atomic_units(value: AtomicUnits) -> Result<Self, ContractError> {
        u64::try_from(value.get())
            .map(Self)
            .map_err(|_| ContractError::TraceCreditOutOfRange)
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum MicrocreditConversionError {
    #[error("credit amount is not an unsigned decimal")]
    Invalid,
    #[error("credit amount has more than six decimal places")]
    ExcessPrecision,
    #[error("credit amount exceeds the microcredit range")]
    Overflow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "InstrumentAwardFields")]
pub struct InstrumentAward {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
}

/// Loaded `InstrumentAward` fields, checked by `InstrumentAward::new`.
#[derive(Deserialize)]
struct InstrumentAwardFields {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
}

impl TryFrom<InstrumentAwardFields> for InstrumentAward {
    type Error = ContractError;

    fn try_from(fields: InstrumentAwardFields) -> Result<Self, Self::Error> {
        Self::new(fields.instrument_id, fields.atomic_units)
    }
}

fn require_trace_credit_range(
    instrument_id: &InstrumentId,
    atomic_units: AtomicUnits,
) -> Result<(), ContractError> {
    if instrument_id.as_str() == TRACE_CREDIT_INSTRUMENT_ID
        && atomic_units.get() > u128::from(MAX_TRACE_CREDIT_MICROCREDITS)
    {
        return Err(ContractError::TraceCreditOutOfRange);
    }
    Ok(())
}

impl InstrumentAward {
    pub fn new(
        instrument_id: InstrumentId,
        atomic_units: AtomicUnits,
    ) -> Result<Self, ContractError> {
        if atomic_units == AtomicUnits::ZERO {
            return Err(ContractError::ZeroInstrumentAward);
        }
        require_trace_credit_range(&instrument_id, atomic_units)?;
        Ok(Self {
            instrument_id,
            atomic_units,
        })
    }

    pub fn trace_credit(microcredits: Microcredits) -> Result<Self, ContractError> {
        Self::new(
            InstrumentId::trace_credit(),
            microcredits.into_atomic_units(),
        )
    }

    pub fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    pub const fn atomic_units(&self) -> AtomicUnits {
        self.atomic_units
    }

    pub fn trace_credit_microcredits(&self) -> Result<Microcredits, ContractError> {
        if self.instrument_id.as_str() != TRACE_CREDIT_INSTRUMENT_ID {
            return Err(ContractError::NotTraceCredit);
        }
        Microcredits::from_atomic_units(self.atomic_units)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "Vec<InstrumentAward>", into = "Vec<InstrumentAward>")]
pub struct InstrumentAwards(Vec<InstrumentAward>);

impl TryFrom<Vec<InstrumentAward>> for InstrumentAwards {
    type Error = ContractError;

    fn try_from(awards: Vec<InstrumentAward>) -> Result<Self, Self::Error> {
        Self::new(awards)
    }
}

impl From<InstrumentAwards> for Vec<InstrumentAward> {
    fn from(awards: InstrumentAwards) -> Self {
        awards.0
    }
}

impl InstrumentAwards {
    pub fn new(mut awards: Vec<InstrumentAward>) -> Result<Self, ContractError> {
        awards.sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));
        if awards
            .windows(2)
            .any(|pair| pair[0].instrument_id == pair[1].instrument_id)
        {
            return Err(ContractError::DuplicateInstrumentId);
        }
        Ok(Self(awards))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &InstrumentAward> {
        self.0.iter()
    }

    pub fn get(&self, instrument_id: &InstrumentId) -> Option<&InstrumentAward> {
        self.0
            .binary_search_by(|award| award.instrument_id.cmp(instrument_id))
            .ok()
            .map(|index| &self.0[index])
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Canonical identity for the complete ordered award set. Each amount is
    /// its full 16-byte big-endian `u128`.
    pub fn canonical_id(&self) -> String {
        let mut bytes = b"trace-commons-instrument-awards\0".to_vec();
        encode_len(&mut bytes, self.0.len());
        for award in &self.0 {
            encode_string(&mut bytes, award.instrument_id.as_str());
            bytes.extend_from_slice(&award.atomic_units.0.to_be_bytes());
        }
        sha256_prefixed(&bytes)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaRef {
    pub id: String,
    pub version: u32,
}

impl SchemaRef {
    pub fn pipeline_v1() -> Self {
        Self {
            id: PIPELINE_OUTCOME_SCHEMA_ID.to_string(),
            version: PIPELINE_OUTCOME_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRef {
    pub policy_id: String,
    pub implementation_id: String,
    pub configuration_hash: String,
    pub data_artifact_hashes: Vec<String>,
    pub projection_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    /// A NEAR fungible token.
    Nep141,
    /// An EVM fungible token.
    Erc20,
    /// An off-chain credit account. It is not a token.
    CreditAccount,
}

impl InstrumentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nep141 => "nep141",
            Self::Erc20 => "erc20",
            Self::CreditAccount => "credit_account",
        }
    }
}

/// The token or account that an instrument settles in, and the scale of its
/// atomic units. A bundle manifest pins one descriptor for each instrument, so
/// a signed award always says which token it pays and at what scale.
///
/// A descriptor never changes for an instrument. A different contract,
/// network, kind, or `decimals` is a new `InstrumentId`, never a new reading
/// of awards already signed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstrumentDescriptor {
    pub kind: InstrumentKind,
    /// `nep141`: the NEAR network, such as `mainnet`. `erc20`: the EIP-155
    /// chain id in decimal, such as `1`. `credit_account`: the ledger label.
    pub network: String,
    /// `nep141`: the token's NEAR account id. `erc20`: the lowercase
    /// `0x`-prefixed contract address. `credit_account`: the account label.
    pub contract: String,
    /// One whole token is `10^decimals` atomic units.
    pub decimals: u8,
}

impl InstrumentDescriptor {
    /// Checks the network and contract spelling for the kind, and the
    /// `decimals` bound. Each accepted form has one spelling, so equal
    /// descriptors give equal bundle identifiers.
    pub fn validate(&self) -> Result<(), ContractError> {
        let located = match self.kind {
            InstrumentKind::Nep141 => {
                is_safe_identifier(&self.network) && is_near_account_id(&self.contract)
            }
            InstrumentKind::Erc20 => {
                is_evm_chain_id(&self.network) && is_evm_address(&self.contract)
            }
            InstrumentKind::CreditAccount => {
                is_safe_identifier(&self.network) && is_safe_identifier(&self.contract)
            }
        };
        if !located || self.decimals > MAX_INSTRUMENT_DECIMALS {
            return Err(ContractError::InvalidInstrumentDescriptor);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleManifest {
    pub format_version: u32,
    pub admission: PolicyRef,
    pub review: PolicyRef,
    pub score: PolicyRef,
    pub settle: PolicyRef,
    /// The instruments that this bundle can award, each pinned to one
    /// descriptor. An award for an instrument that is not here is refused.
    #[serde(deserialize_with = "unique_instruments")]
    pub instruments: BTreeMap<InstrumentId, InstrumentDescriptor>,
}

/// Loads the pinned instruments and refuses a repeated instrument. A plain
/// map keeps the last copy, and a reader that keeps the first copy would pin
/// a different descriptor.
fn unique_instruments<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<InstrumentId, InstrumentDescriptor>, D::Error> {
    struct UniqueInstruments;

    impl<'de> serde::de::Visitor<'de> for UniqueInstruments {
        type Value = BTreeMap<InstrumentId, InstrumentDescriptor>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map from instrument identifier to descriptor")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            use serde::de::Error as _;

            let mut instruments = BTreeMap::new();
            while let Some((instrument_id, descriptor)) = map.next_entry()? {
                if instruments.insert(instrument_id, descriptor).is_some() {
                    return Err(A::Error::custom(ContractError::DuplicateInstrumentId));
                }
            }
            Ok(instruments)
        }
    }

    deserializer.deserialize_map(UniqueInstruments)
}

impl BundleManifest {
    /// Stable length-prefixed encoding. Lists are sorted before encoding.
    /// A duplicate entry is an error. It is not dropped. The pinned
    /// instruments follow the four policies, in instrument order.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContractError> {
        if self.format_version != BUNDLE_MANIFEST_FORMAT_VERSION {
            return Err(ContractError::UnsupportedManifestVersion);
        }
        let mut bytes = b"trace-commons-bundle-manifest\0".to_vec();
        bytes.extend_from_slice(&self.format_version.to_be_bytes());
        for policy in [&self.admission, &self.review, &self.score, &self.settle] {
            encode_policy(&mut bytes, policy)?;
        }
        encode_len(&mut bytes, self.instruments.len());
        for (instrument_id, descriptor) in &self.instruments {
            encode_instrument(&mut bytes, instrument_id, descriptor)?;
        }
        Ok(bytes)
    }

    pub fn bundle_id(&self) -> Result<String, ContractError> {
        Ok(sha256_prefixed(&self.canonical_bytes()?))
    }

    /// The descriptor that this bundle pins for an instrument.
    pub fn instrument(&self, instrument_id: &InstrumentId) -> Option<&InstrumentDescriptor> {
        self.instruments.get(instrument_id)
    }

    /// Refuses an award set that names an instrument this bundle does not
    /// pin. A runner applies this to Score's awards before it commits the
    /// Score outcome, so no settlement starts for an unpinned instrument.
    pub fn require_pinned(&self, awards: &InstrumentAwards) -> Result<(), ContractError> {
        if awards
            .iter()
            .all(|award| self.instruments.contains_key(award.instrument_id()))
        {
            Ok(())
        } else {
            Err(ContractError::UnpinnedInstrument)
        }
    }

    fn referenced_artifacts(&self) -> Result<BTreeSet<String>, ContractError> {
        let mut hashes = BTreeSet::new();
        for policy in [&self.admission, &self.review, &self.score, &self.settle] {
            for hash in std::iter::once(&policy.configuration_hash)
                .chain(policy.data_artifact_hashes.iter())
            {
                if !is_sha256(hash) {
                    return Err(ContractError::InvalidArtifactHash);
                }
                hashes.insert(hash.clone());
            }
        }
        Ok(hashes)
    }
}

/// Every length and count in a canonical encoding is a big-endian `u64`.
fn encode_len(output: &mut Vec<u8>, len: usize) {
    output.extend_from_slice(&(len as u64).to_be_bytes());
}

fn encode_string(output: &mut Vec<u8>, value: &str) {
    encode_len(output, value.len());
    output.extend_from_slice(value.as_bytes());
}

fn encode_list(output: &mut Vec<u8>, values: &[String]) -> Result<(), ContractError> {
    let mut values = values.to_vec();
    values.sort();
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ContractError::DuplicatePolicyListEntry);
    }
    encode_len(output, values.len());
    for value in &values {
        encode_string(output, value);
    }
    Ok(())
}

fn encode_policy(output: &mut Vec<u8>, policy: &PolicyRef) -> Result<(), ContractError> {
    if policy.policy_id.is_empty() || policy.implementation_id.is_empty() {
        return Err(ContractError::MissingPolicyIdentity);
    }
    encode_string(output, &policy.policy_id);
    encode_string(output, &policy.implementation_id);
    encode_string(output, &policy.configuration_hash);
    encode_list(output, &policy.data_artifact_hashes)?;
    encode_list(output, &policy.projection_ids)?;
    Ok(())
}

fn encode_instrument(
    output: &mut Vec<u8>,
    instrument_id: &InstrumentId,
    descriptor: &InstrumentDescriptor,
) -> Result<(), ContractError> {
    descriptor.validate()?;
    // `Microcredits` reads a Trace Credit atomic unit as one microcredit.
    if instrument_id.as_str() == TRACE_CREDIT_INSTRUMENT_ID
        && descriptor.decimals != TRACE_CREDIT_DECIMALS
    {
        return Err(ContractError::TraceCreditDecimals);
    }
    encode_string(output, instrument_id.as_str());
    encode_string(output, descriptor.kind.as_str());
    encode_string(output, &descriptor.network);
    encode_string(output, &descriptor.contract);
    output.push(descriptor.decimals);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundlePackage {
    pub bundle_id: String,
    pub manifest: BundleManifest,
    #[serde(with = "hex_artifacts")]
    pub artifacts: BTreeMap<String, Vec<u8>>,
}

/// Serializes artifact bytes as lowercase hex strings. The derived encoding,
/// one JSON integer per byte, made a large artifact fail JSONB storage.
mod hex_artifacts {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        artifacts: &BTreeMap<String, Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(artifacts.iter().map(|(hash, bytes)| (hash, encode(bytes))))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<String, Vec<u8>>, D::Error> {
        BTreeMap::<String, String>::deserialize(deserializer)?
            .into_iter()
            .map(|(hash, hex)| {
                decode(&hex)
                    .map(|bytes| (hash, bytes))
                    .ok_or_else(|| D::Error::custom("artifact bytes are not lowercase hex"))
            })
            .collect()
    }

    pub(super) fn encode(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
            hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        hex
    }

    pub(super) fn decode(hex: &str) -> Option<Vec<u8>> {
        let (pairs, remainder) = hex.as_bytes().as_chunks::<2>();
        if !remainder.is_empty() {
            return None;
        }
        pairs
            .iter()
            .map(|&[high, low]| Some((nibble(high)? << 4) | nibble(low)?))
            .collect()
    }

    fn nibble(digit: u8) -> Option<u8> {
        match digit {
            b'0'..=b'9' => Some(digit - b'0'),
            b'a'..=b'f' => Some(digit - b'a' + 10),
            _ => None,
        }
    }
}

impl BundlePackage {
    /// Canonical package bytes used by an external package signature.
    ///
    /// The manifest already binds every policy input by content hash. The
    /// package encoding adds a domain separator, the bundle identifier, the
    /// canonical manifest, and the sorted artifact hashes. Artifact bytes are
    /// validated before encoding, so signing this value binds the complete
    /// package without depending on a JSON serializer.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContractError> {
        self.validate()?;
        let mut bytes = b"trace-commons-bundle-package\0".to_vec();
        encode_string(&mut bytes, &self.bundle_id);
        let manifest = self.manifest.canonical_bytes()?;
        encode_len(&mut bytes, manifest.len());
        bytes.extend_from_slice(&manifest);
        encode_len(&mut bytes, self.artifacts.len());
        for hash in self.artifacts.keys() {
            encode_string(&mut bytes, hash);
        }
        Ok(bytes)
    }

    pub fn package_hash(&self) -> Result<String, ContractError> {
        Ok(sha256_prefixed(&self.canonical_bytes()?))
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.bundle_id != self.manifest.bundle_id()? {
            return Err(ContractError::BundleIdMismatch);
        }
        let expected = self.manifest.referenced_artifacts()?;
        let actual = self.artifacts.keys().cloned().collect::<BTreeSet<_>>();
        if expected != actual {
            return Err(ContractError::ArtifactSetMismatch);
        }
        for (expected_hash, artifact) in &self.artifacts {
            if &sha256_prefixed(artifact) != expected_hash {
                return Err(ContractError::ArtifactHashMismatch);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("unsupported bundle manifest version")]
    UnsupportedManifestVersion,
    #[error("policy identity is missing")]
    MissingPolicyIdentity,
    #[error("artifact hash is malformed")]
    InvalidArtifactHash,
    #[error("bundle identifier does not match its manifest")]
    BundleIdMismatch,
    #[error("bundle artifact set does not match its manifest")]
    ArtifactSetMismatch,
    #[error("bundle artifact bytes do not match their hash")]
    ArtifactHashMismatch,
    #[error("reason code is not a safe label")]
    InvalidReasonCode,
    #[error("instrument identifier is not a bounded safe label")]
    InvalidInstrumentId,
    #[error("an instrument award must contain positive atomic units")]
    ZeroInstrumentAward,
    #[error("an instrument appears more than once")]
    DuplicateInstrumentId,
    #[error("a policy list contains a duplicate entry")]
    DuplicatePolicyListEntry,
    #[error("atomic units exceed the supported integer range")]
    AtomicUnitOverflow,
    #[error("atomic units are not a canonical unsigned decimal string")]
    NonCanonicalAtomicUnits,
    #[error("instrument descriptor does not match the form its kind requires")]
    InvalidInstrumentDescriptor,
    #[error("the trace_credit instrument must pin six decimals")]
    TraceCreditDecimals,
    #[error("an award names an instrument that the bundle does not pin")]
    UnpinnedInstrument,
    #[error("the award does not use the Trace Credit instrument")]
    NotTraceCredit,
    #[error("a Trace Credit award exceeds the credit ledger's signed 64-bit range")]
    TraceCreditOutOfRange,
    #[error("settlement operation references must be SHA-256 hashes")]
    InvalidSettlementReference,
    #[error("settlement operations do not exactly match the score awards")]
    SettlementOperationMismatch,
    #[error("provenance label is not a bounded safe identifier")]
    InvalidProvenanceLabel,
    #[error("review output does not match its decision and evidence")]
    ReviewOutputMismatch,
    #[error("index command is malformed")]
    InvalidIndexCommand,
    #[error("score output does not match its evidence")]
    ScoreOutputMismatch,
    #[error("a hash field is not a lowercase SHA-256 reference")]
    MalformedHash,
    #[error("tenant storage reference is not in its derived form")]
    InvalidTenantStorageRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdmissionDecision {
    Admit,
    Quarantine { reason: ReasonCode },
    Reject { reason: ReasonCode },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved { registry_revision_id: Uuid },
    Rejected { reason: ReasonCode },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreDecision {
    pub awards: InstrumentAwards,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IndexMembershipDecision {
    Exclude {
        reason: ReasonCode,
    },
    Include {
        command_hash: String,
        entry_count: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "SettleDecisionFields")]
pub struct SettleDecision {
    pub index_membership: IndexMembershipDecision,
    settlement_operations: Vec<InstrumentSettlement>,
}

/// Loaded `SettleDecision` fields. Loading repeats every check that does not
/// need the Score awards; the runner matches those against the committed
/// Score outcome.
#[derive(Deserialize)]
struct SettleDecisionFields {
    index_membership: IndexMembershipDecision,
    settlement_operations: Vec<InstrumentSettlement>,
}

impl TryFrom<SettleDecisionFields> for SettleDecision {
    type Error = ContractError;

    fn try_from(fields: SettleDecisionFields) -> Result<Self, Self::Error> {
        Self::from_parts(fields.index_membership, fields.settlement_operations)
    }
}

impl SettleDecision {
    /// Builds a decision for the committed Score outcome. The operations must
    /// match its awards exactly.
    pub fn new(
        index_membership: IndexMembershipDecision,
        score: &ScoreDecision,
        settlement_operations: Vec<InstrumentSettlement>,
    ) -> Result<Self, ContractError> {
        let decision = Self::from_parts(index_membership, settlement_operations)?;
        decision.matches_score(score)?;
        Ok(decision)
    }

    /// One operation per award, with the same instrument and amount. A loaded
    /// decision is checked with this against the committed Score outcome.
    pub fn matches_score(&self, score: &ScoreDecision) -> Result<(), ContractError> {
        let awards = &score.awards;
        let operations_match =
            awards
                .iter()
                .zip(&self.settlement_operations)
                .all(|(award, operation)| {
                    award.instrument_id == operation.instrument_id
                        && award.atomic_units == operation.atomic_units
                });
        if awards.iter().len() != self.settlement_operations.len() || !operations_match {
            return Err(ContractError::SettlementOperationMismatch);
        }
        Ok(())
    }

    /// Sorts operations by instrument and refuses a second operation for one
    /// instrument. This scan is the guard against settling an instrument
    /// twice.
    fn from_parts(
        index_membership: IndexMembershipDecision,
        mut settlement_operations: Vec<InstrumentSettlement>,
    ) -> Result<Self, ContractError> {
        if let IndexMembershipDecision::Include { command_hash, .. } = &index_membership {
            require_sha256([Some(command_hash.as_str())])?;
        }
        settlement_operations.sort_by(|left, right| {
            left.instrument_id
                .cmp(&right.instrument_id)
                .then_with(|| left.operation_ref_hash.cmp(&right.operation_ref_hash))
        });
        if settlement_operations
            .windows(2)
            .any(|pair| pair[0].instrument_id == pair[1].instrument_id)
        {
            return Err(ContractError::DuplicateInstrumentId);
        }
        Ok(Self {
            index_membership,
            settlement_operations,
        })
    }

    pub fn settlement_operations(&self) -> &[InstrumentSettlement] {
        &self.settlement_operations
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "InstrumentSettlementFields")]
pub struct InstrumentSettlement {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
    operation_ref_hash: String,
    outcome: InstrumentSettlementOutcome,
}

/// Loaded `InstrumentSettlement` fields, checked by its constructor.
#[derive(Deserialize)]
struct InstrumentSettlementFields {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
    operation_ref_hash: String,
    outcome: InstrumentSettlementOutcome,
}

impl TryFrom<InstrumentSettlementFields> for InstrumentSettlement {
    type Error = ContractError;

    fn try_from(fields: InstrumentSettlementFields) -> Result<Self, Self::Error> {
        Self::with_outcome(
            fields.instrument_id,
            fields.atomic_units,
            fields.operation_ref_hash,
            fields.outcome,
        )
    }
}

/// How an instrument operation ended. Both states are terminal, so Settle can
/// complete with either.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InstrumentSettlementOutcome {
    Completed {
        result_ref_hash: String,
    },
    /// Withdrawal committed before the operation completed. Credit that is
    /// not settled is forfeited, as on the legacy path; settled credit stays.
    Forfeited {
        reason: ReasonCode,
    },
}

impl InstrumentSettlement {
    pub fn new(
        instrument_id: InstrumentId,
        atomic_units: AtomicUnits,
        operation_ref_hash: impl Into<String>,
        result_ref_hash: impl Into<String>,
    ) -> Result<Self, ContractError> {
        Self::with_outcome(
            instrument_id,
            atomic_units,
            operation_ref_hash.into(),
            InstrumentSettlementOutcome::Completed {
                result_ref_hash: result_ref_hash.into(),
            },
        )
    }

    pub fn forfeited(
        instrument_id: InstrumentId,
        atomic_units: AtomicUnits,
        operation_ref_hash: impl Into<String>,
        reason: ReasonCode,
    ) -> Result<Self, ContractError> {
        Self::with_outcome(
            instrument_id,
            atomic_units,
            operation_ref_hash.into(),
            InstrumentSettlementOutcome::Forfeited { reason },
        )
    }

    fn with_outcome(
        instrument_id: InstrumentId,
        atomic_units: AtomicUnits,
        operation_ref_hash: String,
        outcome: InstrumentSettlementOutcome,
    ) -> Result<Self, ContractError> {
        if atomic_units == AtomicUnits::ZERO {
            return Err(ContractError::ZeroInstrumentAward);
        }
        require_trace_credit_range(&instrument_id, atomic_units)?;
        let result_ref_hash = match &outcome {
            InstrumentSettlementOutcome::Completed { result_ref_hash } => Some(result_ref_hash),
            InstrumentSettlementOutcome::Forfeited { .. } => None,
        };
        if !is_sha256(&operation_ref_hash) || result_ref_hash.is_some_and(|hash| !is_sha256(hash)) {
            return Err(ContractError::InvalidSettlementReference);
        }
        Ok(Self {
            instrument_id,
            atomic_units,
            operation_ref_hash,
            outcome,
        })
    }

    pub fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    pub const fn atomic_units(&self) -> AtomicUnits {
        self.atomic_units
    }

    pub fn operation_ref_hash(&self) -> &str {
        &self.operation_ref_hash
    }

    pub fn outcome(&self) -> &InstrumentSettlementOutcome {
        &self.outcome
    }

    /// The adapter result reference. `None` when the operation was forfeited.
    pub fn result_ref_hash(&self) -> Option<&str> {
        match &self.outcome {
            InstrumentSettlementOutcome::Completed { result_ref_hash } => Some(result_ref_hash),
            InstrumentSettlementOutcome::Forfeited { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "AdmissionEvidenceFields")]
pub struct AdmissionEvidence {
    pub request_content_hash: String,
    // No validity flag has a serde default: a stored record that lacks one
    // fails to load instead of reading as `false`.
    pub schema_valid: bool,
    pub authority_valid: bool,
    pub contribution_path_valid: bool,
    pub grant_valid: bool,
    pub consent_valid: bool,
    pub allowed_uses_valid: bool,
    pub quota_counted: bool,
    #[serde(default)]
    pub detector_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_risk: Option<PrivacyRisk>,
}

/// Loaded `AdmissionEvidence`. `validate` runs again, so a stored hash cannot
/// skip the check the output path uses.
#[derive(Deserialize)]
struct AdmissionEvidenceFields {
    request_content_hash: String,
    schema_valid: bool,
    authority_valid: bool,
    contribution_path_valid: bool,
    grant_valid: bool,
    consent_valid: bool,
    allowed_uses_valid: bool,
    quota_counted: bool,
    #[serde(default)]
    detector_ids: Vec<String>,
    #[serde(default)]
    privacy_risk: Option<PrivacyRisk>,
}

impl TryFrom<AdmissionEvidenceFields> for AdmissionEvidence {
    type Error = ContractError;

    fn try_from(fields: AdmissionEvidenceFields) -> Result<Self, Self::Error> {
        let evidence = Self {
            request_content_hash: fields.request_content_hash,
            schema_valid: fields.schema_valid,
            authority_valid: fields.authority_valid,
            contribution_path_valid: fields.contribution_path_valid,
            grant_valid: fields.grant_valid,
            consent_valid: fields.consent_valid,
            allowed_uses_valid: fields.allowed_uses_valid,
            quota_counted: fields.quota_counted,
            detector_ids: fields.detector_ids,
            privacy_risk: fields.privacy_risk,
        };
        evidence.validate()?;
        Ok(evidence)
    }
}

/// Residual privacy risk that Admission reads. The wire values match the
/// protocol's `ResidualPiiRisk`; an unknown value fails to load.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyRisk {
    Low,
    Medium,
    High,
}

impl AdmissionEvidence {
    pub fn validate(&self) -> Result<(), ContractError> {
        require_sha256([Some(self.request_content_hash.as_str())])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmissionEvaluation {
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "ReviewEvidenceFields")]
pub struct ReviewEvidence {
    pub source_content_hash: String,
    /// Hash of the approved bytes. Equals `source_content_hash` for a
    /// pass-through or a rejection.
    pub result_content_hash: String,
    pub content_changed: bool,
    /// Worker that produced the approved content. Set only on approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transformation_metadata_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_assessment_hash: Option<String>,
    #[serde(default)]
    pub resolved_quarantine_reasons: Vec<ReasonCode>,
}

#[derive(Deserialize)]
struct ReviewEvidenceFields {
    source_content_hash: String,
    result_content_hash: String,
    content_changed: bool,
    #[serde(default)]
    worker_identity: Option<String>,
    #[serde(default)]
    transformation_metadata_hash: Option<String>,
    #[serde(default)]
    human_assessment_hash: Option<String>,
    #[serde(default)]
    resolved_quarantine_reasons: Vec<ReasonCode>,
}

impl TryFrom<ReviewEvidenceFields> for ReviewEvidence {
    type Error = ContractError;

    fn try_from(fields: ReviewEvidenceFields) -> Result<Self, Self::Error> {
        let evidence = Self {
            source_content_hash: fields.source_content_hash,
            result_content_hash: fields.result_content_hash,
            content_changed: fields.content_changed,
            worker_identity: fields.worker_identity,
            transformation_metadata_hash: fields.transformation_metadata_hash,
            human_assessment_hash: fields.human_assessment_hash,
            resolved_quarantine_reasons: fields.resolved_quarantine_reasons,
        };
        evidence.validate()?;
        Ok(evidence)
    }
}

impl ReviewEvidence {
    pub fn validate(&self) -> Result<(), ContractError> {
        require_sha256([
            Some(self.source_content_hash.as_str()),
            Some(self.result_content_hash.as_str()),
            self.transformation_metadata_hash.as_deref(),
            self.human_assessment_hash.as_deref(),
        ])?;
        if self
            .worker_identity
            .as_deref()
            .is_some_and(|identity| !is_safe_identifier(identity))
        {
            return Err(ContractError::InvalidProvenanceLabel);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEvaluation {
    pub rule_id: String,
}

/// Approved content that a Review policy hands to the runner.
///
/// The bytes are transient. The runner encrypts and stores them, then commits
/// only their reference with the Review outcome. This type is never
/// serialized, and its `Debug` output withholds the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ApprovedContent {
    bytes: Vec<u8>,
    content_hash: String,
    worker_identity: String,
    transformation_metadata_hash: Option<String>,
}

impl ApprovedContent {
    pub fn new(
        bytes: Vec<u8>,
        worker_identity: impl Into<String>,
        transformation_metadata_hash: Option<String>,
    ) -> Result<Self, ContractError> {
        let worker_identity = worker_identity.into();
        if !is_safe_identifier(&worker_identity) {
            return Err(ContractError::InvalidProvenanceLabel);
        }
        if transformation_metadata_hash
            .as_deref()
            .is_some_and(|hash| !is_sha256(hash))
        {
            return Err(ContractError::InvalidArtifactHash);
        }
        Ok(Self {
            content_hash: sha256_prefixed(&bytes),
            bytes,
            worker_identity,
            transformation_metadata_hash,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn worker_identity(&self) -> &str {
        &self.worker_identity
    }

    pub fn transformation_metadata_hash(&self) -> Option<&str> {
        self.transformation_metadata_hash.as_deref()
    }
}

impl fmt::Debug for ApprovedContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApprovedContent")
            .field("content_hash", &self.content_hash)
            .field("byte_len", &self.bytes.len())
            .field("worker_identity", &self.worker_identity)
            .field(
                "transformation_metadata_hash",
                &self.transformation_metadata_hash,
            )
            .finish()
    }
}

/// What a Review policy returns: the persisted result and, on approval, the
/// approved content. The constructors tie the content to the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewOutput {
    result: PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation>,
    approved_content: Option<ApprovedContent>,
}

impl ReviewOutput {
    pub fn approved(
        result: PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation>,
        content: ApprovedContent,
    ) -> Result<Self, ContractError> {
        let evidence = &result.evidence;
        evidence.validate()?;
        let consistent = matches!(result.decision, ReviewDecision::Approved { .. })
            && evidence.result_content_hash == content.content_hash
            && evidence.content_changed
                != (evidence.result_content_hash == evidence.source_content_hash)
            && evidence.worker_identity.as_deref() == Some(content.worker_identity())
            && evidence.transformation_metadata_hash.as_deref()
                == content.transformation_metadata_hash();
        if !consistent {
            return Err(ContractError::ReviewOutputMismatch);
        }
        Ok(Self {
            result,
            approved_content: Some(content),
        })
    }

    pub fn rejected(
        result: PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation>,
    ) -> Result<Self, ContractError> {
        let evidence = &result.evidence;
        evidence.validate()?;
        if !matches!(result.decision, ReviewDecision::Rejected { .. })
            || evidence.result_content_hash != evidence.source_content_hash
            || evidence.content_changed
            || evidence.worker_identity.is_some()
            || evidence.transformation_metadata_hash.is_some()
        {
            return Err(ContractError::ReviewOutputMismatch);
        }
        Ok(Self {
            result,
            approved_content: None,
        })
    }

    pub fn result(&self) -> &PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation> {
        &self.result
    }

    pub fn approved_content(&self) -> Option<&ApprovedContent> {
        self.approved_content.as_ref()
    }

    pub fn into_parts(
        self,
    ) -> (
        PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation>,
        Option<ApprovedContent>,
    ) {
        (self.result, self.approved_content)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "ScoreEvidenceFields")]
pub struct ScoreEvidence {
    pub fixed_awards: InstrumentAwards,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_artifact_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scorer_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedder_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_id: Option<String>,
    /// Hash of the input the projection embedded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_input_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perplexity_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_fraction_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub novelty_score_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_perplexity_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_novelty_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub novelty_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nearest_neighbor_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_cardinality: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_tokens: Option<u64>,
    /// Chunks scored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u32>,
    /// Chunks in the whole trace before the per-trace cap. With
    /// `chunk_count`, it states the coverage of a capped trace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_chunk_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_capped: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_eligible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_quality_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_quality_version: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighbor_artifact_hash: Option<String>,
}

#[derive(Deserialize)]
struct ScoreEvidenceFields {
    fixed_awards: InstrumentAwards,
    #[serde(default)]
    embedding_artifact_hash: Option<String>,
    #[serde(default)]
    index_id: Option<String>,
    #[serde(default)]
    index_snapshot_id: Option<String>,
    #[serde(default)]
    index_snapshot_hash: Option<String>,
    #[serde(default)]
    scorer_model_id: Option<String>,
    #[serde(default)]
    embedder_model_id: Option<String>,
    #[serde(default)]
    projection_id: Option<String>,
    #[serde(default)]
    projection_input_hash: Option<String>,
    #[serde(default)]
    perplexity_micros: Option<u64>,
    #[serde(default)]
    tail_fraction_micros: Option<u64>,
    #[serde(default)]
    novelty_score_micros: Option<u64>,
    #[serde(default)]
    peak_perplexity_micros: Option<u64>,
    #[serde(default)]
    peak_novelty_micros: Option<u64>,
    #[serde(default)]
    quality_passed: Option<bool>,
    #[serde(default)]
    novelty_passed: Option<bool>,
    #[serde(default)]
    nearest_neighbor_hash: Option<String>,
    #[serde(default)]
    index_cardinality: Option<u64>,
    #[serde(default)]
    coverage_tokens: Option<u64>,
    #[serde(default)]
    chunk_count: Option<u32>,
    #[serde(default)]
    total_chunk_count: Option<u32>,
    #[serde(default)]
    chunks_capped: Option<bool>,
    #[serde(default)]
    include_eligible: Option<bool>,
    #[serde(default)]
    credit_quality_micros: Option<u64>,
    #[serde(default)]
    credit_quality_version: Option<i32>,
    #[serde(default)]
    neighbor_artifact_hash: Option<String>,
}

impl TryFrom<ScoreEvidenceFields> for ScoreEvidence {
    type Error = ContractError;

    fn try_from(fields: ScoreEvidenceFields) -> Result<Self, Self::Error> {
        let evidence = Self {
            fixed_awards: fields.fixed_awards,
            embedding_artifact_hash: fields.embedding_artifact_hash,
            index_id: fields.index_id,
            index_snapshot_id: fields.index_snapshot_id,
            index_snapshot_hash: fields.index_snapshot_hash,
            scorer_model_id: fields.scorer_model_id,
            embedder_model_id: fields.embedder_model_id,
            projection_id: fields.projection_id,
            projection_input_hash: fields.projection_input_hash,
            perplexity_micros: fields.perplexity_micros,
            tail_fraction_micros: fields.tail_fraction_micros,
            novelty_score_micros: fields.novelty_score_micros,
            peak_perplexity_micros: fields.peak_perplexity_micros,
            peak_novelty_micros: fields.peak_novelty_micros,
            quality_passed: fields.quality_passed,
            novelty_passed: fields.novelty_passed,
            nearest_neighbor_hash: fields.nearest_neighbor_hash,
            index_cardinality: fields.index_cardinality,
            coverage_tokens: fields.coverage_tokens,
            chunk_count: fields.chunk_count,
            total_chunk_count: fields.total_chunk_count,
            chunks_capped: fields.chunks_capped,
            include_eligible: fields.include_eligible,
            credit_quality_micros: fields.credit_quality_micros,
            credit_quality_version: fields.credit_quality_version,
            neighbor_artifact_hash: fields.neighbor_artifact_hash,
        };
        evidence.validate()?;
        Ok(evidence)
    }
}

impl ScoreEvidence {
    pub fn fixed(fixed_awards: InstrumentAwards) -> Self {
        Self {
            fixed_awards,
            embedding_artifact_hash: None,
            index_id: None,
            index_snapshot_id: None,
            index_snapshot_hash: None,
            scorer_model_id: None,
            embedder_model_id: None,
            projection_id: None,
            projection_input_hash: None,
            perplexity_micros: None,
            tail_fraction_micros: None,
            novelty_score_micros: None,
            peak_perplexity_micros: None,
            peak_novelty_micros: None,
            quality_passed: None,
            novelty_passed: None,
            nearest_neighbor_hash: None,
            index_cardinality: None,
            coverage_tokens: None,
            chunk_count: None,
            total_chunk_count: None,
            chunks_capped: None,
            include_eligible: None,
            credit_quality_micros: None,
            credit_quality_version: None,
            neighbor_artifact_hash: None,
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        require_sha256([
            self.embedding_artifact_hash.as_deref(),
            self.index_snapshot_hash.as_deref(),
            self.projection_input_hash.as_deref(),
            self.nearest_neighbor_hash.as_deref(),
            self.neighbor_artifact_hash.as_deref(),
        ])
    }

    /// Coverage fields are all absent, or all present with
    /// `chunk_count <= total_chunk_count` and `chunks_capped` set exactly
    /// when chunks were dropped.
    fn coverage_is_consistent(&self) -> bool {
        match (self.chunk_count, self.total_chunk_count, self.chunks_capped) {
            (None, None, None) => true,
            (Some(scored), Some(total), Some(capped)) => {
                scored <= total && capped == (scored < total)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreEvaluation {
    pub rule_id: String,
    pub awards: InstrumentAwards,
}

/// Index entries that Score proposes for one approved revision.
///
/// Score computes the entries. The server encrypts and stores the command
/// before the Score outcome commits, and Score evidence names it by
/// `content_hash()`. Settle applies the stored command; it does not query the
/// live index or compute new embeddings. `Debug` withholds the embeddings.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "SealedIndexCommandFields")]
pub struct SealedIndexCommand {
    schema: String,
    index_id: String,
    revision_id: Uuid,
    projection_id: String,
    model_id: String,
    entries: Vec<SealedIndexEntry>,
}

/// Loaded `SealedIndexCommand` fields. Loading keeps the stored entry order,
/// so a stored command out of chunk order fails `validate`.
#[derive(Deserialize)]
struct SealedIndexCommandFields {
    schema: String,
    index_id: String,
    revision_id: Uuid,
    projection_id: String,
    model_id: String,
    entries: Vec<SealedIndexEntry>,
}

impl TryFrom<SealedIndexCommandFields> for SealedIndexCommand {
    type Error = ContractError;

    fn try_from(fields: SealedIndexCommandFields) -> Result<Self, Self::Error> {
        let command = Self {
            schema: fields.schema,
            index_id: fields.index_id,
            revision_id: fields.revision_id,
            projection_id: fields.projection_id,
            model_id: fields.model_id,
            entries: fields.entries,
        };
        command.validate()?;
        Ok(command)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct SealedIndexEntry {
    /// The chunk's position in the trace, not its position in `entries`.
    pub chunk: u32,
    pub content_hash: String,
    pub embedding: Vec<f32>,
}

impl SealedIndexCommand {
    pub fn new(
        index_id: impl Into<String>,
        revision_id: Uuid,
        projection_id: impl Into<String>,
        model_id: impl Into<String>,
        mut entries: Vec<SealedIndexEntry>,
    ) -> Result<Self, ContractError> {
        entries.sort_by_key(|entry| entry.chunk);
        let command = Self {
            schema: INDEX_COMMAND_SCHEMA.to_string(),
            index_id: index_id.into(),
            revision_id,
            projection_id: projection_id.into(),
            model_id: model_id.into(),
            entries,
        };
        command.validate()?;
        Ok(command)
    }

    /// Checks identity labels, unique chunks in order, hashes, and vectors of
    /// one finite, non-empty dimension.
    pub fn validate(&self) -> Result<(), ContractError> {
        let dimension = self.entries.first().map(|entry| entry.embedding.len());
        let valid = self.schema == INDEX_COMMAND_SCHEMA
            && [&self.index_id, &self.projection_id, &self.model_id]
                .into_iter()
                .all(|label| is_safe_identifier(label))
            && dimension.is_some_and(|dimension| dimension > 0)
            && self
                .entries
                .windows(2)
                .all(|pair| pair[0].chunk < pair[1].chunk)
            && self.entries.iter().all(|entry| {
                is_sha256(&entry.content_hash)
                    && Some(entry.embedding.len()) == dimension
                    && entry.embedding.iter().all(|value| value.is_finite())
            });
        if valid {
            Ok(())
        } else {
            Err(ContractError::InvalidIndexCommand)
        }
    }

    /// Hash of a stable binary encoding. Vector components are hashed by
    /// their exact bit patterns.
    pub fn content_hash(&self) -> Result<String, ContractError> {
        self.validate()?;
        let mut bytes = b"trace-commons-index-command\0".to_vec();
        for label in [&self.schema, &self.index_id] {
            encode_string(&mut bytes, label);
        }
        bytes.extend_from_slice(self.revision_id.as_bytes());
        for label in [&self.projection_id, &self.model_id] {
            encode_string(&mut bytes, label);
        }
        encode_len(&mut bytes, self.entries.len());
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.chunk.to_be_bytes());
            encode_string(&mut bytes, &entry.content_hash);
            encode_len(&mut bytes, entry.embedding.len());
            for value in &entry.embedding {
                bytes.extend_from_slice(&value.to_bits().to_be_bytes());
            }
        }
        Ok(sha256_prefixed(&bytes))
    }

    pub fn index_id(&self) -> &str {
        &self.index_id
    }

    pub fn revision_id(&self) -> Uuid {
        self.revision_id
    }

    pub fn projection_id(&self) -> &str {
        &self.projection_id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn entries(&self) -> &[SealedIndexEntry] {
        &self.entries
    }

    /// The index key for one of this command's entries.
    pub fn entry_key(
        &self,
        tenant: &TenantStorageRef,
        entry: &SealedIndexEntry,
    ) -> crate::vector_index::IndexEntryKey {
        crate::vector_index::IndexEntryKey {
            tenant_storage_ref: tenant.as_str().to_string(),
            index_id: self.index_id.clone(),
            revision_id: self.revision_id,
            projection_id: self.projection_id.clone(),
            model_id: self.model_id.clone(),
            chunk: entry.chunk,
        }
    }
}

impl fmt::Debug for SealedIndexCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedIndexCommand")
            .field("schema", &self.schema)
            .field("index_id", &self.index_id)
            .field("revision_id", &self.revision_id)
            .field("projection_id", &self.projection_id)
            .field("model_id", &self.model_id)
            .field("entry_count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// What a Score policy returns: the persisted result and the transient
/// artifacts that its evidence names by hash. The server stores both
/// artifacts encrypted before the Score outcome commits.
#[derive(Clone, PartialEq)]
pub struct ScoreOutput {
    result: PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation>,
    index_command: Option<SealedIndexCommand>,
    neighbor_artifact: Option<Vec<u8>>,
}

impl ScoreOutput {
    pub fn new(
        result: PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation>,
        index_command: Option<SealedIndexCommand>,
        neighbor_artifact: Option<Vec<u8>>,
    ) -> Result<Self, ContractError> {
        let evidence = &result.evidence;
        evidence.validate()?;
        // The award set appears three times; all copies must agree.
        if result.decision.awards != evidence.fixed_awards
            || result.decision.awards != result.evaluation.awards
        {
            return Err(ContractError::ScoreOutputMismatch);
        }
        let command_hash = index_command
            .as_ref()
            .map(SealedIndexCommand::content_hash)
            .transpose()?;
        let command_matches = match &index_command {
            None => evidence.include_eligible != Some(true),
            Some(command) => {
                evidence.include_eligible == Some(true)
                    && evidence.index_id.as_deref() == Some(command.index_id())
                    && evidence.projection_id.as_deref() == Some(command.projection_id())
                    && evidence.embedder_model_id.as_deref() == Some(command.model_id())
            }
        };
        if !command_matches
            || !evidence.coverage_is_consistent()
            || evidence.embedding_artifact_hash != command_hash
            || evidence.neighbor_artifact_hash != neighbor_artifact.as_deref().map(sha256_prefixed)
        {
            return Err(ContractError::ScoreOutputMismatch);
        }
        Ok(Self {
            result,
            index_command,
            neighbor_artifact,
        })
    }

    pub fn result(&self) -> &PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation> {
        &self.result
    }

    pub fn index_command(&self) -> Option<&SealedIndexCommand> {
        self.index_command.as_ref()
    }

    pub fn neighbor_artifact(&self) -> Option<&[u8]> {
        self.neighbor_artifact.as_deref()
    }

    pub fn into_parts(
        self,
    ) -> (
        PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation>,
        Option<SealedIndexCommand>,
        Option<Vec<u8>>,
    ) {
        (self.result, self.index_command, self.neighbor_artifact)
    }
}

impl fmt::Debug for ScoreOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScoreOutput")
            .field("result", &self.result)
            .field("index_command", &self.index_command)
            .field(
                "neighbor_artifact_len",
                &self.neighbor_artifact.as_ref().map(Vec::len),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "SettleEvidenceFields")]
pub struct SettleEvidence {
    pub index_operation_required: bool,
    pub settlement_operations_required: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_command_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settlement_progress: Vec<InstrumentSettlementProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_progress: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_operable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_reason: Option<ReasonCode>,
}

#[derive(Deserialize)]
struct SettleEvidenceFields {
    index_operation_required: bool,
    settlement_operations_required: u32,
    #[serde(default)]
    index_command_hash: Option<String>,
    #[serde(default)]
    settlement_progress: Vec<InstrumentSettlementProgress>,
    #[serde(default)]
    index_progress: Option<String>,
    #[serde(default)]
    submission_operable: Option<bool>,
    #[serde(default)]
    guard_reason: Option<ReasonCode>,
}

impl TryFrom<SettleEvidenceFields> for SettleEvidence {
    type Error = ContractError;

    fn try_from(fields: SettleEvidenceFields) -> Result<Self, Self::Error> {
        let evidence = Self {
            index_operation_required: fields.index_operation_required,
            settlement_operations_required: fields.settlement_operations_required,
            index_command_hash: fields.index_command_hash,
            settlement_progress: fields.settlement_progress,
            index_progress: fields.index_progress,
            submission_operable: fields.submission_operable,
            guard_reason: fields.guard_reason,
        };
        evidence.validate()?;
        Ok(evidence)
    }
}

impl SettleEvidence {
    pub fn validate(&self) -> Result<(), ContractError> {
        require_sha256(std::iter::once(self.index_command_hash.as_deref()).chain(
            self.settlement_progress.iter().flat_map(|progress| {
                [
                    Some(progress.operation_ref_hash.as_str()),
                    progress.result_ref_hash.as_deref(),
                ]
            }),
        ))
    }

    pub fn operations(index_operation_required: bool, settlement_operations_required: u32) -> Self {
        Self {
            index_operation_required,
            settlement_operations_required,
            index_command_hash: None,
            settlement_progress: Vec::new(),
            index_progress: None,
            submission_operable: None,
            guard_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstrumentSettlementProgress {
    pub instrument_id: InstrumentId,
    pub operation_ref_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_ref_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleEvaluation {
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhaseResult<D, E, V> {
    pub decision: D,
    pub evidence: E,
    pub evaluation: V,
}

#[derive(Debug, Clone)]
pub struct AdmissionInput {
    pub run_id: Uuid,
    pub tenant_storage_ref: TenantStorageRef,
    pub trace_id: Uuid,
    pub request_content_hash: String,
    pub schema_version: String,
    pub authenticated: bool,
    pub authority_valid: bool,
    pub contribution_path_valid: bool,
    pub grant_valid: bool,
    pub consent_valid: bool,
    pub allowed_uses_valid: bool,
    pub tombstoned: bool,
    pub quota_available: bool,
    pub privacy_risk: PrivacyRisk,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRecommendation {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "HumanReviewAssessmentFields")]
pub struct HumanReviewAssessment {
    pub assessment_id: Uuid,
    pub recommendation: ReviewRecommendation,
    pub reason: ReasonCode,
    pub resolved_quarantine_reasons: Vec<ReasonCode>,
    pub evidence_hash: String,
}

#[derive(Deserialize)]
struct HumanReviewAssessmentFields {
    assessment_id: Uuid,
    recommendation: ReviewRecommendation,
    reason: ReasonCode,
    resolved_quarantine_reasons: Vec<ReasonCode>,
    evidence_hash: String,
}

impl TryFrom<HumanReviewAssessmentFields> for HumanReviewAssessment {
    type Error = ContractError;

    fn try_from(fields: HumanReviewAssessmentFields) -> Result<Self, Self::Error> {
        let assessment = Self {
            assessment_id: fields.assessment_id,
            recommendation: fields.recommendation,
            reason: fields.reason,
            resolved_quarantine_reasons: fields.resolved_quarantine_reasons,
            evidence_hash: fields.evidence_hash,
        };
        assessment.validate()?;
        Ok(assessment)
    }
}

impl HumanReviewAssessment {
    pub fn validate(&self) -> Result<(), ContractError> {
        require_sha256([Some(self.evidence_hash.as_str())])
    }
}

/// `Debug` withholds `source_artifact`, the decrypted trace.
#[derive(Clone)]
pub struct ReviewInput {
    pub run_id: Uuid,
    pub tenant_storage_ref: TenantStorageRef,
    pub trace_id: Uuid,
    pub source_content_hash: String,
    pub source_artifact: Vec<u8>,
    pub admission: AdmissionDecision,
    pub human_assessment: Option<HumanReviewAssessment>,
}

impl fmt::Debug for ReviewInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Hand-written, like `WitnessRequest`: a derived Debug over the
        // decrypted trace is one `?input` away from a whole trace in a log.
        formatter
            .debug_struct("ReviewInput")
            .field("run_id", &self.run_id)
            .field("tenant_storage_ref", &self.tenant_storage_ref)
            .field("trace_id", &self.trace_id)
            .field("source_content_hash", &self.source_content_hash)
            .field("source_artifact", &"<withheld>")
            .field("admission", &self.admission)
            .field("human_assessment", &self.human_assessment)
            .finish()
    }
}

/// `Debug` withholds `reviewed_artifact`, the decrypted approved trace.
#[derive(Clone)]
pub struct ScoreInput {
    pub run_id: Uuid,
    pub tenant_storage_ref: TenantStorageRef,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_content_hash: String,
    pub reviewed_artifact: Vec<u8>,
}

impl fmt::Debug for ScoreInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScoreInput")
            .field("run_id", &self.run_id)
            .field("tenant_storage_ref", &self.tenant_storage_ref)
            .field("trace_id", &self.trace_id)
            .field("registry_revision_id", &self.registry_revision_id)
            .field("source_content_hash", &self.source_content_hash)
            .field("reviewed_artifact", &"<withheld>")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct SettleInput {
    pub run_id: Uuid,
    pub tenant_storage_ref: TenantStorageRef,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_content_hash: String,
    pub score: ScoreDecision,
    pub score_evidence: ScoreEvidence,
    /// The command stored at Score, loaded and checked against
    /// `score_evidence.embedding_artifact_hash` by the server.
    pub index_command: Option<SealedIndexCommand>,
}

/// Why a policy could not produce a result. The runner budgets the two kinds
/// differently: a transient failure is retried without charging the trace's
/// attempt budget; a permanent failure is charged to the trace.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum PolicyError {
    /// A dependency failed: an outage, a timeout, a rate limit, or a payment
    /// or quota error from a backend. The same trace can succeed later.
    #[error("policy dependency failed: {}", .0.as_str())]
    Transient(ReasonCode),
    /// The policy cannot process this trace. Retrying does not help.
    #[error("policy failed: {}", .0.as_str())]
    Permanent(ReasonCode),
}

impl PolicyError {
    pub fn transient(label: impl Into<String>) -> Result<Self, ContractError> {
        Ok(Self::Transient(ReasonCode::new(label)?))
    }

    pub fn permanent(label: impl Into<String>) -> Result<Self, ContractError> {
        Ok(Self::Permanent(ReasonCode::new(label)?))
    }

    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Transient(label) | Self::Permanent(label) => label.as_str(),
        }
    }
}

#[async_trait]
pub trait AdmissionPolicy: Send + Sync {
    async fn execute(
        &self,
        input: &AdmissionInput,
    ) -> Result<PhaseResult<AdmissionDecision, AdmissionEvidence, AdmissionEvaluation>, PolicyError>;
}

#[async_trait]
pub trait ReviewPolicy: Send + Sync {
    async fn execute(&self, input: &ReviewInput) -> Result<ReviewOutput, PolicyError>;
}

#[async_trait]
pub trait ScorePolicy: Send + Sync {
    async fn execute(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError>;
}

#[async_trait]
pub trait SettlePolicy: Send + Sync {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(bytes: &[u8]) -> String {
        sha256_prefixed(bytes)
    }

    fn policy(name: &str, configuration: &[u8]) -> PolicyRef {
        PolicyRef {
            policy_id: name.to_string(),
            implementation_id: format!("{name}.v1"),
            configuration_hash: hash(configuration),
            data_artifact_hashes: Vec::new(),
            projection_ids: Vec::new(),
        }
    }

    fn trace_credit_descriptor() -> InstrumentDescriptor {
        InstrumentDescriptor {
            kind: InstrumentKind::Nep141,
            network: "mainnet".to_string(),
            contract: "trace-credit.golden.near".to_string(),
            decimals: TRACE_CREDIT_DECIMALS,
        }
    }

    fn bat_descriptor() -> InstrumentDescriptor {
        InstrumentDescriptor {
            kind: InstrumentKind::Erc20,
            network: "1".to_string(),
            contract: "0x0d8775f648430679a709e98d2b0cb6250d2887ef".to_string(),
            decimals: 18,
        }
    }

    fn pinned_instruments() -> BTreeMap<InstrumentId, InstrumentDescriptor> {
        BTreeMap::from([
            (InstrumentId::new("bat").unwrap(), bat_descriptor()),
            (InstrumentId::trace_credit(), trace_credit_descriptor()),
        ])
    }

    #[test]
    fn checked_microcredit_conversion_is_exact() {
        assert_eq!(
            Microcredits::from_credit_decimal("1.000001").unwrap().get(),
            1_000_001
        );
        assert_eq!(
            Microcredits::from_credit_decimal("18446744073709.551615")
                .unwrap()
                .get(),
            u64::MAX
        );
        assert_eq!(
            Microcredits::from_credit_decimal("0.0000001"),
            Err(MicrocreditConversionError::ExcessPrecision)
        );
        assert_eq!(
            Microcredits::from_credit_decimal("-1"),
            Err(MicrocreditConversionError::Invalid)
        );
        assert_eq!(
            Microcredits::from_credit_decimal("18446744073709.551616"),
            Err(MicrocreditConversionError::Overflow)
        );
        let award =
            InstrumentAward::trace_credit(Microcredits::from_credit_decimal("2.000001").unwrap())
                .unwrap();
        assert_eq!(
            award.trace_credit_microcredits().unwrap(),
            Microcredits::from_raw(2_000_001)
        );
    }

    fn award(instrument_id: &str, atomic_units: u128) -> InstrumentAward {
        InstrumentAward::new(
            InstrumentId::new(instrument_id).unwrap(),
            AtomicUnits::from_raw(atomic_units),
        )
        .unwrap()
    }

    #[test]
    fn instrument_identity_is_bounded_and_canonical() {
        assert_eq!(
            InstrumentId::new("Trace_Credit"),
            Err(ContractError::InvalidInstrumentId)
        );
        assert_eq!(
            InstrumentId::new("x".repeat(MAX_INSTRUMENT_ID_LEN + 1)),
            Err(ContractError::InvalidInstrumentId)
        );

        let first =
            InstrumentAwards::new(vec![award("storage_rebate", 7), award("trace_credit", 3)])
                .unwrap();
        let second =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        assert_eq!(first.canonical_id(), second.canonical_id());
    }

    #[test]
    fn duplicate_instruments_are_rejected() {
        assert_eq!(
            InstrumentAwards::new(vec![award("trace_credit", 1), award("trace_credit", 2)]),
            Err(ContractError::DuplicateInstrumentId)
        );
    }

    #[test]
    fn instrument_awards_have_deterministic_ordering() {
        let awards =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        let ids = awards
            .iter()
            .map(|award| award.instrument_id().as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["storage_rebate", "trace_credit"]);
    }

    #[test]
    fn atomic_unit_arithmetic_rejects_overflow() {
        assert_eq!(
            AtomicUnits::from_raw(u128::MAX).checked_add(AtomicUnits::from_raw(1)),
            Err(ContractError::AtomicUnitOverflow)
        );
        // One above `u128::MAX`.
        assert_eq!(
            "340282366920938463463374607431768211456".parse::<AtomicUnits>(),
            Err(ContractError::AtomicUnitOverflow)
        );
    }

    /// A NEP-141 balance above `u64::MAX` survives the wire, and only the
    /// canonical decimal spelling of an amount loads.
    #[test]
    fn atomic_units_travel_as_canonical_decimal_strings() {
        use serde_json::{from_value, json, to_value};

        // 10^24 yoctoNEAR is one NEAR.
        let one_near = AtomicUnits::from_raw(10u128.pow(24));
        assert_eq!(
            to_value(one_near).unwrap(),
            json!("1000000000000000000000000")
        );
        for units in [
            AtomicUnits::ZERO,
            one_near,
            AtomicUnits::from_raw(u128::MAX),
        ] {
            let stored = to_value(units).unwrap();
            assert_eq!(from_value::<AtomicUnits>(stored).unwrap(), units);
            assert_eq!(units.to_string().parse::<AtomicUnits>(), Ok(units));
        }

        for malformed in [
            "", "+1", "-1", "01", "00", " 1", "1 ", "1.0", "1e3", "0x10", "\u{ff11}",
        ] {
            assert_eq!(
                malformed.parse::<AtomicUnits>(),
                Err(ContractError::NonCanonicalAtomicUnits),
                "{malformed:?}"
            );
            assert!(from_value::<AtomicUnits>(json!(malformed)).is_err());
        }
        // A JSON number is refused, even a small one.
        assert!(from_value::<AtomicUnits>(json!(3)).is_err());
        assert!(
            from_value::<InstrumentAward>(json!({
                "instrument_id": "storage_rebate",
                "atomic_units": 3,
            }))
            .is_err()
        );
    }

    #[test]
    fn award_set_identity_encodes_each_amount_in_sixteen_bytes() {
        let awards = InstrumentAwards::new(vec![award("storage_rebate", u128::MAX)]).unwrap();
        let mut expected = b"trace-commons-instrument-awards\0".to_vec();
        expected.extend_from_slice(&1u64.to_be_bytes());
        expected.extend_from_slice(&("storage_rebate".len() as u64).to_be_bytes());
        expected.extend_from_slice(b"storage_rebate");
        expected.extend_from_slice(&[0xff; 16]);
        assert_eq!(awards.canonical_id(), hash(&expected));
    }

    #[test]
    fn settle_decision_preserves_two_simultaneous_instruments() {
        let awards =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        let trace_credit = InstrumentSettlement::new(
            InstrumentId::new("trace_credit").unwrap(),
            AtomicUnits::from_raw(3),
            hash(b"trace-credit-operation"),
            hash(b"trace-credit-result"),
        )
        .unwrap();
        let storage_rebate = InstrumentSettlement::new(
            InstrumentId::new("storage_rebate").unwrap(),
            AtomicUnits::from_raw(7),
            hash(b"storage-rebate-operation"),
            hash(b"storage-rebate-result"),
        )
        .unwrap();

        let decision = SettleDecision::new(
            IndexMembershipDecision::Exclude {
                reason: ReasonCode::new("not_selected").unwrap(),
            },
            &ScoreDecision {
                awards: awards.clone(),
            },
            vec![trace_credit, storage_rebate],
        )
        .unwrap();

        assert_eq!(decision.settlement_operations().len(), 2);
        assert_eq!(
            decision.settlement_operations()[0].instrument_id().as_str(),
            "storage_rebate"
        );
        assert_eq!(
            decision.settlement_operations()[1].result_ref_hash(),
            Some(hash(b"trace-credit-result").as_str())
        );

        let missing_operation = InstrumentSettlement::new(
            InstrumentId::new("trace_credit").unwrap(),
            AtomicUnits::from_raw(3),
            hash(b"trace-credit-operation"),
            hash(b"trace-credit-result"),
        )
        .unwrap();
        assert_eq!(
            SettleDecision::new(
                IndexMembershipDecision::Exclude {
                    reason: ReasonCode::new("not_selected").unwrap(),
                },
                &ScoreDecision { awards },
                vec![missing_operation],
            ),
            Err(ContractError::SettlementOperationMismatch)
        );
    }

    fn review_result(
        decision: ReviewDecision,
        result_bytes: &[u8],
        worker_identity: Option<&str>,
    ) -> PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation> {
        PhaseResult {
            decision,
            evidence: ReviewEvidence {
                source_content_hash: hash(b"source"),
                result_content_hash: hash(result_bytes),
                content_changed: result_bytes != b"source",
                worker_identity: worker_identity.map(str::to_string),
                transformation_metadata_hash: None,
                human_assessment_hash: None,
                resolved_quarantine_reasons: Vec::new(),
            },
            evaluation: ReviewEvaluation {
                rule_id: "review_rule".to_string(),
            },
        }
    }

    #[test]
    fn review_output_ties_approved_content_to_evidence() {
        let approved = ReviewDecision::Approved {
            registry_revision_id: Uuid::nil(),
        };
        let content = ApprovedContent::new(b"scrubbed".to_vec(), "pii_scrubber.v1", None).unwrap();
        let output = ReviewOutput::approved(
            review_result(approved.clone(), b"scrubbed", Some("pii_scrubber.v1")),
            content.clone(),
        )
        .unwrap();
        assert_eq!(output.approved_content(), Some(&content));
        assert_eq!(content.content_hash(), hash(b"scrubbed"));
        // A derived `Debug` would print the bytes; "scr" is 115, 99, 114.
        assert!(!format!("{content:?}").contains("115, 99, 114"));
        assert!(!format!("{output:?}").contains("115, 99, 114"));

        // Evidence that names other bytes, another worker, or a rejection is refused.
        for result in [
            review_result(approved.clone(), b"other", Some("pii_scrubber.v1")),
            review_result(approved.clone(), b"scrubbed", Some("other_worker")),
            review_result(
                ReviewDecision::Rejected {
                    reason: ReasonCode::new("rejected").unwrap(),
                },
                b"scrubbed",
                Some("pii_scrubber.v1"),
            ),
        ] {
            assert_eq!(
                ReviewOutput::approved(result, content.clone()),
                Err(ContractError::ReviewOutputMismatch)
            );
        }
        assert_eq!(
            ReviewOutput::rejected(review_result(approved, b"source", None)),
            Err(ContractError::ReviewOutputMismatch)
        );
        assert_eq!(
            ApprovedContent::new(b"x".to_vec(), "Not Safe", None),
            Err(ContractError::InvalidProvenanceLabel)
        );
    }

    fn index_entry(chunk: u32, embedding: Vec<f32>) -> SealedIndexEntry {
        SealedIndexEntry {
            chunk,
            content_hash: hash(format!("chunk-{chunk}").as_bytes()),
            embedding,
        }
    }

    fn index_command(entries: Vec<SealedIndexEntry>) -> Result<SealedIndexCommand, ContractError> {
        SealedIndexCommand::new(
            "index-v1",
            Uuid::nil(),
            "projection-v1",
            "embedder-v1",
            entries,
        )
    }

    fn score_result(
        embedding_artifact_hash: Option<String>,
        include_eligible: bool,
    ) -> PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation> {
        let mut evidence = ScoreEvidence::fixed(InstrumentAwards::default());
        evidence.embedding_artifact_hash = embedding_artifact_hash;
        evidence.include_eligible = Some(include_eligible);
        evidence.index_id = Some("index-v1".to_string());
        evidence.projection_id = Some("projection-v1".to_string());
        evidence.embedder_model_id = Some("embedder-v1".to_string());
        PhaseResult {
            decision: ScoreDecision {
                awards: InstrumentAwards::default(),
            },
            evidence,
            evaluation: ScoreEvaluation {
                rule_id: "score_rule".to_string(),
                awards: InstrumentAwards::default(),
            },
        }
    }

    #[test]
    fn score_output_names_every_stored_chunk_by_hash() {
        let command = index_command(vec![
            index_entry(7, vec![0.5, -0.25]),
            index_entry(2, vec![0.125, 1.0]),
        ])
        .unwrap();
        let chunks = command
            .entries()
            .iter()
            .map(|entry| entry.chunk)
            .collect::<Vec<_>>();
        assert_eq!(chunks, [2, 7]);
        let command_hash = command.content_hash().unwrap();

        let output = ScoreOutput::new(
            score_result(Some(command_hash.clone()), true),
            Some(command.clone()),
            None,
        )
        .unwrap();
        assert_eq!(output.index_command(), Some(&command));
        assert!(!format!("{output:?}").contains("0.125"));

        // One changed bit in one component changes the command identity.
        let mut changed = command.clone();
        changed.entries[1].embedding[0] = f32::from_bits(0.5f32.to_bits() + 1);
        assert_ne!(changed.content_hash().unwrap(), command_hash);

        for (result, command) in [
            (
                score_result(Some(hash(b"other")), true),
                Some(command.clone()),
            ),
            (
                score_result(Some(command_hash.clone()), false),
                Some(command.clone()),
            ),
            (score_result(None, true), None),
        ] {
            assert_eq!(
                ScoreOutput::new(result, command, None),
                Err(ContractError::ScoreOutputMismatch)
            );
        }

        let neighbors = b"neighbor-list".to_vec();
        let mut result = score_result(None, false);
        result.evidence.neighbor_artifact_hash = Some(hash(&neighbors));
        assert!(ScoreOutput::new(result.clone(), None, Some(neighbors)).is_ok());
        assert_eq!(
            ScoreOutput::new(result, None, None),
            Err(ContractError::ScoreOutputMismatch)
        );

        for entries in [
            Vec::new(),
            vec![index_entry(1, vec![f32::NAN])],
            vec![index_entry(1, Vec::new())],
            vec![index_entry(1, vec![0.0]), index_entry(1, vec![1.0])],
            vec![index_entry(1, vec![0.0]), index_entry(2, vec![1.0, 2.0])],
        ] {
            assert_eq!(
                index_command(entries).err(),
                Some(ContractError::InvalidIndexCommand)
            );
        }
    }

    #[test]
    fn score_output_states_capped_coverage() {
        for (scored, total, capped, consistent) in [
            (Some(3), Some(5), Some(true), true),
            (Some(5), Some(5), Some(false), true),
            (Some(3), Some(5), Some(false), false),
            (Some(5), Some(5), Some(true), false),
            (Some(6), Some(5), Some(true), false),
            (Some(3), None, Some(true), false),
        ] {
            let mut result = score_result(None, false);
            result.evidence.chunk_count = scored;
            result.evidence.total_chunk_count = total;
            result.evidence.chunks_capped = capped;
            assert_eq!(
                ScoreOutput::new(result, None, None).is_ok(),
                consistent,
                "{scored:?} of {total:?}, capped {capped:?}"
            );
        }
    }

    #[test]
    fn hash_fields_require_lowercase_sha256() {
        let upper = hash(b"x").to_uppercase().replace("SHA256:", "sha256:");
        assert!(!is_sha256(&upper));

        let mut review = review_result(
            ReviewDecision::Rejected {
                reason: ReasonCode::new("rejected").unwrap(),
            },
            b"source",
            None,
        );
        review.evidence.human_assessment_hash = Some(upper.clone());
        assert_eq!(
            ReviewOutput::rejected(review),
            Err(ContractError::MalformedHash)
        );

        let mut score = score_result(None, false);
        score.evidence.nearest_neighbor_hash = Some("sha256:short".to_string());
        assert_eq!(
            ScoreOutput::new(score, None, None),
            Err(ContractError::MalformedHash)
        );

        assert_eq!(
            SettleDecision::new(
                IndexMembershipDecision::Include {
                    command_hash: upper.clone(),
                    entry_count: 1,
                },
                &ScoreDecision {
                    awards: InstrumentAwards::default(),
                },
                Vec::new(),
            ),
            Err(ContractError::MalformedHash)
        );

        let mut settle = SettleEvidence::operations(false, 1);
        settle
            .settlement_progress
            .push(InstrumentSettlementProgress {
                instrument_id: InstrumentId::trace_credit(),
                operation_ref_hash: hash(b"operation"),
                result_ref_hash: Some(upper),
            });
        assert_eq!(settle.validate(), Err(ContractError::MalformedHash));
    }

    #[test]
    fn loaded_evidence_rejects_a_malformed_hash() {
        use serde_json::{from_value, json, to_value};

        let digest = hash(b"evidence");
        let bad = "sha256:not-lowercase-sha256";

        let admission = AdmissionEvidence {
            request_content_hash: digest.clone(),
            schema_valid: true,
            authority_valid: true,
            contribution_path_valid: true,
            grant_valid: true,
            consent_valid: true,
            allowed_uses_valid: true,
            quota_counted: true,
            detector_ids: Vec::new(),
            privacy_risk: None,
        };
        assert_eq!(
            from_value::<AdmissionEvidence>(to_value(&admission).unwrap()).unwrap(),
            admission
        );
        let mut stored = to_value(&admission).unwrap();
        stored["request_content_hash"] = json!(bad);
        assert!(from_value::<AdmissionEvidence>(stored).is_err());

        let review = ReviewEvidence {
            source_content_hash: digest.clone(),
            result_content_hash: digest.clone(),
            content_changed: false,
            worker_identity: None,
            transformation_metadata_hash: None,
            human_assessment_hash: None,
            resolved_quarantine_reasons: Vec::new(),
        };
        assert_eq!(
            from_value::<ReviewEvidence>(to_value(&review).unwrap()).unwrap(),
            review
        );
        let mut stored = to_value(&review).unwrap();
        stored["human_assessment_hash"] = json!(bad);
        assert!(from_value::<ReviewEvidence>(stored).is_err());

        let score = ScoreEvidence::fixed(InstrumentAwards::default());
        assert_eq!(
            from_value::<ScoreEvidence>(to_value(&score).unwrap()).unwrap(),
            score
        );
        let mut stored = to_value(&score).unwrap();
        stored["nearest_neighbor_hash"] = json!(bad);
        assert!(from_value::<ScoreEvidence>(stored).is_err());

        let settle = SettleEvidence::operations(false, 0);
        assert_eq!(
            from_value::<SettleEvidence>(to_value(&settle).unwrap()).unwrap(),
            settle
        );
        let mut stored = to_value(&settle).unwrap();
        stored["index_command_hash"] = json!(bad);
        assert!(from_value::<SettleEvidence>(stored).is_err());

        let assessment = HumanReviewAssessment {
            assessment_id: Uuid::nil(),
            recommendation: ReviewRecommendation::Reject,
            reason: ReasonCode::new("unsafe").unwrap(),
            resolved_quarantine_reasons: Vec::new(),
            evidence_hash: digest,
        };
        assert_eq!(
            from_value::<HumanReviewAssessment>(to_value(&assessment).unwrap()).unwrap(),
            assessment
        );
        let mut stored = to_value(&assessment).unwrap();
        stored["evidence_hash"] = json!(bad);
        assert!(from_value::<HumanReviewAssessment>(stored).is_err());
    }

    #[test]
    fn decisions_use_snake_case_tags() {
        use serde::de::IntoDeserializer;
        use serde::de::value::Error;

        fn load<T: serde::de::DeserializeOwned>(
            fields: &[(&'static str, &'static str)],
        ) -> Option<T> {
            let map = fields.iter().copied().collect::<BTreeMap<_, _>>();
            T::deserialize(IntoDeserializer::<Error>::into_deserializer(map)).ok()
        }

        assert_eq!(
            load::<AdmissionDecision>(&[("kind", "admit")]),
            Some(AdmissionDecision::Admit)
        );
        assert_eq!(
            load::<AdmissionDecision>(&[("kind", "quarantine"), ("reason", "privacy_review")]),
            Some(AdmissionDecision::Quarantine {
                reason: ReasonCode::new("privacy_review").unwrap()
            })
        );
        assert_eq!(
            load::<IndexMembershipDecision>(&[("kind", "exclude"), ("reason", "withdrawn")]),
            Some(IndexMembershipDecision::Exclude {
                reason: ReasonCode::new("withdrawn").unwrap()
            })
        );
        assert_eq!(
            load::<ReviewDecision>(&[("kind", "rejected"), ("reason", "unsafe")]),
            Some(ReviewDecision::Rejected {
                reason: ReasonCode::new("unsafe").unwrap()
            })
        );
        assert_eq!(load::<AdmissionDecision>(&[("kind", "Admit")]), None);
    }

    /// A self-describing value, so serde paths can be tested without a
    /// format crate.
    #[derive(Clone, Copy)]
    enum TestValue {
        Str(&'static str),
        Bool(bool),
    }

    impl serde::de::IntoDeserializer<'_, serde::de::value::Error> for TestValue {
        type Deserializer = Self;

        fn into_deserializer(self) -> Self {
            self
        }
    }

    impl<'de> serde::Deserializer<'de> for TestValue {
        type Error = serde::de::value::Error;

        fn deserialize_any<V: serde::de::Visitor<'de>>(
            self,
            visitor: V,
        ) -> Result<V::Value, Self::Error> {
            match self {
                Self::Str(value) => visitor.visit_str(value),
                Self::Bool(value) => visitor.visit_bool(value),
            }
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes byte_buf option unit unit_struct newtype_struct seq tuple
            tuple_struct map struct enum identifier ignored_any
        }
    }

    fn load_test_value<T: serde::de::DeserializeOwned>(
        fields: &[(&'static str, TestValue)],
    ) -> Result<T, serde::de::value::Error> {
        use serde::de::IntoDeserializer;
        let map = fields.iter().copied().collect::<BTreeMap<_, _>>();
        T::deserialize(map.into_deserializer())
    }

    #[test]
    fn admission_evidence_has_no_favorable_defaults() {
        use TestValue::{Bool, Str};

        let fields = [
            (
                "request_content_hash",
                Str("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            ),
            ("schema_valid", Bool(true)),
            ("authority_valid", Bool(true)),
            ("contribution_path_valid", Bool(true)),
            ("grant_valid", Bool(true)),
            ("consent_valid", Bool(true)),
            ("allowed_uses_valid", Bool(true)),
            ("quota_counted", Bool(true)),
        ];
        assert!(load_test_value::<AdmissionEvidence>(&fields).is_ok());
        for missing in 1..fields.len() {
            let mut partial = fields.to_vec();
            let (name, _) = partial.remove(missing);
            assert!(
                load_test_value::<AdmissionEvidence>(&partial).is_err(),
                "{name} defaulted"
            );
        }

        for (value, risk) in [
            ("low", Some(PrivacyRisk::Low)),
            ("medium", Some(PrivacyRisk::Medium)),
            ("high", Some(PrivacyRisk::High)),
            ("Low", None),
            ("unknown", None),
        ] {
            let loaded = PrivacyRisk::deserialize(serde::de::IntoDeserializer::<
                serde::de::value::Error,
            >::into_deserializer(value));
            assert_eq!(loaded.ok(), risk, "{value}");
        }
    }

    #[test]
    fn loading_repeats_constructor_checks() {
        use serde_json::{from_value, json, to_value};

        assert!(from_value::<ReasonCode>(json!("acct:alice@example.com")).is_err());
        assert!(from_value::<InstrumentId>(json!("Trace Credit")).is_err());

        let awards =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        let stored = to_value(&awards).unwrap();
        assert_eq!(
            stored,
            json!([
                {"instrument_id": "storage_rebate", "atomic_units": "7"},
                {"instrument_id": "trace_credit", "atomic_units": "3"},
            ])
        );
        assert_eq!(from_value::<InstrumentAwards>(stored).unwrap(), awards);
        let reordered = json!([
            {"instrument_id": "trace_credit", "atomic_units": "3"},
            {"instrument_id": "storage_rebate", "atomic_units": "7"},
        ]);
        assert_eq!(from_value::<InstrumentAwards>(reordered).unwrap(), awards);
        for refused in [
            json!([{"instrument_id": "trace_credit", "atomic_units": "0"}]),
            json!([
                {"instrument_id": "trace_credit", "atomic_units": "3"},
                {"instrument_id": "trace_credit", "atomic_units": "4"},
            ]),
        ] {
            assert!(from_value::<InstrumentAwards>(refused).is_err());
        }

        let operation = |operation_ref: &str, result_ref: &str| {
            json!({
                "instrument_id": "trace_credit",
                "atomic_units": "3",
                "operation_ref_hash": operation_ref,
                "outcome": {"status": "completed", "result_ref_hash": result_ref},
            })
        };
        let valid = operation(&hash(b"operation"), &hash(b"result"));
        assert!(from_value::<InstrumentSettlement>(valid.clone()).is_ok());
        for refused in [
            operation("not-a-hash", &hash(b"result")),
            operation(&hash(b"operation"), "not-a-hash"),
        ] {
            assert!(from_value::<InstrumentSettlement>(refused).is_err());
        }

        let decision = json!({
            "index_membership": {"kind": "exclude", "reason": "not_selected"},
            "settlement_operations": [valid.clone(), valid],
        });
        assert!(from_value::<SettleDecision>(decision).is_err());

        let command =
            index_command(vec![index_entry(1, vec![0.5]), index_entry(2, vec![0.25])]).unwrap();
        let mut stored = to_value(&command).unwrap();
        assert_eq!(
            from_value::<SealedIndexCommand>(stored.clone()).unwrap(),
            command
        );
        stored["entries"].as_array_mut().unwrap().reverse();
        assert!(from_value::<SealedIndexCommand>(stored).is_err());
    }

    #[test]
    fn settle_operations_follow_the_committed_score() {
        let credit = InstrumentAwards::new(vec![award("trace_credit", 3)]).unwrap();
        let other = InstrumentAwards::new(vec![award("trace_credit", 4)]).unwrap();

        // Score cannot commit copies of its award set that disagree.
        for (evidence_awards, evaluation_awards) in [
            (other.clone(), credit.clone()),
            (credit.clone(), other.clone()),
        ] {
            let mut result = score_result(None, false);
            result.decision.awards = credit.clone();
            result.evidence.fixed_awards = evidence_awards;
            result.evaluation.awards = evaluation_awards;
            assert_eq!(
                ScoreOutput::new(result, None, None),
                Err(ContractError::ScoreOutputMismatch)
            );
        }

        let operation = InstrumentSettlement::new(
            InstrumentId::trace_credit(),
            AtomicUnits::from_raw(3),
            hash(b"operation"),
            hash(b"result"),
        )
        .unwrap();
        let exclude = IndexMembershipDecision::Exclude {
            reason: ReasonCode::new("not_selected").unwrap(),
        };
        let score = ScoreDecision { awards: credit };
        let decision =
            SettleDecision::new(exclude.clone(), &score, vec![operation.clone()]).unwrap();
        assert_eq!(
            SettleDecision::new(
                exclude,
                &ScoreDecision {
                    awards: other.clone()
                },
                vec![operation]
            ),
            Err(ContractError::SettlementOperationMismatch)
        );

        // A loaded decision is checked against the Score outcome it settles.
        let loaded: SettleDecision =
            serde_json::from_value(serde_json::to_value(&decision).unwrap()).unwrap();
        assert_eq!(loaded.matches_score(&score), Ok(()));
        assert_eq!(
            loaded.matches_score(&ScoreDecision { awards: other }),
            Err(ContractError::SettlementOperationMismatch)
        );
    }

    #[test]
    fn trace_credit_awards_fit_the_credit_ledger() {
        let ledger_max = AtomicUnits::from_raw(i64::MAX as u128);
        let over = AtomicUnits::from_raw(i64::MAX as u128 + 1);
        assert!(InstrumentAward::new(InstrumentId::trace_credit(), ledger_max).is_ok());
        assert_eq!(
            InstrumentAward::new(InstrumentId::trace_credit(), over),
            Err(ContractError::TraceCreditOutOfRange)
        );
        assert_eq!(
            InstrumentAward::trace_credit(Microcredits::from_raw(u64::MAX)),
            Err(ContractError::TraceCreditOutOfRange)
        );
        // Other instruments keep the full range the settlement table stores.
        let rebate = InstrumentId::new("storage_rebate").unwrap();
        assert!(InstrumentAward::new(rebate, AtomicUnits::from_raw(u128::MAX)).is_ok());
        // A loaded award is bounded too.
        assert!(
            serde_json::from_value::<InstrumentAward>(serde_json::json!({
                "instrument_id": "trace_credit",
                "atomic_units": (i64::MAX as u128 + 1).to_string(),
            }))
            .is_err()
        );
    }

    #[test]
    fn trace_credit_settlements_fit_the_credit_ledger() {
        let ledger_max = AtomicUnits::from_raw(i64::MAX as u128);
        let over = AtomicUnits::from_raw(i64::MAX as u128 + 1);
        let operation = hash(b"operation");
        let result = hash(b"result");
        assert_eq!(
            InstrumentSettlement::new(
                InstrumentId::trace_credit(),
                over,
                operation.clone(),
                result.clone(),
            ),
            Err(ContractError::TraceCreditOutOfRange)
        );
        assert_eq!(
            InstrumentSettlement::forfeited(
                InstrumentId::trace_credit(),
                over,
                operation.clone(),
                ReasonCode::new("withdrawn").unwrap(),
            ),
            Err(ContractError::TraceCreditOutOfRange)
        );
        assert!(
            InstrumentSettlement::new(
                InstrumentId::trace_credit(),
                ledger_max,
                operation.clone(),
                result.clone(),
            )
            .is_ok()
        );
        assert!(
            InstrumentSettlement::new(
                InstrumentId::new("storage_rebate").unwrap(),
                AtomicUnits::from_raw(u128::MAX),
                operation.clone(),
                result.clone(),
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<InstrumentSettlement>(serde_json::json!({
                "instrument_id": "trace_credit",
                "atomic_units": (i64::MAX as u128 + 1).to_string(),
                "operation_ref_hash": operation,
                "outcome": {"status": "completed", "result_ref_hash": result},
            }))
            .is_err()
        );
    }

    #[test]
    fn only_trace_credit_awards_convert_to_microcredits() {
        assert_eq!(
            award("trace_credit", 3).trace_credit_microcredits(),
            Ok(Microcredits::from_raw(3))
        );
        assert_eq!(
            award("storage_rebate", 3).trace_credit_microcredits(),
            Err(ContractError::NotTraceCredit)
        );
    }

    #[test]
    fn policy_errors_say_whether_the_trace_is_at_fault() {
        let outage = PolicyError::transient("scorer_unavailable").unwrap();
        let refused = PolicyError::permanent("unsupported_schema").unwrap();
        assert!(outage.is_transient());
        assert!(!refused.is_transient());
        assert_eq!(outage.label(), "scorer_unavailable");
        assert_eq!(
            outage.to_string(),
            "policy dependency failed: scorer_unavailable"
        );
        assert_eq!(refused.to_string(), "policy failed: unsupported_schema");
        // Labels stay safe: a raw backend message is refused, not forwarded.
        assert_eq!(
            PolicyError::transient("HttpStatusError status=502"),
            Err(ContractError::InvalidReasonCode)
        );
    }

    #[test]
    fn tenant_storage_refs_take_only_the_derived_form() {
        let derived = format!("tenant_sha256:{}", "0123456789abcdef".repeat(2));
        assert_eq!(
            TenantStorageRef::new(derived.clone()).unwrap().as_str(),
            derived
        );
        for refused in [
            "tenant-a".to_string(),
            "7b1f0c36-8f5e-4f55-9d1e-3b2a6c0e9f11".to_string(),
            derived.to_uppercase(),
            format!("tenant_sha256:{}", "0".repeat(64)),
            format!("sha256:{}", "0".repeat(32)),
        ] {
            assert_eq!(
                TenantStorageRef::new(refused),
                Err(ContractError::InvalidTenantStorageRef)
            );
        }
    }

    #[test]
    fn debug_output_withholds_trace_bodies_and_vectors() {
        let tenant_storage_ref =
            TenantStorageRef::new(format!("tenant_sha256:{}", "a".repeat(32))).unwrap();
        let body = b"key AKIAIOSFODNN7EXAMPLE mail alice@example.com".to_vec();
        // How a derived `Debug` prints the body's first bytes, "key".
        let printed_bytes = "107, 101, 121";

        let review = ReviewInput {
            run_id: Uuid::nil(),
            tenant_storage_ref: tenant_storage_ref.clone(),
            trace_id: Uuid::nil(),
            source_content_hash: hash(&body),
            source_artifact: body.clone(),
            admission: AdmissionDecision::Admit,
            human_assessment: None,
        };
        let score = ScoreInput {
            run_id: Uuid::nil(),
            tenant_storage_ref: tenant_storage_ref.clone(),
            trace_id: Uuid::nil(),
            registry_revision_id: Uuid::nil(),
            source_content_hash: hash(&body),
            reviewed_artifact: body,
        };
        let command = index_command(vec![index_entry(1, vec![0.125])]).unwrap();
        let settle = SettleInput {
            run_id: Uuid::nil(),
            tenant_storage_ref,
            trace_id: Uuid::nil(),
            registry_revision_id: Uuid::nil(),
            source_content_hash: hash(b"source"),
            score: ScoreDecision {
                awards: InstrumentAwards::default(),
            },
            score_evidence: score_result(None, false).evidence,
            index_command: Some(command),
        };

        for printed in [
            format!("{review:?}"),
            format!("{score:?}"),
            format!("{settle:?}"),
        ] {
            assert!(!printed.contains(printed_bytes), "{printed}");
            assert!(!printed.contains("0.125"), "{printed}");
        }
        assert!(format!("{review:?}").contains("<withheld>"));
        assert!(format!("{score:?}").contains("<withheld>"));
    }

    fn golden_manifest() -> BundleManifest {
        let policy = |name: &str| PolicyRef {
            policy_id: format!("trace_commons.{name}.golden"),
            implementation_id: format!("trace_commons.{name}.golden.v1"),
            configuration_hash: hash(format!("{name}-configuration").as_bytes()),
            data_artifact_hashes: vec![hash(format!("{name}-data").as_bytes())],
            projection_ids: vec![format!("{name}-projection")],
        };
        BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy("admission"),
            review: policy("review"),
            score: policy("score"),
            settle: policy("settle"),
            instruments: pinned_instruments(),
        }
    }

    #[test]
    fn duplicate_policy_list_entries_are_rejected() {
        let mut repeated_projection = golden_manifest();
        repeated_projection.score.projection_ids = vec!["proj".to_string(), "proj".to_string()];
        assert_eq!(
            repeated_projection.bundle_id(),
            Err(ContractError::DuplicatePolicyListEntry)
        );

        let mut repeated_hash = golden_manifest();
        let digest = hash(b"shared");
        repeated_hash.score.data_artifact_hashes = vec![digest.clone(), digest];
        assert_eq!(
            repeated_hash.bundle_id(),
            Err(ContractError::DuplicatePolicyListEntry)
        );

        let mut forward = golden_manifest();
        forward.score.projection_ids = vec!["a".to_string(), "b".to_string()];
        let mut reversed = golden_manifest();
        reversed.score.projection_ids = vec!["b".to_string(), "a".to_string()];
        assert_eq!(forward.bundle_id().unwrap(), reversed.bundle_id().unwrap());
    }

    /// Fixed identities. A change to a canonical encoding changes these values,
    /// and with them every stored `bundle_id`, signed `package_hash`, award-set
    /// identity, or stored index command. A failure here means an encoding
    /// changed: bump the affected format or schema version, do not just update
    /// the value.
    #[test]
    fn golden_identities_are_stable() {
        let manifest = golden_manifest();

        // The manifest encoding, written out: domain, u32 format version, then
        // for each phase in order its policy id, implementation id,
        // configuration hash, and sorted data hashes and projection ids. Then
        // the pinned instruments in instrument order: id, kind, network,
        // contract, and one `decimals` byte. Every length and count is a
        // big-endian u64.
        let string = |bytes: &mut Vec<u8>, value: &str| {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        };
        let mut expected = b"trace-commons-bundle-manifest\0".to_vec();
        expected.extend_from_slice(&1u32.to_be_bytes());
        for name in ["admission", "review", "score", "settle"] {
            string(&mut expected, &format!("trace_commons.{name}.golden"));
            string(&mut expected, &format!("trace_commons.{name}.golden.v1"));
            string(
                &mut expected,
                &hash(format!("{name}-configuration").as_bytes()),
            );
            expected.extend_from_slice(&1u64.to_be_bytes());
            string(&mut expected, &hash(format!("{name}-data").as_bytes()));
            expected.extend_from_slice(&1u64.to_be_bytes());
            string(&mut expected, &format!("{name}-projection"));
        }
        expected.extend_from_slice(&2u64.to_be_bytes());
        for (id, kind, network, contract, decimals) in [
            (
                "bat",
                "erc20",
                "1",
                "0x0d8775f648430679a709e98d2b0cb6250d2887ef",
                18,
            ),
            (
                "trace_credit",
                "nep141",
                "mainnet",
                "trace-credit.golden.near",
                6,
            ),
        ] {
            for value in [id, kind, network, contract] {
                string(&mut expected, value);
            }
            expected.push(decimals);
        }
        assert_eq!(manifest.canonical_bytes().unwrap(), expected);
        assert_eq!(
            manifest.bundle_id().unwrap(),
            "sha256:c913706a95ab42d083b633979578fc4e46b39fe8f4204620ba7c6a2e3d6e1b76"
        );

        let artifacts = ["admission", "review", "score", "settle"]
            .into_iter()
            .flat_map(|name| {
                [
                    format!("{name}-configuration").into_bytes(),
                    format!("{name}-data").into_bytes(),
                ]
            })
            .map(|bytes| (hash(&bytes), bytes))
            .collect();
        let package = BundlePackage {
            bundle_id: manifest.bundle_id().unwrap(),
            manifest,
            artifacts,
        };
        assert_eq!(
            package.package_hash().unwrap(),
            "sha256:d0e78bc1bec67983332689133b313bd6ab92acc9ac44f2112a653022f3cdbe29"
        );

        let awards =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        assert_eq!(
            awards.canonical_id(),
            "sha256:c53b5473d879aeb407a52c4740aea317aa5f90cb41c0b62bfa9c932332889955"
        );

        let command = SealedIndexCommand::new(
            "index-v1",
            Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            "projection-v1",
            "embedder-v1",
            vec![
                index_entry(2, vec![0.25, -1.5]),
                index_entry(9, vec![1.0, 0.0]),
            ],
        )
        .unwrap();
        assert_eq!(
            command.content_hash().unwrap(),
            "sha256:5aba5d8eff158ed94aaffb1d59fe64a00dbbbfceafed7cdc4a2df968086612c7"
        );
    }

    /// Mutants the review found surviving: a decimal fraction read with the
    /// wrong scale, a Settle match that ignores amounts, and a zero award.
    #[test]
    fn amounts_are_exact_and_positive() {
        for (decimal, microcredits) in [
            ("1.5", 1_500_000),
            ("1.05", 1_050_000),
            ("0.000001", 1),
            ("12", 12_000_000),
        ] {
            assert_eq!(
                Microcredits::from_credit_decimal(decimal).map(Microcredits::get),
                Ok(microcredits),
                "{decimal}"
            );
        }

        let score = ScoreDecision {
            awards: InstrumentAwards::new(vec![award("trace_credit", 3)]).unwrap(),
        };
        let wrong_amount = InstrumentSettlement::new(
            InstrumentId::trace_credit(),
            AtomicUnits::from_raw(4),
            hash(b"operation"),
            hash(b"result"),
        )
        .unwrap();
        assert_eq!(
            SettleDecision::new(
                IndexMembershipDecision::Exclude {
                    reason: ReasonCode::new("not_selected").unwrap(),
                },
                &score,
                vec![wrong_amount],
            ),
            Err(ContractError::SettlementOperationMismatch)
        );

        assert_eq!(
            InstrumentAward::new(InstrumentId::trace_credit(), AtomicUnits::ZERO),
            Err(ContractError::ZeroInstrumentAward)
        );
    }

    #[test]
    fn settle_decision_completes_with_a_forfeited_operation() {
        let awards =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        let forfeited = InstrumentSettlement::forfeited(
            InstrumentId::new("trace_credit").unwrap(),
            AtomicUnits::from_raw(3),
            hash(b"trace-credit-operation"),
            ReasonCode::new("withdrawn").unwrap(),
        )
        .unwrap();
        let completed = InstrumentSettlement::new(
            InstrumentId::new("storage_rebate").unwrap(),
            AtomicUnits::from_raw(7),
            hash(b"storage-rebate-operation"),
            hash(b"storage-rebate-result"),
        )
        .unwrap();

        let decision = SettleDecision::new(
            IndexMembershipDecision::Exclude {
                reason: ReasonCode::new("withdrawn").unwrap(),
            },
            &ScoreDecision { awards },
            vec![forfeited, completed],
        )
        .unwrap();

        let trace_credit = &decision.settlement_operations()[1];
        assert_eq!(trace_credit.atomic_units(), AtomicUnits::from_raw(3));
        assert_eq!(trace_credit.result_ref_hash(), None);
        assert_eq!(
            trace_credit.outcome(),
            &InstrumentSettlementOutcome::Forfeited {
                reason: ReasonCode::new("withdrawn").unwrap()
            }
        );
        assert_eq!(
            decision.settlement_operations()[0].result_ref_hash(),
            Some(hash(b"storage-rebate-result").as_str())
        );

        assert_eq!(
            InstrumentSettlement::forfeited(
                InstrumentId::new("trace_credit").unwrap(),
                AtomicUnits::from_raw(3),
                "not-a-hash",
                ReasonCode::new("withdrawn").unwrap(),
            ),
            Err(ContractError::InvalidSettlementReference)
        );
    }

    #[test]
    fn package_validation_binds_manifest_and_all_artifacts() {
        let policies = [
            ("admission", b"admission-config".as_slice()),
            ("review", b"review-config".as_slice()),
            ("score", b"score-config".as_slice()),
            ("settle", b"settle-config".as_slice()),
        ];
        let manifest = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy(policies[0].0, policies[0].1),
            review: policy(policies[1].0, policies[1].1),
            score: policy(policies[2].0, policies[2].1),
            settle: policy(policies[3].0, policies[3].1),
            instruments: pinned_instruments(),
        };
        let artifacts = policies
            .iter()
            .map(|(_, config)| (hash(config), config.to_vec()))
            .collect();
        let package = BundlePackage {
            bundle_id: manifest.bundle_id().unwrap(),
            manifest,
            artifacts,
        };
        package.validate().unwrap();
        let canonical = package.canonical_bytes().unwrap();
        assert!(canonical.starts_with(b"trace-commons-bundle-package\0"));
        assert_eq!(package.package_hash().unwrap().len(), 71);

        let mut altered = package.clone();
        altered.artifacts.values_mut().next().unwrap().push(0);
        assert_eq!(altered.validate(), Err(ContractError::ArtifactHashMismatch));

        // Policy code is not a package artifact; an extra descriptor is refused.
        let mut extra = package.clone();
        extra
            .artifacts
            .insert(hash(b"admission-code"), b"admission-code".to_vec());
        assert_eq!(extra.validate(), Err(ContractError::ArtifactSetMismatch));

        let mut missing = package;
        missing.artifacts.pop_first();
        assert_eq!(missing.validate(), Err(ContractError::ArtifactSetMismatch));
    }

    #[test]
    fn package_artifacts_serialize_as_lowercase_hex() {
        use serde::de::IntoDeserializer;
        use serde::de::value::Error;

        let every_byte = (0..=u8::MAX).collect::<Vec<_>>();
        let hex = hex_artifacts::encode(&every_byte);
        assert_eq!(hex.len(), 512);
        assert!(hex.starts_with("000102") && hex.ends_with("fdfeff"));
        assert_eq!(hex_artifacts::decode(&hex), Some(every_byte.clone()));
        for malformed in ["0", "0g", "0A", " 00"] {
            assert_eq!(hex_artifacts::decode(malformed), None);
        }

        let stored = BTreeMap::from([(hash(&every_byte), hex)]);
        let loaded =
            hex_artifacts::deserialize(IntoDeserializer::<Error>::into_deserializer(stored));
        assert_eq!(
            loaded.unwrap(),
            BTreeMap::from([(hash(&every_byte), every_byte)])
        );
        let upper = BTreeMap::from([(hash(b"x"), "FF".to_string())]);
        let refused =
            hex_artifacts::deserialize(IntoDeserializer::<Error>::into_deserializer(upper));
        assert!(refused.is_err());
    }

    #[test]
    fn canonical_encodings_frame_every_length_as_u64() {
        let mut bytes = Vec::new();
        encode_list(&mut bytes, &["b".to_string(), "a".to_string()]).unwrap();
        assert_eq!(
            bytes,
            [
                &2u64.to_be_bytes()[..],
                &1u64.to_be_bytes(),
                b"a",
                &1u64.to_be_bytes(),
                b"b",
            ]
            .concat()
        );

        let manifest = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy("admission", b"configuration"),
            review: policy("review", b"configuration"),
            score: policy("score", b"configuration"),
            settle: policy("settle", b"configuration"),
            instruments: pinned_instruments(),
        };
        let canonical = manifest.canonical_bytes().unwrap();
        let domain = b"trace-commons-bundle-manifest\0".len();
        let first_len = &canonical[domain + 4..domain + 12];
        assert_eq!(first_len, &("admission".len() as u64).to_be_bytes());
    }

    #[test]
    fn every_immutable_policy_input_changes_bundle_identity() {
        let mut base = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy("admission", b"configuration"),
            review: policy("review", b"configuration"),
            score: policy("score", b"configuration"),
            settle: policy("settle", b"configuration"),
            instruments: pinned_instruments(),
        };
        let base_id = base.bundle_id().unwrap();

        let mut changed = base.clone();
        changed.review.configuration_hash = hash(b"changed-configuration");
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed
            .score
            .data_artifact_hashes
            .push(hash(b"changed-data"));
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed
            .settle
            .projection_ids
            .push("changed-projection".to_string());
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed.admission.policy_id = "changed-policy".to_string();
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed.review.implementation_id = "changed-implementation".to_string();
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        // A pinned descriptor is part of the bundle identity.
        let bat = InstrumentId::new("bat").unwrap();
        let changes: [fn(&mut InstrumentDescriptor); 4] = [
            |descriptor| descriptor.kind = InstrumentKind::Nep141,
            |descriptor| descriptor.network = "10".to_string(),
            |descriptor| descriptor.contract = format!("0x{}", "1".repeat(40)),
            |descriptor| descriptor.decimals = 8,
        ];
        for change in changes {
            let mut changed = base.clone();
            change(changed.instruments.get_mut(&bat).unwrap());
            assert_ne!(changed.bundle_id().unwrap(), base_id);
        }
        let mut changed = base.clone();
        changed.instruments.remove(&bat);
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        base.format_version += 1;
        assert_eq!(
            base.bundle_id(),
            Err(ContractError::UnsupportedManifestVersion)
        );
    }

    #[test]
    fn sealed_command_entry_keys_carry_the_tenant_reference_and_chunk() {
        let tenant =
            TenantStorageRef::new("tenant_sha256:00112233445566778899aabbccddeeff").unwrap();
        let entry = |chunk| SealedIndexEntry {
            chunk,
            content_hash: format!("sha256:{}", "a".repeat(64)),
            embedding: vec![0.5, 0.25],
        };
        let command = SealedIndexCommand::new(
            "pipeline-test-index-v1",
            Uuid::nil(),
            "pipeline-test-projection-v1",
            "reference-embedder-v1",
            vec![entry(7), entry(3)],
        )
        .unwrap();
        let keys: Vec<_> = command
            .entries()
            .iter()
            .map(|entry| command.entry_key(&tenant, entry))
            .collect();
        assert_eq!(
            keys.iter().map(|key| key.chunk).collect::<Vec<_>>(),
            vec![3, 7]
        );
        assert!(
            keys.iter()
                .all(|key| key.tenant_storage_ref == tenant.as_str())
        );
        assert!(
            keys.iter()
                .all(|key| key.model_id == "reference-embedder-v1")
        );
    }

    #[test]
    fn instrument_descriptors_have_one_spelling_per_kind() {
        let credit_account = InstrumentDescriptor {
            kind: InstrumentKind::CreditAccount,
            network: "trace_commons".to_string(),
            contract: "inference_credit".to_string(),
            decimals: 0,
        };
        for valid in [trace_credit_descriptor(), bat_descriptor(), credit_account] {
            assert_eq!(valid.validate(), Ok(()), "{valid:?}");
        }
        let implicit = InstrumentDescriptor {
            contract: "a".repeat(64),
            ..trace_credit_descriptor()
        };
        assert_eq!(implicit.validate(), Ok(()));

        let malformed = |change: fn(&mut InstrumentDescriptor), base: InstrumentDescriptor| {
            let mut descriptor = base;
            change(&mut descriptor);
            descriptor.validate()
        };
        let near: [fn(&mut InstrumentDescriptor); 8] = [
            |d| d.contract = "a".to_string(),
            |d| d.contract = "a".repeat(65),
            |d| d.contract = "-credit.near".to_string(),
            |d| d.contract = "credit.near.".to_string(),
            |d| d.contract = "credit..near".to_string(),
            |d| d.contract = "Credit.near".to_string(),
            |d| d.contract = "credit near".to_string(),
            |d| d.network = "Mainnet".to_string(),
        ];
        for change in near {
            assert_eq!(
                malformed(change, trace_credit_descriptor()),
                Err(ContractError::InvalidInstrumentDescriptor)
            );
        }
        let evm: [fn(&mut InstrumentDescriptor); 7] = [
            |d| d.network = "0".to_string(),
            |d| d.network = "01".to_string(),
            |d| d.network = "eip155:1".to_string(),
            |d| d.network = "18446744073709551616".to_string(),
            |d| d.contract = "0x0D8775F648430679A709E98D2B0CB6250D2887EF".to_string(),
            |d| d.contract = "0d8775f648430679a709e98d2b0cb6250d2887ef".to_string(),
            |d| d.contract = "0x0d8775f648430679a709e98d2b0cb6250d2887e".to_string(),
        ];
        for change in evm {
            assert_eq!(
                malformed(change, bat_descriptor()),
                Err(ContractError::InvalidInstrumentDescriptor)
            );
        }
        assert_eq!(
            malformed(|d| d.decimals = MAX_INSTRUMENT_DECIMALS, bat_descriptor()),
            Ok(())
        );
        assert_eq!(
            malformed(
                |d| d.decimals = MAX_INSTRUMENT_DECIMALS + 1,
                bat_descriptor()
            ),
            Err(ContractError::InvalidInstrumentDescriptor)
        );

        // A manifest refuses a malformed descriptor, and Trace Credit must
        // pin six decimals so one atomic unit stays one microcredit.
        let mut manifest = golden_manifest();
        manifest
            .instruments
            .get_mut(&InstrumentId::new("bat").unwrap())
            .unwrap()
            .network = "0".to_string();
        assert_eq!(
            manifest.bundle_id(),
            Err(ContractError::InvalidInstrumentDescriptor)
        );
        let mut manifest = golden_manifest();
        manifest
            .instruments
            .get_mut(&InstrumentId::trace_credit())
            .unwrap()
            .decimals = 18;
        assert_eq!(
            manifest.bundle_id(),
            Err(ContractError::TraceCreditDecimals)
        );
    }

    #[test]
    fn manifest_loading_pins_each_instrument_once() {
        use serde_json::{from_str, from_value, json, to_value};

        let manifest = golden_manifest();
        let stored = to_value(&manifest).unwrap();
        assert_eq!(
            stored["instruments"],
            json!({
                "bat": {
                    "kind": "erc20",
                    "network": "1",
                    "contract": "0x0d8775f648430679a709e98d2b0cb6250d2887ef",
                    "decimals": 18,
                },
                "trace_credit": {
                    "kind": "nep141",
                    "network": "mainnet",
                    "contract": "trace-credit.golden.near",
                    "decimals": 6,
                },
            })
        );
        assert_eq!(
            from_value::<BundleManifest>(stored.clone()).unwrap(),
            manifest
        );

        // A manifest without pinned instruments fails to load. It does not
        // read as an empty set.
        let mut missing = stored.clone();
        missing.as_object_mut().unwrap().remove("instruments");
        assert!(from_value::<BundleManifest>(missing).is_err());

        // JSON text can repeat a key. The loader refuses it.
        let text = serde_json::to_string(&stored).unwrap();
        let bat = serde_json::to_string(&stored["instruments"]["bat"]).unwrap();
        let repeated = text.replacen(
            "\"instruments\":{",
            &format!("\"instruments\":{{\"bat\":{bat},"),
            1,
        );
        assert_ne!(repeated, text);
        assert!(from_str::<BundleManifest>(&repeated).is_err());
    }

    #[test]
    fn awards_for_unpinned_instruments_are_refused() {
        let manifest = golden_manifest();
        assert_eq!(
            manifest.instrument(&InstrumentId::trace_credit()),
            Some(&trace_credit_descriptor())
        );
        assert_eq!(
            manifest.instrument(&InstrumentId::new("storage_rebate").unwrap()),
            None
        );

        let pinned =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("bat", 10u128.pow(18))])
                .unwrap();
        assert_eq!(manifest.require_pinned(&pinned), Ok(()));
        assert_eq!(
            manifest.require_pinned(&InstrumentAwards::default()),
            Ok(())
        );
        for unpinned in [
            vec![award("storage_rebate", 7)],
            vec![award("trace_credit", 3), award("storage_rebate", 7)],
        ] {
            assert_eq!(
                manifest.require_pinned(&InstrumentAwards::new(unpinned).unwrap()),
                Err(ContractError::UnpinnedInstrument)
            );
        }
    }
}
