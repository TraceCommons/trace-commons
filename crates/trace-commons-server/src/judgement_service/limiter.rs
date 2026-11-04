// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The account-bound spend limiter (spec section 6).
//!
//! # Keys
//!
//! The binary derives a 32-byte root with `GetKey(path =
//! "judgement-limiter")` (slice 5). [`LimiterKeys::from_root`] expands it
//! with HKDF-SHA256 into the `account-digest` HMAC key and the `ledger-seal`
//! AEAD key. The root is bound to the app id and released only to
//! measurements the app's KMS admits, so an in-place upgrade keeps every
//! account's spend, and a new app id starts a fresh, deliberately unreadable
//! ledger.
//!
//! The limiter keys on `HMAC-SHA256(account-digest, "insights-judgement/v1"
//! || account_ref)`, never the account. The raw `account_ref` lives only in
//! request-scoped memory.
//!
//! # Metering spend, not calls
//!
//! Units are integer nano-USD, the catalog's `scale: 9`. Before each model
//! call the service **reserves** `(request_bytes + 512) * input_price +
//! max_tokens * output_price` against the account and the operator total,
//! checked and added in one step under one lock; a reservation that would
//! pass either cap is refused and no model call is made.
//!
//! That figure is an upper bound under one stated assumption: a byte-level
//! BPE tokenizer emits at most one token per byte of request text, and
//! [`CHAT_TEMPLATE_ALLOWANCE_TOKENS`] covers the chat template the provider
//! adds on top. `max_tokens` bounds every completion token, reasoning
//! included. If a model ever reports more than its reservation, the bound
//! did not hold for it: [`Limiter::settle`] charges the reported figure in
//! full and says so with [`Settled::OverReservation`], and the caller
//! latches that model (see [`super::receipt_check`]).
//!
//! After the call the caller **settles**: to the verified usage when the
//! receipt verified, to the larger of the reservation and the reported
//! usage when it did not, and to the reservation when no usage came back. A
//! crash between reserve and settle leaves the reservation charged: every
//! failure errs toward over-counting. Arithmetic is checked; an overflow
//! refuses, it never wraps or saturates into a pass.
//!
//! The ledger month follows the verified claim's `iat`, not the service
//! clock; see [`roll_month`].
//!
//! Every reserve and every settle is written to the sealed ledger before
//! the call goes on. A write that fails rolls the in-memory change back
//! toward the larger charge. At reserve that refuses with
//! `judgement_saturated` and no call is made; at settle the call was already
//! made and billed, so it refuses with `judgement_service_fault`, which is
//! not retried.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Datelike, Utc};
use trace_commons_protocol::insights_judgement::{JudgementRefusal, SpendBucket};
use zeroize::Zeroizing;

use super::catalog::TokenPrices;
use super::ledger::{LedgerState, LedgerStore};
use super::model::TokenUsage;

/// Tokens reserved on top of one token per request byte, for the chat
/// template the provider wraps around the request's text (role markers,
/// the system prompt's framing, the structured-output instructions).
pub const CHAT_TEMPLATE_ALLOWANCE_TOKENS: u64 = 512;

/// HKDF info for the account-digest HMAC key.
pub const ACCOUNT_DIGEST_INFO: &[u8] = b"account-digest";
/// HKDF info for the ledger AEAD key.
pub const LEDGER_SEAL_INFO: &[u8] = b"ledger-seal";
/// The HMAC message prefix of the keyed account digest.
pub const ACCOUNT_DIGEST_PREFIX: &[u8] = b"insights-judgement/v1";
/// Nano-USD in one USD.
pub const NANOUSD_PER_USD: u64 = 1_000_000_000;
/// Environment variable carrying the per-account monthly cap, whole USD.
pub const ACCOUNT_MONTHLY_CAP_ENV: &str = "TRACE_COMMONS_JUDGEMENT_ACCOUNT_MONTHLY_CAP_USD";
/// Environment variable carrying the operator-wide monthly budget, whole USD.
pub const OPERATOR_MONTHLY_BUDGET_ENV: &str = "TRACE_COMMONS_JUDGEMENT_OPERATOR_MONTHLY_BUDGET_USD";

/// The two keys the limiter root expands into.
pub struct LimiterKeys {
    account_digest: Zeroizing<[u8; 32]>,
    ledger_seal: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for LimiterKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LimiterKeys(..)")
    }
}

