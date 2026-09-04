import CTraceCommons
import Foundation

/// The redaction witness surface, read from and written to the Rust rather
/// than modelled here.
///
/// Handle-free for the same reason `TCScrubInfo` and `TCRoutingCopy` are:
/// these calls read and write a config file, and the screen that uses them
/// is often the one deciding whether a daemon should be running at all. A
/// changed witness takes effect on the next submission -- the upload path
/// reloads the config per upload -- with no daemon restart.
///
/// Nothing in this file is a word. The vocabulary crosses as JSON and the
/// sentences cross already assembled, so there is no template for this shell
/// to fill in. Decoding and the mapping live in `TCShellCore`, where they are
/// unit-tested without linking the dylib.
///
/// This is the only place in the macOS shell that touches these symbols;
/// raw-pointer work is confined to this target.
public enum TCWitness {

    /// What a call that writes answered: either it did the thing, or it
    /// refused with a FIXED LABEL from the ABI.
    ///
    /// The label is an operator string like `witness-pin-required`, not
    /// wording. It never carries a path, a token, a URL or trace content, so
    /// it is safe to put in front of a person -- but no shell may build a
    /// sentence around it, because that sentence would exist in one shell
    /// only.
    public enum Outcome: Equatable, Sendable {
        /// The write succeeded. `changed` is false only for a clear that
        /// found no witness to remove, which is idempotent and not an error.
        case done(changed: Bool)
        case refused(label: String)
    }

    // MARK: - Reading

    /// What the witness is doing, as a `TC_WITNESS_STATE_*` value.
    ///
    /// The one call to make before rendering anything about the witness.
    /// `WitnessTrustState.fromABI` in `TCShellCore` is what turns it into a
    /// case, and a value this build cannot name becomes `.unnameable` there
    /// rather than `.absent`.
    public static func trustState(configDir: String) -> Int32 {
        configDir.withCString { tc_witness_trust_state($0) }
    }

    /// What the read side answered: the configuration, or the ABI's fixed
    /// label.
    ///
    /// **A refusal here is never "no witness"** -- that is `state: "absent"`
    /// on a successful read. It is the device not being enrolled, or a
    /// config that could not be read, and the second of those is a state in
    /// which nothing goes out at all.
    public enum StatusRead: Equatable, Sendable {
        case status(String)
        /// The ABI's fixed label: `witness-not-enrolled` or
        /// `witness-config-unreadable`. Neither means "no witness".
        case refused(label: String)
    }

    /// The whole configuration as JSON, or the ABI's fixed label when the
    /// device is not enrolled or the config cannot be read.
    ///
    /// The state to render in the refused case still comes from
    /// `trustState(configDir:)`, which answers for every input.
    public static func statusJSON(configDir: String) -> StatusRead {
        var errPtr: UnsafeMutablePointer<CChar>?
        let raw = configDir.withCString { cDir in
            withUnsafeMutablePointer(to: &errPtr) { errOut in
                tc_witness_status_json(cDir, errOut)
            }
        }
        guard let raw else {
            return .refused(label: takeLabel(&errPtr))
        }
        freeIfPresent(&errPtr)
        defer { tc_string_free(raw) }
        return .status(String(cString: raw))
    }

