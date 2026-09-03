//! The routing surface's words, in one place, for all three shells.
//!
//! This module used to be a region of the GTK shell's `copy.rs`. It is here
//! because the same surface is now rendered by three shells, and a word kept
//! in three places is three words that have not diverged *yet*. GTK links
//! this crate directly; the macOS and Windows shells reach it across the C
//! ABI through `tc_routing_copy` and the sentence exports beside it.
//!
//! # What crosses the boundary, and what does not
//!
//! Both the vocabulary and the sentences cross, and the sentences cross
//! **already assembled**. Nothing here is handed to a shell as a template
//! with a hole in it: `{path}`, `{port}` and the time are filled in on this
//! side, by the functions below, and a shell receives finished text.
//!
//! That is the whole point. A template a shell fills in is a fourth place
//! the wording can drift -- three shells' worth of format calls, each free
//! to add a word, drop a full stop, or reorder the clause around the hole,
//! and no test anywhere that would notice. Sentences assembled here are
//! covered by [`the_tools_surface_says_nothing_it_should_not`], which
//! renders them and sweeps the result.
//!
//! The single thing that does not cross is the *humanised timestamp* --
//! "an hour ago", "yesterday". That is a rendering of a `DateTime`, not
//! wording about routing, and every shell already has a localised one.
//! [`last_checked_line`] takes it as a string and owns the sentence around
//! it, so "Last checked" is still written once.

// --- Tools -------------------------------------------------------------
//
// One tool, one word.
//
// TOOLS-SURFACE-BEGIN
//
// Everything between this marker and TOOLS-SURFACE-END is swept by
// `the_tools_surface_says_nothing_it_should_not`, which reads this file
// rather than a hand-kept list of names. A string literal added anywhere in
// this region is checked automatically; one added outside it is not.
//
// A contributor running IronWire has one question to answer about each of
// their tools: is IronWire handling it. "Destination", "backend", "route"
// and "proxy" are our vocabulary for the mechanism, and none of them is a
// thing this person has to learn in order to answer it. The controls
// underneath the words are the exception rather than the front door: the
// conventional port is already filled in and the folder box is for an
// unusual install.
//
// # The one word that claims privacy
//
// Exactly one word here -- `TOOL_PRIVATE` -- makes a privacy claim, and no
// word denies one. What backs the claim, and the gap under it, is written
// out on that constant; read it before touching any of these four.
//
// The claim is made only from IronWire's own per-tool answer. The word this
// surface printed before was computed from a single switch declaring
// IronWire in *this* app, which was wrong twice over: it was not per-tool
// -- declaring IronWire here has no causal relation to whether Codex is
// configured to send through it -- and it had no third state, so a dead
// proxy and an unlisted tool both rendered as a confident verdict.

pub const TOOLS_HEADING: &str = "Tools";

/// IronWire answered, and reports this tool as pointed at a local address.
///
/// # What this word claims, and the gap under it
///
/// "Private" is what a contributor came here to learn, and naming the
/// vendor instead tells them nothing they can act on. So this is the word.
///
/// Be clear-eyed about what backs it. IronWire reports a tool as wired when
/// its config names **any loopback host, on any port, with the path
/// `/anthropic`** -- deliberately, so `ironwire connect` can follow a port
/// change. Nothing on that response carries a port or a URL, so this app
/// cannot today distinguish IronWire from some other local proxy answering
/// on the same path. In that configuration this word would be wrong.
///
/// That configuration is unusual, and the alternative -- printing a vendor
/// name and leaving the person to work out whether it means their code is
/// exposed -- is worse for every ordinary case. The gap closes properly
/// when IronWire exposes either the URL a tool points at or a `wired`
/// computed against the running port; the ask is with them, and
/// `points_at_us` already parses that port and discards it.
///
/// Until then: this word is only ever printed from IronWire's per-tool
/// answer, never from our own switch, and never when the probe did not
/// reach.
pub const TOOL_PRIVATE: &str = "Private";
/// IronWire answered, and this tool is not pointed at it.
pub const TOOL_DIRECT: &str = "Sends direct";
/// Nothing usable answered, or the answer did not mention this tool.
///
/// Not a fault and not a verdict. Gemini CLI reaches this state on every
/// machine today, because IronWire's tool list does not carry a row for it
/// at all -- which is exactly the case the old single-switch word got
/// confidently wrong.
pub const TOOL_UNKNOWN: &str = "Not known";
/// The contributor said they do not use this tool. Nothing is read from
/// it, so no question about handling arises.
pub const TOOL_NOT_USED: &str = "Not used";

/// The tools this window has a word for. Short names, because the word
/// beside them is doing the work.
pub const TOOL_CLAUDE: &str = "Claude Code";
pub const TOOL_CODEX: &str = "Codex";
pub const TOOL_GEMINI: &str = "Gemini CLI";

