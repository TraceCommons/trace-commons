using System;
using System.Runtime.InteropServices;

namespace TraceCommons.Interop;

/// <summary>
/// Raw P/Invoke declarations for the trace-commons-contributor-ffi C ABI.
///
/// This file is the hand-maintained mirror of
/// <c>macos/Sources/CTraceCommons/include/trace_commons.h</c>, which is itself
/// the hand-maintained mirror of
/// <c>crates/trace-commons-contributor-ffi/src/lib.rs</c>. All three change
/// together or not at all.
///
/// NOTHING outside <see cref="TcDaemon"/>, <see cref="TcPreview"/> and
/// <see cref="TcSubscription"/> may call into this class. That restriction is
/// what lets the ownership rules below be stated once and enforced in one
/// place, exactly as the Swift binding does it.
///
/// THREE MARSHALLING RULES, and every one of them is load-bearing:
///
/// 1. Owned returns are <see cref="IntPtr"/>, never <c>string</c>. Declaring a
///    <c>char*</c> return as a marshalled <c>string</c> makes the CLR free the
///    pointer with <c>Marshal.FreeCoTaskMem</c>, which is not the allocator
///    Rust used. That is heap corruption, and it is silent until it isn't. So
///    every owned return crosses as IntPtr and is converted with
///    <c>Marshal.PtrToStringUTF8</c> then released with
///    <see cref="tc_string_free"/>.
///
/// 2. Inputs are <c>LPUTF8Str</c>. The ABI is UTF-8 throughout; the .NET
///    default for <c>string</c> on Windows is UTF-16, and the "ANSI" fallback
///    is a lossy code-page conversion. A project label with a non-ASCII
///    character would arrive mangled.
///
/// 3. Borrowed returns (<c>const char*</c>) are also IntPtr, and must NOT be
///    passed to tc_string_free. The header's ownership rule is stated once
///    and is absolute: owned means returned <c>char*</c>, borrowed means
///    <c>const char*</c> and lives until its owning handle is freed.
/// </summary>
internal static class NativeMethods
{
    // Windows activation argument parsing; these are OS imports, not C ABI exports.
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern IntPtr CommandLineToArgvW(string commandLine, out int count);

    [DllImport("kernel32.dll")]
    internal static extern IntPtr LocalFree(IntPtr memory);

    /// <summary>
    /// The cdylib's base name. .NET's probing appends the platform decoration:
    /// <c>trace_commons_contributor_ffi.dll</c> on Windows,
    /// <c>libtrace_commons_contributor_ffi.dylib</c> on macOS,
    /// <c>libtrace_commons_contributor_ffi.so</c> on Linux. The name is
    /// therefore written undecorated so the interop tests can run against a
    /// macOS or Linux build of the very same crate the Windows app ships.
    /// </summary>
    internal const string Library = "trace_commons_contributor_ffi";

    /// <summary>
    /// The tc_subscribe callback. Held alive by <see cref="TcSubscription"/>
    /// for as long as the subscription can fire: a delegate passed to native
    /// code is not rooted by the native side, and letting the GC collect it
    /// while Rust still holds the function pointer is a hard crash.
    /// </summary>
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate void TcEventCallback(IntPtr eventJson, IntPtr ctx);

