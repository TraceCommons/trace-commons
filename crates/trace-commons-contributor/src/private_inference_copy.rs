//! The private-inference surface's words, in one place, for all three shells.
//!
//! `private_inference` starts a listener on this machine that answers the
//! model calls a contributor's tools make. Task 2 shipped the switch and the
//! state it reports; this module is every sentence printed about either, so
//! that the offer, the settings row and the state lines are written
//! once rather than three times.
//!
//! # What crosses the boundary
//!
//! The same contract [`crate::routing_copy`] states: both the vocabulary and
//! the sentences cross, and the sentences cross **already assembled**. The
//! one interpolated sentence here -- [`serving_line`] -- is finished on this
//! side from a port number, not handed to a shell as a template. A template a
//! shell fills in is a fourth place the wording can drift.
//!
//! The branch tables cross too. [`state_line`], [`state_tone`],
//! [`should_offer`], [`write_confirmed`] and [`quit_needs_notice`] each own a
//! shared decision, and each of
//! them is the kind of `switch` that has historically been written out again
//! in Swift, in C# and in Rust, and then disagreed in silence.
//!
//! # The sign-in row
//!
//! The destination can also hold a sign-in of its own, obtained by the
//! ceremony in [`crate::daemon::nearai_credential`], and its words are here
//! for the same reason the rest are: one row, three shells. Its branch
//! tables are [`credential_state_line`], [`credential_state_tone`] and
//! [`credential_action`], and the third of those is the one that matters
//! most -- it decides which BUTTON a state gets, and one of the buttons
//! opens a browser and mints a key at a company that is not this app.
//!
//! Two rules govern it. Nothing on it ever renders a key, a prefix, an id or
//! an account name. And a state that could not be read gets its own sentence
//! and NO action, rather than degrading to "no key here" -- which would
//! invite a contributor who already has one to mint a second.
//!
//! # The words this surface may not use
//!
//! Swept by `the_offer_surface_says_nothing_it_should_not`: no vendor name,
//! no "proxy"/"backend"/"route", and -- the important one -- no claim of
//! privacy, safety or encryption. Turning this on does not make a
//! contributor's calls private. It moves where they are answered from and
//! keeps the record here; each call still goes on to whoever was configured
//! to answer it.
//!
//! # The one exception: the product name
//!
//! The destination is called **Private AI**, and the sweep strips
//! [`DESTINATION`] before applying the ban so that name may be said and
//! nothing else may say "private".
//!
//! The rule was originally absolute, on the reasoning that the setting's
//! internal name says `private` and this surface must not repeat it as a
//! promise. The distinction that reopened it is between a promise and a name.
//! The mental model is a VPN: traffic goes through something the user
//! controls, which sees it and decides where it goes. "Private" in "VPN" has
//! never meant the destination cannot see you, and that is exactly the shape
//! of this feature.
//!
//! What the ban still forbids is any SENTENCE claiming privacy --
//! "your calls are private" remains false and remains unwriteable. The
//! subtitle says what actually happens, and [`OFFER_EXPOSURE`] still states
//! in full what turning this on lets anything else on the machine do. The
//! name carries the model; the sentences carry the truth.

// PRIVATE-INFERENCE-SURFACE-BEGIN
//
// Everything between this marker and its closing twin below is swept
// by `the_offer_surface_says_nothing_it_should_not`, which reads this file
// rather than a hand-kept list of names. A string literal added anywhere in
// this region is checked automatically; one added outside it is not.

/// The switcher label for the top-level destination this surface owns.
///
/// "Model calls" and not the setting's internal name: `private` is on the
/// list of words this surface may not say, because the feature makes no
/// privacy claim, and a nav label is the most-read string of the lot.
pub const DESTINATION: &str = "Private AI";

/// The one line under the destination's title saying what it is for.
pub const SUBTITLE: &str = "Your AI calls go out through this computer, so you can see them and \
     choose who answers.";

/// The offer's heading. Names the machine, because "on this computer" is the
/// whole of what changes and the only part a contributor can check.
pub const OFFER_TITLE: &str = "Answer model calls on this computer";

/// What saying yes does, with no claim attached to it.
///
/// The sentence deliberately ends by saying each call is still passed on. An
/// earlier draft stopped after "keep the record of them on this computer",
/// which reads as though the call never leaves -- the exact misreading
/// [`crate::routing_copy::TOOL_PRIVATE`]'s doc is careful about.
pub const OFFER_WHAT: &str = "This app can answer the model calls your tools make, from here, and keep \
     the record of them on this computer. Each call is still passed on to \
     whoever you have set up to answer it.";

/// What turning it on exposes. **Required, not optional.**
///
/// The listener's control side wants a token file; the answering side does
/// not. Anything that can reach loopback can therefore send calls through it,
/// charged to whatever accounts the home is configured with. A contributor
/// deciding on a single-user laptop and a contributor deciding on a shared
/// build box are answering different questions, and this is the sentence that
/// lets them tell which one they are.
pub const OFFER_EXPOSURE: &str = "While it is on, anything else running on this computer can send calls \
     through it as well, charged to the accounts you have set up here. On a \
     computer only you use that is your own software; on a shared one it is \
     anyone who can log in.";

/// Saying yes to this is not saying yes to repointing a tool.
pub const OFFER_NO_REPOINT: &str = "Turning this on does not change where any tool sends its calls. That \
     stays a separate choice, made one tool at a time.";

/// The accept button.
pub const OFFER_ACCEPT: &str = "Turn it on";

/// The decline button. "Not now" rather than "No", because the switch stays.
pub const OFFER_DECLINE: &str = "Not now";

/// Why the offer will not come back, said at the moment of the decision.
///
/// A contributor who declines is not asked again, and one who accepts is not
/// congratulated on the next launch either. Saying so here is what makes "Not
/// now" an honest button: it is not a deferral, it is an answer, and the way
/// back is the switch this sentence names.
///
/// It named Settings until the switch moved out of Settings to its own
/// destination, at which point the second clause was simply false -- pointing
/// a contributor who had just declined at a place the control is no longer in.
/// The first clause stays true here and only here: this is the first-run
/// offer, and the harness gate asks again on purpose, which is why that
/// surface does not show this sentence at all.
///
/// The destination is named as a literal because a payload sentence may not
/// interpolate, and `the_way_back_names_the_destination_it_is_in` pins it
/// against [`DESTINATION`] so the two cannot drift apart again.
pub const OFFER_ASKED_ONCE: &str = "Either way, this is the only time you will be asked. The switch is on the \
     Private AI screen.";

/// The settings section's heading.
pub const SETTINGS_TITLE: &str = "Private AI on this computer";

/// The settings switch.
pub const SETTINGS_TOGGLE: &str = "Answer model calls on this computer";

/// Changes are not deferred to a restart, and the line beneath the switch is
/// what actually happened rather than what was asked for.
pub const SETTINGS_APPLIES_AT_ONCE: &str =
    "Changes here apply straight away, and the line below says what happened.";

/// `off`.
pub const STATE_OFF: &str = "Off. This app is not answering model calls.";

/// A settings response without this status may come from an older daemon.
pub const STATE_UNREPORTED: &str = "This daemon does not report model-call status.";

/// An unrecognized daemon state is not evidence of shutdown.
pub const STATE_UNKNOWN: &str =
    "The current state is unavailable. Check again before relying on this app to answer calls.";

/// The switch records a request; retained ownership reports actual cleanup.
pub const STATE_STOPPING: &str =
    "Stopping. Waiting for any calls in progress and cleanup to finish.";

/// `running`.
pub const STATE_RUNNING: &str = "On. Calls sent to this computer are being answered.";

/// `running_answered_elsewhere`.
///
/// The sentence this whole surface was missing. Everything works: the
/// listener is up, a tool is connected, calls are answered. They are just not
/// answered by the destination the person believes, because the credentials
/// their tools already had are the ones being used, and nothing here has one
/// for DESTINATION.
///
/// Deliberately not phrased as a fault. For someone with no NEAR AI
/// relationship this is the correct resting state, not a failure, and calling
/// it an error would send them looking for something to repair. Its tone is
/// [`PrivateInferenceTone::Attention`] and never
/// [`PrivateInferenceTone::Clear`]: it is working, and it is not doing what
/// the reader thinks.
///
/// May only be shown on a fresh affirmative reading. It accuses the product
/// of not doing what a person expects, so a guess is worse here than silence
/// -- see [`STATE_RUNNING_DESTINATION_UNKNOWN`].
pub const STATE_RUNNING_ANSWERED_ELSEWHERE: &str = "On, and your calls are being answered using accounts already set up on \
     this computer. Nothing is set up here for Private AI to answer them \
     instead.";

/// `running_destination_unknown`.
///
/// Not knowing is its own answer and gets its own sentence. The two facts
/// this surface can report -- answered here, answered elsewhere -- are both
/// claims about where a person's work goes, and neither may be made up. When
/// the reading fails, this says the switch is on and stops there.
///
/// Its tone is [`PrivateInferenceTone::Attention`]. Painting it
/// [`PrivateInferenceTone::Clear`] would be the exact failure this state
/// exists to prevent: an unread destination shown as a working one.
pub const STATE_RUNNING_DESTINATION_UNKNOWN: &str = "On. Calls sent to this computer are being answered. Which account is \
     answering them could not be read just now.";

/// `running_no_backends`.
///
/// The state this vocabulary exists for. The listener is up and answers a
/// health check, and no call can pass through it, so anything painting this
/// the same as [`STATE_RUNNING`] would show a working light over something
/// that cannot work. Its tone is [`PrivateInferenceTone::Attention`] and
/// never [`PrivateInferenceTone::Clear`].
pub const STATE_RUNNING_NO_BACKENDS: &str = "On, but nothing is set up here for it to pass calls on to, so no call can \
     get through it yet.";

/// `running_elsewhere`.
///
/// Not a fault and not this app's doing. Something already holds the place
/// this would have taken; it was left alone, and nothing here was started or
/// stopped. Saying "left alone" rather than "already on" matters: a
/// contributor reading "already on" would go looking in this app's settings
/// for something this app does not control.
pub const STATE_RUNNING_ELSEWHERE: &str = "Another program is using this computer's model-call setup. This \
     app started nothing and stopped nothing; whether calls can get through is not confirmed.";

/// `port_in_use`.
pub const STATE_PORT_IN_USE: &str = "Not on. Something else on this computer is holding the number this needs. \
     Free it up, then turn this off and on again.";

/// `start_failed`.
pub const STATE_START_FAILED: &str =
    "Not on. It would not start. Turn this off and on again to try once more.";

/// `crashed`.
///
/// Sticky on purpose, and the sentence says so rather than leaving a
/// contributor to discover it. A listener that cannot stay up and is retried
/// on every poll tick reads as a light flickering, which is how a real fault
/// becomes invisible.
pub const STATE_CRASHED: &str = "The model-call state could not be confirmed. It may have stopped unexpectedly \
     or cleanup may still be pending. It will not retry by itself. Turn this off and on again to retry; this app will not start \
     another listener while the previous instance still owns its setup.";

/// Said at the moment of quitting, on the two platforms where the app is the
/// daemon.
///
/// Task 3's plan carries this as a requirement: the existing quit
/// confirmation explains that quitting stops the watcher, and with the
/// listener inside the same process it now stops that too. A shell appends
/// this to its own confirmation only when the switch is on -- a contributor
/// who never turned it on should not be warned about losing it.
pub const QUIT_ALSO_STOPS: &str = "Quitting also ends any model calls still handled by this app. Tools pointed \
     here cannot get answers until this app is open and answering.";

/// The reported local port, without claiming readiness, assembled rather than exported as a
/// template with a hole in it.
///
/// A port outside 1..=65535 -- including the `0` a caller passes when there is
/// no port -- produces the empty string rather than a sentence naming a number
/// nobody bound. A shell shows nothing for an empty string; the state line
/// above it has already said everything that is true.
#[must_use]
pub fn serving_line(port: Option<u16>) -> String {
    match port {
        Some(port) if port != 0 => format!("Reported local listener number: {port}."),
        _ => String::new(),
    }
}

/// How firmly one state reads.
///
/// Five values and not four, because this surface has a refusal and the
/// routing surface does not. `port_in_use`, `start_failed` and `crashed` are
/// each "you asked for this, it is not happening, and here is the way out",
/// which is neither [`Self::Attention`]'s "something here wants a look" nor
/// [`Self::Neutral`]'s silence.
///
/// The numbering that crosses the C ABI is deliberately disjoint from both
/// `TC_ROUTING_TONE_*` (0..=3) and `TC_WITNESS_TONE_*` (10..=14), for the
/// reason the witness header states: a shell that cross-wired two mappers
/// with overlapping ranges would render a refusal as "nothing to say", and a
/// disjoint range makes that mistake wrong for every value rather than only
/// for the dangerous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateInferenceTone {
    /// No readiness or shutdown confirmation is claimed.
    Neutral,
    /// Ownership or cleanup is held; readiness is not claimed.
    Held,
    /// On, answering, and with somewhere to pass calls on to. The only value
    /// that may be painted as working.
    Clear,
    /// On, and something on this machine wants attention before a call can
    /// get through.
    Attention,
    /// Asked for and not happening. Always paired with a sentence naming the
    /// way out.
    Refused,
}

impl PrivateInferenceTone {
    /// Whether an indicator may paint this tone as working. [`Self::Clear`]
    /// alone.
    ///
    /// The one predicate every shell indicator asks -- a tab badge, a tray
    /// glyph, a menu-bar dot. Painting [`Self::Refused`] or [`Self::Held`] as
    /// on is the fail-open this surface exists to prevent, and asking the
    /// settings switch instead is the other way to arrive at it: the switch
    /// says what was asked for, this says what is true.
    #[must_use]
    pub fn reads_as_working(self) -> bool {
        matches!(self, PrivateInferenceTone::Clear)
    }
}