/// The one paragraph that has to be true.
///
/// The sentence it replaced promised that the proxy "keeps what your tools
/// send to a model private" and that this page "can tell you which of your
/// tools are covered". The first is a claim about a destination this app
/// cannot see; the second was not per-tool at all. What is left is what the
/// evidence supports: a record on this machine is read, per tool, and the
/// answer is about the first hop.
///
/// # Why the proxy is not a character here
///
/// It used to be. The vendor ran, answered, kept files and had a name, and
/// a contributor had to learn all of that before they could read a single
/// word about their own tools. This surface now has exactly two actors --
/// this app, and the contributor's tools -- and the proxy appears only as
/// what it is to us: a record kept on this machine that we can read or
/// cannot. Nothing below names a vendor, and
/// [`the_tools_surface_says_nothing_it_should_not`] holds that.
///
/// The cost is real and worth stating: the failure sentences no longer say
/// what to go start. What they have left is the port and the folder, which
/// are the two things a contributor can actually act on from this window.
pub const IRONWIRE_INTRO: &str = concat!(
    crate::app_name!(),
    " can read a record kept on this machine of where your tools send their requests first, \
     and says so below, one tool at a time. That is a fact about this machine alone: it says \
     where a request goes first, not what happens to it afterwards."
);

pub const IRONWIRE_TOGGLE: &str = "Read the local record on this machine";

/// Said out loud because the obvious worry is that it is not true.
/// Nothing here waits on the app being started again.
pub const IRONWIRE_APPLIES_AT_ONCE: &str = "Changes here apply straight away.";

pub const IRONWIRE_PORT_TITLE: &str = "Port";
pub const IRONWIRE_PORT_NOTE: &str =
    "Already set to the usual number. Change it only if the record is kept on a different one.";

pub const IRONWIRE_FOLDER_TITLE: &str = "Folder";
/// The note when nothing on this machine can say where the usual place is.
///
/// Kept as a constant because the payload's shape requires one, and used as
/// the fallback [`ironwire_folder_note`] returns when no folder resolved. A
/// build with no home directory to read has no better sentence available.
pub const IRONWIRE_FOLDER_NOTE: &str =
    "Leave this empty unless the record is kept somewhere other than the usual place.";

/// The folder note, naming the folder it is talking about.
///
/// Every failure sentence on this surface ends by sending a contributor to
/// this one field -- "Name the folder below", "Point the folder below at
/// where the record is kept" -- and the field then declined to say which
/// folder it meant or what it would read if left empty. "Somewhere other
/// than the usual place" resolved to nothing anywhere on the screen, so the
/// instruction had no answer for the person following it.
///
/// The path arrives as an argument rather than as wording, for the same
/// reason [`ironwire_token_line`]'s does: it is the one place a vendor name
/// can still reach this screen, and somebody being sent to look at a folder
/// has to be told the folder that is really there.
#[must_use]
pub fn ironwire_folder_note(default_dir: Option<&str>) -> String {
    match default_dir {
        Some(dir) => {
            format!("Leave this empty unless the record is kept somewhere other than {dir}.")
        }
        None => IRONWIRE_FOLDER_NOTE.to_string(),
    }
}

/// [`ironwire_folder_note`] over the folder this machine would really read.
///
/// The resolution is
/// [`crate::daemon::settings::ironwire_default_token_dir`], the last step of
/// the token search order, so this sentence cannot name one folder while the
/// daemon reads another. Assembled here and not in each shell: GTK calls it,
/// and the other two receive its answer inside [`routing_copy`].
#[must_use]
pub fn ironwire_folder_note_here() -> String {
    ironwire_folder_note(
        crate::daemon::settings::ironwire_default_token_dir()
            .as_deref()
            .and_then(std::path::Path::to_str),
    )
}

pub const IRONWIRE_APPLY: &str = "Apply and check";
pub const IRONWIRE_CHECKING: &str = "Checking...";

/// The check itself could not be run -- not a fact about IronWire, so it
/// must not send anybody to look at a port or a file that is fine.
pub const IRONWIRE_CHECK_UNAVAILABLE: &str =
    "That check couldn't be run just now. Nothing changed.";

pub const IRONWIRE_PROBE_REACHABLE: &str =
    concat!(crate::app_name!(), " can read the local record.");

pub const IRONWIRE_STATE_OFF: &str = concat!(
    "Off. ",
    crate::app_name!(),
    " is not reading the local record."
);
/// Not a fault, and the copy has to say so. A record read from a
/// freshly-built reader starts empty by construction, so a contributor who
/// just turned this on -- or just changed the port -- sees this state.
pub const IRONWIRE_STATE_WAITING: &str = "On. Nothing recorded yet, which is normal just after you turn this on or change something \
     here.";
pub const IRONWIRE_STATE_READING: &str = concat!(
    "On, and ",
    crate::app_name!(),
    " is reading the local record."
);

/// What IronWire answered about one tool, as far as this page may use it.
///
/// Deliberately three states and not a boolean. The old word was computed
/// from `ironwire.mode == "watch"` -- a declaration in this app -- which
/// has no third state and therefore no way to say "nobody has told us".
/// That missing state is the whole defect: a dead proxy and an unlisted
/// tool both used to render as a confident verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolWiring {
    /// IronWire listed this tool and said it is pointed at a local address.
    Wired,
    /// IronWire listed this tool and said it is not.
    NotWired,
    /// Nothing usable answered, IronWire did not list this tool, or it
    /// listed it as not present on this machine. No verdict is available.
    Unknown,
}

