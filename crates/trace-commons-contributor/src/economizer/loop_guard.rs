//! The loop guard: one redirect for a turn that is stuck.
//!
//! A turn is the agent's work on one prompt. It counts as stuck only when
//! all of these hold, so a hard but productive turn (many edits, changing
//! failures) is left alone:
//!
//! - it has run at least [`Thresholds::min_calls`] tool calls;
//! - it churns: one file edited [`Thresholds::edit_churn`] times, or edits
//!   spread over [`Thresholds::distinct_edit_files`] files;
//! - it is noisy: [`Thresholds::failures`] failing outputs;
//! - it repeats itself: at least [`Thresholds::stagnation`] of its failures
//!   are the same failure seen earlier in the turn;
//! - it is longer than this folder's turns that ended well usually are
//!   ([`Envelope`]), which needs [`Thresholds::envelope_min_samples`] of
//!   them first.
//!
//! A stuck turn gets one redirect ([`redirect_text`]), made only of counts
//! of the agent's own actions. The guard never stops a turn and calls no
//! model.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::signals::{CallKind, ToolCall, failure_signature, patch_paths, short_digest};

/// When a turn counts as stuck. The defaults are TokenEconomizer's
/// published no-judge defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub min_calls: u32,
    pub edit_churn: u32,
    pub distinct_edit_files: usize,
    pub failures: u32,
    pub stagnation: f64,
    pub envelope_percentile: usize,
    pub envelope_min_samples: usize,
    pub redirects_per_turn: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            min_calls: 20,
            edit_churn: 4,
            distinct_edit_files: 3,
            failures: 6,
            stagnation: 0.20,
            envelope_percentile: 90,
            envelope_min_samples: 5,
            redirects_per_turn: 1,
        }
    }
}

/// Counts for one turn, folded one call at a time. Files and failures are
/// held as digests.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnState {
    pub calls: u32,
    /// Edits per file, by a digest of the file's path.
    pub edits: BTreeMap<String, u32>,
    pub failures: u32,
    pub checks: u32,
    /// Failing outputs per failure signature.
    pub failure_signatures: BTreeMap<String, u32>,
    /// Failing outputs whose signature was already seen in this turn.
    pub repeated_failures: u32,
    pub redirects: u32,
    /// Whether the latest check since the turn's last edit failed. A turn
    /// that ends with this set did not end well.
    pub failed_check_since_edit: bool,
}

impl TurnState {
    /// Fold one finished call into the turn.
    pub fn record(&mut self, call: &ToolCall) {
        self.calls += 1;
        let edited: Vec<String> = match &call.kind {
            CallKind::Edit { paths } if !call.failed => paths.clone(),
            // Codex edits through a patch inside a shell command.
            CallKind::Command { command } => patch_paths(command),
            _ => Vec::new(),
        };
        for path in edited.iter().filter(|path| !path.is_empty()) {
            let key = short_digest("file", path);
            *self.edits.entry(key).or_default() += 1;
        }
        if !edited.is_empty() {
            self.failed_check_since_edit = false;
        }
        let failure = call.is_failure();
        // The latest check counts: a failed one that a later check passes
        // no longer marks the turn.
        if call.is_check() {
            self.failed_check_since_edit = failure;
        }
        if call.is_check() {
            self.checks += 1;
        }
        if failure {
            self.failures += 1;
            if let Some(signature) = failure_signature(&call.output) {
                let seen = self.failure_signatures.entry(signature).or_default();
                if *seen > 0 {
                    self.repeated_failures += 1;
                }
                *seen += 1;
            }
        }
    }

    pub fn edit_churn(&self) -> u32 {
        self.edits.values().copied().max().unwrap_or(0)
    }

    pub fn edit_count(&self) -> u32 {
        self.edits.values().sum()
    }

    /// Share of signed failures that repeat an earlier one in this turn.
    pub fn stagnation(&self) -> f64 {
        let total: u32 = self.failure_signatures.values().sum();
        if total == 0 {
            0.0
        } else {
            f64::from(self.repeated_failures) / f64::from(total)
        }
    }

    /// The strict stuck test, without the envelope.
    pub fn looks_stuck(&self, thresholds: &Thresholds) -> bool {
        self.calls >= thresholds.min_calls
            && (self.edit_churn() >= thresholds.edit_churn
                || self.edits.len() >= thresholds.distinct_edit_files)
            && self.failures >= thresholds.failures
            && self.stagnation() >= thresholds.stagnation
    }

