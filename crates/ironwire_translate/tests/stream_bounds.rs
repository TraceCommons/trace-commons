//! The three places a foreign upstream controls how much IronWire allocates.
//!
//! The translated lane will point at any OpenAI-compatible endpoint a user
//! names, so "the upstream is well behaved" is not an assumption available
//! here. Each of these was unbounded until this suite was written; a broken or
//! hostile endpoint could take the daemon down and with it every conversation
//! on the machine.

use ironwire_core::protocol::Protocol;
use ironwire_translate::Translator;

/// The pair these bounds were originally written against. They are
/// properties of the shared framer and accumulator now, so any pair would
/// exercise them — this one keeps the assertions about Anthropic output
/// that the tests already make.
fn translator() -> Translator {
    Translator::new(
        Protocol::OpenAiChat,
        Protocol::AnthropicMessages,
        "claude-opus-4-6",
    )
}

fn frame(payload: &str) -> Vec<u8> {
    format!("data: {payload}\n\n").into_bytes()
}

/// Built by substitution rather than `format!`: these payloads are mostly
/// braces, and escaping them for a format string obscures what is being tested.
const TOOL_CALL_TEMPLATE: &str = r#"{"choices":[{"delta":{"tool_calls":[{"index":INDEX,"id":"call_INDEX","function":{"name":"tool_INDEX","arguments":"{}"}}]}}]}"#;

const ARGUMENT_TEMPLATE: &str =
    r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"BODY"}}]}}]}"#;

#[test]
fn an_upstream_that_never_sends_a_frame_boundary_does_not_grow_the_buffer() {
    // The SSE buffer holds bytes until `\n\n`. Without a bound, an endpoint
    // that simply never sends one grows it until the process dies.
    let mut stream = translator();
    let junk = "x".repeat(64 * 1024);
    for _ in 0..200 {
        // 12 MB with no boundary anywhere.
        let _ = stream.push(junk.as_bytes());
    }

    // **One frame is lost on resync, and that is the correct behaviour.** The
    // discarded bytes had no boundary, so there is no way to know whether what
    // follows is a new frame or the tail of the one we dropped. Discarding
    // through the next boundary necessarily sacrifices it — which is strictly
    // better than the alternatives: growing until the process dies, or gluing
    // junk onto a real frame and silently swallowing it.
    let sacrificed = stream.push(&frame(r#"{"choices":[{"delta":{"content":"lost"}}]}"#));
    assert!(
        !String::from_utf8_lossy(&sacrificed).contains("lost"),
        "the frame straddling the resync boundary should not be trusted"
    );

    // From the next frame on, the stream is fully usable again.
    let out = stream.push(&frame(r#"{"choices":[{"delta":{"content":"hello"}}]}"#));
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("hello"),
        "the stream never recovered after discarding junk:\n{text}"
    );
}

#[test]
fn an_implausible_tool_call_index_does_not_allocate() {
    // `index` is upstream-controlled and drove a `Vec::resize`. One frame
    // claiming four billion parallel tool calls was enough to abort.
    let mut stream = translator();
    let out = stream.push(&frame(
        r#"{"choices":[{"delta":{"tool_calls":[{"index":4000000000,"id":"call_x","function":{"name":"boom","arguments":"{}"}}]}}]}"#,
    ));
    let _ = out;

    // The stream stays usable and the bogus call is simply absent.
    let mut out = stream.push(&frame(
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_ok","function":{"name":"real","arguments":"{}"}}]}}]}"#,
    ));
    out.extend_from_slice(&stream.finish());
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("real"),
        "the legitimate call was lost:\n{text}"
    );
    assert!(
        !text.contains("boom"),
        "the implausible call was emitted:\n{text}"
    );
}

#[test]
fn a_high_but_plausible_tool_call_index_still_works() {
    // The bound must not refuse real parallelism. Models do emit several
    // concurrent calls; the limit is only there to stop the absurd.
    let mut stream = translator();
    for index in 0..8 {
        let payload = TOOL_CALL_TEMPLATE.replace("INDEX", &index.to_string());
        let _ = stream.push(&frame(&payload));
    }
    let out = stream.finish();
    let text = String::from_utf8_lossy(&out);
    for index in 0..8 {
        assert!(text.contains(&format!("tool_{index}")), "lost call {index}");
    }
}

#[test]
fn unbounded_tool_arguments_are_refused_and_the_call_is_dropped() {
    // Arguments arrive as fragments that are concatenated. Truncating them
    // would hand the client a `tool_use` block with unparseable input, which it
    // would pass to a tool — dropping the call is visibly incomplete rather
    // than silently wrong.
    let mut stream = translator();
    let _ = stream.push(&frame(
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_big","function":{"name":"huge","arguments":"start"}}]}}]}"#,
    ));

    let chunk = "y".repeat(64 * 1024);
    for _ in 0..200 {
        let payload = ARGUMENT_TEMPLATE.replace("BODY", &chunk);
        let _ = stream.push(&frame(&payload));
    }

    let out = stream.finish();
    let text = String::from_utf8_lossy(&out);
    assert!(
        !text.contains("huge"),
        "a call with overflowing arguments was emitted anyway:\n{}",
        &text[..text.len().min(400)]
    );
    // And the client still gets a well-formed, terminated message.
    assert!(text.contains("message_stop"), "the stream was left open");
}

#[test]
fn ordinary_tool_arguments_are_unaffected() {
    // The bound is generous by design; a realistic call must not notice it.
    let mut stream = translator();
    let body = "z".repeat(32 * 1024);
    let _ = stream.push(&frame(
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"write","arguments":"start"}}]}}]}"#,
    ));
    let payload = ARGUMENT_TEMPLATE.replace("BODY", &body);
    let _ = stream.push(&frame(&payload));
    let out = stream.finish();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("write"),
        "a 32 KB argument was refused:\n{}",
        &text[..text.len().min(300)]
    );
}

#[test]
fn a_client_always_gets_a_terminated_message_whatever_the_upstream_did() {
    // The invariant behind all three bounds: however badly the upstream
    // behaves, an agent waiting on `message_stop` must not hang until its own
    // timeout.
    for hostile in [
        &b"garbage with no structure at all"[..],
        b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":99999999}]}}]}\n\n",
        b"data: not json\n\n",
        b"\n\n\n\n",
        b"",
    ] {
        let mut stream = translator();
        let mut out = stream.push(hostile);
        out.extend_from_slice(&stream.finish());
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("message_stop"),
            "no terminal event after {:?}",
            String::from_utf8_lossy(&hostile[..hostile.len().min(40)])
        );
    }
}