/// One tool's word, from what the contributor said about that tool's
/// sessions and what IronWire said about that tool.
///
/// `source_mode` is `get_settings`'s `*_source_mode`: `off`, `watch` or
/// `unset`. Only `off` means the tool is not used -- `unset` watches the
/// conventional location, which is a tool in use.
///
/// The declaration switch is **not** an input. It was the only input
/// before, and that is what let a contributor read "Private" on the same
/// card as "Nothing answered on port 8463".
#[must_use]
pub fn tool_word(source_mode: &str, wiring: ToolWiring) -> &'static str {
    if source_mode == "off" {
        return TOOL_NOT_USED;
    }
    match wiring {
        ToolWiring::Wired => TOOL_PRIVATE,
        ToolWiring::NotWired => TOOL_DIRECT,
        ToolWiring::Unknown => TOOL_UNKNOWN,
    }
}

/// How one tool's word is painted.
///
/// Two values and not a `bool`. A boolean meaning "this is the privacy
/// word" is one refactor away from a shell recovering it by comparing the
/// rendered word against `TOOL_PRIVATE` -- and `Private` is a substring of
/// the denial that must never come back, which is the same shape that once
/// let `contains("reachable")` match `"unreachable"` on this surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolTone {
    /// Says nothing either way. Every word but the wired one gets this,
    /// "not used" included: that is a preference, not an outcome.
    Neutral,
    /// The reassuring reading. Only the wired word gets it.
    Clear,
}

/// The tone [`tool_word`]'s answer is painted in, from the same two inputs.
///
/// ONE BRANCH TABLE, NOT TWO. This takes what [`tool_word`] takes, so the
/// two stay in step by construction. A shell must call this rather than
/// test the rendered word: the word is a string that three shells print and
/// the tone is a styling decision, and a styling decision that reads a
/// rendered privacy claim is a substring match waiting to happen.
#[must_use]
pub fn tool_tone(source_mode: &str, wiring: ToolWiring) -> ToolTone {
    if source_mode == "off" {
        return ToolTone::Neutral;
    }
    match wiring {
        ToolWiring::Wired => ToolTone::Clear,
        ToolWiring::NotWired | ToolWiring::Unknown => ToolTone::Neutral,
    }
}

/// The file could not be used: either it is not there, or what was in it
/// is no longer accepted.
///
/// Names the file, because that is the one fact that makes this fixable,
/// and it is the failure a real contributor hits: a GUI never sees
/// `IRONWIRE_HOME`, so it reads `~/.ironwire` whatever a shell profile
/// says. The path is absent, not empty, when nothing resolved at all.
///
/// That path is also the one place a vendor name can still reach the
/// screen, and it may. It arrives as this function's argument, not as
/// wording -- the sentence around it names nobody -- and a path a person is
/// being sent to look at has to be the path that is really there.
#[must_use]
pub fn ironwire_token_line(token_path: Option<&str>) -> String {
    match token_path {
        Some(path) => format!(
            concat!(
                crate::app_name!(),
                " could not use the file at {path}. Either it is not there, or it is no longer \
                 valid. Point the folder below at where the record is kept."
            ),
            path = path
        ),
        None => concat!(
            crate::app_name!(),
            " could not work out where the record is kept. Name the folder below."
        )
        .to_string(),
    }
}

/// Nothing usable answered. Names the port that was tried.
#[must_use]
pub fn ironwire_unreachable_line(port: Option<u16>) -> String {
    match port {
        Some(port) => format!(
            "Nothing answered on port {port}. Check that this is the right number, or name the \
             folder below."
        ),
        None => "Nothing answered on this machine.".to_string(),
    }
}

/// The daemon's three states, in words. A state this build does not know
/// says what the off state says: it claims nothing.
#[must_use]
pub fn ironwire_state_line(state: &str) -> &'static str {
    match state {
        "awaiting_rows" => IRONWIRE_STATE_WAITING,
        "rows_seen" => IRONWIRE_STATE_READING,
        _ => IRONWIRE_STATE_OFF,
    }
}

/// How firmly a daemon state reads.
///
/// Three values, and none of them is a fault: `awaiting_rows` is
/// [`StateTone::Held`] and never an error. A reader built a moment ago
/// starts empty by construction, and a declaration change puts a working
/// install back into that state, so this is what a contributor sees
/// immediately after touching anything on this card. Painting it as broken
/// would accuse a working proxy at exactly that moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateTone {
    /// Nothing is declared, so nothing is claimed.
    Neutral,
    /// Declared, and no answer has arrived yet. Normal, not broken.
    Held,
    /// Declared, and answers are arriving.
    Clear,
}

/// The tone [`ironwire_state_line`]'s sentence is painted in.
///
/// ONE BRANCH TABLE, NOT TWO -- and, since this crossed the ABI, not four.
/// This takes what the sentence takes, so the two stay in step by
/// construction, and no shell may recover it by comparing the rendered
/// sentence against one of the three state constants.
///
/// A state this build has never heard of is [`StateTone::Neutral`], which
/// claims nothing, exactly as its sentence does.
#[must_use]
pub fn ironwire_state_tone(state: &str) -> StateTone {
    match state {
        "awaiting_rows" => StateTone::Held,
        "rows_seen" => StateTone::Clear,
        _ => StateTone::Neutral,
    }
}

/// Whether the "last checked" stamp says anything on this state.
///
/// It is a per-process stamp on the running daemon -- never an install
/// date, never a connected-since -- and it starts empty again every time
/// that process comes back up. On a state that has had no answer at all
/// there is nothing for it to report.
///
/// Derived from [`ironwire_state_tone`] rather than matched again, so a
/// state added later cannot be given a sentence and a tone here and then
/// silently disagree with the three shells about the stamp.
#[must_use]
pub fn ironwire_shows_last_checked(state: &str) -> bool {
    ironwire_state_tone(state) != StateTone::Neutral
}