    /// What the last submission THIS PROCESS made did about the witness, as
    /// JSON. Process-local: a freshly started shell reports `not_observed`.
    ///
    /// Not consumed by the settings card, which prints
    /// `lastResultLine()` instead -- this payload's `refusal` is a fixed
    /// operator label and its `n_of_m` is a pair a shell must not phrase for
    /// itself. Exposed because it is part of the surface and is asserted
    /// against the real dylib.
    public static func lastResultJSON() -> String? {
        guard let raw = tc_witness_last_result_json() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    // MARK: - Writing

    /// Configure a witness.
    ///
    /// `measurementsJSON` is a JSON array of strings -- build it with
    /// `WitnessForm.measurementsJSON`, never by concatenation. The ABI will
    /// not write an unpinned witness: an empty array is refused with
    /// `witness-pin-required` and an unparsable one with
    /// `witness-pin-malformed`, because either produces a client that
    /// refuses every submission from the moment it is saved.
    ///
    /// Nothing here is applied optimistically. The caller re-reads the
    /// status afterwards and publishes what comes back.
    public static func configure(
        configDir: String,
        url: String,
        signingAddress: String,
        measurementsJSON: String
    ) -> Outcome {
        var errPtr: UnsafeMutablePointer<CChar>?
        let code = configDir.withCString { cDir in
            url.withCString { cURL in
                signingAddress.withCString { cAddr in
                    measurementsJSON.withCString { cPins in
                        withUnsafeMutablePointer(to: &errPtr) { errOut in
                            tc_witness_configure(cDir, cURL, cAddr, cPins, errOut)
                        }
                    }
                }
            }
        }
        if code == 0 {
            freeIfPresent(&errPtr)
            return .done(changed: true)
        }
        return .refused(label: takeLabel(&errPtr))
    }

    /// Remove the configured witness.
    ///
    /// This returns the client to LOCAL REDACTION, a supported mode rather
    /// than a broken one -- which is why the card says what is happening
    /// rather than presenting it as switching a setting off. Idempotent:
    /// clearing a witness that is not there is `.done(changed: false)`.
    public static func clear(configDir: String) -> Outcome {
        var errPtr: UnsafeMutablePointer<CChar>?
        let code = configDir.withCString { cDir in
            withUnsafeMutablePointer(to: &errPtr) { errOut in
                tc_witness_clear(cDir, errOut)
            }
        }
        if code >= 0 {
            freeIfPresent(&errPtr)
            return .done(changed: code == 1)
        }
        return .refused(label: takeLabel(&errPtr))
    }

    // MARK: - The words

    /// Every fixed word on the surface, as a JSON object, or nil if the ABI
    /// reported a caught panic.
    ///
    /// GENERATED from `trace_commons_contributor::witness_copy`. One call,
    /// not one per string: a shell handed the words one at a time takes some
    /// of them and writes the rest, and several of these are privacy claims.
    public static func copyJSON() -> String? {
        guard let raw = tc_witness_copy() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The sentence for a state, or nil for a value this build cannot name.
    ///
    /// A shell that gets nil must render NO witness sentence rather than one
    /// of its own, and pairs that with `stateTone`, which fails closed to
    /// `TC_WITNESS_TONE_REFUSED` on the same input.
    public static func stateLine(state: Int32) -> String? {
        guard let raw = tc_witness_state_line(state) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How that sentence is painted: `TC_WITNESS_TONE_*`.
    ///
    /// ONE BRANCH TABLE, NOT TWO -- this takes what the sentence takes. Do
    /// not recover the tone by comparing the rendered sentence against
    /// anything. Never fails: a state this build cannot name answers
    /// refused, which is the fail-closed direction.
    public static func stateTone(state: Int32) -> Int32 {
        tc_witness_state_tone(state)
    }

    /// What the last submission did, in one sentence. The only form a shell
    /// may print. When a certificate carried a receipt count this sentence
    /// already contains it as the pair -- never the word "attested", and
    /// never a claim that a session is clean.
    public static func lastResultLine() -> String? {
        guard let raw = tc_witness_last_result_line() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How that sentence is painted: `TC_WITNESS_TONE_*`.
    public static func lastResultTone() -> Int32 {
        tc_witness_last_result_tone()
    }

    // MARK: - Owned strings

    /// Take the ABI's fixed label out of an `err` out-parameter and free it.
    ///
    /// `"panic"` when the call failed without setting one; the ABI records a
    /// label for every documented refusal, so this stands for a caught panic
    /// and nothing else. Never an invented sentence.
    private static func takeLabel(_ errPtr: inout UnsafeMutablePointer<CChar>?) -> String {
        guard let e = errPtr else { return "panic" }
        errPtr = nil
        defer { tc_string_free(e) }
        return String(cString: e)
    }

    /// A successful call can still have written an `err`; free it rather
    /// than leaking it.
    private static func freeIfPresent(_ errPtr: inout UnsafeMutablePointer<CChar>?) {
        if let e = errPtr {
            errPtr = nil
            tc_string_free(e)
        }
    }
}