/// The sentence for one state label, as reported by `private_inference_state`.
///
/// Missing status is unreported; unfamiliar nonempty states are unavailable.
/// They must not claim that a listener has stopped or can answer calls.
#[must_use]
pub fn state_line(label: &str) -> &'static str {
    match label {
        "" => STATE_UNREPORTED,
        LABEL_OFF => STATE_OFF,
        LABEL_STOPPING => STATE_STOPPING,
        LABEL_RUNNING => STATE_RUNNING,
        LABEL_RUNNING_NO_BACKENDS => STATE_RUNNING_NO_BACKENDS,
        LABEL_RUNNING_ANSWERED_ELSEWHERE => STATE_RUNNING_ANSWERED_ELSEWHERE,
        LABEL_RUNNING_DESTINATION_UNKNOWN => STATE_RUNNING_DESTINATION_UNKNOWN,
        LABEL_RUNNING_ELSEWHERE => STATE_RUNNING_ELSEWHERE,
        LABEL_PORT_IN_USE => STATE_PORT_IN_USE,
        LABEL_START_FAILED => STATE_START_FAILED,
        LABEL_CRASHED => STATE_CRASHED,
        _ => STATE_UNKNOWN,
    }
}

/// The tone [`state_line`]'s sentence is painted in.
///
/// ONE BRANCH TABLE, NOT FOUR. This takes what the sentence takes, so the two
/// stay in step by construction, and no shell may recover it by comparing the
/// rendered sentence against one of the constants.
///
/// An unknown label is [`PrivateInferenceTone::Neutral`], matching its
/// sentence. Neutral is the safe direction here because the dangerous value on
/// this surface is [`PrivateInferenceTone::Clear`]: a state nobody has words
/// for must never be painted as working.
#[must_use]
pub fn state_tone(label: &str) -> PrivateInferenceTone {
    match label {
        LABEL_RUNNING => PrivateInferenceTone::Clear,
        LABEL_RUNNING_NO_BACKENDS
        // Both are working states that are not doing what the reader
        // believes, which is what `Attention` is for. Mapped explicitly
        // rather than left to the `Neutral` default below: degrading to a
        // tone that happens not to be `Clear` would be right by accident,
        // and the accident stops holding the moment somebody reorders this.
        | LABEL_RUNNING_ANSWERED_ELSEWHERE
        | LABEL_RUNNING_DESTINATION_UNKNOWN => PrivateInferenceTone::Attention,
        LABEL_RUNNING_ELSEWHERE | LABEL_STOPPING => PrivateInferenceTone::Held,
        LABEL_PORT_IN_USE | LABEL_START_FAILED | LABEL_CRASHED => PrivateInferenceTone::Refused,
        _ => PrivateInferenceTone::Neutral,
    }
}

/// Whether a shell should put the offer in front of the contributor.
///
/// Two inputs and one rule, crossing the ABI for the reason the tone table
/// does: three shells each deciding when to interrupt somebody is three
/// chances to nag a contributor who already said no.
///
/// - `answered` is the persisted `private_inference_offer_seen` setting. It is
///   written by *either* answer, so declining is remembered exactly as
///   accepting is. Its `serde(default)` is what makes the offer appear on the
///   first start after an upgrade as well as on a fresh install: a settings
///   file written before the key existed loads with it false.
/// - `on` is the `private_inference` switch. Somebody who already turned it on
///   -- by editing the settings file, or from another shell -- is not offered
///   something they have.
///
/// # The switch enabled out of band, and why the marker is not set for it
///
/// A contributor who turns this on by hand -- editing `daemon-settings.json`,
/// or calling `set_settings` from the CLI -- is never offered it, and so never
/// meets [`OFFER_EXPOSURE`] *on the offer*. Turning it off again by hand then
/// surfaces the offer, because the question genuinely has not been put.
///
/// That is the intended behaviour, and the alternative was considered and
/// rejected: having the daemon write `private_inference_offer_seen = true`
/// whenever it observes the switch on would record that a question was asked
/// when none was. The key's entire contract is that it marks an *asking*, and
/// a shell reading back a marker it could not distinguish from an inference
/// would stop being able to tell an answered contributor from an unasked one.
/// It would also be the daemon writing a settings key nobody asked it to
/// write.
///
/// What closes the gap instead is that every shell puts [`OFFER_EXPOSURE`] on
/// the settings card as well as in the offer. Somebody who enabled this out of
/// band did so from a settings surface, and that sentence is on it.
#[must_use]
pub fn should_offer(answered: bool, on: bool) -> bool {
    !answered && !on
}

/// A write is acknowledged only by a successful daemon echo of its marker
/// and any explicitly requested switch value. `None` is a marker-only decline;
/// missing echoed values never stand in for an explicit false.
#[must_use]
pub fn write_confirmed(
    requested_on: Option<bool>,
    echoed_seen: Option<bool>,
    echoed_on: Option<bool>,
) -> bool {
    echoed_seen == Some(true) && requested_on.is_none_or(|on| echoed_on == Some(on))
}

/// A transport failure may arrive after persistence; do not claim nothing changed.
pub const WRITE_UNCONFIRMED: &str =
    "The change could not be confirmed. Check the app's status and try again.";

/// Whether quitting may end this app's model-call work. Requested off does
/// not prove cleanup completed; foreign ownership is never this app's work.
#[must_use]
pub fn quit_needs_notice(requested_on: bool, label: &str) -> bool {
    match label {
        LABEL_OFF | LABEL_RUNNING_ELSEWHERE => false,
        LABEL_RUNNING | LABEL_RUNNING_NO_BACKENDS | LABEL_STOPPING => true,
        _ => requested_on,
    }
}

/// Every fixed string on this surface, in one payload.
///
/// Shaped for the C ABI: `tc_private_inference_copy` serialises this and hands
/// a shell one owned JSON object. One call and not one per string, for the
/// reason `tc_routing_copy` gives -- a per-string export lets a shell take
/// four of the sentences and hand-write the fifth, and the hand-written one
/// here would be the exposure sentence.
///
/// The state sentences are in the payload *and* reachable through
/// [`state_line`]. A shell renders them through the branch table; they are
/// carried here as well so a test on the far side can pin the set it was built
/// against.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct PrivateInferenceCopy {
    pub destination: &'static str,
    pub subtitle: &'static str,
    pub offer_title: &'static str,
    pub offer_what: &'static str,
    pub offer_exposure: &'static str,
    pub offer_no_repoint: &'static str,
    pub offer_accept: &'static str,
    pub offer_decline: &'static str,
    pub offer_asked_once: &'static str,
    pub settings_title: &'static str,
    pub settings_toggle: &'static str,
    pub settings_applies_at_once: &'static str,
    pub state_off: &'static str,
    pub state_unknown: &'static str,
    pub state_unreported: &'static str,
    pub state_stopping: &'static str,
    pub state_running: &'static str,
    pub state_running_no_backends: &'static str,
    /// [`STATE_RUNNING_ANSWERED_ELSEWHERE`].
    pub state_running_answered_elsewhere: &'static str,
    /// [`STATE_RUNNING_DESTINATION_UNKNOWN`].
    pub state_running_destination_unknown: &'static str,
    pub state_running_elsewhere: &'static str,
    pub state_port_in_use: &'static str,
    pub state_start_failed: &'static str,
    pub state_crashed: &'static str,
    pub quit_also_stops: &'static str,
    pub write_unconfirmed: &'static str,
    pub settings_moved: &'static str,
    pub tray_turn_off: &'static str,
    pub tray_open_to_turn_on: &'static str,
    pub harnesses_title: &'static str,
    pub harnesses_what: &'static str,
    pub harnesses_spend_scope: &'static str,
    pub harness_not_installed: &'static str,
    pub harness_not_connected: &'static str,
    pub harness_connected_nothing_seen: &'static str,
    pub harness_answering: &'static str,
    pub harness_connect: &'static str,
    pub harness_disconnect: &'static str,
    pub harness_preview_title: &'static str,
    pub harness_preview_confirm: &'static str,
    pub harness_preview_cancel: &'static str,
    pub harness_slot_taken: &'static str,
    pub harness_needs_restart: &'static str,
    pub harnesses_none_found: &'static str,
    pub harness_unreadable_config: &'static str,
    pub harness_plan_nothing_to_change: &'static str,
    pub harness_plan_entry_unusable: &'static str,
    pub harness_plan_no_config_path: &'static str,
    pub credential_title: &'static str,
    pub credential_what: &'static str,
    pub credential_cost: &'static str,
    pub credential_obtain: &'static str,
    pub credential_cancel: &'static str,
    pub credential_forget: &'static str,
    pub credential_forget_explains: &'static str,
    pub credential_absent: &'static str,
    pub credential_obtaining: &'static str,
    pub credential_failed: &'static str,
    pub credential_cancelled: &'static str,
    pub credential_present: &'static str,
    /// [`CREDENTIAL_UNKNOWN`]. An unread state, never a stand-in for
    /// [`CREDENTIAL_ABSENT`].
    pub credential_unknown: &'static str,
    pub credential_unreported: &'static str,
    /// [`HARNESS_NEEDS_CREDENTIAL`]. Drawn only through
    /// [`harness_credential_notice`], never on a shell's own reading of a
    /// boolean.
    pub harness_needs_credential: &'static str,
    /// The four `eligibility` states a queue entry can carry, and the
    /// thirteen `eligibility_reason` labels. Rendered through
    /// [`eligibility_state_line`] and [`eligibility_reason_line`]; carried
    /// here as well so a test on the far side can pin the set it was built
    /// against.
    pub eligibility_eligible: &'static str,
    pub eligibility_ineligible_permanent: &'static str,
    pub eligibility_ineligible_configuration: &'static str,
    pub eligibility_unknown: &'static str,
    pub eligibility_reason_no_call: &'static str,
    pub eligibility_reason_capture_off: &'static str,
    pub eligibility_reason_digest_absent: &'static str,
    pub eligibility_reason_upstream_id_absent: &'static str,
    pub eligibility_reason_digest_mismatch: &'static str,
    pub eligibility_reason_reference_malformed: &'static str,
    pub eligibility_reason_bodies_unreadable: &'static str,
    pub eligibility_reason_body_not_utf8: &'static str,
    pub eligibility_reason_body_too_large: &'static str,
    pub eligibility_reason_evidence_capture_off: &'static str,
    pub eligibility_reason_marker_absent: &'static str,
    pub eligibility_reason_request_malformed: &'static str,
    pub eligibility_reason_receipt_unavailable: &'static str,
}

/// The sentence the settings card shows once the control has moved out of it.
///
/// The card stays: a contributor who learned where the switch was should find
/// a pointer there, not a hole.
pub const SETTINGS_MOVED: &str = "Private AI has its own screen now.";

/// The tray action while it is on.
///
/// Turning it OFF from a menu is safe in a way turning it on is not: it only
/// ever reduces what this computer will answer, so it needs no sentence in
/// front of it.
pub const TRAY_TURN_OFF: &str = "Stop answering model calls";

/// The tray action while it is off.
///
/// Trailing ellipsis because it opens the screen rather than acting: turning
/// it ON changes what anything else on this computer may send through, and
/// that is not a decision to take from a menu with the consequence off-screen.
pub const TRAY_OPEN_TO_TURN_ON: &str = "Answer model calls on this computer…";

/// The heading over the list of tools found on this computer.
///
/// The destination leads with this list rather than with the switch, because
/// a tool is the unit a contributor can decide about. Answering model calls
/// at all is a consequence of connecting one, not a question to be settled
/// first.
pub const HARNESSES_TITLE: &str = "Tools on this computer";

/// The one line under that heading.
///
/// Two things have to be said in it. That the choice is per tool -- the same
/// promise [`OFFER_NO_REPOINT`] makes -- and that the list is what this app
/// knows how to look for, not a claim about every tool that exists. An
/// unqualified list reads as the second, and a contributor whose tool is
/// missing from it would conclude their tool cannot be connected rather than
/// that this app has not been taught about it yet.
pub const HARNESSES_WHAT: &str = "Each of these can be set to send its model calls to this computer, one \
     tool at a time. The list is what this app knows how to look for, not \
     every tool there is.";

/// What the amount [`harness_spend_line`] names does and does not cover.
///
/// **A number without this sentence is a lie of omission.** The figure is
/// measured where the calls were answered -- here -- and a contributor
/// signed in to the same account on a second computer, or in a browser, has
/// spent more than it says. Stating the scope is the same duty
/// [`HARNESSES_WHAT`] discharges for the list above it, which says what this
/// app knows how to look for rather than implying it knows about everything.
///
/// The second omission is subtler and needs saying just as plainly: work
/// served by something already paid for by the month is not added in. That
/// is deliberate upstream -- summing it would produce a figure for a day on
/// which nothing was billed -- but a contributor who saw a busy afternoon
/// come back as nothing would conclude the number was broken rather than
/// that it was narrow.
pub const HARNESSES_SPEND_SCOPE: &str = "Only calls answered on this computer are in that figure. Calls answered \
     on another machine, or in a browser, are not, and neither is work a \
     monthly plan has already paid for.";

/// A tool this app could not find on this computer.
///
/// **Listed, not hidden.** A tool left out of the list cannot be told apart
/// from a tool this app has never been taught about, and the contributor's
/// question in both cases is the same one. So the row stays and says what
/// was looked for.
///
/// It must be rendered INSTEAD of [`HARNESS_NOT_CONNECTED`] and never
/// beside it. That sentence says a tool's own settings still send its calls
/// somewhere, which is a claim about settings belonging to something that is
/// not on this computer at all.
pub const HARNESS_NOT_INSTALLED: &str = "Not found on this computer. This app looked where this tool keeps its \
     own settings and found nothing there, so there is nothing here to \
     connect.";

/// A tool whose own settings still send its calls wherever they went before.
///
/// Said as a fact about the tool's settings rather than as a fault. Nothing
/// is wrong with a tool nobody has connected.
pub const HARNESS_NOT_CONNECTED: &str = "Not connected. Its own settings still send its calls wherever they went \
     before.";

