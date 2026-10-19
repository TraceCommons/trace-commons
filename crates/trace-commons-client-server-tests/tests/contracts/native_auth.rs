use trace_commons_contributor::account_auth::wire::{
    CODE_CHALLENGE_METHOD, LOOPBACK_WAIT, NATIVE_REDIRECT_PATH, challenge_for_verifier,
    generate_verifier,
};
use trace_commons_server::account_native_auth as server;

/// The client's wire constants must equal the server's. Neither side copies
/// the other's implementation, so this detects drift across the boundary.
#[test]
fn constants_match_the_server() {
    assert_eq!(NATIVE_REDIRECT_PATH, server::NATIVE_REDIRECT_PATH);
    assert_eq!(CODE_CHALLENGE_METHOD, server::NATIVE_CODE_CHALLENGE_METHOD);
    assert_eq!(
        LOOPBACK_WAIT.as_secs(),
        server::NATIVE_AUTH_REQUEST_TTL.as_secs(),
        "the listener must not outlive the server's pending request"
    );
}

/// The verifier this client generates must satisfy the server's own
/// well-formedness check, and the challenge must be the one the server derives.
#[test]
fn generated_pkce_satisfies_the_server() {
    let verifier = generate_verifier().expect("verifier");
    assert!(server::verifier_is_wellformed(&verifier));
    let challenge = challenge_for_verifier(&verifier);
    assert!(server::challenge_is_wellformed(&challenge));
    assert_eq!(challenge, server::challenge_for_verifier(&verifier));
}

/// The redirect this client registers must be one the server accepts.
#[test]
fn the_registered_redirect_is_one_the_server_accepts() {
    let uri = format!("http://127.0.0.1:49152{NATIVE_REDIRECT_PATH}");
    assert!(server::validate_loopback_redirect_uri(&uri).is_some());
}