/// When the daemon last got an answer.
///
/// "Last checked", never "connected since" and never a date this install
/// began: the stamp lives in the running daemon and starts empty again
/// every time that process comes back up.
///
/// `when` is the already-humanised time -- "an hour ago", "yesterday". That
/// is the one part of this surface each shell renders for itself, because it
/// is a rendering of a `DateTime` and not wording about routing. The
/// sentence around it is assembled here so that "Last checked" is written
/// once for all three.
#[must_use]
pub fn last_checked_line(when: &str) -> String {
    format!("Last checked {when}")
}

/// Every fixed string on this surface, in one payload.
///
/// Shaped for the C ABI: `tc_routing_copy` serialises this and hands the
/// shell one owned JSON object. One call and not one per string, unlike
/// `tc_scrub_detector_names`'s single-list shape -- that export answers one
/// question, and this one is a whole screen's worth of wording that must
/// arrive as a set. A per-string export would let a shell take four of the
/// words and hand-write the fifth, which is the failure this module exists
/// to remove.
///
/// The sentences that interpolate are NOT here, because they cannot be
/// finished without an argument. They cross as their own exports, already
/// assembled: see [`ironwire_token_line`], [`ironwire_unreachable_line`]
/// and [`last_checked_line`].
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct RoutingCopy {
    pub tools_heading: &'static str,
    pub word_private: &'static str,
    pub word_direct: &'static str,
    pub word_unknown: &'static str,
    pub word_not_used: &'static str,
    pub tool_claude: &'static str,
    pub tool_codex: &'static str,
    pub tool_gemini: &'static str,
    pub intro: &'static str,
    pub toggle: &'static str,
    pub applies_at_once: &'static str,
    pub port_title: &'static str,
    pub port_note: &'static str,
    pub folder_title: &'static str,
    /// The only field that is not a fixed string: it names the folder this
    /// machine would read, which is a path and therefore not wording. See
    /// [`ironwire_folder_note`].
    pub folder_note: String,
    pub apply: &'static str,
    pub checking: &'static str,
    pub check_unavailable: &'static str,
    pub probe_reachable: &'static str,
    pub state_off: &'static str,
    pub state_waiting: &'static str,
    pub state_reading: &'static str,
}

/// The payload, built from the constants above -- and, for the folder note
/// alone, from the folder this machine resolves. That one sentence cannot be
/// finished without a path, and unlike the others it has no shell to pass it
/// one: all three read it from here.
#[must_use]
pub fn routing_copy() -> RoutingCopy {
    RoutingCopy {
        tools_heading: TOOLS_HEADING,
        word_private: TOOL_PRIVATE,
        word_direct: TOOL_DIRECT,
        word_unknown: TOOL_UNKNOWN,
        word_not_used: TOOL_NOT_USED,
        tool_claude: TOOL_CLAUDE,
        tool_codex: TOOL_CODEX,
        tool_gemini: TOOL_GEMINI,
        intro: IRONWIRE_INTRO,
        toggle: IRONWIRE_TOGGLE,
        applies_at_once: IRONWIRE_APPLIES_AT_ONCE,
        port_title: IRONWIRE_PORT_TITLE,
        port_note: IRONWIRE_PORT_NOTE,
        folder_title: IRONWIRE_FOLDER_TITLE,
        folder_note: ironwire_folder_note_here(),
        apply: IRONWIRE_APPLY,
        checking: IRONWIRE_CHECKING,
        check_unavailable: IRONWIRE_CHECK_UNAVAILABLE,
        probe_reachable: IRONWIRE_PROBE_REACHABLE,
        state_off: IRONWIRE_STATE_OFF,
        state_waiting: IRONWIRE_STATE_WAITING,
        state_reading: IRONWIRE_STATE_READING,
    }
}

// TOOLS-SURFACE-END

#[cfg(test)]
mod tests {
    use super::*;

    /// The one-word vocabulary, and the trap inside it.
    ///
    /// "Private" used to be one of these words and is a substring of "Not
    /// private" -- the exact shape that made `contains("reachable")` match
    /// "unreachable" earlier on this plan. Both words are gone, and this
    /// test now asserts the property rather than the historical pair: no
    /// word in the vocabulary contains any other, in either case, so an
    /// assertion written with `contains` cannot silently match the wrong
    /// one. It fails on the next word that reintroduces the shape.
    #[test]
    fn no_tool_word_contains_another_so_contains_cannot_match_the_wrong_one() {
        let words = [TOOL_PRIVATE, TOOL_DIRECT, TOOL_UNKNOWN, TOOL_NOT_USED];
        for (i, one) in words.iter().enumerate() {
            for (j, other) in words.iter().enumerate() {
                if i == j {
                    continue;
                }
                assert_ne!(one, other);
                assert!(
                    !one.to_lowercase().contains(&other.to_lowercase()),
                    "{other:?} is a substring of {one:?}"
                );
            }
        }
    }

