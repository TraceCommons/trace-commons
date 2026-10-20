//! Regression: a placeholder that was altered *mid-stream* must fail, not flush.
//!
//! The first version of the reverser held an unterminated candidate only until
//! it grew longer than any placeholder could be, then flushed it as ordinary
//! text — correct for a stray `⟦` in someone's source, and silently wrong for
//! one of our own tokens that a model had truncated. The integration test
//! caught it; these pin it at the unit level, where the failure is legible.

use ironwire_privacy::{Class, Map, ReverseError, Reverser, Salt};

fn fixture() -> (Map, String) {
    let salt = Salt::fixed(1);
    let mut map = Map::new();
    let token = map.insert(&salt, Class::Named, "alice@corp.com");
    (map, token)
}

fn run(map: &Map, text: &str) -> (String, Result<String, ReverseError>) {
    let mut reverser = Reverser::new();
    let emitted = reverser.push(map, text);
    (emitted, reverser.finish(map))
}

#[test]
fn a_truncated_token_followed_by_more_text_fails() {
    // What a model paraphrasing a summary produces, and the case that matters
    // most: on a compaction turn the result is written into the client's
    // permanent history and can never be reversed again.
    let (map, token) = fixture();
    let truncated: String = token.chars().take(token.chars().count() - 3).collect();
    let stream =
        format!("data: {{\"text\":\"contacting {truncated}\"}}\n\nevent: done\ndata: {{}}\n\n");

    let (emitted, finished) = run(&map, &stream);
    assert!(
        !emitted.contains('\u{27e6}'),
        "a fragment of our own token was forwarded: {emitted}"
    );
    assert_eq!(finished, Err(ReverseError::Unreversed { count: 1 }));
}

#[test]
fn a_token_rewritten_in_the_middle_is_passed_through_and_counted() {
    // **The limit of what this can detect, stated rather than implied.**
    //
    // A *truncated* token is recognisable: its prefix matches one we minted and
    // then the stream diverges. A token whose middle was rewritten —
    // `⟦named. abc⟧` — is not distinguishable from one the model invented, and
    // guessing would mean fuzzy-matching arbitrary text against our tokens,
    // where a false positive fails a working stream.
    //
    // So it is passed through, and *counted*. The count is what
    // `ironwire log` surfaces: a turn that substituted three values and passed
    // through three placeholder-shaped strings is a turn the user should look
    // at. See `docs/PRIVACY.md` §5.
    let (map, token) = fixture();
    let mangled = token.replace('.', ". ");

    let mut reverser = Reverser::new();
    let mut all = reverser.push(&map, &format!("see {mangled} and then a lot more text"));
    all.push_str(&reverser.finish(&map).expect("not a detectable truncation"));

    assert!(all.contains(&mangled), "the text was altered: {all}");
    assert!(
        !all.contains("alice@corp.com"),
        "a wrong value was substituted in"
    );
    assert_eq!(
        reverser.passed_through(),
        1,
        "a placeholder-shaped string we did not mint must be counted, so the \
         ledger can show the user something went sideways"
    );
}

#[test]
fn a_lone_delimiter_in_source_code_still_streams() {
    // The other side of the trade. `⟦` appears in mathematical notation and
    // occasionally in source; holding the stream on one would read to the user
    // as a hung agent, and failing on one would break a working session.
    let (map, _token) = fixture();
    let text = format!("denotational semantics: ⟦e⟧ where {}", "x".repeat(500));
    let (emitted, finished) = run(&map, &text);
    let all = emitted + &finished.expect("must not fail on ordinary text");
    assert_eq!(all, text);
}

#[test]
fn a_placeholder_shaped_string_the_model_invented_is_passed_through() {
    // Not ours, so not reversed — and not a failure either. The model is
    // allowed to write whatever it likes.
    let (map, _token) = fixture();
    let text = "the format is ⟦email.aaaaaaaaaaaa⟧ apparently, plus more text";
    let (emitted, finished) = run(&map, text);
    let all = emitted + &finished.expect("must not fail");
    assert_eq!(all, text);
}

#[test]
fn a_whole_token_immediately_followed_by_more_text_still_reverses() {
    // The boundary between "mangled" and "fine" — the token is complete, and
    // what follows is unrelated.
    let (map, token) = fixture();
    let text = format!("{token}, and then a long tail {}", "y".repeat(200));
    let (emitted, finished) = run(&map, &text);
    let all = emitted + &finished.expect("must not fail");
    assert!(all.starts_with("alice@corp.com,"), "got {all}");
}