/// A tool whose settings are right and from which nothing has arrived yet.
///
/// The state this three-way split exists for. A settings file with the right
/// value in it is not evidence that a single call was ever answered, and a
/// surface that showed those two the same way would tell a contributor their
/// tool was working while it sent every call somewhere else.
pub const HARNESS_CONNECTED_NOTHING_SEEN: &str = "Connected, and nothing has arrived from it yet. Its settings send its \
     calls here; none has come in so far.";

/// A tool a call actually arrived from. The only state that means it works.
///
/// Paired with [`harness_last_call_line`], which says when. The sentence
/// stops at what this computer did, because which tool a call came from is
/// worked out from how the call was phrased, and two tools that phrase calls
/// the same way cannot be told apart.
pub const HARNESS_ANSWERING: &str =
    "Answering. A call from it reached this computer and was answered here.";

/// The action that connects one tool.
pub const HARNESS_CONNECT: &str = "Send this tool's calls here";

/// The action that disconnects one tool.
///
/// Says what the tool stops doing, not what this app stops doing: the file
/// being changed is the tool's, and the listener is left exactly as it was
/// for every other tool.
pub const HARNESS_DISCONNECT: &str = "Stop sending this tool's calls here";

/// The heading over the preview shown before anything is written.
///
/// This app is about to edit a file it does not own, so the change is shown
/// before it is made. The same reason the destination exists at all: the
/// consequence is stated where the decision is taken.
pub const HARNESS_PREVIEW_TITLE: &str = "What would change in this tool's own settings file";

/// The button that writes the change.
pub const HARNESS_PREVIEW_CONFIRM: &str = "Make this change";

/// The button that does not.
///
/// Not "Cancel". The file is the contributor's, and the outcome of saying no
/// is that it keeps every value it has.
pub const HARNESS_PREVIEW_CANCEL: &str = "Leave the file as it is";

/// A slot that already had a value in it, which was left alone.
///
/// **Not a fault, and not an offer.** The value in that slot is somebody's
/// deliberate choice, and taking it over would move their calls without
/// telling them. So this sentence reports what was left alone and stops
/// there: it must not read as an error to be cleared, and it must not
/// suggest that this app could take the slot if asked.
pub const HARNESS_SLOT_TAKEN: &str = "This tool is already set to send those calls somewhere, so that setting \
     was left exactly as you had it. Nothing here changed it, and nothing \
     here will.";

/// A tool holding an old setting in a process that is still running.
///
/// Kept in front of the contributor until a call actually arrives, because
/// the alternative is a list claiming a tool sends its calls here while the
/// window in front of them does not.
pub const HARNESS_NEEDS_RESTART: &str = "Its settings changed while it was running. Quit this tool and open it \
     again; until then the copy of it that is running still has the old \
     setting.";

/// No tool was found.
///
/// Says what was looked for. An empty list that explains nothing cannot be
/// told apart from a broken one, and the contributor's next question is
/// always which tools were even considered.
pub const HARNESSES_NONE_FOUND: &str = "None of the tools this app knows about was found here. It looked for \
     each of them in the place that tool keeps its own settings, and found \
     no settings file to work with.";

/// A settings file that could not be read, and was therefore not touched.
///
/// Distinct from having nothing to change, and the distinction is the whole
/// point of the sentence. A file this app cannot make sense of might already
/// say the right thing or nothing at all; either way it is refused, so that
/// somebody's own mistake in their own file never comes back looking like
/// this app's.
pub const HARNESS_UNREADABLE_CONFIG: &str = "This app could not make sense of the settings file named above, so it \
     changed nothing in it. This is a refusal, not a file that already said \
     the right thing: open it yourself, or use the command shown, and the \
     file stays exactly as it is until you do.";

/// A plan that found the file already saying what the action wanted.
///
/// **Not a failure, and not a blank sheet.** Before this sentence existed a
/// plan with nothing in it opened a preview holding a title, a path and
/// nothing else, which reads as this app having lost the change rather than
/// as there being none to make.
pub const HARNESS_PLAN_NOTHING_TO_CHANGE: &str = "There is nothing to change. This tool's own settings file already says \
     what this would have written, so it was left exactly as it is.";

/// This app's own description of a tool did not survive checking.
///
/// Says whose fault it is, because the file named above is the
/// contributor's and this one is not their doing. A refusal that let them
/// think their own file was at fault would send them to edit something that
/// is already right.
pub const HARNESS_PLAN_ENTRY_UNUSABLE: &str = "This app's own description of this tool did not hold up when it was \
     checked, so nothing was worked out and nothing was written. That is a \
     fault here, not in any file of yours.";

/// This build could not work out where a tool keeps its settings.
///
/// Names the way forward rather than stopping at the refusal: the command
/// shown on the row does by hand exactly what this app could not work out
/// how to do.
pub const HARNESS_PLAN_NO_CONFIG_PATH: &str = "This app could not work out where this tool keeps its own settings on \
     this computer, so it has no file to change. The command shown on the \
     tool does the same thing by hand.";

/// Why a tool this app hosts the answering for cannot be connected yet.
///
/// **A next step, not a fault.** A contributor who has not signed in has done
/// nothing wrong, and the sentence is shaped the way
/// [`STATE_RUNNING_ANSWERED_ELSEWHERE`] is: here is what is true, here is what
/// to do about it. Without it a shell simply hides the connect control, which
/// is the silent dead end -- no way to connect a tool and no reason given.
///
/// **It is scoped to a destination this app hosts, and must stay that way.**
/// A proxy the contributor declared and runs themselves answers from an
/// account whose key was never handed to us; `destination_credentialed` is
/// true for it, this sentence is never drawn for it, and the wording does not
/// claim a tool needs our sign-in in general. It says what would answer, and
/// what is missing from the thing that would answer.
///
/// The next step is spelled the way the button that performs it is spelled --
/// see [`CREDENTIAL_OBTAIN`] -- so a contributor reading this sentence is
/// looking for words that exist somewhere on the screen.
pub const HARNESS_NEEDS_CREDENTIAL: &str = "This computer would be the one answering this tool's calls, and no key \
     is kept here to answer them with yet. Sign in to Private AI first, and \
     this tool can be connected after that.";

/// The sentence for one tool's state, or the empty string.
///
/// ONE TABLE, NOT THREE. Every shell used to hold its own map from a
/// `harness_list` row's `state` onto one of the sentences above -- Swift,
/// C# and Rust, three copies of one decision, agreeing today and drifting
/// in silence tomorrow. This is that decision, in the only place it may
/// live; the shells reach it through `tc_harness_state_line`.
///
/// Two states answer the empty string, which a shell draws as no line at
/// all, and the emptiness is the point:
///
/// - [`HarnessState::ActivityShared`] must not borrow [`HARNESS_ANSWERING`].
///   That sentence says a call from *it* was answered here, and the pronoun
///   names the row's own tool -- which is precisely what this state says
///   cannot be worked out. Nor may it borrow
///   [`HARNESS_CONNECTED_NOTHING_SEEN`], which would be false: something did
///   arrive.
/// - [`HarnessState::Unknown`], and a label this build has never heard of,
///   claim nothing rather than take the nearest sentence.
///
/// A row with no state line claims nothing, and claiming nothing is the
/// honest answer to a question the ledger cannot settle.
///
/// [`HarnessState::ActivityShared`]: crate::harness_state::HarnessState::ActivityShared
/// [`HarnessState::Unknown`]: crate::harness_state::HarnessState::Unknown
#[must_use]
pub fn harness_state_line(state: &str) -> &'static str {
    use crate::harness_state::HarnessState;
    match HarnessState::from_label(state) {
        Some(HarnessState::NotConnected) => HARNESS_NOT_CONNECTED,
        Some(HarnessState::ConnectedNoCalls) => HARNESS_CONNECTED_NOTHING_SEEN,
        Some(HarnessState::Answering) => HARNESS_ANSWERING,
        Some(HarnessState::ActivityShared | HarnessState::Unknown) | None => "",
    }
}

/// The sentence explaining a connect that is not on offer, or the empty
/// string.
///
/// `credentialed` is `harness_list`'s `destination_credentialed`. It is an
/// `Option` and not a `bool` because the three answers are three different
/// facts, and the third is the one a shell would get wrong on its own:
///
/// - `Some(false)` -- this app hosts the answering and holds no key. The
///   connect control is not on offer and [`HARNESS_NEEDS_CREDENTIAL`] says
///   why.
/// - `Some(true)` -- nothing to explain. Either a key is here, or the
///   destination is one the contributor runs themselves, which this gate has
///   no business asking about.
/// - `None` -- the field was not in the response, so this daemon does not
///   gate connects at all. THE EMPTY STRING, never the sentence: telling
///   somebody to sign in before connecting would be false on a build where
///   connecting needs no sign-in.
///
/// A shell draws this once, beside the connect controls, rather than once per
/// row -- the fact is about the destination and not about any one tool.
#[must_use]
pub fn harness_credential_notice(credentialed: Option<bool>) -> &'static str {
    match credentialed {
        Some(false) => HARNESS_NEEDS_CREDENTIAL,
        Some(true) | None => "",
    }
}

/// The sentence a plan's outcome carries, or the empty string.
///
/// ONE TABLE, NOT THREE -- the same rule [`harness_state_line`] follows, for
/// the same reason, and reached by the shells through
/// `tc_harness_outcome_line`. Before this existed only `unparseable` had a
/// sentence anywhere, and each shell wrote that one arm itself: macOS in
/// `outcomeSentence`, Windows in a `== Unparseable` branch, GNOME in a
/// `Some(PlanOutcome::Unparseable)` arm. Three copies of one decision.
///
/// [`PlanOutcome::Changes`] answers the EMPTY STRING, and the emptiness is
/// the point: a plan with changes in it shows them, and a sentence above
/// them saying there are changes would be this app narrating its own list.
/// Every other outcome answers a sentence, because every other outcome
/// writes nothing, and a preview that says nothing about why is a dialog
/// holding a title, a path and a way out.
///
/// A label this build has never heard of also answers the empty string. It
/// may not borrow the nearest sentence: guessing which refusal an unknown
/// outcome is would tell a contributor something nobody worked out.
///
/// [`PlanOutcome::Changes`]: crate::harness_state::PlanOutcome::Changes
#[must_use]
pub fn harness_outcome_line(outcome: &str) -> &'static str {
    use crate::harness_state::PlanOutcome;
    match PlanOutcome::from_label(outcome) {
        Some(PlanOutcome::Changes) | None => "",
        Some(PlanOutcome::Noop) => HARNESS_PLAN_NOTHING_TO_CHANGE,
        Some(PlanOutcome::Unparseable) => HARNESS_UNREADABLE_CONFIG,
        Some(PlanOutcome::NotInstalled) => HARNESS_NOT_INSTALLED,
        Some(PlanOutcome::EntryUnusable) => HARNESS_PLAN_ENTRY_UNUSABLE,
        Some(PlanOutcome::NoConfigPath) => HARNESS_PLAN_NO_CONFIG_PATH,
    }
}