    /// Exactly one word claims privacy, and no word denies it.
    ///
    /// The defect this surface exists to remove was a wrong *declaration*
    /// producing a confident privacy claim. The claim itself is fine --
    /// it is what a contributor came to learn -- as long as it is printed
    /// only from IronWire's per-tool answer, which the state-mapping tests
    /// pin.
    ///
    /// What must not come back is a word that **denies** privacy. "Private"
    /// is a substring of "Not private", so the two together are the exact
    /// shape that let `contains("reachable")` match "unreachable" earlier on
    /// this surface. `TOOL_DIRECT` says "Sends direct" for that reason, and
    /// this test is what stops somebody tidying it to "Not private".
    #[test]
    fn only_the_wired_word_claims_privacy_and_none_denies_it() {
        assert!(TOOL_PRIVATE.to_lowercase().contains("privat"));
        for word in [TOOL_DIRECT, TOOL_UNKNOWN, TOOL_NOT_USED] {
            assert!(
                !word.to_lowercase().contains("privat"),
                "a word that denies privacy reintroduces the substring trap: {word}"
            );
        }
    }

    /// One tool, one word, and the switch is not an input to any of them.
    ///
    /// A tool the contributor said they do not use reads "Not used"
    /// whatever IronWire says -- there is nothing of theirs being read
    /// either way.
    #[test]
    fn each_tool_reads_exactly_one_of_four_words() {
        for wiring in [ToolWiring::Wired, ToolWiring::NotWired, ToolWiring::Unknown] {
            assert_eq!(tool_word("off", wiring), TOOL_NOT_USED);
        }
        assert_eq!(tool_word("watch", ToolWiring::Wired), TOOL_PRIVATE);
        assert_eq!(tool_word("watch", ToolWiring::NotWired), TOOL_DIRECT);
        assert_eq!(tool_word("watch", ToolWiring::Unknown), TOOL_UNKNOWN);
        // "unset" means the conventional location is watched, which is a
        // tool in use.
        assert_eq!(tool_word("unset", ToolWiring::Wired), TOOL_PRIVATE);
        assert_eq!(tool_word("unset", ToolWiring::Unknown), TOOL_UNKNOWN);
        // A mode this build has never heard of is still a tool in use, and
        // still gets no verdict without evidence.
        assert_eq!(tool_word("", ToolWiring::Unknown), TOOL_UNKNOWN);
    }

    /// Two tools in use, one declaration, two different words.
    ///
    /// The second Critical this change closes: the word is per tool, so a
    /// machine where Claude Code is pointed at IronWire and Codex is not
    /// cannot render as one verdict repeated three times.
    #[test]
    fn two_tools_under_one_declaration_can_read_differently() {
        assert_ne!(
            tool_word("watch", ToolWiring::Wired),
            tool_word("watch", ToolWiring::NotWired)
        );
        assert_ne!(
            tool_word("watch", ToolWiring::NotWired),
            tool_word("watch", ToolWiring::Unknown)
        );
    }

    /// The state's tone and its sentence are one decision.
    ///
    /// Asserted over every state this build knows plus ones it does not, so
    /// the two tables cannot disagree about what a state means -- which is
    /// the whole reason both cross the ABI rather than being written out in
    /// each shell.
    #[test]
    fn every_state_tone_agrees_with_the_sentence_that_state_gets() {
        for state in [
            "not_declared",
            "awaiting_rows",
            "rows_seen",
            "",
            "ROWS_SEEN",
            "a_state_from_a_later_daemon",
        ] {
            let line = ironwire_state_line(state);
            let tone = ironwire_state_tone(state);
            let expected = match line {
                IRONWIRE_STATE_WAITING => StateTone::Held,
                IRONWIRE_STATE_READING => StateTone::Clear,
                _ => StateTone::Neutral,
            };
            assert_eq!(tone, expected, "{state:?} reads {line:?} as {tone:?}");
            // The stamp is shown exactly where the state has had an answer.
            assert_eq!(
                ironwire_shows_last_checked(state),
                tone != StateTone::Neutral,
                "{state:?}"
            );
        }

        // Named rather than left to the loop: the state that is normal and
        // must never read as a fault.
        assert_eq!(ironwire_state_tone("awaiting_rows"), StateTone::Held);
        assert_eq!(ironwire_state_tone("not_declared"), StateTone::Neutral);
        assert!(!ironwire_shows_last_checked("not_declared"));
    }

    /// The tone and the word are one decision, asserted over every input
    /// pair rather than on the three a screenshot would show.
    ///
    /// This is what lets all three shells style from [`tool_tone`] and
    /// never from the rendered string: if the two branch tables ever
    /// disagree, they disagree here first.
    #[test]
    fn the_reassuring_tone_falls_exactly_on_the_word_that_claims_privacy() {
        for mode in ["off", "watch", "unset", "", "OFF", "something_new"] {
            for wiring in [ToolWiring::Wired, ToolWiring::NotWired, ToolWiring::Unknown] {
                let word = tool_word(mode, wiring);
                let tone = tool_tone(mode, wiring);
                assert_eq!(
                    tone == ToolTone::Clear,
                    word == TOOL_PRIVATE,
                    "{mode:?}/{wiring:?} rendered {word:?} with {tone:?}"
                );
            }
        }
        // Named rather than left to the loop: the two cases a reader of the
        // screen would check.
        assert_eq!(tool_tone("watch", ToolWiring::Wired), ToolTone::Clear);
        assert_eq!(tool_tone("off", ToolWiring::Wired), ToolTone::Neutral);
    }