    /// <summary>
    /// Starts the daemon loop on its own thread with its own runtime.
    /// Returns NULL and sets <paramref name="err"/> on failure -- most
    /// notably when another daemon already holds daemon.lock for this
    /// config_dir. <paramref name="err"/> is owned; free it with
    /// <see cref="tc_string_free"/>.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_daemon_start(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configDir,
        out IntPtr err);

    /// <summary>
    /// As <see cref="tc_daemon_start"/>, but applies settings BEFORE the
    /// watcher's first tick.
    ///
    /// This is the only way to point the daemon at a session store other than
    /// the real <c>~/.claude</c> / <c>~/.codex</c> from the very first pass:
    /// <c>set_settings</c> takes effect only on an already-running daemon, by
    /// which point the first scan has happened. The interop tests depend on it
    /// for exactly that reason, and so should any host that watches a
    /// relocated store.
    ///
    /// <paramref name="settingsJson"/> may be null, meaning "use whatever is
    /// persisted". Note the marshalling attribute is applied to a nullable
    /// string: LPUTF8Str marshals null to a null pointer, which is what the
    /// ABI documents as the "use persisted settings" case.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_daemon_start_with_settings(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string configDir,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? settingsJson,
        out IntPtr err);

    /// <summary>
    /// Stops the daemon loop. Idempotent, safe from any thread, safe with
    /// NULL. Does NOT free the handle, does NOT end subscriptions, and is NOT
    /// a teardown barrier for a second concurrent caller.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void tc_daemon_stop(IntPtr handle);

    /// <summary>
    /// The only function that reclaims what tc_daemon_start returned. Must run
    /// on a plain thread outside any tokio runtime context, and must not race
    /// any other call still using the pointer.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void tc_handle_free(IntPtr handle);

    /// <summary>
    /// Account-free synchronous local Insights. Request is UTF-8 bytes without
    /// a trailing NUL, at most 65536 bytes. Call off the UI thread and retain
    /// the buffer until return. Returns owned JSON or NULL plus owned error;
    /// both use tc_string_free. Closing a window does not cancel started IO.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_insights_call(
        [In] byte[] request, UIntPtr requestLen, out IntPtr error);

    /// <summary>Account-free bounded local mission draft inbox.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_mission_drafts_call(
        [In] byte[] request, UIntPtr requestLen, out IntPtr error);

    /// <summary>
    /// Calls a daemon method in-process. Returns an owned NUL-terminated JSON
    /// response -- never NULL, even for a bad handle or malformed params; a
    /// JSON error frame comes back instead.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_call(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string method,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string paramsJson);

    /// <summary>
    /// Registers an event callback, invoked on a Rust background thread.
    /// Returns 0 on failure; 0 is never a valid token. <paramref name="ctx"/>
    /// must stay valid until tc_unsubscribe RETURNS -- not until
    /// tc_daemon_stop returns.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern ulong tc_subscribe(IntPtr handle, TcEventCallback cb, IntPtr ctx);

    /// <summary>
    /// The ABI's only real barrier: blocks until the callback is guaranteed
    /// not to fire again. Returns void and REFUSES SILENTLY when called from a
    /// thread inside any tokio runtime context, so callers must compare
    /// <see cref="tc_last_error"/> across the call to learn whether the
    /// barrier actually held. <see cref="TcDaemon"/> does that.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void tc_unsubscribe(IntPtr handle, ulong token);

    /// <summary>
    /// Opens a preview: reads the session file and runs the real redaction
    /// pipeline. Blocks the calling thread for the redaction pass.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_preview_open(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        out IntPtr err);

    /// <summary>Borrowed. The redacted transcript, UTF-8. Do not free.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_preview_body(IntPtr preview);

    /// <summary>Borrowed. Counts, sizes, opening prompt. Do not free.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_preview_summary_json(IntPtr preview);

    /// <summary>
    /// Returns the match count, or -1 on error. On success
    /// <paramref name="matchesJson"/> is an owned JSON array of UTF-8 BYTE
    /// offsets -- not character offsets, and not UTF-16 indices. See
    /// <see cref="TcPreview.Search"/> for the conversion, which is required
    /// before those offsets can index a C# string.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_preview_search(
        IntPtr preview,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string needle,
        out IntPtr matchesJson);

    /// <summary>
    /// Counts occurrences of <paramref name="needle"/> in an entry's
    /// PRE-redaction session text. Returns the count, or -1 on error.
    /// </summary>
    /// <remarks>
    /// A COUNT, never content -- that is the whole bound of this call, and the
    /// reason it is allowed to read unredacted bytes at all. It takes a handle
    /// and an entry id rather than a preview because a preview lives as long
    /// as its sheet, and an unredacted transcript must not: the daemon reads
    /// the file, counts, and drops it.
    /// </remarks>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_search_original(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string needle);

    /// <summary>
    /// Frees a preview. Invalidates every borrowed pointer previously handed
    /// out for it. Safe with NULL.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void tc_preview_free(IntPtr preview);

    /// <summary>
    /// Describes the session stores on this machine, so the roots screen can
    /// ask about something specific rather than showing an empty field.
    ///
    /// Takes no handle by design: it runs BEFORE any daemon exists, because
    /// the screen that uses it is the one clearing the refusal that stops a
    /// daemon from starting.
    ///
    /// Returns an owned JSON array; free it with
    /// <see cref="tc_string_free"/>, which <see cref="TakeOwnedString"/>
    /// does. NULL only on a caught panic. See
    /// <see cref="SourceDiscovery"/> for the element shape.
    ///
    /// This is the one call in this ABI that deliberately returns a
    /// filesystem path, and the reason is the same one that keeps paths out
    /// of everything else: here the caller is the contributor's own machine
    /// asking which of their own folders to watch, and a consent prompt that
    /// will not name what it is asking about is not a consent prompt.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_discover_sources();

    /// <summary>
    /// The names of the scrubber's secret detectors, as an owned JSON array of
    /// strings; free it with <see cref="tc_string_free"/>, which
    /// <see cref="TakeOwnedString"/> does. NULL only on a caught panic.
    ///
    /// NAMES ONLY. This deliberately does not expose the patterns: publishing
    /// the regexes would tell someone trying to slip a secret past the scrubber
    /// exactly what to avoid. The names exist so a shell can tell a contributor
    /// what is caught without the list being maintained by hand and quietly
    /// going stale.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_scrub_detector_names();

    /// <summary>
    /// Every fixed word on the routing surface, as an owned JSON object; free
    /// it with <see cref="tc_string_free"/>, which
    /// <see cref="TakeOwnedString"/> does. NULL only on a caught panic.
    ///
    /// ONE CALL, NOT ONE PER STRING. This is a whole screen's wording and it
    /// arrives as a set, so this shell cannot take four of the words and
    /// hand-write the fifth. Exactly one of them claims privacy; a
    /// hand-written copy of that claim would stop matching the other two
    /// shells the day the claim changed, and nothing would notice.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_routing_copy();

    /// <summary>
    /// Every fixed sentence on the consent surface, as an owned JSON object;
    /// free it with <see cref="tc_string_free"/>, which
    /// <see cref="TakeOwnedString"/> does. NULL only on a caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_consent_copy();

    /// <summary>
    /// Which of the two Contribute tooltips applies, chosen on the Rust
    /// side. 1 is pinned; 0 and anything else are not.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_consent_gate_help(int pinned);

    /// <summary>
    /// Every fixed word on the private-inference offer and settings card, as
    /// an owned JSON object.
    ///
    /// ONE CALL, NOT ONE PER STRING, for the reason
    /// <see cref="tc_routing_copy"/> gives. The sentence a per-string export
    /// would invite this shell to hand-write is the one saying that while the
    /// switch is on, anything else running on this computer can send calls
    /// through it, charged to the accounts configured here -- which is the
    /// reason the offer is allowed to exist at all.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_private_inference_copy();

    /// <summary>
    /// The sentence for one <c>private_inference_state</c> label. A label
    /// this build has never heard of -- and a NULL one -- reads as the off
    /// sentence, which claims nothing; it never falls through to one of the
    /// three "on" sentences. NULL only on a caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_private_inference_state_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// How that sentence is painted, as a raw
    /// <c>TC_PRIVATE_INFERENCE_TONE_*</c> value.
    ///
    /// Takes what the sentence takes, so the two cannot drift apart. This
    /// shell must NOT recover the tone by reading the rendered sentence:
    /// three of the seven begin with the same two words. Never fails --
    /// anything unreadable answers the neutral tone, which claims nothing and
    /// is never the working light.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_private_inference_state_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// Where the listener is answering, already assembled. A port outside
    /// 1..65535 -- including the 0 this shell passes for a JSON null -- gives
    /// the EMPTY STRING rather than a sentence naming a number nobody bound.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_private_inference_serving_line(int port);

    /// <summary>
    /// Whether to put the offer in front of the contributor. Non-zero to ask.
    ///
    /// THE BRANCH TABLE CROSSES, and this branch decides whether to interrupt
    /// somebody. Three shells each deciding when to ask is three chances to
    /// re-ask a contributor who already said no. Answers 0 on a caught panic:
    /// silence is the safe direction for an interruption.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_private_inference_should_offer(int answered, int on);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_private_inference_write_confirmed(int requestedOn, int echoedSeen, int echoedOn);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_private_inference_quit_needs_notice(int requestedOn, [MarshalAs(UnmanagedType.LPUTF8Str)] string state);

    /// <summary>
    /// Why a connect control is not on offer, or the EMPTY STRING.
    ///
    /// <paramref name="credentialed"/> is <c>harness_list</c>'s
    /// <c>destination_credentialed</c> as a tri-state: negative for an absent
    /// field, 0 false, 1 true. AN ABSENT FIELD IS NOT A REFUSED CONNECT -- a
    /// daemon that predates the gate answers the empty string, because telling
    /// somebody to sign in before connecting a tool they can connect right now
    /// would be false.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_harness_credential_notice(int credentialed);

    /// <summary>
    /// The sentence for one <c>near_ai_credential_status</c> state label.
    ///
    /// An empty, NULL or non-UTF-8 label reports that this daemon does not
    /// answer the question; an unfamiliar one reports that the state could
    /// not be read. NEITHER SAYS THAT NO KEY IS KEPT HERE -- that is a claim
    /// about the machine, and a shell that made it up would invite a second
    /// sign-in. NULL only on a caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_near_ai_credential_state_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// How that sentence is painted, as a raw
    /// <c>TC_PRIVATE_INFERENCE_TONE_*</c> value -- the same five the listener
    /// row uses, deliberately, so this shell keeps one mapping onto colours
    /// rather than two that must agree.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_near_ai_credential_state_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// The one action a shell may offer for a credential state, as a raw
    /// <c>TC_CREDENTIAL_ACTION_*</c> value.
    ///
    /// THE BRANCH TABLE CROSSES, NOT ONLY THE WORDS. The button in question
    /// opens a browser and mints a key at a third party, and the safe
    /// direction is NONE: drawing obtain beside a state nobody could read is
    /// how a contributor ends up holding a second key.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_near_ai_credential_action(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// The sentence for one <c>near_ai_balance</c> state.
    ///
    /// An empty, NULL or non-UTF-8 state reports that this daemon does not
    /// answer the question; a state this build has never heard of gets its
    /// own sentence and BORROWS NOBODY'S. Neither may degrade to the one
    /// saying no sign-in is kept here, which is a claim about this machine.
    ///
    /// <c>known</c> answers the EMPTY STRING, and the emptiness is the point:
    /// that state's row is figures, and a sentence above them announcing that
    /// the read succeeded is this app narrating itself. NULL only on a caught
    /// panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_near_ai_balance_state_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// How firmly that sentence reads, as a raw
    /// <c>TC_PRIVATE_INFERENCE_TONE_*</c> value.
    ///
    /// <c>known</c> is the only clear one, and it means THE READ SUCCEEDED,
    /// not that the balance is healthy. Nothing across this ABI judges an
    /// amount, so a shell painting a low figure red would be inventing a
    /// threshold nobody set, on an account whose ceiling may not exist at
    /// all.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_near_ai_balance_state_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// The one action a shell may offer beside a balance state, as a raw
    /// <c>TC_CREDENTIAL_ACTION_*</c> value.
    ///
    /// The sign-in row's enum and not a second one, because the only action
    /// this row has ever needed is that row's obtain. Two states answer it,
    /// and a refused session gets it WITHOUT a forget first: the ceremony
    /// overwrites both records, and forgetting would throw away a working key
    /// to fix an unrelated sign-in.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_near_ai_balance_action(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// One balance figure as money.
    ///
    /// <paramref name="present"/> IS A SEPARATE ARGUMENT, DELIBERATELY. Every
    /// other money export here encodes absence as an out-of-range integer;
    /// this one cannot, because these amounts are SIGNED, and folding "null"
    /// onto "negative" would render a real debt as no figure at all. 0 is the
    /// wire's null and gives the EMPTY STRING.
    ///
    /// AN EMPTY STRING IS NEVER <c>$0.00</c>. A null here means we know we do
    /// not know; a zero is a real balance and means the money is gone.
    ///
    /// <paramref name="scale"/> IS THE WIRE'S OWN <c>scale</c> FIELD, not a
    /// constant. It is on the wire because a daemon may change it, and a
    /// shell dividing by a billion of its own would then be wrong by a factor
    /// of a thousand.
    ///
    /// Returns an owned string; free it with <see cref="tc_string_free"/>,
    /// which <see cref="TakeOwnedString"/> does.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_near_ai_balance_amount(int present, long nanos, byte scale);

    /// <summary>
    /// What is left, as a finished sentence.
    ///
    /// <c>present == 0</c> DOES NOT GIVE THE EMPTY STRING HERE. It gives the
    /// sentence for an account with no spending limit set, which is the
    /// ordinary case for an account nobody has capped -- and that contributor
    /// must not be told they have $0.00 left.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_near_ai_balance_remaining_line(
        int present, long nanos, byte scale);

    /// <summary>
    /// The configured ceiling, as a finished sentence, or the EMPTY STRING.
    ///
    /// <c>present == 0</c> is empty and not a sentence: the remaining line
    /// has already said the part that matters about an uncapped account, and
    /// saying it twice is once too many.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_near_ai_balance_limit_line(
        int present, long nanos, byte scale);

    /// <summary>
    /// What the WHOLE ACCOUNT has spent, as a finished sentence, or the empty
    /// string. A zero is not that: an account that has spent nothing renders
    /// $0.00, which is true.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_near_ai_balance_spent_line(
        int present, long nanos, byte scale);

    /// <summary>
    /// How long ago THIS COMPUTER asked, assembled.
    ///
    /// ABSENCE IS AN OUT-OF-RANGE INTEGER, the convention
    /// <see cref="tc_harness_last_call_line"/> uses: any negative value gives
    /// the empty string. The timestamp behind it is the daemon's own clock at
    /// the moment the service answered, so the sentence says when the
    /// question was put and never that anything was updated then.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_near_ai_balance_observed_line(long secondsAgo);

    /// <summary>
    /// The sentence for one queue entry's <c>eligibility</c> label.
    ///
    /// A ROW THAT CARRIED NO <c>eligibility</c> FIELD MUST NOT REACH HERE. An
    /// absent field means the contributor was invited and has no eligibility
    /// question, and answering one they do not have puts a caveat on work
    /// that carries none. Absent is not "unknown".
    ///
    /// An empty, NULL or unfamiliar state reports that the answer has not
    /// been worked out. IT NEVER REPORTS AN INELIGIBILITY: a state this build
    /// cannot read is not evidence about a contributor's session. NULL only
    /// on a caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_contribution_eligibility_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// How that sentence is painted, as a raw
    /// <c>TC_PRIVATE_INFERENCE_TONE_*</c> value -- the same five the listener
    /// row uses, so this shell keeps one mapping onto colours.
    ///
    /// A permanent ineligibility is deliberately NOT <c>_REFUSED</c>: nothing
    /// was refused and nothing went wrong, and painting a contributor's
    /// ordinary older work as a failure is a judgement this surface has no
    /// business making.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_contribution_eligibility_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// The one control a shell may offer for an eligibility state, as a raw
    /// <c>TC_CONTRIBUTION_CONTROL_*</c> value.
    ///
    /// THE BRANCH TABLE CROSSES, NOT ONLY THE WORDS. Three shells each
    /// deciding which rows get a send button is three chances to offer one
    /// beside a session the server will refuse -- the defect this surface
    /// exists to remove, and worse than an inert button: pressing it sends a
    /// contributor's work and has it turned away.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_contribution_eligibility_control(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// The sentence for one queue entry's <c>eligibility_reason</c> label, or
    /// THE EMPTY STRING for an absent, NULL or unfamiliar reason -- for which
    /// a shell renders nothing.
    ///
    /// Deliberately not the hedge the state line makes: the state sentence
    /// has already said what is true, and a second sentence guessing at a
    /// reason this build does not know would add a detail nobody
    /// established. NULL only on a caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_contribution_eligibility_reason_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? reason);

    /// <summary>
    /// The sentence for one queue entry's <c>attestation</c> mark: whether
    /// that session carries proof of the model call that produced it.
    ///
    /// EVERY ROW REACHES HERE, INCLUDING AN INVITED CONTRIBUTOR'S. That is
    /// the opposite of the eligibility line's rule, and deliberately so: the
    /// mark is a fact about the trace rather than an answer to a permission
    /// question nobody asked.
    ///
    /// An empty, NULL or unfamiliar mark reports that the answer is not
    /// known, which is also the honest reading of a daemon too old to send
    /// the field. IT NEVER REPORTS AN UNATTESTED SESSION. NULL only on a
    /// caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_contribution_attestation_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? mark);

    /// <summary>
    /// The sentence for one row of the certificate-held list.
    /// </summary>
    /// <remarks>
    /// <c>evidenceAdmitted</c> is the daemon's
    /// <c>admission_evidence_required</c> VERBATIM, never its negation. That
    /// flag is true for a contributor who signed up through NEAR and
    /// therefore has NO invite, so a non-zero argument returns the candidate
    /// reading. Passing the negation would swap both readings and compile.
    /// </remarks>
    /// <summary>
    /// The sentence for one NEAR AI login-enrolment control name.
    /// </summary>
    /// <remarks>
    /// Ten labels, ten sentences, and anything else reaching the generic one.
    /// Never the empty string: a refusal this build cannot name is the whole
    /// of what a contributor is being told.
    /// </remarks>
    /// <summary>
    /// What the outcome list says about a refused contribution, or the empty
    /// string when the label is not one of the five.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_outcome_refusal_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? label);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_queue_outcome_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? label);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_near_ai_enroll_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? label);

    /// <summary>How firmly that sentence reads.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_near_ai_enroll_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? label);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_certificate_row_line(int evidenceAdmitted);

    /// <summary>The heading over that list, on the same argument.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_certificate_list_title(int evidenceAdmitted);

    /// <summary>
    /// How that sentence is painted, as a raw
    /// <c>TC_PRIVATE_INFERENCE_TONE_*</c> value.
    ///
    /// A permanently unattested session is deliberately NOT
    /// <c>_REFUSED</c> and not even attention: nothing was refused and
    /// nothing went wrong, the session simply has no attached call. Only the
    /// configuration mark asks for attention, because a setting decides
    /// whether the next session carries proof.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_contribution_attestation_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? mark);

    /// <summary>
    /// The sentence for one queue entry's <c>attestation_reason</c> label, or
    /// THE EMPTY STRING for an absent, NULL or unfamiliar reason -- for which
    /// a shell renders nothing.
    ///
    /// THE SAME THIRTEEN LABELS AS
    /// <see cref="tc_contribution_eligibility_reason_line"/>, WITH DIFFERENT
    /// SENTENCES. Never substitute one for the other: eligibility's read as
    /// refusals of a request, and for an invited contributor nothing was
    /// requested. NULL only on a caught panic.
    /// </summary>
    /// <remarks>
    /// There is no <c>tc_contribution_attestation_control</c> to go with
    /// these two, and that absence is the contract. The mark describes the
    /// trace and offers nothing to press; whether a session may be sent stays
    /// <see cref="tc_contribution_eligibility_control"/>'s question.
    /// </remarks>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_contribution_attestation_reason_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? reason);

    /// <summary>
    /// How many sessions a group submit is leaving behind, as a sentence, or
    /// THE EMPTY STRING for zero and for a negative.
    /// </summary>
    /// <remarks>
    /// Says how many and NOT why. The reason a particular session cannot be
    /// sent is that row's own sentence one level in; a summary here would
    /// stand for up to thirteen different reasons and say nothing true about
    /// any of them. NULL only on a caught panic.
    /// </remarks>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_contribution_withheld_line(long withheld);

    /// <summary>
    /// Whether a group's submit control may be offered, as a raw
    /// <c>TC_CONTRIBUTION_CONTROL_*</c> value.
    /// </summary>
    /// <remarks>
    /// <paramref name="contributable"/> is the project row's
    /// <c>contributable_count</c>, or <b>ANY NEGATIVE VALUE when that key was
    /// ABSENT</b> -- an invited contributor, for whom every pending session is
    /// sendable.
    ///
    /// <para>
    /// ABSENT IS NOT ZERO, and it is the distinction most likely to be got
    /// wrong: zero means the question applies and nothing here can be sent, so
    /// nothing is offered; negative means the question does not apply and the
    /// control is offered on <paramref name="pending"/> alone. Passing 0 for an
    /// absent field would refuse a control to somebody whose sessions are all
    /// perfectly sendable -- which is what a <c>?? 0</c> on a nullable count
    /// does, so do not write one.
    /// </para>
    /// </remarks>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_contribution_group_control(long pending, long contributable);

    /// <summary>
    /// One <c>harness_list</c> row's state, as a TC_HARNESS_STATE_* code.
    ///
    /// THE BRANCH TABLE CROSSES. "answering" is the only value meaning a call
    /// was actually served, and it is the one a shell is most tempted to infer
    /// from "connected". Do not infer it: a config file naming this computer is
    /// not evidence that anything was ever sent. A label this build has never
    /// heard of answers TC_HARNESS_STATE_UNKNOWN.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_harness_state_code(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// One <c>harness_plan</c> outcome, as a TC_HARNESS_PLAN_* code.
    ///
    /// The branch that matters is unparseable against noop: one is "nothing to
    /// change", the other is "we refused to rewrite a file we could not read".
    /// An outcome this build has never heard of answers TC_HARNESS_PLAN_UNKNOWN,
    /// which is not committable.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_harness_plan_outcome_code(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? outcome);

    /// <summary>
    /// The sentence for one <c>harness_list</c> row's state.
    ///
    /// THE SENTENCE CROSSES, NOT ONLY THE CODE. With the code alone this shell
    /// held its own map from a state onto one of
    /// <c>tc_private_inference_copy</c>'s fields, and so did the other two:
    /// three copies of one decision.
    ///
    /// TWO STATES ANSWER THE EMPTY STRING, AND THE EMPTINESS IS THE POINT.
    /// "activity_shared" and "unknown" have no sentence and may not borrow one
    /// -- the answering sentence says a call from IT reached this computer, and
    /// the pronoun names the row's own tool, which is what activity_shared says
    /// cannot be worked out. Draw an empty string as no line at all. A label
    /// this build has never heard of, a NULL one, and a caught panic answer the
    /// same way.
    ///
    /// Returns an owned string; free it with <see cref="tc_string_free"/>,
    /// which <see cref="TakeOwnedString"/> does.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_harness_state_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// The sentence one <c>harness_plan</c> outcome carries.
    ///
    /// THE SENTENCE CROSSES, NOT ONLY THE CODE, for the reason
    /// <see cref="tc_harness_state_line"/> gives. "unparseable" was the only
    /// outcome with a sentence anywhere, and this shell held that arm itself;
    /// the other four non-committable outcomes had none, so a preview for one
    /// of them opened holding a title, a path, no changes, no explanation and
    /// a way out.
    ///
    /// "changes" ANSWERS THE EMPTY STRING, and the emptiness is the point: the
    /// preview shows the changes, and a sentence above them announcing that
    /// there are changes is this app narrating its own list. An outcome this
    /// build has never heard of answers the same way, rather than borrowing
    /// the nearest refusal, as do a NULL one and a caught panic.
    ///
    /// Returns an owned string; free it with <see cref="tc_string_free"/>,
    /// which <see cref="TakeOwnedString"/> does.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_harness_outcome_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? outcome);

    /// <summary>
    /// When the last call from a connected tool was answered here, assembled.
    ///
    /// ABSENCE IS AN OUT-OF-RANGE INTEGER, the convention
    /// <see cref="tc_private_inference_serving_line"/> already uses: any
    /// negative value -- including the one this shell passes for an absent or
    /// unparseable timestamp -- gives the EMPTY STRING rather than a sentence
    /// about a call nobody saw.
    ///
    /// Returns an owned string; free it with <see cref="tc_string_free"/>,
    /// which <see cref="TakeOwnedString"/> does.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_harness_last_call_line(long secondsAgo);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_harness_spend_line(long micros);

    /// <summary>
    /// Whether one action may be offered for a tool in this state. Non-zero to
    /// offer.
    ///
    /// The second rule is the one worth crossing the ABI for: a tool that is
    /// not installed cannot be connected, but a tool that IS connected can
    /// always be disconnected, installed or not. Answers 0 -- do not offer --
    /// for an action this build does not know.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_harness_action_available(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? action,
        int installed,
        int connected);

    /// <summary>
    /// The routing surface's "that file could not be used" sentence, already
    /// assembled. <paramref name="tokenPath"/> may be NULL, which is the
    /// "nothing resolved at all" case and a different sentence, not an error.
    ///
    /// ASSEMBLED ON THE RUST SIDE. This ABI exports no template with a hole in
    /// it, because a template this shell fills in is another place the wording
    /// lives. Do not rebuild these sentences from parts.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_routing_token_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? tokenPath);

    /// <summary>
    /// The routing surface's "nothing answered" sentence, already assembled.
    /// A port outside 1..65535 -- including the 0 for "no port was tried" --
    /// produces the sentence that names no port.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_routing_unreachable_line(int port);

    /// <summary>
    /// The routing surface's discovery sentence, already assembled.
    /// <paramref name="port"/> is what <c>discover_routing</c> reported, or 0
    /// for a machine that published no pointer -- which is the ordinary
    /// machine and NOT an error. A port outside 1..65535 is that same case.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_routing_discovery_line(int port);

    /// <summary>
    /// The routing surface's "Last checked ..." sentence, assembled around
    /// this shell's own humanised time. NULL, with an error recorded, for a
    /// NULL or non-UTF-8 argument: "Last checked " with nothing after it is
    /// worse than no line at all.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_routing_last_checked(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? when);

    /// <summary>
    /// One tool's word, from the contributor's per-source mode and what
    /// IronWire said about that tool.
    ///
    /// THE BRANCH TABLE CROSSES, NOT ONLY THE WORDS. Without this call this
    /// shell would decide which of the four words a tool gets, and three
    /// native copies of that decision can drift apart silently while every
    /// string stays identical. <paramref name="wiring"/> is
    /// <c>(int)ToolWiring</c>; anything the Rust does not know is the unknown
    /// state, which claims nothing.
    ///
    /// NULL, with an error recorded, for a NULL or non-UTF-8 source mode.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_routing_tool_word(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? sourceMode,
        int wiring);

    /// <summary>
    /// How the word <see cref="tc_routing_tool_word"/> returned is painted:
    /// <c>(int)RoutingTone</c>'s neutral or clear.
    ///
    /// Takes the same two inputs as the word, so the two cannot drift apart.
    /// This shell must NOT recover the tone by comparing the rendered word
    /// against the private one: that is a text comparison against a privacy
    /// claim, and "Private" is a substring of "Not private".
    ///
    /// Never fails -- anything unreadable answers the neutral tone, which
    /// claims nothing.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_routing_tool_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? sourceMode,
        int wiring);

    /// <summary>
    /// The daemon's routing state, in words. A state this build has never
    /// heard of -- and a NULL one -- reads as the off line, which claims
    /// nothing; it never falls through to either "on" sentence.
    ///
    /// Exported for the reason <see cref="tc_routing_tool_word"/> is: three
    /// copies of a branch can disagree while three copies of a string cannot.
    /// NULL only on a caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_routing_state_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// How firmly the sentence <see cref="tc_routing_state_line"/> returned
    /// reads: <c>TC_ROUTING_TONE_*</c>.
    ///
    /// The last routing branch table that was still written out natively in
    /// each shell. <c>awaiting_rows</c> is held and never a fault: a reader
    /// built a moment ago starts cold by construction.
    ///
    /// Never fails -- a state this build has never heard of, and a NULL one,
    /// both answer the neutral tone, exactly as their sentence claims
    /// nothing.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_routing_state_tone(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? state);

    /// <summary>
    /// The settings screen's session-source row for one tool, already
    /// assembled. <paramref name="tool"/> is "claude", "codex", "gemini" or
    /// "cline";
    /// <paramref name="sourceMode"/> is get_settings's *_source_mode --
    /// "watch", "off" or "unset".
    ///
    /// THREE MODES, THREE SENTENCES. ClaudeRootConfigured is (mode ==
    /// "watch") and so is false for "off" as well as for "unset". Branching
    /// on it told a contributor who declared a tool off that its sessions
    /// were being read from the usual place -- false, on the one screen they
    /// would check. Pass the mode and render what comes back; do not add a
    /// branch of your own here.
    ///
    /// NULL with "unknown-source-tool", "null-pointer", "invalid-utf8" or
    /// "panic" recorded for tc_last_error.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_source_check_line(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? tool,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? sourceMode);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_source_settings_copy();

    // --- The redaction witness ------------------------------------------
    //
    // THERE IS NO BOOLEAN IN THIS BLOCK AND THERE MUST NEVER BE ONE. "Is a
    // witness configured?" has two yes-answers that are opposites: a pinned
    // witness certifies every submission, and an unpinned one REFUSES every
    // submission before it touches the network. tc_witness_trust_state is the
    // one answer, with a value per condition.
    //
    // The tone calls here return TC_WITNESS_TONE_* -- 10..14 -- which are
    // DELIBERATELY DISJOINT from the routing tones above. Do not feed them to
    // RoutingSurface's mapper: it sends anything it does not recognise to
    // neutral, and a refusal painted neutral says "nothing to say" about a
    // total upload outage. WitnessTools.FromAbiTone is the mapper for these,
    // and its unknown arm is refused.

    /// <summary>
    /// What the witness is doing, as a <c>TC_WITNESS_STATE_*</c> value.
    ///
    /// Needs no handle: it reads the config file, and the screen that calls it
    /// is often the one deciding whether to start a daemon at all. Records a
    /// fixed <see cref="tc_last_error"/> label for the two negative values. A
    /// value this build does not name must be rendered as not usable, NEVER as
    /// TC_WITNESS_STATE_ABSENT.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_witness_trust_state(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? configDir);

    /// <summary>
    /// The whole witness configuration, as an owned JSON object; free it with
    /// <see cref="tc_string_free"/>, which <see cref="TakeOwnedString"/> does.
    ///
    /// NULL with <paramref name="err"/> set (owned; free it too) when the
    /// device is not enrolled or the config cannot be read. A NULL return is
    /// never "no witness": that is state absent on a successful call.
    ///
    /// The URL and signing address come back verbatim, one of the ABI's three
    /// named exemptions from the no-identifiers rule -- they are the
    /// contributor's own configuration, and a screen that will not show what
    /// it is asking them to trust with their raw session is not a settings
    /// screen. Nothing else about the witness path crosses.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_witness_status_json(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? configDir,
        out IntPtr err);

    /// <summary>
    /// Configures a witness. Returns 0 on success, -1 on failure with
    /// <paramref name="err"/> set (owned; free with
    /// <see cref="tc_string_free"/>).
    ///
    /// <paramref name="measurementsJson"/> is a JSON array of strings. THIS
    /// CALL WILL NOT WRITE AN UNPINNED WITNESS: an empty array and an
    /// unparsable one are both refused, because either produces a client that
    /// refuses every submission from the moment it is saved.
    ///
    /// Takes effect on the next submission, with no daemon restart.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_witness_configure(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? configDir,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? url,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? signingAddress,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? measurementsJson,
        out IntPtr err);

    /// <summary>
    /// Removes the configured witness. 1 if one was removed, 0 if there was
    /// none, -1 on failure with <paramref name="err"/> set (owned).
    ///
    /// Idempotent, and a return to LOCAL REDACTION rather than to a broken
    /// state -- but still a real change, because later submissions carry a
    /// self-reported residual-risk verdict rather than a certified one.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern int tc_witness_clear(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? configDir,
        out IntPtr err);

    /// <summary>
    /// What the last submission THIS PROCESS made did about the witness, as an
    /// owned JSON object. Process-local by design; NULL only on a caught
    /// panic.
    /// </summary>
    /// <remarks>
    /// Declared for completeness with the header. The sentence form --
    /// <see cref="tc_witness_last_result_line"/> -- is the only form a shell
    /// may print: this payload's refusal is a fixed operator label rather than
    /// wording, and its receipt count is a pair no shell may phrase itself.
    /// </remarks>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_witness_last_result_json();

    /// <summary>
    /// Every fixed word on the witness surface, as an owned JSON object; free
    /// it with <see cref="tc_string_free"/>. NULL only on a caught panic.
    ///
    /// ONE CALL, NOT ONE PER STRING. A shell handed the words one at a time
    /// takes some of them and writes the rest, and a hand-written word on this
    /// surface is a privacy claim that stops matching what the other two
    /// shells print.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_witness_copy();

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_onboarding_copy();

    /// <summary>
    /// The sentence for a <c>TC_WITNESS_STATE_*</c> value, as an owned char*.
    ///
    /// NULL, with the fixed label <c>witness-state-unknown</c> recorded, for a
    /// value this build cannot name. A caller that gets NULL must render NO
    /// witness sentence rather than one of its own.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_witness_state_line(int stateCode);

    /// <summary>
    /// The tone <see cref="tc_witness_state_line"/>'s sentence is painted in,
    /// as a <c>TC_WITNESS_TONE_*</c> value.
    ///
    /// ONE BRANCH TABLE, NOT TWO: it takes what the sentence takes. A state
    /// this build cannot name is TC_WITNESS_TONE_REFUSED, not neutral -- the
    /// fail-closed direction, deliberately.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_witness_state_tone(int stateCode);

    /// <summary>
    /// The sentence for what the last submission this process made did about
    /// the witness, as an owned char*. NULL only on a caught panic.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_witness_last_result_line();

    /// <summary>
    /// The tone <see cref="tc_witness_last_result_line"/>'s sentence is
    /// painted in. A refused send is TC_WITNESS_TONE_REFUSED and never
    /// ATTENTION: nothing was sent at all, which is not a
    /// degraded-but-working state.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_witness_last_result_tone();

    /// <summary>
    /// The only valid way to free a char* this library returns. Safe with
    /// NULL.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void tc_string_free(IntPtr s);

    /// <summary>
    /// The last error recorded ON THE CALLING THREAD, or NULL. Borrowed; do
    /// not free.
    ///
    /// Thread-locality is the trap for a C# binding: an <c>await</c> between
    /// a failing call and this read can resume on a different pool thread and
    /// silently report NULL. Every read here must be on the same thread as
    /// the call it is interrogating, with no await in between.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_last_error();

    /// <summary>
    /// The instance an invite link names, as an owned char*, or NULL if the
    /// argument is not a usable invite.
    ///
    /// A pure function of its argument: no handle, no daemon state. It is on
    /// the binding rather than the daemon protocol because adding a method
    /// would change the pinned METHODS array that hello advertises.
    /// </summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    internal static extern IntPtr tc_invite_issuer_host(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? invite);

    /// <summary>
    /// Copies a borrowed <c>const char*</c> into a managed string without
    /// freeing it. NULL becomes null.
    /// </summary>
    internal static string? BorrowedString(IntPtr ptr) =>
        ptr == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(ptr);

    /// <summary>
    /// Copies an owned <c>char*</c> into a managed string and releases it with
    /// tc_string_free. NULL becomes null. This is the only place outside the
    /// free-on-error paths that calls tc_string_free.
    /// </summary>
    internal static string? TakeOwnedString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero)
        {
            return null;
        }

        try
        {
            return Marshal.PtrToStringUTF8(ptr);
        }
        finally
        {
            tc_string_free(ptr);
        }
    }
}