/// When the last call from a connected tool was answered here, assembled on
/// this side rather than exported as a sentence with a hole in it.
///
/// The same rule [`serving_line`] follows, for the same reason: a shell
/// handed a template is a fourth place the wording can drift. `None` -- and
/// nothing to report -- produces the empty string, which a shell draws as no
/// line at all, because [`HARNESS_ANSWERING`] above it has already said the
/// part that is true.
///
/// The buckets are coarse on purpose. A timestamp to the second invites a
/// contributor to read it as a live count of calls; all this sentence is for
/// is settling whether anything has ever come through.
#[must_use]
pub fn harness_last_call_line(seconds_ago: Option<u64>) -> String {
    let Some(seconds) = seconds_ago else {
        return String::new();
    };
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let (count, unit) = if seconds < 60 {
        return "Last call answered here: just now.".to_string();
    } else if minutes < 60 {
        (minutes, "minute")
    } else if hours < 24 {
        (hours, "hour")
    } else {
        (days, "day")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("Last call answered here: {count} {unit}{plural} ago.")
}

/// What the calls answered here have cost since midnight, assembled on this
/// side, or the empty string.
///
/// `micros` is millionths of a dollar. An integer rather than a float
/// because it crosses the C ABI as one, and because a sentence about money
/// should round exactly once, here, rather than once per shell.
///
/// # `None` is not zero
///
/// `None` means nothing could be read -- no ledger, an answer that did not
/// arrive, an answer this build could not parse -- and it produces the empty
/// string, which a shell draws as no line at all. It must never be rendered
/// as `$0.00`. A day on which nothing was spent and a day nobody could
/// measure are different facts, and collapsing them would put a confident
/// figure in front of a contributor that nothing supports. It is the same
/// rule [`harness_state_line`] follows for a state this build has no words
/// for, and the same rule [`harness_last_call_line`] follows for a call
/// nobody saw.
///
/// # The window is in the sentence
///
/// Since the most recent midnight, which is the window the figure is kept
/// over. A sentence that named an amount without naming its window would
/// invite it to be read as a total, exactly as `ACTIVITY_WINDOW_HOURS` would
/// be misread if the state sentence left it out.
///
/// The scope the figure does *not* cover is [`HARNESSES_SPEND_SCOPE`], drawn
/// beside this sentence and only when this sentence is drawn at all.
#[must_use]
pub fn harness_spend_line(micros: Option<u64>) -> String {
    let Some(micros) = micros else {
        return String::new();
    };
    // Rounded to cents ONCE, before splitting. Dividing first and rounding
    // the remainder separately carries into a third digit -- 1_999_999
    // micros renders as "$1.100" -- which is the shape of a bug a
    // contributor would read as a price.
    let cents = (micros + 5_000) / 10_000;
    if cents == 0 && micros > 0 {
        // Too small to show, and not nothing. Rounding it down into the
        // sentence a day with no calls gets would say something false about
        // a day that had them.
        return "Cost of calls answered here since midnight: less than $0.01.".to_string();
    }
    format!(
        "Cost of calls answered here since midnight: ${}.{:02}.",
        cents / 100,
        cents % 100
    )
}

/// The heading over the sign-in card on the destination.
///
/// Names the machine for the reason [`OFFER_TITLE`] does: what changes is
/// what this computer holds, and that is the only part a contributor can go
/// and check.
pub const CREDENTIAL_TITLE: &str = "A Private AI sign-in on this computer";

/// Why the card is there at all.
///
/// Stops at what is missing and what can be done about it. It does not say
/// that calls fail without one -- they do not; they are answered using
/// whatever accounts a contributor's tools already had, which is what
/// [`STATE_RUNNING_ANSWERED_ELSEWHERE`] says on the state row above.
pub const CREDENTIAL_WHAT: &str = "For Private AI to answer your calls, this computer needs a sign-in of \
     its own. This app can get one and keep it here.";

/// What getting one actually costs. **Required, not optional.**
///
/// The counterpart of [`OFFER_EXPOSURE`], and it exists for the same reason:
/// a button that opens a browser, signs somebody in to a company that is not
/// this app, and mints a key this app then keeps, is not one frictionless
/// tap, and a surface that draws it as one has decided on the contributor's
/// behalf.
///
/// The last clause is the part a contributor needs later rather than now:
/// the key is theirs, it is listed in their own account, and removing it
/// there is the thing this app cannot do for them --
/// [`CREDENTIAL_FORGET_EXPLAINS`] says so again at the moment it matters.
pub const CREDENTIAL_COST: &str = "Getting one opens your browser, signs you in to Private AI, and makes a \
     new key that this app then keeps on this computer. The sign-in is with \
     a company that is not this app. The key is yours: it is listed in your \
     own Private AI account, and you can remove it there.";

/// The button that starts the ceremony.
pub const CREDENTIAL_OBTAIN: &str = "Sign in to Private AI";

/// The button shown while one is running.
///
/// Says what stops -- this computer's waiting -- and not "cancel the
/// sign-in", which would suggest reaching into a browser tab this app does
/// not control. Anything the contributor already finished over there stands.
pub const CREDENTIAL_CANCEL: &str = "Stop waiting for the browser";

/// The button that removes a stored key from this machine.
pub const CREDENTIAL_FORGET: &str = "Forget this key";

/// What forgetting does, and the larger part it does not do.
///
/// **Local only, and the sentence says so.** The key stays valid at the
/// service until the contributor removes it there, and this app cannot do it
/// for them: revoking needs the sign-in, and the sign-in was discarded the
/// moment the key was minted. A confirmation that said "removed" and let it
/// be read as "revoked" would be the claim this codebase does not make --
/// `handle_forget` answers `revoked: false` for the same reason.
pub const CREDENTIAL_FORGET_EXPLAINS: &str = "Forgetting removes the key from this computer and does nothing else. It \
     keeps working until you remove it in your own Private AI account, and \
     this app cannot do that for you: the sign-in that would be needed was \
     thrown away as soon as the key was made.";

/// `absent`.
pub const CREDENTIAL_ABSENT: &str = "No key is kept here for Private AI, so nothing on this computer can ask \
     it to answer a call.";

/// `obtaining`.
///
/// Names the five minutes because the listener gives the browser exactly
/// that long and then releases the port. A contributor who walked away and
/// came back to a card still saying "waiting" would be waiting on nothing.
pub const CREDENTIAL_OBTAINING: &str = "Waiting for you to finish signing in, in your browser. Nothing is kept \
     here until you do, and this stops waiting after five minutes.";

/// `failed`.
///
/// A refusal with a way out -- the rule every failure sentence on this
/// surface follows. What went wrong is not named because the service's own
/// refusal is a 400 with an empty body: there is nothing more specific to
/// say that would be true, and a sentence that guessed would be carrying a
/// guess into a contributor's head.
pub const CREDENTIAL_FAILED: &str = "The sign-in did not finish, and nothing was kept here. Sign in again to \
     try once more.";

/// `cancelled`.
///
/// Not a fault and not phrased as one. Stopping was the contributor's own
/// doing, and the sentence reports it and stops.
pub const CREDENTIAL_CANCELLED: &str =
    "You stopped the sign-in before it finished, and nothing was kept here.";

/// `present`.
///
/// Says the key is here and that this screen will not show it. It does not
/// say the key stays on this computer, which would be false: every call
/// answered with it carries it to whoever answers.
pub const CREDENTIAL_PRESENT: &str = "A key from Private AI is kept on this computer, and calls answered here \
     can use it. The key itself is never shown on this screen.";

/// A state this build has no words for, or a read that did not arrive.
///
/// **The tri-state sentence, and the reason there is one.** Not knowing must
/// not degrade to [`CREDENTIAL_ABSENT`]: "there is no key here" in front of
/// somebody who has one is an invitation to sign in again and end up holding
/// a second key, in their own account, that nothing on this screen will ever
/// mention again. So this says what was not read, and names the risk rather
/// than leaving it to be discovered.
///
/// The same shape as [`STATE_RUNNING_DESTINATION_UNKNOWN`], for the same
/// reason: an unread fact gets its own sentence rather than borrowing a
/// known one.
pub const CREDENTIAL_UNKNOWN: &str = "Whether a key is kept here could not be read just now. Check again \
     before signing in, so you do not end up with a second key you did not \
     mean to make.";

/// A daemon that does not answer this at all.
///
/// Distinct from [`CREDENTIAL_UNKNOWN`] on purpose: one is a read that
/// failed, the other is a build that was never asked to answer, and telling
/// somebody to check again is useless advice for the second.
pub const CREDENTIAL_UNREPORTED: &str =
    "This daemon does not report whether a Private AI key is kept here.";

/// What a shell may offer for one credential state.
///
/// ONE TABLE, NOT THREE. The alternative is each shell deciding from the
/// state -- or worse, from two booleans -- which button to draw, and the
/// button in question mints a key at a third party. [`Self::None`] for a
/// state nobody could read is the whole point: offering `Obtain` there is
/// how a contributor ends up with a second key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialAction {
    /// Draw no action. Nothing is known well enough to offer one.
    None,
    /// Start the ceremony. The only action that opens a browser, and the
    /// sentence in front of it is [`CREDENTIAL_COST`].
    Obtain,
    /// Stop waiting on the browser.
    Cancel,
    /// Remove the stored key from this machine, with
    /// [`CREDENTIAL_FORGET_EXPLAINS`] beside it.
    Forget,
}

/// The sentence for one `near_ai_credential_status` state label.
///
/// Missing status is unreported; a label this build has never heard of is
/// unavailable. NEITHER MAY DEGRADE TO [`CREDENTIAL_ABSENT`], which is a
/// claim about what this machine holds and would be made up.
#[must_use]
pub fn credential_state_line(label: &str) -> &'static str {
    match label {
        "" => CREDENTIAL_UNREPORTED,
        LABEL_CREDENTIAL_ABSENT => CREDENTIAL_ABSENT,
        LABEL_CREDENTIAL_OBTAINING => CREDENTIAL_OBTAINING,
        LABEL_CREDENTIAL_FAILED => CREDENTIAL_FAILED,
        LABEL_CREDENTIAL_CANCELLED => CREDENTIAL_CANCELLED,
        LABEL_CREDENTIAL_PRESENT => CREDENTIAL_PRESENT,
        _ => CREDENTIAL_UNKNOWN,
    }
}

/// The tone [`credential_state_line`]'s sentence is painted in.
///
/// Takes what the sentence takes, so the two stay in step by construction --
/// the rule [`state_tone`] follows. The enum is shared with the state row
/// deliberately: a shell already maps these five values onto colours once,
/// and a second enum with the same five meanings is a second mapping to keep
/// in agreement.
///
/// [`PrivateInferenceTone::Clear`] is `present` alone. `obtaining` is
/// [`PrivateInferenceTone::Held`] -- work is under way and no outcome is
/// claimed -- and everything unread is [`PrivateInferenceTone::Neutral`],
/// the safe direction here for the reason it is safe on the state row: the
/// dangerous value is the one that reads as settled.
#[must_use]
pub fn credential_state_tone(label: &str) -> PrivateInferenceTone {
    match label {
        LABEL_CREDENTIAL_PRESENT => PrivateInferenceTone::Clear,
        LABEL_CREDENTIAL_OBTAINING => PrivateInferenceTone::Held,
        LABEL_CREDENTIAL_FAILED => PrivateInferenceTone::Refused,
        LABEL_CREDENTIAL_ABSENT | LABEL_CREDENTIAL_CANCELLED => PrivateInferenceTone::Neutral,
        _ => PrivateInferenceTone::Neutral,
    }
}

/// The one action a shell may offer for a credential state.
///
/// A state this build cannot read answers [`CredentialAction::None`]. Every
/// other surface in this module treats an unknown label as "claim nothing";
/// here it also means "offer nothing", because the offer is what mints the
/// second key.
#[must_use]
pub fn credential_action(label: &str) -> CredentialAction {
    match label {
        LABEL_CREDENTIAL_ABSENT | LABEL_CREDENTIAL_FAILED | LABEL_CREDENTIAL_CANCELLED => {
            CredentialAction::Obtain
        }
        LABEL_CREDENTIAL_OBTAINING => CredentialAction::Cancel,
        LABEL_CREDENTIAL_PRESENT => CredentialAction::Forget,
        _ => CredentialAction::None,
    }
}

/// `eligible`.
///
/// **States an expectation and stops.** The cheap checks passed and the
/// marked call is here; the expensive ones still run when the contribution is
/// sent, and the server decides. A sentence promising acceptance would be
/// making a claim this side is not in a position to make, and would be read
/// as one on the day the server says no.
pub const ELIGIBILITY_ELIGIBLE: &str = "This session has what a contribution needs. The last checks happen when \
     you send it.";

/// `ineligible_permanent`.
///
/// Says the second half out loud. Somebody who is told only "this cannot be
/// sent" tries again, and again, and the surface owes them the fact that
/// trying is wasted work on their own finished session.
pub const ELIGIBILITY_INELIGIBLE_PERMANENT: &str = "This session cannot be sent, and nothing you change will alter that. \
     Trying again will not help.";

/// `ineligible_configuration`.
///
/// **The only state whose sentence names a setting**, because it is the only
/// one where changing something helps. The row stays about its own session:
/// what the setting changes is the sessions recorded from now on, and the
/// sentence says exactly that rather than implying this one can be rescued.
pub const ELIGIBILITY_INELIGIBLE_CONFIGURATION: &str = "This session cannot be sent. A setting decides whether the ones you \
     record from now on can be.";

/// `unknown`, and any state label this build has never heard of.
///
/// **Must not degrade to an ineligibility.** "Could not tell" turned into
/// "no" invites a contributor to conclude something false about their own
/// work and to stop offering it. The same tri-state rule
/// [`CREDENTIAL_UNKNOWN`] follows, and for the same reason: an unread fact
/// gets its own sentence rather than borrowing a known one.
pub const ELIGIBILITY_UNKNOWN: &str = "Whether this session can be sent has not been worked out. That is not \
     the same as a no.";

/// `no_inference_call`.
///
/// The case the whole surface exists for: everything a contributor recorded
/// before they started having their model calls answered here.
pub const ELIGIBILITY_REASON_NO_CALL: &str = "No model call was answered on this computer while this session ran, so \
     there is nothing recorded to send with it.";

/// `capture_off`.
pub const ELIGIBILITY_REASON_CAPTURE_OFF: &str = "The last model call in this session was answered without keeping a copy \
     of it. Whether copies are kept is a setting, and it decides the sessions \
     you record from now on.";

/// `digest_absent`.
pub const ELIGIBILITY_REASON_DIGEST_ABSENT: &str = "The last model call in this session did not finish cleanly, so what was \
     kept of it is incomplete.";

/// `upstream_id_absent`.
pub const ELIGIBILITY_REASON_UPSTREAM_ID_ABSENT: &str = "Nothing was written down for the last model call in this session that \
     would let anyone check it afterwards.";

/// `digest_mismatch`.
pub const ELIGIBILITY_REASON_DIGEST_MISMATCH: &str = "What was kept of the last model call in this session does not match what \
     was written down about it, so it cannot be sent as it stands.";

/// `reference_malformed`.
pub const ELIGIBILITY_REASON_REFERENCE_MALFORMED: &str = "The note saying where this session's kept copy lives is not one this \
     computer could have written.";

/// `bodies_unreadable`.
pub const ELIGIBILITY_REASON_BODIES_UNREADABLE: &str = "The kept copy of this session's last model call could not be read back \
     from this computer.";

/// `body_not_utf8`.
pub const ELIGIBILITY_REASON_BODY_NOT_UTF8: &str = "The kept copy of this session's last model call is not text this app can \
     carry without changing it, and changing it would make it worthless.";

/// `body_too_large`.
pub const ELIGIBILITY_REASON_BODY_TOO_LARGE: &str =
    "The kept copy of this session's last model call is too large to send.";

/// `evidence_capture_off`.
///
/// The one reason that is about the machine rather than about this session,
/// and it names the setting for the same reason `capture_off` does.
pub const ELIGIBILITY_REASON_EVIDENCE_CAPTURE_OFF: &str = "This computer keeps no copy of the model calls it answers, so no session \
     recorded here has one to send. That is a setting you can change.";

/// `marker_absent`.
///
/// Names both ways it happens, because they land on different people: work
/// finished before the mark existed, and work done through a tool that does
/// not add it. Neither is a mistake the contributor made.
pub const ELIGIBILITY_REASON_MARKER_ABSENT: &str = "The last model call in this session went out without the mark a \
     contribution is accepted on. It was made before that was set up, or by \
     a tool that does not add it.";

/// `request_malformed`.
pub const ELIGIBILITY_REASON_REQUEST_MALFORMED: &str = "The last model call in this session was not written down in a shape this \
     app can read, so the mark cannot be found in it.";

/// `receipt_unavailable`.
///
/// The one reason that is about right now rather than about this session,
/// which is why the state beside it is `unknown` and why this sentence is
/// the only one here that suggests trying later.
pub const ELIGIBILITY_REASON_RECEIPT_UNAVAILABLE: &str = "The proof that goes with this session's last model call could not be \
     fetched just now. It may work later.";