impl LimiterKeys {
    /// HKDF-SHA256 over the enclave-derived root, no salt, one info string
    /// per key.
    pub fn from_root(root: &[u8; 32]) -> Self {
        let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(None, root);
        let mut account_digest = Zeroizing::new([0u8; 32]);
        let mut ledger_seal = Zeroizing::new([0u8; 32]);
        hkdf.expand(ACCOUNT_DIGEST_INFO, account_digest.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        hkdf.expand(LEDGER_SEAL_INFO, ledger_seal.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self {
            account_digest,
            ledger_seal,
        }
    }

    /// The keyed account digest the limiter keys on.
    pub fn account_key(&self, account_ref: &str) -> AccountKey {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, self.account_digest.as_slice());
        let mut context = ring::hmac::Context::with_key(&key);
        context.update(ACCOUNT_DIGEST_PREFIX);
        context.update(account_ref.as_bytes());
        let tag = context.sign();
        AccountKey(tag.as_ref().try_into().expect("HMAC-SHA256 is 32 bytes"))
    }

    /// The ledger's AEAD key, for [`super::ledger::SealedLedger::open`].
    pub fn ledger_seal_key(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(*self.ledger_seal)
    }
}

/// A keyed account digest. Never logged, never hashed into a log: that would
/// link one account's calls across time.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountKey([u8; 32]);

impl AccountKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for AccountKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountKey(..)")
    }
}

/// The two monthly caps, nano-USD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimiterCaps {
    account_nanousd: u64,
    operator_nanousd: u64,
}

impl LimiterCaps {
    /// From whole USD, as the measured compose environment carries them.
    /// `None` on overflow. A zero cap is allowed: it is the roll-back
    /// switch, turning every call into a capacity refusal.
    pub fn from_usd(account_usd: u64, operator_usd: u64) -> Option<Self> {
        Some(Self {
            account_nanousd: account_usd.checked_mul(NANOUSD_PER_USD)?,
            operator_nanousd: operator_usd.checked_mul(NANOUSD_PER_USD)?,
        })
    }

    pub fn account_nanousd(&self) -> u64 {
        self.account_nanousd
    }

    pub fn operator_nanousd(&self) -> u64 {
        self.operator_nanousd
    }
}

/// The two clocks a reservation is made under: the verified claim's
/// issuer-signed `iat`, which the ledger month follows, and the service's
/// own `now`, which the claim check has already bounded to within 60 s of
/// the issuer's times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimTime {
    pub iat: i64,
    pub now: i64,
}

impl ClaimTime {
    /// Both clocks at one instant. Tests and the drill.
    pub fn at(instant: i64) -> Self {
        Self {
            iat: instant,
            now: instant,
        }
    }
}

/// The UTC month of `now` as `year * 12 + (month - 1)`. `None` outside
/// chrono's range or before year 0.
pub fn month_index(now: i64) -> Option<u32> {
    let now = DateTime::<Utc>::from_timestamp(now, 0)?;
    let year = u32::try_from(now.year()).ok()?;
    year.checked_mul(12)?.checked_add(now.month0())
}

/// The reservation for one call: `(request_bytes +
/// CHAT_TEMPLATE_ALLOWANCE_TOKENS) * input_price + max_tokens *
/// output_price`, one input token per request byte. `None` on overflow.
pub fn reservation_nanousd(
    request_bytes: usize,
    max_tokens: u32,
    prices: TokenPrices,
) -> Option<u64> {
    let input_tokens = u64::try_from(request_bytes)
        .ok()?
        .checked_add(CHAT_TEMPLATE_ALLOWANCE_TOKENS)?;
    input_tokens
        .checked_mul(prices.input_nanousd)?
        .checked_add(u64::from(max_tokens).checked_mul(prices.output_nanousd)?)
}

/// The settled charge: `prompt_tokens * input_price + completion_tokens *
/// output_price`. `None` on overflow.
pub fn charge_nanousd(usage: TokenUsage, prices: TokenPrices) -> Option<u64> {
    usage
        .prompt_tokens
        .checked_mul(prices.input_nanousd)?
        .checked_add(usage.completion_tokens.checked_mul(prices.output_nanousd)?)
}

/// How a settle came out against its reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The charge was at or below the reservation (or none was reported).
    WithinReservation,
    /// The charge was above the reservation and was charged in full. The
    /// reservation's bound did not hold for this model.
    OverReservation,
}

/// A reserved amount, charged until settled. Dropping one without settling
/// leaves the charge, which is the crash rule.
#[must_use = "a reservation is settled after the call, or stands as the charge"]
#[derive(Debug)]
pub struct Reservation {
    account: AccountKey,
    amount_nanousd: u64,
    month: u32,
}

impl Reservation {
    pub fn amount_nanousd(&self) -> u64 {
        self.amount_nanousd
    }
}

/// The limiter. One per service; the ledger is its only state.
pub struct Limiter {
    caps: LimiterCaps,
    state: Mutex<LedgerState>,
    store: Arc<dyn LedgerStore>,
}

impl std::fmt::Debug for Limiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limiter")
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

impl Limiter {
    /// A limiter over the state read back from the ledger.
    pub fn new(caps: LimiterCaps, state: LedgerState, store: Arc<dyn LedgerStore>) -> Self {
        Self {
            caps,
            state: Mutex::new(state),
            store,
        }
    }

