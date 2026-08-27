//! Guards on `scripts/contribute.sh`, the one-time contribution script.
//!
//! These are text assertions rather than an end-to-end run: the real thing
//! downloads a signed release binary and uploads traces. What is checked here
//! is the handful of properties the script is only safe to advertise in its
//! `curl | sh` form because it has: it executes nothing when truncated, it
//! never puts the invite in argv, and it does not reimplement verification.

fn script() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/contribute.sh")
        .canonicalize()
        .expect("scripts/contribute.sh exists");
    std::fs::read_to_string(path).expect("scripts/contribute.sh is readable")
}

/// Every executable line lives inside the function, which is invoked on the
/// last line. A download cut short therefore defines a function nobody calls.
#[test]
fn a_truncated_script_executes_nothing() {
    let text = script();
    let lines: Vec<&str> = text.lines().collect();
    let invocations: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            let l = l.trim();
            !l.starts_with('#') && l.contains("tc_contribute_main") && !l.contains("() {")
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        invocations.len(),
        1,
        "exactly one call site, found {invocations:?}"
    );
    let last_code_line = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .expect("script is not empty");
    assert_eq!(
        invocations[0], last_code_line,
        "the call must be the last line, or a truncated download can run part of the script"
    );
}

/// The invite is a bearer secret. In argv it is visible to every user on the
/// machine through `ps`, and lands in shell history.
#[test]
fn the_invite_never_reaches_argv() {
    let text = script();
    assert!(
        !text.contains("--invite"),
        "the invite must travel in TRACE_COMMONS_INVITE, never as a flag"
    );
    assert!(
        text.contains("TRACE_COMMONS_INVITE=\"$invite\""),
        "the invite is passed to the CLI through the environment"
    );
}

/// Verification lives in install.sh, which has no --force and no
/// --skip-verify. A second download path with its own checks would be a
/// second chance to get verification wrong.
#[test]
fn verification_is_delegated_to_install_sh_and_not_reimplemented() {
    let text = script();
    assert!(
        text.contains("scripts/install.sh") && text.contains("--dir"),
        "the binary is fetched by install.sh --dir"
    );
    for reimplemented in ["shasum", "sha256sum", "codesign", "releases/download"] {
        assert!(
            !text.contains(reimplemented),
            "contribute.sh must not reimplement verification ({reimplemented})"
        );
    }
}

/// Piped through `sh`, stdin is the script itself. A confirmation prompt or a
/// consent question reading that stdin would consume script text as its
/// answer. Both must come from the terminal, and with no terminal the run
/// stops rather than proceeding unconfirmed.
#[test]
fn interactive_answers_come_from_the_terminal_not_the_pipe() {
    let text = script();
    assert!(
        text.contains("\"$cli\" submit < /dev/tty"),
        "the CLI's stdin must be the terminal, not the piped script"
    );
    assert!(
        text.contains("[ ! -r /dev/tty ]"),
        "with no terminal there is nobody to confirm the upload; the run must refuse"
    );
}

/// The keep is the only route to withdrawal, so the script says so on every
/// run, says where it is, and says how to delete it.
#[test]
fn every_run_names_the_keep_its_purpose_and_how_to_delete_it() {
    let text = script();
    assert!(text.contains("$keep_dir"), "the keep's path is printed");
    assert!(
        text.contains("withdraw"),
        "the run says the keep is what withdrawal needs"
    );
    assert!(
        text.contains("rm -rf \\\"$keep_dir\\\""),
        "the run says how to delete the keep"
    );
    assert!(
        !text.contains("--no-keep"),
        "there is deliberately no --no-keep"
    );
}

/// `--no-cache` removes the binary and nothing else.
///
/// The distinction is the whole point of the flag. Someone asking for "no
/// installation" means the program, and that is safe to give them. Reading it
/// as "no state" would take the keep with it, and the keep is the device key
/// the account is minted from -- discarding it is what leaves a contributor
/// unable to withdraw what they just uploaded. One of those is a preference
/// about disk; the other is a consent failure.
#[test]
fn no_cache_removes_the_binary_and_leaves_the_keep() {
    let s = script();
    assert!(s.contains("--no-cache"), "the flag exists");
    assert!(
        s.contains(r#"trap 'rm -rf "$bin_dir"' EXIT INT TERM"#),
        "the temporary bin dir is removed on every exit path, including a signal"
    );
    // The trap must name bin_dir and never keep_dir: a cleanup that took the
    // keep with it would silently make the run's traces unwithdrawable.
    for line in s.lines().filter(|l| l.contains("trap ")) {
        assert!(
            !line.contains("keep_dir"),
            "no cleanup path may remove the keep: {line}"
        );
    }
    assert!(
        s.contains("It does NOT touch the keep"),
        "the script says which of the two --no-cache applies to"
    );
}

/// `--with-export` must be opt-in, pinned, and unable to fail the run.
///
/// The script's whole security story is that it fetches one artifact whose
/// checksum must match and whose macOS signature must name our Developer ID,
/// with no flag to skip either. The exporter is an npm package: signed by the
/// registry, but with no provenance attestation tying the tarball to our
/// source. Fetching it is a real widening of that surface, so it may not
/// happen to someone who did not ask, and it may not decide whether traces get
/// submitted.
#[test]
fn with_export_is_opt_in_pinned_and_never_fatal() {
    let s = script();

    // Opt-in: the exporter may only run inside the flag's branch.
    assert!(s.contains("--with-export"), "the flag exists");
    let guarded = s
        .split("if [ -n \"$with_export\" ]; then")
        .nth(1)
        .expect("the exporter runs inside the flag's branch");
    assert!(
        guarded.contains("npx --yes"),
        "npx is invoked only under --with-export"
    );
    // Count real invocations, not the word: the script mentions npx in a
    // comment and in the message shown when it is missing, and an assertion
    // that counts those would fail for the wrong reason and get loosened.
    let invocations = s
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with('#') && t.contains("npx --yes")
        })
        .count();
    assert_eq!(
        invocations, 1,
        "exactly one npx invocation, inside the guard"
    );

    // Pinned: npx resolves a range at runtime, and this script is advertised
    // in its piped form. "Whatever is latest right now" is not something to
    // run unattended over session transcripts.
    assert!(
        s.contains("@tracecommons/trajectory-export@0.1.0"),
        "the exporter version is pinned exactly"
    );

    // Never fatal, twice over: the script runs under `set -e`, the exporter
    // exits non-zero when it finds nothing to export, and a machine that only
    // runs natively-read harnesses is the common case. Either failure must
    // leave the submission -- the actual point of the run -- to proceed.
    assert!(
        guarded.contains("||"),
        "a failing exporter is tolerated, not fatal"
    );
    assert!(
        guarded.contains("command -v npx"),
        "a machine without npx is told and continues"
    );

    // Order: exporting after `submit` would write files nothing then reads.
    let export_at = s.find("normalizing sessions").expect("export step present");
    let submit_at = s.find("\"$cli\" submit").expect("submit step present");
    assert!(
        export_at < submit_at,
        "the export must run before submit discovers what it wrote"
    );
}
