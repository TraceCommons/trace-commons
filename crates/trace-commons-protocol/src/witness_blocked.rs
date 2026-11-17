//! How many submissions a contributor's daemon could not send because the
//! redaction witness was busy, did not answer in time, or could not be
//! reached.
//!
//! The witness cannot count these itself: a refusal at the gateway in front
//! of it, or a client that gave up waiting, never reaches its own counters.
//! So the device counts them, by label only, and reports the totals on its
//! next ordinary history read (`POST /v1/contributors/me/submission-status`)
//! in the [`WITNESS_BLOCKED_HEADER`] header. Ingest adds them to a daily total
//! for the authenticated tenant. Nothing identifies a device, a trace or a
//! time finer than the day ingest received the report.
//!
//! The header is a short list of `label=count` pairs, for example
//! `busy=3,unreachable=1`. Both sides use [`WitnessBlockedCounts::header_value`]
//! and [`WitnessBlockedCounts::parse_header_value`], so the format has one
//! definition. A value the parser refuses is dropped whole by ingest; it
//! never fails the request it rode on.

use serde::{Deserialize, Serialize};

/// The request header the counts travel in.
pub const WITNESS_BLOCKED_HEADER: &str = "x-trace-witness-blocked";

/// The most one report carries for one label. A device holds at most this
/// many per label between reports (further ones are not counted), and ingest
/// refuses a report with a larger value.
pub const WITNESS_BLOCKED_REPORT_MAX: u32 = 1_000;

/// The longest header value the parser reads. Three labels at the cap fit
/// in 44 bytes.
pub const WITNESS_BLOCKED_HEADER_MAX_LEN: usize = 64;

/// Why a submission could not be sent, as reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WitnessBlockedLabel {
    /// The witness was at capacity (`503 witness_saturated`).
    Busy,
    /// The witness did not answer within the client's deadline, or answered
    /// that its own deadline ran out (`504 witness_request_timed_out`).
    TimedOut,
    /// The witness could not be connected to, or did not hand over its
    /// attestation (which is where a refusal by the gateway in front of it
    /// surfaces).
    Unreachable,
}

impl WitnessBlockedLabel {
    /// Every label, in wire order.
    pub const ALL: [Self; 3] = [Self::Busy, Self::TimedOut, Self::Unreachable];

    /// The wire and storage spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::TimedOut => "timed_out",
            Self::Unreachable => "unreachable",
        }
    }

    /// The label for a wire spelling, or `None`.
    pub fn from_wire(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|label| label.as_str() == value)
    }
}

/// Why a header value was refused. Labels only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitnessBlockedRefusal {
    /// Empty, too long, or not `label=count` pairs separated by commas.
    Malformed,
    /// A label outside [`WitnessBlockedLabel::ALL`].
    UnknownLabel,
    /// The same label twice.
    DuplicateLabel,
    /// A count above [`WITNESS_BLOCKED_REPORT_MAX`].
    OverCap,
    /// Every count is zero: there is nothing to record.
    Empty,
}

impl WitnessBlockedRefusal {
    /// The log label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Malformed => "witness_blocked_report_malformed",
            Self::UnknownLabel => "witness_blocked_report_unknown_label",
            Self::DuplicateLabel => "witness_blocked_report_duplicate_label",
            Self::OverCap => "witness_blocked_report_over_cap",
            Self::Empty => "witness_blocked_report_empty",
        }
    }
}

/// Blocked submissions per label. Held by the daemon between reports, and
/// what one report carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessBlockedCounts {
    #[serde(default)]
    pub busy: u32,
    #[serde(default)]
    pub timed_out: u32,
    #[serde(default)]
    pub unreachable: u32,
}

impl WitnessBlockedCounts {
    /// The count for one label.
    pub const fn get(&self, label: WitnessBlockedLabel) -> u32 {
        match label {
            WitnessBlockedLabel::Busy => self.busy,
            WitnessBlockedLabel::TimedOut => self.timed_out,
            WitnessBlockedLabel::Unreachable => self.unreachable,
        }
    }