/// What a shell may offer for one eligibility state.
///
/// ONE TABLE, NOT THREE -- the rule [`CredentialAction`] states, applied to
/// the control that sends a contributor's work. Three shells each deciding
/// which rows get a send button is three chances to offer one beside a
/// session the server will refuse, which is the defect this whole surface
/// exists to remove.
///
/// [`Self::None`] is not "hide the row". Every session is shown; hiding a
/// contributor's own work is its own dishonesty, and makes the app look as
/// though it had not noticed files the contributor knows it can see. The row
/// is present, unoffered, and carries its reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContributionControl {
    /// Draw no send control. The row still renders, with its sentence.
    None,
    /// Offer to send this session.
    Contribute,
}

/// The sentence for one queue entry's `eligibility` label.
///
/// An unfamiliar or empty label answers [`ELIGIBILITY_UNKNOWN`]. IT MUST NOT
/// BORROW AN INELIGIBILITY SENTENCE: a state this build cannot read is not
/// evidence that a contributor's session is unsendable, and saying it is
/// would stop them offering work that is fine.
///
/// A shell that received no `eligibility` field at all should call none of
/// this: an absent field means the contributor was invited and has no
/// eligibility question. See `daemon::contribution_eligibility`.
#[must_use]
pub fn eligibility_state_line(label: &str) -> &'static str {
    match label {
        ELIGIBILITY_STATE_ELIGIBLE => ELIGIBILITY_ELIGIBLE,
        ELIGIBILITY_STATE_INELIGIBLE_PERMANENT => ELIGIBILITY_INELIGIBLE_PERMANENT,
        ELIGIBILITY_STATE_INELIGIBLE_CONFIGURATION => ELIGIBILITY_INELIGIBLE_CONFIGURATION,
        _ => ELIGIBILITY_UNKNOWN,
    }
}

/// How firmly [`eligibility_state_line`]'s sentence reads.
///
/// Takes what the sentence takes, so the two stay in step by construction.
///
/// [`PrivateInferenceTone::Attention`] is `ineligible_configuration` alone --
/// the one state where there is something to do. A permanent ineligibility is
/// [`PrivateInferenceTone::Neutral`] and deliberately not
/// [`PrivateInferenceTone::Refused`]: nothing was refused, nothing went
/// wrong, and painting a contributor's ordinary older work as a failure is a
/// judgement on it that this surface has no business making. An unread state
/// is Neutral for the reason it is everywhere else here: the dangerous value
/// is the one that reads as settled.
#[must_use]
pub fn eligibility_state_tone(label: &str) -> PrivateInferenceTone {
    match label {
        ELIGIBILITY_STATE_ELIGIBLE => PrivateInferenceTone::Clear,
        ELIGIBILITY_STATE_INELIGIBLE_CONFIGURATION => PrivateInferenceTone::Attention,
        _ => PrivateInferenceTone::Neutral,
    }
}

/// The one control a shell may offer for an eligibility state.
///
/// `eligible` alone. Every other state -- including one this build has never
/// heard of -- offers nothing, because the alternative is a send button on a
/// session that cannot be sent, discovered on the press.
#[must_use]
pub fn eligibility_control(label: &str) -> ContributionControl {
    match label {
        ELIGIBILITY_STATE_ELIGIBLE => ContributionControl::Contribute,
        _ => ContributionControl::None,
    }
}

/// The sentence for one queue entry's `eligibility_reason` label.
///
/// **The empty string for an unfamiliar or absent reason**, and a shell
/// renders nothing for it. That is not the tri-state hedge the state line
/// makes: the state sentence beside it has already said what is true, and a
/// second sentence guessing at a reason this build does not know would be
/// adding a detail nobody established. An `eligible` row has no reason at
/// all, for the same reason -- there is nothing to explain.
#[must_use]
pub fn eligibility_reason_line(label: &str) -> &'static str {
    match label {
        REASON_NO_CALL => ELIGIBILITY_REASON_NO_CALL,
        REASON_CAPTURE_OFF => ELIGIBILITY_REASON_CAPTURE_OFF,
        REASON_DIGEST_ABSENT => ELIGIBILITY_REASON_DIGEST_ABSENT,
        REASON_UPSTREAM_ID_ABSENT => ELIGIBILITY_REASON_UPSTREAM_ID_ABSENT,
        REASON_DIGEST_MISMATCH => ELIGIBILITY_REASON_DIGEST_MISMATCH,
        REASON_REFERENCE_MALFORMED => ELIGIBILITY_REASON_REFERENCE_MALFORMED,
        REASON_BODIES_UNREADABLE => ELIGIBILITY_REASON_BODIES_UNREADABLE,
        REASON_BODY_NOT_UTF8 => ELIGIBILITY_REASON_BODY_NOT_UTF8,
        REASON_BODY_TOO_LARGE => ELIGIBILITY_REASON_BODY_TOO_LARGE,
        REASON_EVIDENCE_CAPTURE_OFF => ELIGIBILITY_REASON_EVIDENCE_CAPTURE_OFF,
        REASON_MARKER_ABSENT => ELIGIBILITY_REASON_MARKER_ABSENT,
        REASON_REQUEST_MALFORMED => ELIGIBILITY_REASON_REQUEST_MALFORMED,
        REASON_RECEIPT_UNAVAILABLE => ELIGIBILITY_REASON_RECEIPT_UNAVAILABLE,
        _ => "",
    }
}

/// The payload, built from the constants above.
#[must_use]
pub fn private_inference_copy() -> PrivateInferenceCopy {
    PrivateInferenceCopy {
        destination: DESTINATION,
        subtitle: SUBTITLE,
        offer_title: OFFER_TITLE,
        offer_what: OFFER_WHAT,
        offer_exposure: OFFER_EXPOSURE,
        offer_no_repoint: OFFER_NO_REPOINT,
        offer_accept: OFFER_ACCEPT,
        offer_decline: OFFER_DECLINE,
        offer_asked_once: OFFER_ASKED_ONCE,
        settings_title: SETTINGS_TITLE,
        settings_toggle: SETTINGS_TOGGLE,
        settings_applies_at_once: SETTINGS_APPLIES_AT_ONCE,
        state_off: STATE_OFF,
        state_unknown: STATE_UNKNOWN,
        state_unreported: STATE_UNREPORTED,
        state_stopping: STATE_STOPPING,
        state_running: STATE_RUNNING,
        state_running_no_backends: STATE_RUNNING_NO_BACKENDS,
        state_running_answered_elsewhere: STATE_RUNNING_ANSWERED_ELSEWHERE,
        state_running_destination_unknown: STATE_RUNNING_DESTINATION_UNKNOWN,
        state_running_elsewhere: STATE_RUNNING_ELSEWHERE,
        state_port_in_use: STATE_PORT_IN_USE,
        state_start_failed: STATE_START_FAILED,
        state_crashed: STATE_CRASHED,
        quit_also_stops: QUIT_ALSO_STOPS,
        write_unconfirmed: WRITE_UNCONFIRMED,
        settings_moved: SETTINGS_MOVED,
        tray_turn_off: TRAY_TURN_OFF,
        tray_open_to_turn_on: TRAY_OPEN_TO_TURN_ON,
        harnesses_title: HARNESSES_TITLE,
        harnesses_what: HARNESSES_WHAT,
        harnesses_spend_scope: HARNESSES_SPEND_SCOPE,
        harness_not_installed: HARNESS_NOT_INSTALLED,
        harness_not_connected: HARNESS_NOT_CONNECTED,
        harness_connected_nothing_seen: HARNESS_CONNECTED_NOTHING_SEEN,
        harness_answering: HARNESS_ANSWERING,
        harness_connect: HARNESS_CONNECT,
        harness_disconnect: HARNESS_DISCONNECT,
        harness_preview_title: HARNESS_PREVIEW_TITLE,
        harness_preview_confirm: HARNESS_PREVIEW_CONFIRM,
        harness_preview_cancel: HARNESS_PREVIEW_CANCEL,
        harness_slot_taken: HARNESS_SLOT_TAKEN,
        harness_needs_restart: HARNESS_NEEDS_RESTART,
        harnesses_none_found: HARNESSES_NONE_FOUND,
        harness_unreadable_config: HARNESS_UNREADABLE_CONFIG,
        harness_plan_nothing_to_change: HARNESS_PLAN_NOTHING_TO_CHANGE,
        harness_plan_entry_unusable: HARNESS_PLAN_ENTRY_UNUSABLE,
        harness_plan_no_config_path: HARNESS_PLAN_NO_CONFIG_PATH,
        credential_title: CREDENTIAL_TITLE,
        credential_what: CREDENTIAL_WHAT,
        credential_cost: CREDENTIAL_COST,
        credential_obtain: CREDENTIAL_OBTAIN,
        credential_cancel: CREDENTIAL_CANCEL,
        credential_forget: CREDENTIAL_FORGET,
        credential_forget_explains: CREDENTIAL_FORGET_EXPLAINS,
        credential_absent: CREDENTIAL_ABSENT,
        credential_obtaining: CREDENTIAL_OBTAINING,
        credential_failed: CREDENTIAL_FAILED,
        credential_cancelled: CREDENTIAL_CANCELLED,
        credential_present: CREDENTIAL_PRESENT,
        credential_unknown: CREDENTIAL_UNKNOWN,
        credential_unreported: CREDENTIAL_UNREPORTED,
        harness_needs_credential: HARNESS_NEEDS_CREDENTIAL,
        eligibility_eligible: ELIGIBILITY_ELIGIBLE,
        eligibility_ineligible_permanent: ELIGIBILITY_INELIGIBLE_PERMANENT,
        eligibility_ineligible_configuration: ELIGIBILITY_INELIGIBLE_CONFIGURATION,
        eligibility_unknown: ELIGIBILITY_UNKNOWN,
        eligibility_reason_no_call: ELIGIBILITY_REASON_NO_CALL,
        eligibility_reason_capture_off: ELIGIBILITY_REASON_CAPTURE_OFF,
        eligibility_reason_digest_absent: ELIGIBILITY_REASON_DIGEST_ABSENT,
        eligibility_reason_upstream_id_absent: ELIGIBILITY_REASON_UPSTREAM_ID_ABSENT,
        eligibility_reason_digest_mismatch: ELIGIBILITY_REASON_DIGEST_MISMATCH,
        eligibility_reason_reference_malformed: ELIGIBILITY_REASON_REFERENCE_MALFORMED,
        eligibility_reason_bodies_unreadable: ELIGIBILITY_REASON_BODIES_UNREADABLE,
        eligibility_reason_body_not_utf8: ELIGIBILITY_REASON_BODY_NOT_UTF8,
        eligibility_reason_body_too_large: ELIGIBILITY_REASON_BODY_TOO_LARGE,
        eligibility_reason_evidence_capture_off: ELIGIBILITY_REASON_EVIDENCE_CAPTURE_OFF,
        eligibility_reason_marker_absent: ELIGIBILITY_REASON_MARKER_ABSENT,
        eligibility_reason_request_malformed: ELIGIBILITY_REASON_REQUEST_MALFORMED,
        eligibility_reason_receipt_unavailable: ELIGIBILITY_REASON_RECEIPT_UNAVAILABLE,
    }
}

// PRIVATE-INFERENCE-SURFACE-END

