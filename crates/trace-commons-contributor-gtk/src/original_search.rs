//! Telling "this value was never here" apart from "this value was removed".
//!
//! The sheet's search scans the REDACTED body -- the same bytes the
//! transcript tab shows -- so a value the scrubber took out returns zero
//! matches. So does a value that was never in the session. Those two are the
//! opposite of each other to a contributor checking whether a client name
//! got out, and the count alone cannot separate them.
//!
//! The daemon's `search_original` counts the same needle in the
//! pre-redaction session text and answers with a COUNT and nothing else --
//! that bound is the reason the method is allowed to read unredacted bytes
//! at all.
//!
//! `Unknown` exists so a failed call never renders as a clean result. There
//! is exactly one direction this module must not fail in, and it is
//! reporting "not in this session" about a value that is in it.

/// What a search found, once both counts are in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// In neither text.
    Absent,
    /// In the original, in none of what would be sent.
    AllRemoved(u32),
    /// Still in what would be sent.
    SomeRemain { remaining: u32, total: u32 },
    /// The original could not be searched. Not a result.
    Unknown,
}

/// `remaining` is matches in the redacted body, which is always known;
/// `original` is the daemon's count, `None` on any failure.
pub fn classify(remaining: u32, original: Option<u32>) -> Outcome {
    let Some(original) = original else {
        // Fail toward what is certain. The redacted body is in hand, so
        // matches in it are known; the absence of a check is not a clean
        // result and must never render as one.
        return if remaining > 0 {
            Outcome::SomeRemain {
                remaining,
                total: remaining,
            }
        } else {
            Outcome::Unknown
        };
    };
    if remaining > 0 {
        return Outcome::SomeRemain {
            remaining,
            // An original count below the remaining count is impossible from
            // a correct daemon. Reporting "1 matches -- 2 would still be
            // sent" would be worse than falling back to what is certain.
            total: original.max(remaining),
        };
    }
    if original > 0 {
        Outcome::AllRemoved(original)
    } else {
        Outcome::Absent
    }
}

/// The sentence for an outcome. The wording lives in `copy`, with the rest
/// of the wording.
pub fn sentence(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Absent => crate::copy::search_absent(),
        Outcome::AllRemoved(total) => crate::copy::search_all_removed(*total),
        Outcome::SomeRemain { remaining, total } => {
            crate::copy::search_some_remain(*remaining, *total)
        }
        Outcome::Unknown => crate::copy::search_unknown(),
    }
}

/// Whether this outcome is the one worth a tone.
///
/// Only a match that would still be sent. `AllRemoved` is the scrubber
/// working, and `Unknown` is a missing answer rather than a bad one --
/// putting either in the attention tone would spend it where nothing is
/// wrong, and it would then be spent when something is.
pub fn is_alarming(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::SomeRemain { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nowhere_in_either_text_is_absent() {
        assert!(matches!(classify(0, Some(0)), Outcome::Absent));
    }

    #[test]
    fn present_originally_and_gone_now_is_all_removed() {
        assert!(matches!(classify(0, Some(3)), Outcome::AllRemoved(3)));
    }

    #[test]
    fn still_present_is_some_remain() {
        assert!(matches!(
            classify(2, Some(5)),
            Outcome::SomeRemain {
                remaining: 2,
                total: 5
            }
        ));
    }

    /// Reporting "not in this session" because a call failed would be the
    /// single most dangerous wrong answer this tab can give.
    #[test]
    fn a_failed_original_search_is_unknown_not_absent() {
        assert!(matches!(classify(0, None), Outcome::Unknown));
        assert!(matches!(
            classify(2, None),
            Outcome::SomeRemain {
                remaining: 2,
                total: 2
            }
        ));
    }

    #[test]
    fn an_original_count_below_the_remaining_count_falls_back_to_what_is_certain() {
        assert!(matches!(
            classify(2, Some(1)),
            Outcome::SomeRemain {
                remaining: 2,
                total: 2
            }
        ));
    }

    #[test]
    fn the_sentences_say_which_case_it_is() {
        assert_eq!(
            sentence(&Outcome::Absent),
            "0 matches \u{2014} not in this session"
        );
        assert_eq!(
            sentence(&Outcome::AllRemoved(3)),
            "3 matches \u{2014} all 3 were removed"
        );
        assert_eq!(
            sentence(&Outcome::SomeRemain {
                remaining: 2,
                total: 5
            }),
            "5 matches \u{2014} 2 would still be sent"
        );
    }

    #[test]
    fn only_a_remaining_match_is_alarming() {
        assert!(is_alarming(&Outcome::SomeRemain {
            remaining: 1,
            total: 1
        }));
        assert!(!is_alarming(&Outcome::AllRemoved(3)));
        assert!(!is_alarming(&Outcome::Absent));
        assert!(!is_alarming(&Outcome::Unknown));
    }

    /// `Unknown` must not read as a clean result. It is the one arm where
    /// the app does not know, and saying so is the whole reason it exists.
    #[test]
    fn the_unknown_sentence_does_not_claim_the_value_is_absent() {
        let said = sentence(&Outcome::Unknown);
        assert!(!said.contains("not in this session"), "{said}");
        assert!(said.to_lowercase().contains("couldn't"), "{said}");
    }
}
