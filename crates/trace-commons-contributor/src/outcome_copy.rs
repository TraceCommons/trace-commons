//! Outcome and correction disclosure shared by contributor shells.

pub const VERDICT_QUESTION: &str = "Did this session do what you asked?";
pub const VERDICT_WORKED: &str = "Worked";
pub const VERDICT_PARTLY: &str = "Partly";
pub const VERDICT_FAILED: &str = "Failed";
pub const VERDICT_CAPTION: &str =
    "Optional. This is recorded as the trace outcome; the preview above does not show it.";
pub const CORRECTION_QUESTION: &str = "What did it get wrong?";
pub const CORRECTION_PLACEHOLDER: &str = "Optional";
pub const CORRECTION_CAPTION: &str = "Stored exactly as you write it. Unlike the rest of the trace, a correction is not scrubbed here or on the server -- so leave out anything you would not want in the corpus: someone else's personal information, employer-confidential material, or anything you are not free to share.";
pub const CORRECTION_CREDENTIAL_HEADLINE: &str =
    "Nothing was sent. Your correction looks like it contains a credential.";
pub const CORRECTION_CREDENTIAL_BODY: &str = "A correction is stored as you write it, so this one was refused rather than masked. Take the credential out and submit again -- and rotate it, because it has already been typed here.";
pub const SUBMIT_ALL_AS: &str = "Submit all as...";
pub const SUBMIT_ALL_AS_TOOLTIP: &str = "Record the same outcome for every session in this group.";

#[derive(serde::Serialize)]
pub struct OutcomeCopy {
    pub verdict_question: &'static str,
    pub worked: &'static str,
    pub partly: &'static str,
    pub failed: &'static str,
    pub verdict_caption: &'static str,
    pub correction_question: &'static str,
    pub correction_placeholder: &'static str,
    pub correction_caption: &'static str,
    pub correction_credential_headline: &'static str,
    pub correction_credential_body: &'static str,
    pub submit_all_as: &'static str,
    pub submit_all_as_tooltip: &'static str,
    pub max_correction_chars: usize,
}

#[must_use]
pub fn outcome_copy() -> OutcomeCopy {
    OutcomeCopy {
        verdict_question: VERDICT_QUESTION,
        worked: VERDICT_WORKED,
        partly: VERDICT_PARTLY,
        failed: VERDICT_FAILED,
        verdict_caption: VERDICT_CAPTION,
        correction_question: CORRECTION_QUESTION,
        correction_placeholder: CORRECTION_PLACEHOLDER,
        correction_caption: CORRECTION_CAPTION,
        correction_credential_headline: CORRECTION_CREDENTIAL_HEADLINE,
        correction_credential_body: CORRECTION_CREDENTIAL_BODY,
        submit_all_as: SUBMIT_ALL_AS,
        submit_all_as_tooltip: SUBMIT_ALL_AS_TOOLTIP,
        max_correction_chars: crate::envelope::MAX_CORRECTION_CHARS,
    }
}