    /// Reserve `amount` for `account` at `now`.
    ///
    /// Refuses with [`JudgementRefusal::AccountBudgetExhausted`] or
    /// [`JudgementRefusal::OperatorBudgetExhausted`] when the reservation
    /// would pass that cap (the account cap is checked first), and with
    /// [`JudgementRefusal::Saturated`] when the ledger cannot be written.
    pub fn reserve(
        &self,
        account: &AccountKey,
        amount_nanousd: u64,
        time: ClaimTime,
    ) -> Result<Reservation, JudgementRefusal> {
        // A verified claim's iat is always a representable month. One that is
        // not means the clocks are broken: an internal fault, never a budget
        // the account or the operator could be told to wait for.
        let iat_month = month_index(time.iat).ok_or(JudgementRefusal::ServiceFault)?;
        let now_month = month_index(time.now);
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let before = state.clone();
        roll_month(&mut state, iat_month, now_month);
        let month = state.month;
        // What a cap refusal leaves behind: the rolled month, uncharged, so
        // `reset_at` names it. Not written: nothing was spent.
        let rolled = state.clone();

        let account_spent = state.accounts.get(account.as_bytes()).copied().unwrap_or(0);
        let account_after = account_spent
            .checked_add(amount_nanousd)
            .filter(|after| *after <= self.caps.account_nanousd)
            .ok_or(JudgementRefusal::AccountBudgetExhausted);
        let account_after = match account_after {
            Ok(after) => after,
            Err(refusal) => {
                *state = rolled;
                return Err(refusal);
            }
        };
        let operator_after = match state
            .operator_spent_nanousd
            .checked_add(amount_nanousd)
            .filter(|after| *after <= self.caps.operator_nanousd)
        {
            Some(after) => after,
            None => {
                *state = rolled;
                return Err(JudgementRefusal::OperatorBudgetExhausted);
            }
        };
        state.accounts.insert(*account.as_bytes(), account_after);
        state.operator_spent_nanousd = operator_after;
        if self.store.persist(&state).is_err() {
            // Not recorded, so not spent: the call is refused and never made.
            *state = before;
            return Err(JudgementRefusal::Saturated);
        }
        Ok(Reservation {
            account: *account,
            amount_nanousd,
            month,
        })
    }

    /// Settle a reservation to `charge_nanousd`. `None` leaves the
    /// reservation as the charge. A reservation from a month that has since
    /// rolled over is dropped with that month.
    ///
    /// A charge above the reservation is charged in full, never capped to
    /// it, and comes back as [`Settled::OverReservation`]. A ledger write
    /// that fails refuses with [`JudgementRefusal::ServiceFault`]: the call
    /// was already made and billed, so it is not a capacity refusal.
    pub fn settle(
        &self,
        reservation: Reservation,
        charge_nanousd: Option<u64>,
    ) -> Result<Settled, JudgementRefusal> {
        let Some(charge) = charge_nanousd else {
            // The reservation stands, and it is already on disk.
            return Ok(Settled::WithinReservation);
        };
        let outcome = if charge > reservation.amount_nanousd {
            Settled::OverReservation
        } else {
            Settled::WithinReservation
        };
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.month != reservation.month {
            // That month was dropped at the rollover, reservation and all.
            return Ok(outcome);
        }
        let before = state.clone();
        // Settling only ever moves a figure the reservation already put on
        // the books, so saturating arithmetic errs high, never toward a pass.
        let settle = |spent: u64| {
            spent
                .saturating_sub(reservation.amount_nanousd)
                .saturating_add(charge)
        };
        let account_spent = state
            .accounts
            .get(reservation.account.as_bytes())
            .copied()
            .unwrap_or(0);
        state
            .accounts
            .insert(*reservation.account.as_bytes(), settle(account_spent));
        state.operator_spent_nanousd = settle(state.operator_spent_nanousd);
        if self.store.persist(&state).is_err() {
            // The settled figure was not recorded. Keep the larger of the
            // two in memory, so a failure never under-counts.
            if charge < reservation.amount_nanousd {
                *state = before;
            }
            return Err(JudgementRefusal::ServiceFault);
        }
        Ok(outcome)
    }

    /// The account's remaining monthly budget, as the coarse bucket the
    /// response carries: `over_half` when more than half remains,
    /// `under_tenth` when less than a tenth remains, `under_half` otherwise.
    pub fn remaining_bucket(&self, account: &AccountKey, time: ClaimTime) -> SpendBucket {
        // The month the reservations were made against, not the CVM's.
        let now = time.iat;
        let cap = u128::from(self.caps.account_nanousd);
        let remaining = u128::from(
            self.caps
                .account_nanousd
                .saturating_sub(self.account_spent_nanousd(account, now)),
        );
        if remaining * 2 > cap {
            SpendBucket::OverHalf
        } else if remaining * 10 < cap || cap == 0 {
            SpendBucket::UnderTenth
        } else {
            SpendBucket::UnderHalf
        }
    }

