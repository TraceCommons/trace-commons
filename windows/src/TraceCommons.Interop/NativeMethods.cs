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
    /// assembled. <paramref name="tool"/> is "claude", "codex" or "gemini";
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