    fn slot(&mut self, label: WitnessBlockedLabel) -> &mut u32 {
        match label {
            WitnessBlockedLabel::Busy => &mut self.busy,
            WitnessBlockedLabel::TimedOut => &mut self.timed_out,
            WitnessBlockedLabel::Unreachable => &mut self.unreachable,
        }
    }

    /// Count one blocked submission, up to [`WITNESS_BLOCKED_REPORT_MAX`].
    pub fn record(&mut self, label: WitnessBlockedLabel) {
        let slot = self.slot(label);
        *slot = slot.saturating_add(1).min(WITNESS_BLOCKED_REPORT_MAX);
    }

    /// Add another set of counts back, each held to
    /// [`WITNESS_BLOCKED_REPORT_MAX`]. Used to restore counts a report
    /// failed to deliver.
    pub fn merge(&mut self, other: &Self) {
        for label in WitnessBlockedLabel::ALL {
            let slot = self.slot(label);
            *slot = slot
                .saturating_add(other.get(label))
                .min(WITNESS_BLOCKED_REPORT_MAX);
        }
    }

    /// Whether every count is zero.
    pub const fn is_empty(&self) -> bool {
        self.busy == 0 && self.timed_out == 0 && self.unreachable == 0
    }

    /// The non-zero counts, in wire order.
    pub fn entries(&self) -> impl Iterator<Item = (WitnessBlockedLabel, u32)> + '_ {
        WitnessBlockedLabel::ALL
            .into_iter()
            .map(|label| (label, self.get(label)))
            .filter(|(_, count)| *count > 0)
    }

    /// The header value for these counts: the non-zero labels in wire order,
    /// each held to [`WITNESS_BLOCKED_REPORT_MAX`]. `None` when there is
    /// nothing to report.
    pub fn header_value(&self) -> Option<String> {
        let pairs = self
            .entries()
            .map(|(label, count)| {
                format!(
                    "{}={}",
                    label.as_str(),
                    count.min(WITNESS_BLOCKED_REPORT_MAX)
                )
            })
            .collect::<Vec<_>>();
        (!pairs.is_empty()).then(|| pairs.join(","))
    }

    /// Read a header value, strictly: comma-separated `label=count` pairs,
    /// each label from [`WitnessBlockedLabel::ALL`] at most once, each count
    /// plain ASCII digits no larger than [`WITNESS_BLOCKED_REPORT_MAX`], and
    /// at least one count above zero. Anything else is refused whole.
    pub fn parse_header_value(value: &str) -> Result<Self, WitnessBlockedRefusal> {
        if value.is_empty() || value.len() > WITNESS_BLOCKED_HEADER_MAX_LEN {
            return Err(WitnessBlockedRefusal::Malformed);
        }
        let mut counts = Self::default();
        let mut seen = [false; 3];
        for pair in value.split(',') {
            let (name, digits) = pair
                .split_once('=')
                .ok_or(WitnessBlockedRefusal::Malformed)?;
            // Four digits at most: the cap has four, and a longer run could
            // only be a leading-zero spelling or a value over it.
            if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(WitnessBlockedRefusal::Malformed);
            }
            let label =
                WitnessBlockedLabel::from_wire(name).ok_or(WitnessBlockedRefusal::UnknownLabel)?;
            let index = label as usize;
            if seen[index] {
                return Err(WitnessBlockedRefusal::DuplicateLabel);
            }
            seen[index] = true;
            let count: u32 = digits
                .parse()
                .map_err(|_| WitnessBlockedRefusal::Malformed)?;
            if count > WITNESS_BLOCKED_REPORT_MAX {
                return Err(WitnessBlockedRefusal::OverCap);
            }
            *counts.slot(label) = count;
        }
        if counts.is_empty() {
            return Err(WitnessBlockedRefusal::Empty);
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_are_exact() {
        assert_eq!(WITNESS_BLOCKED_HEADER, "x-trace-witness-blocked");
        let names = WitnessBlockedLabel::ALL.map(WitnessBlockedLabel::as_str);
        assert_eq!(names, ["busy", "timed_out", "unreachable"]);
        for label in WitnessBlockedLabel::ALL {
            assert_eq!(WitnessBlockedLabel::from_wire(label.as_str()), Some(label));
        }
        assert_eq!(WitnessBlockedLabel::from_wire("Busy"), None);
    }

    #[test]
    fn header_round_trips_and_omits_zero_labels() {
        let counts = WitnessBlockedCounts {
            busy: 3,
            timed_out: 0,
            unreachable: 1,
        };
        let value = counts.header_value().expect("non-empty");
        assert_eq!(value, "busy=3,unreachable=1");
        assert_eq!(WitnessBlockedCounts::parse_header_value(&value), Ok(counts));
        assert_eq!(WitnessBlockedCounts::default().header_value(), None);
    }

    #[test]
    fn largest_report_fits_the_length_bound() {
        let counts = WitnessBlockedCounts {
            busy: WITNESS_BLOCKED_REPORT_MAX,
            timed_out: WITNESS_BLOCKED_REPORT_MAX,
            unreachable: WITNESS_BLOCKED_REPORT_MAX,
        };
        let value = counts.header_value().expect("non-empty");
        assert!(value.len() <= WITNESS_BLOCKED_HEADER_MAX_LEN);
        assert_eq!(WitnessBlockedCounts::parse_header_value(&value), Ok(counts));
    }

    #[test]
    fn record_and_merge_hold_the_cap() {
        let mut counts = WitnessBlockedCounts::default();
        for _ in 0..(WITNESS_BLOCKED_REPORT_MAX + 5) {
            counts.record(WitnessBlockedLabel::Busy);
        }
        counts.record(WitnessBlockedLabel::TimedOut);
        assert_eq!(counts.busy, WITNESS_BLOCKED_REPORT_MAX);
        assert_eq!(counts.timed_out, 1);
        assert_eq!(counts.unreachable, 0);

        counts.merge(&WitnessBlockedCounts {
            busy: 7,
            timed_out: 2,
            unreachable: u32::MAX,
        });
        assert_eq!(counts.busy, WITNESS_BLOCKED_REPORT_MAX);
        assert_eq!(counts.timed_out, 3);
        assert_eq!(counts.unreachable, WITNESS_BLOCKED_REPORT_MAX);
    }

    #[test]
    fn parser_refuses_everything_off_contract() {
        use WitnessBlockedRefusal::*;
        let cases: [(&str, WitnessBlockedRefusal); 16] = [
            ("", Malformed),
            ("busy", Malformed),
            ("busy=", Malformed),
            ("busy=-1", Malformed),
            ("busy=+1", Malformed),
            ("busy=1 ", Malformed),
            (" busy=1", UnknownLabel),
            ("busy=1,", Malformed),
            ("busy=1;timed_out=2", Malformed),
            ("busy=0x10", Malformed),
            ("busy=10000", Malformed),
            ("busy=1001", OverCap),
            ("busy=1,busy=2", DuplicateLabel),
            ("tenant=1", UnknownLabel),
            ("BUSY=1", UnknownLabel),
            ("busy=0,unreachable=0", Empty),
        ];
        for (value, refusal) in cases {
            assert_eq!(
                WitnessBlockedCounts::parse_header_value(value),
                Err(refusal),
                "{value:?}"
            );
        }
        // Over the length bound is refused before anything is read.
        let long = format!("busy=1{}", ",".repeat(WITNESS_BLOCKED_HEADER_MAX_LEN));
        assert_eq!(
            WitnessBlockedCounts::parse_header_value(&long),
            Err(Malformed)
        );
    }

    #[test]
    fn parser_accepts_any_label_order_and_leading_zeros_within_four_digits() {
        assert_eq!(
            WitnessBlockedCounts::parse_header_value("unreachable=2,busy=0007"),
            Ok(WitnessBlockedCounts {
                busy: 7,
                timed_out: 0,
                unreachable: 2,
            })
        );
    }
}