    /// The ledger's month, `year * 12 + (month - 1)`: the month the last
    /// reservation was made against. 0 before any.
    pub fn ledger_month(&self) -> u32 {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).month
    }

    /// The `reset_at` of a capacity refusal: the first instant of the month
    /// after the ledger's month. `None` before any reservation.
    pub fn reset_at(&self) -> Option<i64> {
        let month = self.ledger_month();
        if month == 0 {
            return None;
        }
        let start =
            chrono::NaiveDate::from_ymd_opt(i32::try_from(month / 12).ok()?, month % 12 + 1, 1)?
                .and_hms_opt(0, 0, 0)?
                .and_utc()
                .timestamp();
        trace_commons_protocol::insights_judgement::judgement_budget_reset_at(start)
    }

    /// What the account has spent this month. For tests and the drill.
    pub fn account_spent_nanousd(&self, account: &AccountKey, now: i64) -> u64 {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if is_behind(&state, now) {
            return 0;
        }
        state.accounts.get(account.as_bytes()).copied().unwrap_or(0)
    }

    /// What the operator has spent this month.
    pub fn operator_spent_nanousd(&self, now: i64) -> u64 {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if is_behind(&state, now) {
            return 0;
        }
        state.operator_spent_nanousd
    }
}

/// Move the ledger to the verified claim's month, dropping every figure from
/// before it (spec section 6, "The ledger month follows a signed clock").
///
/// - It only rolls forward: a clock that steps back across the 1st keeps
///   charging the later month rather than reopening an empty one.
/// - It moves only to `iat`'s month, never to the service clock's, so a
///   CVM clock reading in the future cannot persist that month.
/// - It moves by one month on `iat` alone. A larger jump (a long idle
///   period, or a fresh ledger) also needs the service clock in that same
///   month, so neither clock alone can skip the ledger ahead. When the two
///   disagree the ledger stays where it is and keeps charging that month,
///   which errs toward over-counting.
fn roll_month(state: &mut LedgerState, iat_month: u32, now_month: Option<u32>) {
    let next = state.month.saturating_add(1);
    let advance = iat_month == next || (iat_month > next && now_month == Some(iat_month));
    if advance {
        *state = LedgerState {
            month: iat_month,
            ..LedgerState::default()
        };
    }
}