    /// Whether the turn ended well: it made calls, and its latest check
    /// since its last edit, if any, passed. Only such turns teach the
    /// envelope.
    pub fn ended_well(&self) -> bool {
        self.calls > 0 && !self.failed_check_since_edit
    }
}

/// How long this folder's turns that ended well run, in tool calls. Keeps
/// the most recent [`Envelope::KEEP`] samples.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub samples: Vec<u32>,
}

impl Envelope {
    pub const KEEP: usize = 50;

    pub fn add(&mut self, calls: u32) {
        self.samples.push(calls);
        if self.samples.len() > Self::KEEP {
            let extra = self.samples.len() - Self::KEEP;
            self.samples.drain(..extra);
        }
    }

    /// The configured percentile of the samples (nearest rank), once there
    /// are enough of them.
    pub fn limit(&self, thresholds: &Thresholds) -> Option<u32> {
        if self.samples.len() < thresholds.envelope_min_samples.max(1) {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let percentile = thresholds.envelope_percentile.clamp(1, 100);
        let rank = (percentile * sorted.len()).div_ceil(100).max(1);
        Some(sorted[rank - 1])
    }
}

/// A judge's answer on whether a turn is stuck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Stuck { confidence: f64 },
    Productive,
    Unsure,
}

/// A model that reads a turn's counts and says whether it is stuck. None is
/// wired yet: the guard runs on the rules alone. A judge must be at least
/// as private as the session it judges (its NEAR AI route, or an attested
/// judgement service), and answer within the hook's budget.
pub trait StuckJudge {
    /// `None` when it could not answer; the turn is then left alone.
    fn judge(&self, turn: &TurnState, envelope: Option<u32>) -> Option<Verdict>;
}

/// The least confidence a judge's "stuck" needs.
pub const JUDGE_CONFIDENCE: f64 = 0.7;

/// Whether a turn shows enough trouble to be worth a judge's look: it has
/// run a while, and a failure repeated or failures piled up. Cheaper and
/// looser than [`TurnState::looks_stuck`].
pub fn worth_judging(turn: &TurnState) -> bool {
    turn.calls >= 18 && (turn.repeated_failures >= 1 || turn.failures >= 8)
}

/// Whether the turn should get a redirect now, on the rules alone. Counts
/// the redirect when it says yes.
pub fn should_redirect(turn: &mut TurnState, envelope: &Envelope, thresholds: &Thresholds) -> bool {
    should_redirect_judged(turn, envelope, thresholds, None)
}

/// [`should_redirect`], with a judge when one is given. With a judge, a
/// turn worth judging is redirected when the judge says stuck with at least
/// [`JUDGE_CONFIDENCE`]; the envelope must still have enough samples, so a
/// judge never acts in a folder the guard knows nothing about.
pub fn should_redirect_judged(
    turn: &mut TurnState,
    envelope: &Envelope,
    thresholds: &Thresholds,
    judge: Option<&dyn StuckJudge>,
) -> bool {
    if turn.redirects >= thresholds.redirects_per_turn {
        return false;
    }
    let Some(limit) = envelope.limit(thresholds) else {
        return false;
    };
    let redirect = match judge {
        Some(judge) => {
            worth_judging(turn)
                && matches!(
                    judge.judge(turn, Some(limit)),
                    Some(Verdict::Stuck { confidence }) if confidence >= JUDGE_CONFIDENCE
                )
        }
        None => turn.looks_stuck(thresholds) && turn.calls >= limit,
    };
    if redirect {
        turn.redirects += 1;
    }
    redirect
}

/// The redirect: the agent's own counts, and a prompt to change approach.
/// It names no file and carries no outside knowledge, and it never asks the
/// agent to stop.
pub fn redirect_text(turn: &TurnState) -> String {
    format!(
        "Automated process check (Trace Commons): this turn has made {edits} edit(s) across \
         {files} file(s), the busiest file {churn} time(s), and ran {checks} check(s); the same \
         failure has come back {repeats} time(s). That looks like a loop. Keep working, but step \
         back first: re-read the failing output, consider a different cause or a different \
         file than the ones you have been editing, and test that idea directly before editing \
         again.",
        edits = turn.edit_count(),
        files = turn.edits.len(),
        churn = turn.edit_churn(),
        checks = turn.checks,
        repeats = turn.repeated_failures,
    )
}