    /// The failure a real contributor hits, and the one fact that fixes
    /// it. A generic "check your configuration" would be useless here.
    #[test]
    fn the_unusable_file_line_names_the_file() {
        let line = ironwire_token_line(Some("/home/x/.ironwire/control.token"));
        assert!(line.contains("/home/x/.ironwire/control.token"), "{line}");
    }

    /// `probe_routing` omits `token_path` entirely when nothing resolved,
    /// so this line must stand on its own rather than print a hole.
    #[test]
    fn a_check_that_resolved_no_file_at_all_still_says_what_to_do() {
        let line = ironwire_token_line(None);
        assert!(!line.contains("None"), "{line}");
        assert!(!line.is_empty());
        assert_ne!(
            line,
            ironwire_token_line(Some("/home/x/.ironwire/control.token"))
        );
    }

    #[test]
    fn a_check_that_reached_nothing_names_the_port_it_tried() {
        let line = ironwire_unreachable_line(Some(8463));
        assert!(line.contains("8463"), "{line}");
        let nameless = ironwire_unreachable_line(None);
        assert!(!nameless.contains("None"), "{nameless}");
        assert_ne!(line, nameless);
    }

    /// Three states, three sentences, and the middle one is not a fault.
    #[test]
    fn declared_but_nothing_seen_yet_does_not_read_as_a_failure() {
        let waiting = ironwire_state_line("awaiting_rows");
        let lower = waiting.to_lowercase();
        for word in ["error", "failed", "problem", "wrong", "not working"] {
            assert!(!lower.contains(word), "{word} in: {waiting}");
        }
        assert!(lower.contains("normal"), "{waiting}");
        assert_ne!(waiting, ironwire_state_line("rows_seen"));
        assert_ne!(waiting, ironwire_state_line("not_declared"));
    }

    /// A state string this build does not know claims nothing. It must not
    /// fall through to either of the two "on" sentences.
    #[test]
    fn an_unreadable_state_says_nothing_rather_than_guessing() {
        assert_eq!(
            ironwire_state_line("something_new"),
            ironwire_state_line("not_declared")
        );
        assert_eq!(ironwire_state_line(""), ironwire_state_line("not_declared"));
    }

    /// `last_refresh_at` is per-process: it resets when the daemon
    /// restarts, so it is a "last checked" and never a "connected since"
    /// or an install date.
    ///
    /// The humanised time is the shell's; the sentence around it is not, so
    /// this pins the sentence against whatever a shell puts in the hole.
    #[test]
    fn the_last_check_is_never_shown_as_a_date_this_install_began() {
        let line = last_checked_line("an hour ago");
        assert_eq!(line, "Last checked an hour ago");
        let lower = line.to_lowercase();
        for word in ["since", "installed", "connected"] {
            assert!(!lower.contains(word), "{word} in: {line}");
        }
    }

    /// The literals this file's tools surface contains, read from the file
    /// rather than listed by hand.
    ///
    /// The list this replaced enumerated 23 names. It diverged: a constant
    /// added to the surface and rendered was simply absent from it, and the
    /// suite stayed green with two forbidden words on screen. "Enforced,
    /// not asserted" held only for the strings somebody had remembered to
    /// add.
    ///
    /// So this walks the region between the `TOOLS-SURFACE-BEGIN` and
    /// `TOOLS-SURFACE-END` markers and returns every string literal in it --
    /// constants and the bodies of the functions that build sentences alike.
    /// A new constant is covered the moment it is written, and moving one
    /// out of the region to dodge the sweep is a visible edit to a marker.
    ///
    /// Deliberately a scanner and not a regex: it has to skip `//` and
    /// `/* */` comments, because the prose in this region quotes forbidden
    /// words on purpose while explaining why they are forbidden.
    fn tools_surface_literals() -> Vec<String> {
        let source = include_str!("routing_copy.rs");
        let begin = source
            .find("// TOOLS-SURFACE-BEGIN")
            .expect("the tools surface must be marked");
        let end = source
            .find("// TOOLS-SURFACE-END")
            .expect("the tools surface must be closed");
        assert!(begin < end, "the markers must be in order");
        let region: Vec<char> = source[begin..end].chars().collect();

        let mut literals = Vec::new();
        let mut i = 0;
        while i < region.len() {
            match region[i] {
                '/' if i + 1 < region.len() && region[i + 1] == '/' => {
                    while i < region.len() && region[i] != '\n' {
                        i += 1;
                    }
                }
                '/' if i + 1 < region.len() && region[i + 1] == '*' => {
                    i += 2;
                    while i + 1 < region.len() && !(region[i] == '*' && region[i + 1] == '/') {
                        i += 1;
                    }
                    i += 2;
                }
                '"' => {
                    i += 1;
                    let mut literal = String::new();
                    while i < region.len() && region[i] != '"' {
                        if region[i] == '\\' {
                            // Only the escapes this file uses: a line
                            // continuation, whose payload is whitespace, and
                            // an escaped quote. Both are folded to nothing
                            // rather than decoded, because the sweep reads
                            // words and not punctuation.
                            i += 2;
                            continue;
                        }
                        literal.push(region[i]);
                        i += 1;
                    }
                    i += 1;
                    literals.push(literal);
                }
                _ => i += 1,
            }
        }
        literals
    }

