import Foundation

/// The one complete `PrivateInferenceCopy` payload the shell's surface
/// tests decode.
///
/// **There used to be five of these, and they were byte-identical.** Every
/// surface suite kept its own copy of the same JSON object, so a new
/// required property on `PrivateInferenceCopy` meant five separate edits
/// and five chances to miss one. #745 made that edit correctly in the four
/// copies that existed when it was written; a fifth was in flight on
/// another branch, could not have been in its enumeration, and decoded
/// into nothing the moment the two met. Nothing caught it until the merge
/// queue, because the merge queue is the only run that tests a branch
/// combined with `main`.
///
/// Collapsing them to one does not make anybody more careful. It makes a
/// new required property a ONE-LINE EDIT HERE that fails every surface
/// suite at once, locally, instead of four edits that pass locally and
/// fail in a queue.
///
/// **This is a hand-maintained fixture and that is a ceiling, not an
/// oversight.** `TCBridgeTests` never needs one: it builds its payload
/// from `TCPrivateInference.copyJSON()`, the live ABI, so it CANNOT go
/// stale -- a new property appears in it for free. The obvious question is
/// why these tests do not do the same, and the answer is deliberate:
/// `TCShellCore` is tested without linking the dylib, which is what lets
/// its surfaces be exercised through injected closures. Reading the live
/// payload here would trade that away. So this file removes the
/// duplication, not the maintenance, and the maintenance is one line.
enum PrivateInferenceCopyFixture {
    /// Every required key, with a placeholder value naming the key it came
    /// from, so an assertion that reads the wrong field says which one.
    static let complete = """
        {"destination":"DESTINATION","subtitle":"SUBTITLE",
         "offer_title":"T","offer_what":"WHAT","offer_exposure":"EXPOSURE",
         "offer_no_repoint":"NO-REPOINT","offer_accept":"ACCEPT",
         "offer_decline":"DECLINE","offer_asked_once":"ONCE",
         "settings_title":"S-TITLE","settings_toggle":"S-TOGGLE",
         "settings_applies_at_once":"S-AT-ONCE","state_off":"S-OFF","state_unknown":"S-UNKNOWN","state_unreported":"S-UNREPORTED","state_stopping":"S-STOPPING",
         "state_running":"S-RUNNING","state_running_no_backends":"S-NO-BACKENDS","state_running_answered_elsewhere":"S-ELSEWHERE","state_running_destination_unknown":"S-DEST-UNKNOWN",
         "state_running_elsewhere":"S-ELSEWHERE","state_port_in_use":"S-PORT",
         "state_start_failed":"S-FAILED","state_crashed":"S-CRASHED",
         "quit_also_stops":"QUIT","write_unconfirmed":"UNCONFIRMED","settings_moved":"MOVED","tray_turn_off":"TRAYOFF","tray_open_to_turn_on":"TRAYON",
         "harnesses_title":"H-TITLE","harnesses_what":"H-WHAT",
         "harnesses_spend_scope":"H-SPEND-SCOPE",
         "harness_not_connected":"H-NOT-CONNECTED",
         "harness_connected_nothing_seen":"H-NOTHING-SEEN",
         "harness_answering":"H-ANSWERING","harness_connect":"H-CONNECT",
         "harness_disconnect":"H-DISCONNECT",
         "harness_preview_title":"H-PREVIEW","harness_preview_confirm":"H-CONFIRM",
         "harness_preview_cancel":"H-CANCEL","harness_slot_taken":"H-TAKEN",
         "harness_needs_restart":"H-RESTART","harnesses_none_found":"H-NONE",
         "harness_unreadable_config":"H-UNREADABLE",
         "harness_not_installed":"H-NOT-INSTALLED",
         "harness_plan_nothing_to_change":"H-NOTHING-TO-CHANGE",
         "harness_plan_entry_unusable":"H-ENTRY-UNUSABLE",
         "harness_plan_no_config_path":"H-NO-CONFIG-PATH",
         "credential_title":"C-TITLE",
         "near_ai_enroll_title":"NEAR-AI-ENROLL-TITLE",
         "near_ai_enroll_what":"NEAR-AI-ENROLL-WHAT",
         "near_ai_enroll_action":"NEAR-AI-ENROLL-ACTION",
         "near_ai_enroll_needs_login":"NEAR-AI-ENROLL-NEEDS-LOGIN",
         "near_ai_enroll_working":"NEAR-AI-ENROLL-WORKING",
         "near_ai_enroll_done":"NEAR-AI-ENROLL-DONE",
         "near_ai_enroll_already_enrolled":"NEAR-AI-ENROLL-ALREADY-ENROLLED",
         "near_ai_enroll_no_session":"NEAR-AI-ENROLL-NO-SESSION",
         "near_ai_enroll_endpoint_refused":"NEAR-AI-ENROLL-ENDPOINT-REFUSED",
         "near_ai_enroll_token_unavailable":"NEAR-AI-ENROLL-TOKEN-UNAVAILABLE",
         "near_ai_enroll_start_failed":"NEAR-AI-ENROLL-START-FAILED",
         "near_ai_enroll_commons_unreachable":"NEAR-AI-ENROLL-COMMONS-UNREACHABLE",
         "near_ai_enroll_commons_unsupported":"NEAR-AI-ENROLL-COMMONS-UNSUPPORTED",
         "near_ai_enroll_invalid":"NEAR-AI-ENROLL-INVALID",
         "near_ai_enroll_verification_failed":"NEAR-AI-ENROLL-VERIFICATION-FAILED",
         "near_ai_enroll_unavailable":"NEAR-AI-ENROLL-UNAVAILABLE","credential_what":"C-WHAT",
         "credential_cost":"C-COST","credential_obtain":"C-OBTAIN",
         "credential_google":"Continue with Google","credential_github":"Continue with GitHub",
         "credential_cancel":"C-CANCEL","credential_forget":"C-FORGET",
         "credential_forget_explains":"C-FORGET-EXPLAINS",
         "credential_absent":"C-ABSENT","credential_obtaining":"C-OBTAINING",
         "credential_failed":"C-FAILED","credential_cancelled":"C-CANCELLED",
         "credential_present":"C-PRESENT","credential_unknown":"C-UNKNOWN",
         "credential_unreported":"C-UNREPORTED",
         "harness_needs_credential":"H-NEEDS-CREDENTIAL",
         "eligibility_eligible":"E-ELIGIBLE",
         "eligibility_ineligible_permanent":"E-PERMANENT",
         "eligibility_ineligible_configuration":"E-CONFIGURATION",
         "eligibility_unknown":"E-UNKNOWN",
         "eligibility_reason_no_call":"R-NO-CALL",
         "eligibility_reason_capture_off":"R-CAPTURE-OFF",
         "eligibility_reason_digest_absent":"R-DIGEST-ABSENT",
         "eligibility_reason_upstream_id_absent":"R-UPSTREAM-ID-ABSENT",
         "eligibility_reason_digest_mismatch":"R-DIGEST-MISMATCH",
         "eligibility_reason_reference_malformed":"R-REFERENCE-MALFORMED",
         "eligibility_reason_bodies_unreadable":"R-BODIES-UNREADABLE",
         "eligibility_reason_body_not_utf8":"R-BODY-NOT-UTF8",
         "eligibility_reason_body_too_large":"R-BODY-TOO-LARGE",
         "eligibility_reason_evidence_capture_off":"R-EVIDENCE-CAPTURE-OFF",
         "eligibility_reason_marker_absent":"R-MARKER-ABSENT",
         "eligibility_reason_request_malformed":"R-REQUEST-MALFORMED",
         "eligibility_reason_receipt_unavailable":"R-RECEIPT-UNAVAILABLE",
         "certificate_row_candidate":"CERT-ROW-CANDIDATE",
         "certificate_row_attested":"CERT-ROW-ATTESTED",
         "certificate_list_candidate":"CERT-LIST-CANDIDATE",
         "certificate_list_attested":"CERT-LIST-ATTESTED",
         "certificate_list_empty":"CERT-LIST-EMPTY",
         "eligibility_reason_receipt_not_issued":"R-RECEIPT-NOT-ISSUED",
         "attestation_attested":"A-ATTESTED",
         "attestation_unattested_permanent":"A-UNATTESTED-PERMANENT",
         "attestation_unattested_configuration":"A-UNATTESTED-CONFIGURATION",
         "attestation_unknown":"A-UNKNOWN",
         "attestation_reason_no_call":"AR-NO-CALL",
         "attestation_reason_capture_off":"AR-CAPTURE-OFF",
         "attestation_reason_digest_absent":"AR-DIGEST-ABSENT",
         "attestation_reason_upstream_id_absent":"AR-UPSTREAM-ID-ABSENT",
         "attestation_reason_digest_mismatch":"AR-DIGEST-MISMATCH",
         "attestation_reason_reference_malformed":"AR-REFERENCE-MALFORMED",
         "attestation_reason_bodies_unreadable":"AR-BODIES-UNREADABLE",
         "attestation_reason_body_not_utf8":"AR-BODY-NOT-UTF8",
         "attestation_reason_body_too_large":"AR-BODY-TOO-LARGE",
         "attestation_reason_evidence_capture_off":"AR-EVIDENCE-CAPTURE-OFF",
         "attestation_reason_marker_absent":"AR-MARKER-ABSENT",
         "attestation_reason_request_malformed":"AR-REQUEST-MALFORMED",
         "attestation_reason_receipt_unavailable":"AR-RECEIPT-UNAVAILABLE",
         "attestation_reason_receipt_not_issued":"AR-RECEIPT-NOT-ISSUED",
         "balance_title":"BALANCE-TITLE",
         "balance_what":"BALANCE-WHAT",
         "balance_no_session":"BALANCE-NO-SESSION",
         "balance_session_expired":"BALANCE-SESSION-EXPIRED",
         "balance_no_organization":"BALANCE-NO-ORGANIZATION",
         "balance_unavailable":"BALANCE-UNAVAILABLE",
         "balance_unknown":"BALANCE-UNKNOWN",
         "balance_unreported":"BALANCE-UNREPORTED",
         "balance_no_remaining":"BALANCE-NO-REMAINING"}
        """
}