/// Whether the ledger's month is before `now`'s, so its figures are last
/// month's and nothing is spent yet this month. An unrepresentable `now` is
/// not behind: the figures stand.
fn is_behind(state: &LedgerState, now: i64) -> bool {
    month_index(now).is_some_and(|month| month > state.month)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::judgement_service::ledger::SealedLedger;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// 2026-10-20T00:00:00Z.
    pub(crate) const NOW: i64 = 1_792_454_400;
    /// 2026-11-01T00:00:00Z.
    const NEXT_MONTH: i64 = 1_793_491_200;

    /// A store that records writes and can be told to fail.
    #[derive(Default)]
    pub(crate) struct MemoryStore {
        pub(crate) fail: AtomicBool,
        pub(crate) writes: AtomicUsize,
        pub(crate) last: Mutex<Option<LedgerState>>,
    }

    impl LedgerStore for MemoryStore {
        fn persist(&self, state: &LedgerState) -> std::io::Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("injected"));
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().expect("lock") = Some(state.clone());
            Ok(())
        }
    }

    fn keys() -> LimiterKeys {
        LimiterKeys::from_root(&[9u8; 32])
    }

    /// $5 per account, $100 operator-wide: the owner's figures.
    fn caps() -> LimiterCaps {
        LimiterCaps::from_usd(5, 100).expect("caps")
    }

    fn limiter(store: Arc<MemoryStore>) -> Limiter {
        Limiter::new(caps(), LedgerState::default(), store)
    }

    #[test]
    fn caps_are_nano_usd_and_overflow_is_refused() {
        let caps = caps();
        assert_eq!(caps.account_nanousd(), 5_000_000_000);
        assert_eq!(caps.operator_nanousd(), 100_000_000_000);
        assert_eq!(LimiterCaps::from_usd(u64::MAX, 1), None);
    }

    #[test]
    fn the_account_key_is_keyed_distinct_and_hides_the_account() {
        let a = keys().account_key("near-a");
        assert_eq!(a, keys().account_key("near-a"));
        assert_ne!(a, keys().account_key("near-b"));
        assert_ne!(a, LimiterKeys::from_root(&[8u8; 32]).account_key("near-a"));
        assert_eq!(format!("{a:?}"), "AccountKey(..)");
        // Not a bare hash of the account: the key matters.
        use sha2::Digest as _;
        assert_ne!(
            a.as_bytes().as_slice(),
            sha2::Sha256::digest(b"insights-judgement/v1near-a").as_slice()
        );
    }

    #[test]
    fn the_two_derived_keys_differ_from_each_other_and_the_root() {
        let keys = keys();
        assert_ne!(*keys.account_digest, *keys.ledger_seal);
        assert_ne!(*keys.ledger_seal, [9u8; 32]);
    }

    #[test]
    fn reservation_and_charge_follow_the_spec_formulas() {
        let prices = TokenPrices {
            input_nanousd: 40,
            output_nanousd: 1350,
        };
        // One input token per request byte, plus the chat-template
        // allowance, plus every completion token.
        assert_eq!(CHAT_TEMPLATE_ALLOWANCE_TOKENS, 512);
        assert_eq!(
            reservation_nanousd(9000, 64, prices),
            Some((9000 + 512) * 40 + 64 * 1350)
        );
        assert_eq!(reservation_nanousd(1, 0, prices), Some(513 * 40));
        assert_eq!(reservation_nanousd(0, 0, prices), Some(512 * 40));
        assert_eq!(
            charge_nanousd(
                TokenUsage {
                    prompt_tokens: 11,
                    completion_tokens: 16
                },
                prices
            ),
            Some(11 * 40 + 16 * 1350)
        );
        let huge = TokenPrices {
            input_nanousd: u64::MAX,
            output_nanousd: u64::MAX,
        };
        assert_eq!(reservation_nanousd(4, 1, huge), None);
        assert_eq!(
            charge_nanousd(
                TokenUsage {
                    prompt_tokens: 2,
                    completion_tokens: 0
                },
                huge
            ),
            None
        );
    }

    #[test]
    fn reserve_then_settle_charges_the_reported_usage() {
        let store = Arc::new(MemoryStore::default());
        let limiter = limiter(store.clone());
        let account = keys().account_key("near-a");
        let reservation = limiter
            .reserve(&account, 1_000, ClaimTime::at(NOW))
            .expect("reserves");
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 1_000);
        assert_eq!(limiter.operator_spent_nanousd(NOW), 1_000);
        limiter.settle(reservation, Some(300)).expect("settles");
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 300);
        assert_eq!(limiter.operator_spent_nanousd(NOW), 300);
        // Written after the reserve and after the settle.
        assert_eq!(store.writes.load(Ordering::SeqCst), 2);
        let persisted = store.last.lock().expect("lock").clone().expect("written");
        assert_eq!(persisted.accounts[account.as_bytes()], 300);
    }

    #[test]
    fn a_charge_above_the_reservation_is_charged_in_full() {
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        let reservation = limiter
            .reserve(&account, 100, ClaimTime::at(NOW))
            .expect("reserves");
        assert_eq!(
            limiter.settle(reservation, Some(250)).expect("settles"),
            Settled::OverReservation
        );
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 250);
        assert_eq!(limiter.operator_spent_nanousd(NOW), 250);
        // At or below the reservation is within it.
        let reservation = limiter
            .reserve(&account, 100, ClaimTime::at(NOW))
            .expect("reserves");
        assert_eq!(
            limiter.settle(reservation, Some(100)).expect("settles"),
            Settled::WithinReservation
        );
        let reservation = limiter
            .reserve(&account, 100, ClaimTime::at(NOW))
            .expect("reserves");
        assert_eq!(
            limiter.settle(reservation, None).expect("settles"),
            Settled::WithinReservation
        );
    }

    #[test]
    fn a_charge_above_the_reservation_can_pass_the_cap_and_the_next_reserve_sees_it() {
        // The reported figure is charged even past the cap: it was billed.
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        let cap = caps().account_nanousd();
        let reservation = limiter
            .reserve(&account, cap - 10, ClaimTime::at(NOW))
            .expect("reserves");
        assert_eq!(
            limiter.settle(reservation, Some(cap + 5)).expect("settles"),
            Settled::OverReservation
        );
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), cap + 5);
        assert_eq!(
            limiter
                .reserve(&account, 1, ClaimTime::at(NOW))
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
    }

    /// `threads` reservations of `amount` each, started together, for the
    /// accounts `account_of(i)` names. Returns how many were admitted and
    /// every refusal.
    fn race(
        limiter: &Limiter,
        threads: usize,
        amount: u64,
        account_of: impl Fn(usize) -> AccountKey + Sync,
    ) -> (usize, Vec<JudgementRefusal>) {
        let barrier = std::sync::Barrier::new(threads);
        let outcomes: Vec<Result<Reservation, JudgementRefusal>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|i| {
                    let barrier = &barrier;
                    let account_of = &account_of;
                    scope.spawn(move || {
                        let account = account_of(i);
                        barrier.wait();
                        limiter.reserve(&account, amount, ClaimTime::at(NOW))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("thread"))
                .collect()
        });
        let mut admitted = 0;
        let mut refusals = Vec::new();
        for outcome in outcomes {
            match outcome {
                // Left standing as the charge, as a crash would.
                Ok(reservation) => {
                    admitted += 1;
                    drop(reservation);
                }
                Err(refusal) => refusals.push(refusal),
            }
        }
        (admitted, refusals)
    }

    #[test]
    fn concurrent_reservations_on_one_account_admit_exactly_what_fits() {
        // $5 fits exactly 5 reservations of $1; 16 race for it.
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        let (admitted, refusals) = race(&limiter, 16, NANOUSD_PER_USD, |_| account);
        assert_eq!(admitted, 5);
        assert_eq!(refusals.len(), 11);
        assert!(
            refusals
                .iter()
                .all(|r| *r == JudgementRefusal::AccountBudgetExhausted)
        );
        assert_eq!(
            limiter.account_spent_nanousd(&account, NOW),
            caps().account_nanousd()
        );
    }

    #[test]
    fn two_devices_on_one_account_cannot_both_take_the_last_of_it() {
        // Each device holds the same account, so both reserve against one
        // entry. Only one of them fits in what is left.
        for _ in 0..50 {
            let limiter = limiter(Arc::new(MemoryStore::default()));
            let account = keys().account_key("near-a");
            let first = limiter
                .reserve(&account, 4 * NANOUSD_PER_USD, ClaimTime::at(NOW))
                .expect("reserves");
            limiter.settle(first, None).expect("settles");
            let device_a = keys().account_key("near-a");
            let device_b = keys().account_key("near-a");
            let (admitted, refusals) = race(&limiter, 2, NANOUSD_PER_USD, |i| {
                if i == 0 { device_a } else { device_b }
            });
            assert_eq!(admitted, 1);
            assert_eq!(refusals, vec![JudgementRefusal::AccountBudgetExhausted]);
            assert_eq!(
                limiter.account_spent_nanousd(&account, NOW),
                caps().account_nanousd()
            );
        }
    }

    #[test]
    fn concurrent_reservations_across_accounts_admit_exactly_the_operator_budget() {
        // $7 operator-wide, $5 per account: 20 accounts race with $1 each,
        // and exactly 7 are admitted.
        let limiter = Limiter::new(
            LimiterCaps::from_usd(5, 7).expect("caps"),
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        );
        let accounts: Vec<AccountKey> = (0..20)
            .map(|i| keys().account_key(&format!("near-{i}")))
            .collect();
        let (admitted, refusals) = race(&limiter, 20, NANOUSD_PER_USD, |i| accounts[i]);
        assert_eq!(admitted, 7);
        assert_eq!(refusals.len(), 13);
        assert!(
            refusals
                .iter()
                .all(|r| *r == JudgementRefusal::OperatorBudgetExhausted)
        );
        assert_eq!(limiter.operator_spent_nanousd(NOW), 7 * NANOUSD_PER_USD);
    }

    #[test]
    fn absent_usage_leaves_the_reservation_as_the_charge() {
        let store = Arc::new(MemoryStore::default());
        let limiter = limiter(store.clone());
        let account = keys().account_key("near-a");
        let reservation = limiter
            .reserve(&account, 1_000, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(reservation, None).expect("settles");
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 1_000);
    }

    #[test]
    fn a_crash_between_reserve_and_settle_leaves_the_reservation_charged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let account = keys().account_key("near-a");
        {
            let (ledger, state) =
                SealedLedger::open(dir.path(), keys().ledger_seal_key()).expect("opens");
            let limiter = Limiter::new(caps(), state, Arc::new(ledger));
            let reservation = limiter
                .reserve(&account, 7_000, ClaimTime::at(NOW))
                .expect("reserves");
            // The process dies here: the reservation is never settled, and
            // the limiter goes with it.
            drop(reservation);
            drop(limiter);
        }
        let (ledger, state) =
            SealedLedger::open(dir.path(), keys().ledger_seal_key()).expect("reopens");
        let limiter = Limiter::new(caps(), state, Arc::new(ledger));
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 7_000);
        assert_eq!(limiter.operator_spent_nanousd(NOW), 7_000);
    }

    #[test]
    fn the_account_cap_refuses_a_reservation_that_would_pass_it() {
        let store = Arc::new(MemoryStore::default());
        let limiter = limiter(store.clone());
        let account = keys().account_key("near-a");
        let cap = caps().account_nanousd();
        let reservation = limiter
            .reserve(&account, cap - 10, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(reservation, None).expect("settles");
        // Exactly at the cap is allowed.
        let reservation = limiter
            .reserve(&account, 10, ClaimTime::at(NOW))
            .expect("reaches the cap");
        limiter.settle(reservation, None).expect("settles");
        let writes = store.writes.load(Ordering::SeqCst);
        assert_eq!(
            limiter
                .reserve(&account, 1, ClaimTime::at(NOW))
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
        // A refusal writes nothing and changes nothing.
        assert_eq!(store.writes.load(Ordering::SeqCst), writes);
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), cap);
        // Another account is unaffected.
        assert!(
            limiter
                .reserve(&keys().account_key("near-b"), 1, ClaimTime::at(NOW))
                .is_ok()
        );
    }

    #[test]
    fn the_operator_budget_refuses_every_account_once_spent() {
        let limiter = Limiter::new(
            LimiterCaps::from_usd(5, 7).expect("caps"),
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        );
        let first = keys().account_key("near-a");
        let second = keys().account_key("near-b");
        let five = 5 * NANOUSD_PER_USD;
        let r = limiter
            .reserve(&first, five, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        let r = limiter
            .reserve(&second, 2 * NANOUSD_PER_USD, ClaimTime::at(NOW))
            .expect("reaches the budget");
        limiter.settle(r, None).expect("settles");
        let third = keys().account_key("near-c");
        assert_eq!(
            limiter.reserve(&third, 1, ClaimTime::at(NOW)).unwrap_err(),
            JudgementRefusal::OperatorBudgetExhausted
        );
        // The account cap is checked first.
        assert_eq!(
            limiter.reserve(&first, 1, ClaimTime::at(NOW)).unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
    }

    #[test]
    fn a_zero_cap_refuses_every_call() {
        let limiter = Limiter::new(
            LimiterCaps::from_usd(0, 100).expect("caps"),
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        );
        assert_eq!(
            limiter
                .reserve(&keys().account_key("near-a"), 1, ClaimTime::at(NOW))
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
    }

    #[test]
    fn an_overflowing_reservation_is_refused_not_wrapped() {
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        let r = limiter
            .reserve(&account, 10, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        assert_eq!(
            limiter
                .reserve(&account, u64::MAX, ClaimTime::at(NOW))
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 10);
    }

    #[test]
    fn a_month_rollover_drops_last_months_spend() {
        let store = Arc::new(MemoryStore::default());
        let limiter = limiter(store.clone());
        let account = keys().account_key("near-a");
        let cap = caps().account_nanousd();
        let r = limiter
            .reserve(&account, cap, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        assert_eq!(
            limiter
                .reserve(&account, 1, ClaimTime::at(NOW))
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
        assert_eq!(limiter.account_spent_nanousd(&account, NEXT_MONTH), 0);
        let r = limiter
            .reserve(&account, 1, ClaimTime::at(NEXT_MONTH))
            .expect("a new month");
        limiter.settle(r, None).expect("settles");
        let persisted = store.last.lock().expect("lock").clone().expect("written");
        assert_eq!(persisted.month, month_index(NEXT_MONTH).expect("month"));
        assert_eq!(persisted.accounts.len(), 1);
        assert_eq!(persisted.operator_spent_nanousd, 1);
    }

    #[test]
    fn a_reservation_settled_after_a_rollover_is_dropped_with_its_month() {
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        let old = limiter
            .reserve(&account, 500, ClaimTime::at(NEXT_MONTH - 1))
            .expect("reserves");
        let new = limiter
            .reserve(&account, 7, ClaimTime::at(NEXT_MONTH))
            .expect("reserves");
        limiter.settle(old, Some(1)).expect("settles");
        limiter.settle(new, None).expect("settles");
        assert_eq!(limiter.account_spent_nanousd(&account, NEXT_MONTH), 7);
    }

    #[test]
    fn a_ledger_write_failure_refuses_and_errs_toward_the_larger_charge() {
        let store = Arc::new(MemoryStore::default());
        let limiter = limiter(store.clone());
        let account = keys().account_key("near-a");
        store.fail.store(true, Ordering::SeqCst);
        assert_eq!(
            limiter
                .reserve(&account, 100, ClaimTime::at(NOW))
                .unwrap_err(),
            JudgementRefusal::Saturated
        );
        // A refused reservation is not charged in memory either.
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 0);

        store.fail.store(false, Ordering::SeqCst);
        let r = limiter
            .reserve(&account, 100, ClaimTime::at(NOW))
            .expect("reserves");
        store.fail.store(true, Ordering::SeqCst);
        // The call was already made and billed: a fault, not capacity.
        assert_eq!(
            limiter.settle(r, Some(10)).unwrap_err(),
            JudgementRefusal::ServiceFault
        );
        // The lower settled charge was not recorded, so it is not believed.
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 100);
    }

    #[test]
    fn the_spend_bucket_is_coarse() {
        let limiter = Limiter::new(
            LimiterCaps::from_usd(10, 100).expect("caps"),
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        );
        let account = keys().account_key("near-a");
        assert_eq!(
            limiter.remaining_bucket(&account, ClaimTime::at(NOW)),
            SpendBucket::OverHalf
        );
        let spend = |amount: u64| {
            let r = limiter
                .reserve(&account, amount, ClaimTime::at(NOW))
                .expect("reserves");
            limiter.settle(r, None).expect("settles");
        };
        spend(4 * NANOUSD_PER_USD); // 6 of 10 remain.
        assert_eq!(
            limiter.remaining_bucket(&account, ClaimTime::at(NOW)),
            SpendBucket::OverHalf
        );
        spend(NANOUSD_PER_USD); // exactly half remains.
        assert_eq!(
            limiter.remaining_bucket(&account, ClaimTime::at(NOW)),
            SpendBucket::UnderHalf
        );
        spend(4 * NANOUSD_PER_USD); // exactly a tenth remains.
        assert_eq!(
            limiter.remaining_bucket(&account, ClaimTime::at(NOW)),
            SpendBucket::UnderHalf
        );
        spend(1); // less than a tenth.
        assert_eq!(
            limiter.remaining_bucket(&account, ClaimTime::at(NOW)),
            SpendBucket::UnderTenth
        );
        let zero = Limiter::new(
            LimiterCaps::from_usd(0, 100).expect("caps"),
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        );
        assert_eq!(
            zero.remaining_bucket(&account, ClaimTime::at(NOW)),
            SpendBucket::UnderTenth
        );
    }

    #[test]
    fn a_clock_stepping_back_a_month_never_resets_spend() {
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        let cap = caps().account_nanousd();
        let r = limiter
            .reserve(&account, cap, ClaimTime::at(NEXT_MONTH))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        // The clock steps back across the 1st: last month's ledger must not
        // come back as an empty one.
        assert_eq!(
            limiter
                .reserve(&account, 1, ClaimTime::at(NEXT_MONTH - 1))
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
        assert_eq!(limiter.account_spent_nanousd(&account, NEXT_MONTH - 1), cap);
        assert_eq!(limiter.operator_spent_nanousd(NEXT_MONTH - 1), cap);
    }

    /// 2027-10-20T00:00:00Z: a CVM clock a year ahead.
    const A_YEAR_AHEAD: i64 = NOW + 365 * 86_400;
    /// 2027-01-10T00:00:00Z.
    const JANUARY: i64 = 1_799_539_200;

    fn persisted_month(store: &MemoryStore) -> u32 {
        store
            .last
            .lock()
            .expect("lock")
            .as_ref()
            .expect("written")
            .month
    }

    #[test]
    fn a_future_clock_month_is_never_persisted() {
        let store = Arc::new(MemoryStore::default());
        let limiter = limiter(store.clone());
        let account = keys().account_key("near-a");
        let october = month_index(NOW).expect("month");
        let r = limiter
            .reserve(&account, 10, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        // The service clock reads a year ahead; the claim's iat does not.
        let r = limiter
            .reserve(
                &account,
                10,
                ClaimTime {
                    iat: NOW + 60,
                    now: A_YEAR_AHEAD,
                },
            )
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        assert_eq!(persisted_month(&store), october);
        // An iat a year ahead with a service clock that disagrees: the month
        // does not skip ahead either.
        let r = limiter
            .reserve(
                &account,
                10,
                ClaimTime {
                    iat: A_YEAR_AHEAD,
                    now: NOW,
                },
            )
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        assert_eq!(persisted_month(&store), october);
        assert_eq!(limiter.ledger_month(), october);
        // All three were charged to October.
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 30);
    }

    #[test]
    fn a_long_gap_skips_months_only_when_both_clocks_agree() {
        let store = Arc::new(MemoryStore::default());
        let limiter = limiter(store.clone());
        let account = keys().account_key("near-a");
        let r = limiter
            .reserve(&account, 10, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        // Three months on, both clocks in January: the ledger moves there.
        let r = limiter
            .reserve(&account, 7, ClaimTime::at(JANUARY))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        assert_eq!(
            persisted_month(&store),
            month_index(JANUARY).expect("month")
        );
        assert_eq!(limiter.account_spent_nanousd(&account, JANUARY), 7);
    }

    #[test]
    fn reset_at_comes_from_the_ledger_month() {
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        assert_eq!(limiter.reset_at(), None);
        let r = limiter
            .reserve(&account, 10, ClaimTime::at(NOW))
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        assert_eq!(limiter.reset_at(), Some(NEXT_MONTH));
        // A service clock a year ahead does not move it.
        let r = limiter
            .reserve(
                &account,
                10,
                ClaimTime {
                    iat: NOW,
                    now: A_YEAR_AHEAD,
                },
            )
            .expect("reserves");
        limiter.settle(r, None).expect("settles");
        assert_eq!(limiter.reset_at(), Some(NEXT_MONTH));
    }

    #[test]
    fn a_refused_first_reservation_still_names_its_month_for_reset_at() {
        let limiter = Limiter::new(
            LimiterCaps::from_usd(0, 100).expect("caps"),
            LedgerState::default(),
            Arc::new(MemoryStore::default()),
        );
        assert_eq!(
            limiter
                .reserve(&keys().account_key("near-a"), 1, ClaimTime::at(NOW))
                .unwrap_err(),
            JudgementRefusal::AccountBudgetExhausted
        );
        assert_eq!(limiter.reset_at(), Some(NEXT_MONTH));
    }

    #[test]
    fn an_unrepresentable_time_is_a_service_fault_not_a_budget() {
        let limiter = limiter(Arc::new(MemoryStore::default()));
        let account = keys().account_key("near-a");
        for time in [
            ClaimTime {
                iat: i64::MAX,
                now: NOW,
            },
            ClaimTime {
                iat: i64::MIN,
                now: NOW,
            },
        ] {
            assert_eq!(
                limiter.reserve(&account, 1, time).unwrap_err(),
                JudgementRefusal::ServiceFault
            );
        }
        assert_eq!(limiter.account_spent_nanousd(&account, NOW), 0);
    }

    #[test]
    fn month_index_is_the_utc_calendar_month() {
        assert_eq!(month_index(NOW), Some(2026 * 12 + 9));
        assert_eq!(month_index(NEXT_MONTH - 1), Some(2026 * 12 + 9));
        assert_eq!(month_index(NEXT_MONTH), Some(2026 * 12 + 10));
    }
}