/// The eligibility state and reason labels, re-exported from the daemon
/// module that produces them -- the same rule the credential labels above
/// follow, and for the same reason: a label spelled twice is two labels that
/// have not disagreed yet.
pub use crate::daemon::contribution_eligibility::{
    REASON_BODIES_UNREADABLE,
    REASON_BODY_NOT_UTF8,
    REASON_BODY_TOO_LARGE,
    REASON_CAPTURE_OFF,
    REASON_DIGEST_ABSENT,
    REASON_DIGEST_MISMATCH,
    REASON_EVIDENCE_CAPTURE_OFF,
    REASON_MARKER_ABSENT,
    REASON_NO_CALL,
    REASON_RECEIPT_UNAVAILABLE,
    REASON_REFERENCE_MALFORMED,
    REASON_REQUEST_MALFORMED,
    REASON_UPSTREAM_ID_ABSENT,
    STATE_ELIGIBLE as ELIGIBILITY_STATE_ELIGIBLE,
    STATE_INELIGIBLE_CONFIGURATION as ELIGIBILITY_STATE_INELIGIBLE_CONFIGURATION,
    STATE_INELIGIBLE_PERMANENT as ELIGIBILITY_STATE_INELIGIBLE_PERMANENT,
    // Aliased, all four, because this module already has a `STATE_UNKNOWN`:
    // the listener's. Two constants named for two different unknowns, one
    // import away from each other, is a collision waiting to be resolved the
    // wrong way round; the prefix says which surface each belongs to.
    STATE_UNKNOWN as ELIGIBILITY_STATE_UNKNOWN,
};
/// The state labels this surface has words for, re-exported from the daemon
/// module that produces them.
///
/// Imported rather than respelled: a label spelled twice is two labels that
/// have not disagreed yet, and the failure mode of a typo here is a state that
/// silently renders as unavailable.
/// The credential state labels, re-exported from the daemon module that
/// produces them -- the same rule the listener labels below follow, and for
/// the same reason.
pub use crate::daemon::nearai_credential::{
    LABEL_CREDENTIAL_ABSENT, LABEL_CREDENTIAL_CANCELLED, LABEL_CREDENTIAL_FAILED,
    LABEL_CREDENTIAL_OBTAINING, LABEL_CREDENTIAL_PRESENT,
};
pub use crate::daemon::private_inference::{
    LABEL_CRASHED, LABEL_OFF, LABEL_PORT_IN_USE, LABEL_RUNNING, LABEL_RUNNING_ANSWERED_ELSEWHERE,
    LABEL_RUNNING_DESTINATION_UNKNOWN, LABEL_RUNNING_ELSEWHERE, LABEL_RUNNING_NO_BACKENDS,
    LABEL_START_FAILED, LABEL_STOPPING,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure this surface exists to prevent, pinned.
    ///
    /// If the destination could not be read, or was read and is not ours, the
    /// shell must not paint a working light. `Clear` is the only tone that
    /// paints one, so this is the whole safety property in one assertion.
    #[test]
    fn a_destination_that_is_not_ours_is_never_painted_as_working() {
        for label in [
            LABEL_RUNNING_ANSWERED_ELSEWHERE,
            LABEL_RUNNING_DESTINATION_UNKNOWN,
        ] {
            assert_eq!(
                state_tone(label),
                PrivateInferenceTone::Attention,
                "{label} must be Attention"
            );
            assert!(
                !state_tone(label).reads_as_working(),
                "{label} must never read as working"
            );
        }
        assert!(state_tone(LABEL_RUNNING).reads_as_working());
    }

    /// Each new label has to reach its own sentence. Falling through to
    /// `STATE_UNKNOWN` would lose the very thing being said.
    #[test]
    fn each_destination_label_reaches_its_own_sentence() {
        assert_eq!(
            state_line(LABEL_RUNNING_ANSWERED_ELSEWHERE),
            STATE_RUNNING_ANSWERED_ELSEWHERE
        );
        assert_eq!(
            state_line(LABEL_RUNNING_DESTINATION_UNKNOWN),
            STATE_RUNNING_DESTINATION_UNKNOWN
        );
        assert_eq!(state_line(LABEL_RUNNING), STATE_RUNNING);
        assert_ne!(state_line(LABEL_RUNNING_ANSWERED_ELSEWHERE), STATE_UNKNOWN);
    }

    /// Only `Clear` may be painted as working. The GTK shell reads this
    /// predicate directly rather than through the ABI, so it is pinned here
    /// as well as in Swift.
    #[test]
    fn only_clear_reads_as_working() {
        assert!(PrivateInferenceTone::Clear.reads_as_working());
        for tone in [
            PrivateInferenceTone::Neutral,
            PrivateInferenceTone::Held,
            PrivateInferenceTone::Attention,
            PrivateInferenceTone::Refused,
        ] {
            assert!(
                !tone.reads_as_working(),
                "{tone:?} must not read as working"
            );
        }
    }

    /// The states a daemon can report, filtered through the shared table:
    /// `running` is the only one an indicator may light up.
    #[test]
    fn only_a_running_listener_reads_as_working() {
        assert!(state_tone(LABEL_RUNNING).reads_as_working());
        for label in [
            "",
            LABEL_OFF,
            LABEL_STOPPING,
            LABEL_RUNNING_NO_BACKENDS,
            LABEL_RUNNING_ELSEWHERE,
            LABEL_PORT_IN_USE,
            LABEL_START_FAILED,
            LABEL_CRASHED,
            "a_state_from_a_later_daemon",
        ] {
            assert!(
                !state_tone(label).reads_as_working(),
                "{label} must not read as working"
            );
        }
    }

    /// The nav wording exists, because a top-level destination needs a label
    /// in the switcher and a line under its title.
    /// The way back names the place the switch is actually in.
    ///
    /// This sentence said "The switch stays in Settings" until the switch
    /// moved to its own destination, and nothing failed -- no test pinned the
    /// copy, so a sentence that had become false went on being shown to every
    /// contributor at first run. Found in review.
    #[test]
    fn the_way_back_names_the_destination_it_is_in() {
        assert!(
            OFFER_ASKED_ONCE.contains(DESTINATION),
            "the offer must name the destination the switch is in, not a stale one: {OFFER_ASKED_ONCE}"
        );
        assert!(
            !OFFER_ASKED_ONCE.contains("Settings"),
            "the switch is no longer in Settings, which holds only a pointer"
        );
    }

    #[test]
    fn the_copy_carries_nav_wording_for_a_top_level_destination() {
        let copy = private_inference_copy();
        assert!(!copy.destination.is_empty(), "the nav item needs a label");
        assert!(
            !copy.subtitle.is_empty(),
            "the destination needs a subtitle"
        );
        // The label sits in a sidebar beside Waiting/History/Computer/Settings.
        assert!(
            copy.destination.chars().count() <= 24,
            "nav label too long for the sidebar: {}",
            copy.destination
        );
    }

    /// The offer has to say what turning the switch on exposes. This is the
    /// one sentence the design will not ship without: the listener's
    /// answering side is unauthenticated, and a contributor on a shared
    /// machine is deciding something different from one on a laptop.
    #[test]
    fn the_offer_says_what_it_exposes() {
        let payload = private_inference_copy();
        assert!(
            payload.offer_exposure.contains("anything else running"),
            "the exposure sentence stopped naming other software: {}",
            payload.offer_exposure
        );
        assert!(
            payload.offer_exposure.contains("shared"),
            "the exposure sentence stopped distinguishing a shared machine: {}",
            payload.offer_exposure
        );
        assert!(
            payload.offer_exposure.contains("accounts"),
            "the exposure sentence stopped naming whose accounts pay: {}",
            payload.offer_exposure
        );
    }

    /// A listener with nowhere to pass calls on to is never painted as
    /// working. This is why `running_no_backends` exists as a state at all.
    #[test]
    fn nothing_is_painted_clear_without_somewhere_to_send() {
        assert_eq!(
            state_tone(LABEL_RUNNING_NO_BACKENDS),
            PrivateInferenceTone::Attention
        );
        assert_ne!(
            state_tone(LABEL_RUNNING_NO_BACKENDS),
            PrivateInferenceTone::Clear
        );
        assert_ne!(
            state_line(LABEL_RUNNING_NO_BACKENDS),
            state_line(LABEL_RUNNING),
            "the two on-states must not share a sentence"
        );
    }

    /// Every failure reads as a refusal and carries a way out. A refusal with
    /// no way out is an accusation.
    #[test]
    fn every_failure_is_a_refusal_with_a_way_out() {
        for label in [LABEL_PORT_IN_USE, LABEL_START_FAILED, LABEL_CRASHED] {
            assert_eq!(
                state_tone(label),
                PrivateInferenceTone::Refused,
                "{label} is not painted as a refusal"
            );
            assert!(
                state_line(label).contains("off and on again"),
                "{label} does not name the way out: {}",
                state_line(label)
            );
        }
    }

    /// The sticky state says it is sticky, so a contributor is not left
    /// waiting for a retry that will not come.
    #[test]
    fn the_crashed_state_says_it_will_stay_that_way() {
        assert!(
            state_line(LABEL_CRASHED).contains("will not retry by itself"),
            "the crashed sentence stopped saying it is sticky: {}",
            state_line(LABEL_CRASHED)
        );
    }

    /// The other instance is described as left alone, not as this app's doing.
    #[test]
    fn another_instance_is_left_alone_and_says_so() {
        assert_eq!(
            state_tone(LABEL_RUNNING_ELSEWHERE),
            PrivateInferenceTone::Held
        );
        let line = state_line(LABEL_RUNNING_ELSEWHERE);
        assert!(
            line.contains("started nothing and stopped nothing"),
            "{line}"
        );
    }

    /// A label this build has never heard of claims nothing, and never falls
    /// through to an on-sentence.
    #[test]
    fn an_unknown_label_claims_nothing() {
        for label in ["something_later", "RUNNING"] {
            assert_eq!(state_line(label), STATE_UNKNOWN, "{label}");
            assert_eq!(state_tone(label), PrivateInferenceTone::Neutral, "{label}");
        }
    }

    #[test]
    fn stopping_and_foreign_ownership_do_not_claim_readiness() {
        assert_eq!(state_line(""), STATE_UNREPORTED);
        assert_ne!(state_line(""), STATE_OFF);
        assert_eq!(state_line(LABEL_OFF), STATE_OFF);
        assert_eq!(state_line(LABEL_STOPPING), STATE_STOPPING);
        assert_eq!(state_tone(LABEL_STOPPING), PrivateInferenceTone::Held);
        assert!(!STATE_OFF.contains("Nothing on this computer"));
        assert!(!STATE_RUNNING_ELSEWHERE.contains("already answering"));
        assert!(!serving_line(Some(8463)).contains("Answering"));
    }

    #[test]
    fn emitted_daemon_states_have_copy_and_owned_work_warns_at_quit() {
        use crate::daemon::private_inference::PrivateInferenceState as State;
        let states = [
            State::Off,
            State::Stopping { port: None },
            State::Stopping { port: Some(8463) },
            State::Running { port: 8463 },
            State::RunningWithoutBackends { port: 8463 },
            State::RunningElsewhere { port: 8463 },
            State::Failed {
                label: LABEL_PORT_IN_USE,
            },
            State::Failed {
                label: LABEL_START_FAILED,
            },
            State::Failed {
                label: LABEL_CRASHED,
            },
        ];
        for state in states {
            // Exhaustive over actual producer variants: adding a lifecycle
            // variant requires deciding its copy and quit semantics here.
            let (line, tone, owned) = match &state {
                State::Off => (STATE_OFF, PrivateInferenceTone::Neutral, false),
                State::Stopping { .. } => (STATE_STOPPING, PrivateInferenceTone::Held, true),
                State::Running { .. } => (STATE_RUNNING, PrivateInferenceTone::Clear, true),
                State::RunningWithoutBackends { .. } => (
                    STATE_RUNNING_NO_BACKENDS,
                    PrivateInferenceTone::Attention,
                    true,
                ),
                State::RunningElsewhere { .. } => {
                    (STATE_RUNNING_ELSEWHERE, PrivateInferenceTone::Held, false)
                }
                State::Failed { label } => {
                    (state_line(label), PrivateInferenceTone::Refused, false)
                }
            };
            assert_eq!(state_line(state.label()), line);
            assert_ne!(line, STATE_UNKNOWN);
            assert_ne!(line, STATE_UNREPORTED);
            assert_eq!(state_tone(state.label()), tone);
            assert_eq!(quit_needs_notice(false, state.label()), owned);
        }
        assert!(!quit_needs_notice(true, LABEL_RUNNING_ELSEWHERE));
        assert!(!quit_needs_notice(true, LABEL_OFF));
        assert!(quit_needs_notice(true, "future_state"));
        assert!(!quit_needs_notice(false, "future_state"));
    }

    /// The offer is asked once, whichever way it was answered, and is not put
    /// to somebody who already has the thing.
    #[test]
    fn the_offer_is_asked_once() {
        assert!(should_offer(false, false), "a fresh install is offered");
        assert!(!should_offer(true, false), "a declined offer is remembered");
        assert!(
            !should_offer(false, true),
            "nobody is offered what they already have"
        );
        assert!(!should_offer(true, true));
    }

    #[test]
    fn write_confirmation_requires_present_matching_echoes() {
        for seen in [None, Some(false), Some(true)] {
            for on in [None, Some(false), Some(true)] {
                assert_eq!(write_confirmed(None, seen, on), seen == Some(true));
                assert_eq!(
                    write_confirmed(Some(true), seen, on),
                    seen == Some(true) && on == Some(true)
                );
                assert_eq!(
                    write_confirmed(Some(false), seen, on),
                    seen == Some(true) && on == Some(false)
                );
            }
        }
    }

    /// The port sentence is finished on this side, and says nothing at all
    /// when there is no port.
    #[test]
    fn the_port_sentence_names_a_port_or_says_nothing() {
        assert!(serving_line(Some(8463)).contains("8463"));
        assert_eq!(serving_line(None), "");
        assert_eq!(serving_line(Some(0)), "");
    }

    /// The three per-harness states are three different sentences, and only
    /// one of them says a call was answered.
    ///
    /// Connected and answering are the pair most likely to be collapsed by a
    /// well-meaning simplification, and collapsing them would tell a
    /// contributor their tool works on the strength of a value in a file.
    #[test]
    fn only_one_harness_state_says_a_call_arrived() {
        let copy = private_inference_copy();
        let states = [
            copy.harness_not_connected,
            copy.harness_connected_nothing_seen,
            copy.harness_answering,
        ];
        for (i, one) in states.iter().enumerate() {
            for other in &states[i + 1..] {
                assert_ne!(one, other, "two harness states share a sentence");
            }
        }
        assert!(
            copy.harness_answering.contains("was answered"),
            "the answering state stopped saying a call was answered: {}",
            copy.harness_answering
        );
        assert!(
            copy.harness_connected_nothing_seen
                .contains("nothing has arrived"),
            "the connected state stopped saying nothing has arrived: {}",
            copy.harness_connected_nothing_seen
        );
    }

    /// An occupied slot is reported, never offered. The sentence says what
    /// was left alone and stops; it must not read as a fault to clear or as
    /// an offer to take the slot.
    #[test]
    fn an_occupied_slot_is_reported_and_never_offered() {
        let taken = private_inference_copy().harness_slot_taken;
        assert!(
            taken.contains("left exactly as you had it"),
            "the occupied sentence stopped saying it was left alone: {taken}"
        );
        for word in ["error", "failed", "instead", "take over", "override"] {
            assert!(
                !taken.to_lowercase().contains(word),
                "the occupied sentence reads as {word}: {taken}"
            );
        }
    }

    /// A file that could not be read is a refusal, and says so in words that
    /// cannot be read as having found nothing to change.
    #[test]
    fn an_unreadable_file_is_a_refusal_and_not_a_no_op() {
        let refused = private_inference_copy().harness_unreadable_config;
        assert!(
            refused.contains("refusal"),
            "the unreadable sentence stopped naming itself a refusal: {refused}"
        );
        assert!(
            refused.contains("changed nothing"),
            "the unreadable sentence stopped saying nothing was written: {refused}"
        );
    }

    /// An empty list says what was looked for. An empty list that explains
    /// nothing cannot be told apart from a broken one.
    #[test]
    fn an_empty_list_says_what_was_looked_for() {
        let copy = private_inference_copy();
        assert!(
            copy.harnesses_none_found.contains("looked for"),
            "the empty-list sentence stopped saying what was looked for: {}",
            copy.harnesses_none_found
        );
        assert!(
            copy.harnesses_what.contains("not"),
            "the list's line stopped qualifying what the list is: {}",
            copy.harnesses_what
        );
    }

    /// One state, one sentence, decided in one place.
    ///
    /// This is the table the three shells used to each hold a copy of. It is
    /// asserted against the payload fields rather than against literals, so
    /// it cannot be the second place a sentence is written.
    #[test]
    fn one_harness_state_table_answers_for_every_shell() {
        use crate::harness_state::HarnessState;
        let copy = private_inference_copy();
        assert_eq!(
            harness_state_line(HarnessState::NotConnected.label()),
            copy.harness_not_connected
        );
        assert_eq!(
            harness_state_line(HarnessState::ConnectedNoCalls.label()),
            copy.harness_connected_nothing_seen
        );
        assert_eq!(
            harness_state_line(HarnessState::Answering.label()),
            copy.harness_answering
        );
    }

    /// The two states that must claim nothing, and must not borrow the
    /// nearest sentence to do it.
    ///
    /// `activity_shared` may not take [`HARNESS_ANSWERING`]: that sentence
    /// says a call from *this tool* was answered, and the pronoun names the
    /// one thing the state exists to say is unknown. Nor may it take
    /// [`HARNESS_CONNECTED_NOTHING_SEEN`], which would be flatly false --
    /// something did arrive. `unknown`, and a label this build has never
    /// heard of, are the same shape for the same reason.
    #[test]
    fn two_harness_states_say_nothing_and_borrow_nothing() {
        use crate::harness_state::HarnessState;
        let copy = private_inference_copy();
        for label in [
            HarnessState::ActivityShared.label(),
            HarnessState::Unknown.label(),
            "a_state_from_a_later_daemon",
            "",
        ] {
            let line = harness_state_line(label);
            assert_eq!(line, "", "{label} grew a sentence");
            assert_ne!(line, copy.harness_answering);
            assert_ne!(line, copy.harness_connected_nothing_seen);
            assert_ne!(line, copy.harness_not_connected);
        }
    }

    /// A tool that is not on this machine says so, and does not borrow the
    /// not-connected sentence to do it.
    ///
    /// The two used to render identically, because only one of them had
    /// words: a missing tool drew "Its own settings still send its calls
    /// wherever they went before", which is a claim about the settings of
    /// something that is not there.
    #[test]
    fn a_missing_tool_says_it_is_missing_and_claims_nothing_about_settings() {
        let copy = private_inference_copy();
        assert_ne!(copy.harness_not_installed, copy.harness_not_connected);
        assert!(
            !copy.harness_not_installed.contains("send its calls"),
            "the missing-tool sentence claims something about its settings: {}",
            copy.harness_not_installed
        );
        assert!(
            copy.harness_not_installed
                .contains("Not found on this computer"),
            "the missing-tool sentence stopped saying it is missing: {}",
            copy.harness_not_installed
        );
    }

    /// One outcome, one sentence, decided in one place.
    ///
    /// Asserted against the payload fields rather than against literals, so
    /// this cannot become the second place a sentence is written.
    #[test]
    fn one_plan_outcome_table_answers_for_every_shell() {
        use crate::harness_state::PlanOutcome;
        let copy = private_inference_copy();
        assert_eq!(
            harness_outcome_line(PlanOutcome::Noop.label()),
            copy.harness_plan_nothing_to_change
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::Unparseable.label()),
            copy.harness_unreadable_config
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::NotInstalled.label()),
            copy.harness_not_installed
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::EntryUnusable.label()),
            copy.harness_plan_entry_unusable
        );
        assert_eq!(
            harness_outcome_line(PlanOutcome::NoConfigPath.label()),
            copy.harness_plan_no_config_path
        );
    }

    /// Every outcome that writes nothing explains itself, and the one that
    /// writes something says nothing.
    ///
    /// This is the finding: four non-committable outcomes opened a preview
    /// with a title, a path, no changes and a way out. `is_noop` is the one
    /// the design names explicitly, and it is in here with the other three.
    #[test]
    fn every_outcome_that_changes_nothing_says_why() {
        use crate::harness_state::PlanOutcome;
        assert_eq!(
            harness_outcome_line(PlanOutcome::Changes.label()),
            "",
            "a plan with changes in it needs no sentence above them"
        );
        let mut seen: Vec<&str> = Vec::new();
        for outcome in [
            PlanOutcome::Noop,
            PlanOutcome::Unparseable,
            PlanOutcome::NotInstalled,
            PlanOutcome::EntryUnusable,
            PlanOutcome::NoConfigPath,
        ] {
            assert!(!outcome.is_committable(), "{outcome:?} became committable");
            let line = harness_outcome_line(outcome.label());
            assert!(
                !line.trim().is_empty(),
                "{outcome:?} opens an empty preview"
            );
            assert!(
                !seen.contains(&line),
                "{outcome:?} borrows another outcome's sentence: {line}"
            );
            seen.push(line);
        }
        for unknown in ["an_outcome_from_a_later_daemon", ""] {
            assert_eq!(
                harness_outcome_line(unknown),
                "",
                "{unknown} borrowed a sentence"
            );
        }
    }

    /// The when-sentence is finished on this side, says nothing when there is
    /// nothing to report, and counts in whole units.
    #[test]
    fn the_last_call_sentence_names_a_time_or_says_nothing() {
        assert_eq!(harness_last_call_line(None), "");
        assert!(harness_last_call_line(Some(0)).contains("just now"));
        assert!(harness_last_call_line(Some(59)).contains("just now"));
        assert!(harness_last_call_line(Some(60)).contains("1 minute ago"));
        assert!(harness_last_call_line(Some(120)).contains("2 minutes ago"));
        assert!(harness_last_call_line(Some(3_600)).contains("1 hour ago"));
        assert!(harness_last_call_line(Some(86_400)).contains("1 day ago"));
        assert!(harness_last_call_line(Some(172_800)).contains("2 days ago"));
    }

    /// UNKNOWN AND NONE-SPENT ARE DIFFERENT FACTS, and the difference is the
    /// whole reason this function takes an `Option` rather than a number.
    ///
    /// A figure nobody could read must draw no line at all. Rendering it as
    /// `$0.00` would be the same defect as painting a state this build has no
    /// words for as `Answering`: a confident claim assembled out of an
    /// absence.
    #[test]
    fn an_unreadable_amount_says_nothing_and_a_measured_zero_says_zero() {
        assert_eq!(harness_spend_line(None), "");
        let zero = harness_spend_line(Some(0));
        assert!(
            zero.contains("$0.00"),
            "a measured zero must name it: {zero}"
        );
        assert_ne!(zero, harness_spend_line(None));
    }

    /// The window is named in the sentence, not left for a shell to add.
    #[test]
    fn the_amount_names_its_own_window() {
        assert!(harness_spend_line(Some(1_230_000)).contains("since midnight"));
        assert!(HARNESSES_SPEND_SCOPE.contains("this computer"));
    }

    /// Whole cents, rounded once, and never a carry that spills a third
    /// digit past the point.
    #[test]
    fn an_amount_reads_as_money() {
        assert!(harness_spend_line(Some(1_230_000)).contains("$1.23"));
        assert!(harness_spend_line(Some(1_999_999)).contains("$2.00"));
        assert!(harness_spend_line(Some(12_345_678)).contains("$12.35"));
        assert!(harness_spend_line(Some(1_000_000_000)).contains("$1000.00"));
    }

    /// An amount too small to show is said to be small, not rounded away to
    /// the sentence a day with nothing on it gets.
    #[test]
    fn a_fraction_of_a_cent_is_not_reported_as_nothing() {
        let tiny = harness_spend_line(Some(1));
        assert!(tiny.contains("less than $0.01"), "{tiny}");
        assert_ne!(tiny, harness_spend_line(Some(0)));
    }

    /// AN UNREAD CREDENTIAL STATE IS NOT AN ABSENT ONE, and this is the
    /// assertion that keeps it that way.
    ///
    /// Both directions of the collapse are wrong and only one is obvious.
    /// Rendering "no key is kept here" for a state nobody could read invites
    /// a contributor who already has a key to mint a second one, at a third
    /// party, that this app will never mention again. Rendering "a key is
    /// kept here" is the other way to be confidently wrong.
    #[test]
    fn an_unread_credential_state_never_borrows_a_known_one() {
        for label in ["", "a_state_from_a_later_daemon", "PRESENT", "  "] {
            let line = credential_state_line(label);
            assert_ne!(line, CREDENTIAL_ABSENT, "{label} claimed nothing is here");
            assert_ne!(line, CREDENTIAL_PRESENT, "{label} claimed a key is here");
            assert_ne!(line, CREDENTIAL_OBTAINING, "{label}");
            assert!(
                !credential_state_tone(label).reads_as_working(),
                "{label} must not read as settled"
            );
            // And it offers nothing: the button in question mints a key.
            assert_eq!(
                credential_action(label),
                CredentialAction::None,
                "{label} offered an action nobody could justify"
            );
        }
        // The two unread cases are told apart. A read that failed is worth
        // checking again; a build that never answers is not.
        assert_eq!(credential_state_line(""), CREDENTIAL_UNREPORTED);
        assert_eq!(
            credential_state_line("a_state_from_a_later_daemon"),
            CREDENTIAL_UNKNOWN
        );
        assert_ne!(CREDENTIAL_UNKNOWN, CREDENTIAL_UNREPORTED);
        // The unknown sentence names the risk rather than leaving it to be
        // found out.
        assert!(
            CREDENTIAL_UNKNOWN.contains("second key"),
            "the unknown sentence stopped naming the risk: {CREDENTIAL_UNKNOWN}"
        );
    }

    /// Every state this daemon reports reaches its own sentence, and no two
    /// share one.
    #[test]
    fn each_credential_state_reaches_its_own_sentence() {
        let labels = [
            LABEL_CREDENTIAL_ABSENT,
            LABEL_CREDENTIAL_OBTAINING,
            LABEL_CREDENTIAL_FAILED,
            LABEL_CREDENTIAL_CANCELLED,
            LABEL_CREDENTIAL_PRESENT,
        ];
        let mut seen: Vec<&str> = Vec::new();
        for label in labels {
            let line = credential_state_line(label);
            assert_ne!(line, CREDENTIAL_UNKNOWN, "{label} fell through");
            assert_ne!(line, CREDENTIAL_UNREPORTED, "{label} fell through");
            assert!(!seen.contains(&line), "{label} borrows another sentence");
            seen.push(line);
        }
        // Only a stored key reads as settled. Waiting on a browser does not.
        assert!(credential_state_tone(LABEL_CREDENTIAL_PRESENT).reads_as_working());
        for label in [
            LABEL_CREDENTIAL_ABSENT,
            LABEL_CREDENTIAL_OBTAINING,
            LABEL_CREDENTIAL_FAILED,
            LABEL_CREDENTIAL_CANCELLED,
        ] {
            assert!(
                !credential_state_tone(label).reads_as_working(),
                "{label} must not read as settled"
            );
        }
        assert_eq!(
            credential_state_tone(LABEL_CREDENTIAL_FAILED),
            PrivateInferenceTone::Refused
        );
        assert_eq!(
            credential_state_tone(LABEL_CREDENTIAL_OBTAINING),
            PrivateInferenceTone::Held
        );
    }

    /// One state, one action, decided here. A shell that branched on two
    /// booleans instead would offer `Obtain` beside a key it already has,
    /// and the three shells would each get it wrong differently.
    #[test]
    fn one_action_table_answers_for_every_shell() {
        assert_eq!(
            credential_action(LABEL_CREDENTIAL_ABSENT),
            CredentialAction::Obtain
        );
        // A failed or a stopped attempt is offered the same way out: try
        // again. Neither is a state to leave somebody stuck in.
        assert_eq!(
            credential_action(LABEL_CREDENTIAL_FAILED),
            CredentialAction::Obtain
        );
        assert_eq!(
            credential_action(LABEL_CREDENTIAL_CANCELLED),
            CredentialAction::Obtain
        );
        assert_eq!(
            credential_action(LABEL_CREDENTIAL_OBTAINING),
            CredentialAction::Cancel
        );
        // Never `Obtain` beside a key that is already here.
        assert_eq!(
            credential_action(LABEL_CREDENTIAL_PRESENT),
            CredentialAction::Forget
        );
    }

    /// The consequence is stated where the decision is taken.
    ///
    /// The counterpart of `the_offer_says_what_it_exposes`. A button that
    /// opens a browser, signs somebody in to a company that is not this app
    /// and mints a key this app keeps is not one frictionless tap, and the
    /// sentence in front of it has to say each of those three things.
    #[test]
    fn getting_a_key_says_what_it_costs() {
        let copy = private_inference_copy();
        for fragment in [
            "browser",
            "signs you in",
            "makes a",
            "keeps on this computer",
        ] {
            assert!(
                copy.credential_cost.contains(fragment),
                "the cost sentence stopped saying {fragment:?}: {}",
                copy.credential_cost
            );
        }
        // The key is the contributor's, and where to go for it is named.
        assert!(
            copy.credential_cost.contains("your own Private AI account"),
            "the cost sentence stopped naming where the key lives: {}",
            copy.credential_cost
        );
    }

    /// Forgetting is local, and the sentence says so rather than letting
    /// "removed" be read as "revoked". `handle_forget` answers
    /// `revoked: false` for the same reason.
    #[test]
    fn forgetting_never_claims_a_revocation_it_cannot_perform() {
        let explains = private_inference_copy().credential_forget_explains;
        assert!(
            explains.contains("from this computer"),
            "the forget sentence stopped saying it is local: {explains}"
        );
        assert!(
            explains.contains("keeps working"),
            "the forget sentence stopped saying the key stays valid: {explains}"
        );
        assert!(
            explains.contains("cannot do that for you"),
            "the forget sentence stopped saying this app cannot revoke: {explains}"
        );
        for word in ["revoked", "cancelled", "deleted everywhere"] {
            assert!(
                !explains.to_lowercase().contains(word),
                "the forget sentence reads as {word}: {explains}"
            );
        }
    }

    /// NOTHING ON THIS SURFACE RENDERS A KEY. Hash-only and label-only, in
    /// copy as in logs: no sentence carries a key, a prefix, an id or an
    /// account name, and none of them may grow a hole for one.
    #[test]
    fn no_credential_sentence_can_carry_a_value() {
        let copy = private_inference_copy();
        for text in [
            copy.credential_title,
            copy.credential_what,
            copy.credential_cost,
            copy.credential_obtain,
            copy.credential_cancel,
            copy.credential_forget,
            copy.credential_forget_explains,
            copy.credential_absent,
            copy.credential_obtaining,
            copy.credential_failed,
            copy.credential_cancelled,
            copy.credential_present,
            copy.credential_unknown,
            copy.credential_unreported,
        ] {
            assert!(!text.contains("sk-"), "a key prefix appears in: {text}");
            for marker in ["{}", "{0}", "{key}", "{account}", "%@", "%s"] {
                assert!(
                    !text.contains(marker),
                    "{text} carries {marker}, which a shell would fill with a value"
                );
            }
        }
        // The one that would be tempting to fill in says instead that it
        // will not be.
        assert!(
            copy.credential_present.contains("never shown"),
            "the present sentence stopped refusing to show the key: {}",
            copy.credential_present
        );
    }

    /// A hidden connect control says why it is hidden, and says it as a next
    /// step rather than as a fault.
    ///
    /// The silent dead end this exists to close: #720 gates connecting a tool
    /// on holding a key, `offers_connect` folds the fact in, and a shell that
    /// simply drew nothing would leave a contributor with no way to connect
    /// their tool and no reason given.
    #[test]
    fn a_connect_that_is_not_on_offer_says_why_and_names_the_next_step() {
        let notice = private_inference_copy().harness_needs_credential;
        // The next step, spelled as the button that performs it is spelled.
        assert!(
            notice.contains(CREDENTIAL_OBTAIN),
            "the notice stopped naming the action that fixes it, which is \
             spelled {CREDENTIAL_OBTAIN:?}: {notice}"
        );
        // Not a fault. None of these words belongs in front of somebody who
        // has done nothing wrong.
        for word in ["error", "failed", "cannot", "invalid", "refused", "must"] {
            assert!(
                !notice.to_lowercase().contains(word),
                "the notice reads as {word}: {notice}"
            );
        }
    }

    /// THE NOTICE IS ABOUT A DESTINATION THIS APP HOSTS, AND SAYS SO.
    ///
    /// A proxy the contributor declared and runs themselves answers from an
    /// account whose key was never handed to us, and `destination_credentialed`
    /// is true for it -- so the sentence is never drawn there. The wording has
    /// to be safe anyway: a sentence claiming a tool needs our sign-in before
    /// it can be connected at all would be false for that contributor, and
    /// would send them looking for a key nothing wants.
    #[test]
    fn the_notice_never_claims_a_key_is_needed_to_connect_anything_anywhere() {
        let notice = private_inference_copy().harness_needs_credential;
        assert!(
            notice.contains("This computer would be the one answering"),
            "the notice stopped scoping itself to what this app answers: {notice}"
        );
        for claim in [
            "every tool",
            "any tool",
            "all tools",
            "tools need",
            "is required",
        ] {
            assert!(
                !notice.to_lowercase().contains(claim),
                "the notice generalises beyond what this app hosts ({claim}): {notice}"
            );
        }
    }

    /// A daemon that does not gate connects says nothing, and the silence is
    /// the point.
    ///
    /// `None` is not `Some(false)`. Drawing the notice on a build that has no
    /// credential gate would tell a contributor to sign in before connecting
    /// a tool they could connect right now -- the same shape of invention as
    /// an unread credential state rendering as `absent`.
    #[test]
    fn a_daemon_that_does_not_gate_connects_draws_no_notice() {
        assert_eq!(
            harness_credential_notice(Some(false)),
            HARNESS_NEEDS_CREDENTIAL
        );
        assert_eq!(harness_credential_notice(Some(true)), "");
        assert_eq!(
            harness_credential_notice(None),
            "",
            "an absent field is not a refused connect"
        );
    }

    /// Every state and every reason the daemon can produce has a sentence,
    /// and no two states share one.
    ///
    /// The set is read from the daemon's own pinned arrays rather than typed
    /// again here: a reason added there with no sentence would otherwise
    /// render as nothing at all, which is the failure this catches.
    #[test]
    fn every_eligibility_label_reaches_its_own_sentence() {
        use crate::daemon::contribution_eligibility::{ALL_REASONS, ALL_STATES};
        let mut seen: Vec<&str> = Vec::new();
        for state in ALL_STATES {
            let line = eligibility_state_line(state);
            assert!(!line.trim().is_empty(), "{state} has no sentence");
            assert!(
                !seen.contains(&line),
                "{state} borrows another state's sentence"
            );
            seen.push(line);
        }
        for reason in ALL_REASONS {
            let line = eligibility_reason_line(reason);
            assert!(!line.trim().is_empty(), "{reason} has no sentence");
        }
    }

    /// **The tri-state rule, on this surface.** A state this build has never
    /// heard of, or an empty one, must not borrow any known state's
    /// sentence -- least of all an ineligibility, which would tell a
    /// contributor their session is unsendable on no evidence at all -- and
    /// must offer no send control.
    #[test]
    fn an_unrecognised_eligibility_state_borrows_nothing() {
        for unknown in ["", "a_state_from_a_later_daemon", "ELIGIBLE", "eligible "] {
            assert_eq!(
                eligibility_state_line(unknown),
                ELIGIBILITY_UNKNOWN,
                "{unknown:?} must read as unevaluated"
            );
            for known in [
                ELIGIBILITY_ELIGIBLE,
                ELIGIBILITY_INELIGIBLE_PERMANENT,
                ELIGIBILITY_INELIGIBLE_CONFIGURATION,
            ] {
                assert_ne!(
                    eligibility_state_line(unknown),
                    known,
                    "{unknown:?} borrowed a known state's sentence"
                );
            }
            assert_eq!(eligibility_control(unknown), ContributionControl::None);
            assert_eq!(
                eligibility_state_tone(unknown),
                PrivateInferenceTone::Neutral
            );
            assert!(!eligibility_state_tone(unknown).reads_as_working());
        }
    }

    /// An unfamiliar reason renders as nothing rather than as a guess. The
    /// state sentence beside it has already said what is true.
    #[test]
    fn an_unrecognised_eligibility_reason_says_nothing() {
        for unknown in ["", "a_reason_from_a_later_daemon", "NO_CALL"] {
            assert_eq!(eligibility_reason_line(unknown), "");
        }
    }

    /// The send control is offered for `eligible` and for nothing else. This
    /// is the safety property of the surface in one assertion: a control on
    /// any other row is an action the transport cannot perform, discovered
    /// on the press.
    #[test]
    fn only_an_eligible_session_is_offered() {
        use crate::daemon::contribution_eligibility::ALL_STATES;
        for state in ALL_STATES {
            let expected = if state == ELIGIBILITY_STATE_ELIGIBLE {
                ContributionControl::Contribute
            } else {
                ContributionControl::None
            };
            assert_eq!(eligibility_control(state), expected, "{state}");
        }
    }

    /// Only the configuration state may name a setting, and it is the only
    /// one painted as actionable.
    #[test]
    fn only_the_configuration_state_reads_as_actionable() {
        use crate::daemon::contribution_eligibility::ALL_STATES;
        for state in ALL_STATES {
            let tone = eligibility_state_tone(state);
            if state == ELIGIBILITY_STATE_INELIGIBLE_CONFIGURATION {
                assert_eq!(tone, PrivateInferenceTone::Attention, "{state}");
            } else {
                assert_ne!(tone, PrivateInferenceTone::Attention, "{state}");
            }
        }
        // And a permanent ineligibility never reads as a working state.
        assert!(!eligibility_state_tone(ELIGIBILITY_STATE_INELIGIBLE_PERMANENT).reads_as_working());
    }

    /// `eligible` promises an expectation, never an outcome. The server
    /// decides, and a sentence that said otherwise would be read as a promise
    /// on the day the server says no.
    #[test]
    fn the_eligible_sentence_promises_nothing() {
        let text = ELIGIBILITY_ELIGIBLE.to_lowercase();
        for claim in ["will be accepted", "guarantee", "accepted"] {
            assert!(
                !text.contains(claim),
                "the eligible sentence claims {claim}"
            );
        }
        assert!(
            text.contains("when you send it"),
            "the eligible sentence must say the last checks are still to come"
        );
    }

    /// Every field of the payload carries a finished sentence: no empties,
    /// and no template markers a shell would have to fill in.
    #[test]
    fn every_sentence_arrives_finished() {
        let payload =
            serde_json::to_value(private_inference_copy()).expect("the payload serialises");
        let fields = payload.as_object().expect("a JSON object");
        assert_eq!(
            fields.len(),
            80,
            "the payload's field count changed -- update the shells' decoders \
             and the tests that pin the set"
        );
        for (field, value) in fields {
            let text = value.as_str().expect("every field is a string");
            assert!(!text.trim().is_empty(), "{field} is empty");
            for marker in ["{}", "{0}", "{port}", "%@", "%s", "%d"] {
                assert!(!text.contains(marker), "{field} carries {marker}: {text}");
            }
        }
    }

    /// Every string literal in the marked region, plus the sentences the
    /// region's functions assemble, swept for words this surface may not say.
    ///
    /// The list is not stylistic. "Private", "secure" and "encrypted" are
    /// claims this feature does not support -- the call still goes on to
    /// whoever answers it -- and the vendor and mechanism words are the
    /// vocabulary a contributor would have to learn before they could read a
    /// single sentence, which is the failure `routing_copy` documents.
    #[test]
    fn the_offer_surface_says_nothing_it_should_not() {
        let source = include_str!("private_inference_copy.rs");
        let region = source
            .split_once("PRIVATE-INFERENCE-SURFACE-BEGIN")
            .expect("begin marker present")
            .1
            .split_once("PRIVATE-INFERENCE-SURFACE-END")
            .expect("end marker present")
            .0;
        // Comment lines are stripped BEFORE splitting on quotes.
        //
        // The extraction below takes every odd-indexed `"`-delimited segment,
        // which is only the literals while the quote count ahead of each one
        // is even. A doc comment carrying an odd number of `"` shifts that
        // parity, and from there every real literal lands in a skipped
        // position -- the sweep still passes, having examined nothing. It is
        // the failure mode this test exists to prevent, in this test.
        let code_only: String = region
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut strings: Vec<String> = code_only
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect();

        // The floor is the number of constants actually declared between the
        // markers, not a number typed once and left behind. A hardcoded floor
        // silently stops meaning anything the moment the module grows past it,
        // which is how a parity shift would have gone unnoticed.
        let declared = code_only.matches("pub const ").count();
        assert!(
            declared >= 18,
            "only {declared} constants between the markers -- did they move out?"
        );
        assert!(
            strings.len() >= declared,
            "the sweep found {} literals for {declared} declared constants -- \
             a literal is being skipped, most likely a quote-parity shift",
            strings.len()
        );
        strings.push(serving_line(Some(8463)));
        for seconds in [None, Some(0), Some(1), Some(60), Some(3_600), Some(86_400)] {
            strings.push(harness_last_call_line(seconds));
        }
        for micros in [None, Some(0), Some(1), Some(1_230_000), Some(1_000_000_000)] {
            strings.push(harness_spend_line(micros));
        }
        for label in [
            LABEL_OFF,
            LABEL_RUNNING,
            LABEL_RUNNING_NO_BACKENDS,
            LABEL_RUNNING_ELSEWHERE,
            LABEL_PORT_IN_USE,
            LABEL_START_FAILED,
            LABEL_CRASHED,
            "a_state_from_a_later_daemon",
        ] {
            strings.push(state_line(label).to_string());
        }
        for label in [
            LABEL_CREDENTIAL_ABSENT,
            LABEL_CREDENTIAL_OBTAINING,
            LABEL_CREDENTIAL_FAILED,
            LABEL_CREDENTIAL_CANCELLED,
            LABEL_CREDENTIAL_PRESENT,
            "",
            "a_credential_state_from_a_later_daemon",
        ] {
            strings.push(credential_state_line(label).to_string());
        }
        for label in [
            ELIGIBILITY_STATE_ELIGIBLE,
            ELIGIBILITY_STATE_INELIGIBLE_PERMANENT,
            ELIGIBILITY_STATE_INELIGIBLE_CONFIGURATION,
            ELIGIBILITY_STATE_UNKNOWN,
            "",
            "an_eligibility_state_from_a_later_daemon",
        ] {
            strings.push(eligibility_state_line(label).to_string());
        }
        for label in crate::daemon::contribution_eligibility::ALL_REASONS
            .iter()
            .copied()
            .chain(["", "a_reason_from_a_later_daemon"])
        {
            strings.push(eligibility_reason_line(label).to_string());
        }
        for outcome in [
            "changes",
            "noop",
            "unparseable",
            "not_installed",
            "entry_unusable",
            "no_config_path",
            "an_outcome_from_a_later_daemon",
        ] {
            strings.push(harness_outcome_line(outcome).to_string());
        }
        for word in [
            "ironwire",
            "iron wire",
            "proxy",
            "backend",
            "route",
            "endpoint",
            "localhost",
            "private",
            "secure",
            "encrypt",
            "anonym",
            "protect",
            "credit",
            "earn",
        ] {
            for text in &strings {
                // The product name is stripped before the check, so
                // "Private AI" may be said and nothing else may say
                // "private". The ban exists to stop a PROMISE -- "your calls
                // are private" is false, because each call still goes on to
                // whoever was configured to answer it. A name is not a
                // promise: the mental model is a VPN, where "private" has
                // never meant the destination cannot see you, and the
                // exposure sentence still says in full what turning this on
                // lets anything else on the machine do.
                let text = text.replace(DESTINATION, "");
                assert!(
                    !text.to_lowercase().contains(word),
                    "{word:?} appears in: {text}"
                );
            }
        }
    }
}