    /// The scanner itself, checked against what it must and must not find.
    ///
    /// A sweep that silently found nothing would pass every assertion built
    /// on it, so this pins both ends: real constants are in, and the prose
    /// that quotes forbidden words while explaining them is out.
    #[test]
    fn the_surface_sweep_reads_the_literals_and_not_the_comments() {
        let literals = tools_surface_literals();
        assert!(
            literals.len() > 20,
            "the sweep found {} literals, which is not a surface",
            literals.len()
        );
        for expected in [
            TOOLS_HEADING,
            TOOL_PRIVATE,
            TOOL_DIRECT,
            TOOL_UNKNOWN,
            TOOL_NOT_USED,
            IRONWIRE_TOGGLE,
            IRONWIRE_APPLY,
        ] {
            assert!(
                literals.iter().any(|found| found == expected),
                "{expected:?} was not swept"
            );
        }
        // The region's own comments say "proxy" and "Private" repeatedly,
        // on purpose. If they were being swept, the rule below could never
        // be stated where it belongs.
        assert!(
            !literals.iter().any(|found| found.contains("local proxy")),
            "comment prose leaked into the sweep"
        );
    }

    /// The rule that governs this whole surface, asserted rather than
    /// promised. This app's user has no invite: they cannot reach a
    /// corpus, credits, ownership or contribution, and a locked door
    /// advertised is worse than no door. Nor may any of it name a restart,
    /// which Task 3 removed the need for.
    ///
    /// `"ironwire"` is on the list, and the probe path below is deliberately
    /// **not** an `~/.ironwire` one. The path in that sentence is the
    /// caller's argument, not our wording: a real one names the vendor and
    /// is allowed to, because the path is the single fact that makes a
    /// broken token file fixable. Sweeping the vendor's own path here would
    /// make the rule unstateable. Probing with a neutral path means the only
    /// way the word can reach this assertion is out of a string this module
    /// wrote, which is exactly what must never happen.
    ///
    /// The input is [`tools_surface_literals`], so this covers every string
    /// in the region and not a list somebody maintains beside it.
    #[test]
    fn the_tools_surface_says_nothing_it_should_not() {
        let mut strings = tools_surface_literals();
        // The constants as the shells actually receive them. The scanner
        // above reads source literals, and a `concat!` constant is several
        // of those -- so a forbidden word could in principle be assembled
        // across a join that neither fragment contains. These are the
        // finished strings.
        let payload = serde_json::to_value(routing_copy()).expect("the payload serialises");
        for (field, value) in payload.as_object().expect("a JSON object") {
            // `folder_note` carries a filesystem path this machine resolved,
            // and a path may spell a vendor's name -- the same exemption
            // `ironwire_token_line`'s argument has, and for the same reason:
            // a folder somebody is being sent to look at has to be the
            // folder that is really there. Its wording is swept below, with
            // a path chosen the way that function's is.
            if field == "folder_note" {
                continue;
            }
            strings.push(value.as_str().expect("every field is a string").to_string());
        }
        // The sentences the region's functions assemble, which exist only
        // once something has been formatted into them.
        strings.push(ironwire_folder_note(Some("/home/x/.config")));
        strings.push(ironwire_folder_note(None));
        strings.push(ironwire_token_line(Some("/home/x/.config/control.token")));
        strings.push(ironwire_token_line(None));
        strings.push(ironwire_unreachable_line(Some(8463)));
        strings.push(ironwire_unreachable_line(None));
        strings.push(last_checked_line("an hour ago"));
        for state in ["not_declared", "awaiting_rows", "rows_seen", "unknown"] {
            strings.push(ironwire_state_line(state).to_string());
        }
        for word in [
            "restart",
            "spent",
            "cost",
            "route",
            "backend",
            "proxy",
            "corpus",
            "share",
            "earn",
            "credit",
            "ironwire",
            "iron wire",
        ] {
            for text in &strings {
                assert!(
                    !text.to_lowercase().contains(word),
                    "{word:?} appears in: {text}"
                );
            }
        }
    }

    /// The sentences that name us take the name from the one definition.
    ///
    /// The forbidden-word sweep is a rule about what must not be said; this
    /// is the other half, and without it the surface could satisfy that
    /// rule by naming nobody at all. Asserted against
    /// [`crate::brand::APP_NAME`] rather than against the literal, so
    /// renaming the app in `brand.rs` moves these sentences with it and a
    /// hand-typed name here would fail.
    #[test]
    fn the_sentences_that_name_this_app_take_the_name_from_one_place() {
        let name = crate::brand::APP_NAME;
        for text in [
            IRONWIRE_INTRO,
            IRONWIRE_PROBE_REACHABLE,
            IRONWIRE_STATE_OFF,
            IRONWIRE_STATE_READING,
        ] {
            assert!(text.contains(name), "{name:?} is not in: {text}");
        }
        assert!(ironwire_token_line(Some("/home/x/.config/t")).contains(name));
        assert!(ironwire_token_line(None).contains(name));
    }

