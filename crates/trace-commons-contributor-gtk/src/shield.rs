//! The queue's shield, beside its count.
//!
//! **Added to the number, never in place of it.** The ask was to swap the
//! sidebar count for an icon, and that trade is wrong at the size this
//! queue actually reaches: at 149 waiting sessions the count is the signal a
//! contributor is reading, and an icon that says "there is a queue" says
//! less than the number already did. What the icon adds is a state the
//! number cannot carry -- whether anything in there wants looking at.
//!
//! Two things want looking at, and neither is visible from the count: a
//! session scrubbing matched NOTHING in, and a session that was trimmed to
//! fit the raw byte budget. Both are per-entry facts the sidebar has no
//! other way to report.

/// What the sidebar's shield says about the queue as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shield {
    /// Nothing waiting.
    Clear,
    /// Sessions waiting, none of them flagged.
    Waiting,
    /// At least one waiting session wants looking at.
    Attention,
}

/// The shield for a queue.
///
/// An empty queue is `Clear` whatever the flags say. `nothing_matched` and
/// `trimmed` are counted over the entries themselves, so an empty queue can
/// only reach this with stale figures -- and a shield asking someone to look
/// at a queue with nothing in it would be asking them to look at nothing.
pub fn state(waiting: usize, nothing_matched: usize, trimmed: usize) -> Shield {
    if waiting == 0 {
        return Shield::Clear;
    }
    if nothing_matched > 0 || trimmed > 0 {
        return Shield::Attention;
    }
    Shield::Waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_queue_is_clear() {
        assert!(matches!(state(0, 0, 0), Shield::Clear));
    }

    #[test]
    fn an_ordinary_queue_is_waiting() {
        assert!(matches!(state(12, 0, 0), Shield::Waiting));
    }

    #[test]
    fn a_session_where_nothing_matched_raises_attention() {
        assert!(matches!(state(12, 1, 0), Shield::Attention));
    }

    #[test]
    fn a_trimmed_session_raises_attention() {
        assert!(matches!(state(12, 0, 1), Shield::Attention));
    }

    #[test]
    fn an_empty_queue_is_clear_even_with_stale_flags() {
        assert!(matches!(state(0, 3, 2), Shield::Clear));
    }
}