    /// The field every failure sentence sends a contributor to says which
    /// folder it is talking about.
    ///
    /// "Point the folder below at where the record is kept" and "Name the
    /// folder below" are instructions with an answer only if this note
    /// carries one. It used to say "somewhere other than the usual place",
    /// and nothing on the screen resolved that phrase.
    #[test]
    fn the_folder_note_names_the_folder_the_failure_lines_send_people_to() {
        let note = ironwire_folder_note(Some("/home/x/.config"));
        assert!(note.contains("/home/x/.config"), "{note}");
        assert!(!note.contains("the usual place"), "{note}");

        // The payload all three shells read carries the assembled sentence
        // and not the pathless constant, so this is not a GTK-only fix.
        assert_eq!(routing_copy().folder_note, ironwire_folder_note_here());

        // A machine that resolves no folder still gets a sentence rather
        // than an empty caption under the field.
        assert_eq!(ironwire_folder_note(None), IRONWIRE_FOLDER_NOTE);
        assert!(!ironwire_folder_note_here().is_empty());
    }

    /// The payload the shells read carries every constant on this surface.
    ///
    /// Counted, not listed. A constant added to the region and left out of
    /// [`RoutingCopy`] is a string that GTK renders and the other two shells
    /// cannot see -- and the shell that cannot see it is the shell that
    /// hand-writes its own, which is the drift this module exists to stop.
    /// Listing the fields here by hand would be the same failure one level
    /// up: the list would be the thing that went stale.
    #[test]
    fn every_constant_on_this_surface_is_in_the_payload_the_shells_read() {
        let source = include_str!("routing_copy.rs");
        let begin = source.find("// TOOLS-SURFACE-BEGIN").expect("marked");
        let end = source.find("// TOOLS-SURFACE-END").expect("closed");
        let declared = source[begin..end]
            .lines()
            .filter(|line| line.starts_with("pub const "))
            .count();

        let json = serde_json::to_value(routing_copy()).expect("the payload serialises");
        let exported = json.as_object().expect("a JSON object").len();

        assert_eq!(
            declared, exported,
            "{declared} constants on the surface but {exported} fields in RoutingCopy"
        );
    }

    /// Every exported value is one of this module's constants, and the four
    /// words arrive under the field a shell will read them from.
    ///
    /// The count above catches a constant that never crossed; this catches a
    /// field wired to the wrong constant, which the count cannot see.
    #[test]
    fn the_payload_carries_the_constants_themselves_and_not_a_transcription() {
        let copy = routing_copy();
        assert_eq!(copy.word_private, TOOL_PRIVATE);
        assert_eq!(copy.word_direct, TOOL_DIRECT);
        assert_eq!(copy.word_unknown, TOOL_UNKNOWN);
        assert_eq!(copy.word_not_used, TOOL_NOT_USED);
        assert_eq!(copy.tools_heading, TOOLS_HEADING);
        assert_eq!(copy.tool_claude, TOOL_CLAUDE);
        assert_eq!(copy.tool_codex, TOOL_CODEX);
        assert_eq!(copy.tool_gemini, TOOL_GEMINI);
        assert_eq!(copy.intro, IRONWIRE_INTRO);
        assert_eq!(copy.toggle, IRONWIRE_TOGGLE);
        assert_eq!(copy.applies_at_once, IRONWIRE_APPLIES_AT_ONCE);
        assert_eq!(copy.port_title, IRONWIRE_PORT_TITLE);
        assert_eq!(copy.port_note, IRONWIRE_PORT_NOTE);
        assert_eq!(copy.folder_title, IRONWIRE_FOLDER_TITLE);
        assert_eq!(copy.folder_note, ironwire_folder_note_here());
        assert_eq!(copy.apply, IRONWIRE_APPLY);
        assert_eq!(copy.checking, IRONWIRE_CHECKING);
        assert_eq!(copy.check_unavailable, IRONWIRE_CHECK_UNAVAILABLE);
        assert_eq!(copy.probe_reachable, IRONWIRE_PROBE_REACHABLE);
        assert_eq!(copy.state_off, IRONWIRE_STATE_OFF);
        assert_eq!(copy.state_waiting, IRONWIRE_STATE_WAITING);
        assert_eq!(copy.state_reading, IRONWIRE_STATE_READING);

        // And the JSON a shell actually reads carries those same values --
        // every field a string, none of them empty. An empty word would
        // render as a blank beside a tool name rather than as a failure.
        let json = serde_json::to_value(&copy).expect("serialises");
        for (field, value) in json.as_object().expect("a JSON object") {
            let text = value
                .as_str()
                .unwrap_or_else(|| panic!("{field} is not a string"));
            assert!(!text.is_empty(), "{field} is empty");
        }
        assert_eq!(json["word_private"], serde_json::json!(TOOL_PRIVATE));
        assert_eq!(json["word_direct"], serde_json::json!(TOOL_DIRECT));
        assert_eq!(json["word_unknown"], serde_json::json!(TOOL_UNKNOWN));
        assert_eq!(json["word_not_used"], serde_json::json!(TOOL_NOT_USED));
    }

    /// The state lines a shell reads out of the payload are the same ones
    /// [`ironwire_state_line`] picks, so a shell that maps the state itself
    /// cannot render a fourth sentence.
    #[test]
    fn the_state_lines_in_the_payload_are_the_ones_the_mapper_returns() {
        let copy = routing_copy();
        assert_eq!(ironwire_state_line("awaiting_rows"), copy.state_waiting);
        assert_eq!(ironwire_state_line("rows_seen"), copy.state_reading);
        assert_eq!(ironwire_state_line("not_declared"), copy.state_off);
    }
}
