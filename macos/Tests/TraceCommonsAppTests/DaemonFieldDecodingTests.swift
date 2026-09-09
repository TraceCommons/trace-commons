import XCTest

@testable import TraceCommonsApp

/// The daemon gained a project path, a session path, distinct redaction
/// counts, an enrollment flag on previews, and a project id on history
/// records. Every one of them must be optional in practice: this app is
/// shipped separately from the daemon and routinely runs against an older
/// one -- and where the absent value gates something, absent is the
/// refusing answer.
final class DaemonFieldDecodingTests: XCTestCase {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(type, from: Data(json.utf8))
    }

    func testQueueEntryDecodesProjectAndSessionPaths() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "project_path":"~/code/repo","session_path":"~/code/repo/crates/inner",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """)
        XCTAssertEqual(entry.projectPath, "~/code/repo")
        XCTAssertEqual(entry.sessionPath, "~/code/repo/crates/inner")
    }

    func testQueueEntryFromAnOlderDaemonHasNoPaths() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """)
        XCTAssertEqual(entry.projectPath, "")
        XCTAssertNil(entry.sessionPath)
    }

    // MARK: - The certificate a row holds

    /// `holds_certificate` decides which rows the certificate-held list
    /// shows. It is optional on this shell for the reason every other new
    /// daemon field is: the app ships separately from the daemon and
    /// routinely runs against an older one, which sends no such key.
    ///
    /// **An absent key must read as "no certificate", never as an error and
    /// never as a certificate.** Reading it as held would put a session in a
    /// list claiming something nobody established.
    ///
    /// The three shells differ here and the difference is worth knowing:
    /// GTK's mirror is `#[serde(default)]` so a missing key is silently
    /// false, and C# System.Text.Json does the same. This shell would throw
    /// on a missing key if the property were non-optional, which would fail
    /// the WHOLE list rather than one row. Optional plus a false default is
    /// what makes all three agree.
    private func row(_ holdsCertificate: String) throws -> QueueEntry {
        try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0\(holdsCertificate)}
        """)
    }

    func testARowSaysWhenACertificateIsHeldForIt() throws {
        XCTAssertTrue(try row(",\"holds_certificate\":true").holdsCertificate)
    }

    func testARowWithoutTheKeyHoldsNoCertificate() throws {
        for answer in [",\"holds_certificate\":false", ""] {
            XCTAssertFalse(
                try row(answer).holdsCertificate,
                "\(answer.isEmpty ? "an absent key" : answer) was read as a held certificate")
        }
    }

    // MARK: - Admission evidence: only an explicit yes is a yes

    /// `admission_evidence_required` decides whether the preview sheet offers
    /// the admission-preparation control at all. It is not a preference: the
    /// daemon sets it true for a contributor who signed up through NEAR and
    /// false for one who came in on an invite, so an invited contributor
    /// shown the control could only ever be refused by it.
    ///
    /// The daemon answers null when it could not read the config
    /// (`add_admission_setting`), and a daemon that predates the key answers
    /// nothing. Neither is a yes. This shell reads the flag through
    /// `admissionEvidenceOffered` rather than at the call site so that the
    /// question can be asked here, of the decoder, in both directions --
    /// GTK and Windows hold the same contract in their own suites.
    private func settings(_ admissionEvidence: String) throws -> DaemonSettingsView {
        try decode(DaemonSettingsView.self, """
        {"quiescence_secs":30,"digest_interval_secs":3600,
         "local_notifications":true,"queue_ttl_days":7,
         "max_queue_entries":100,"max_uploads_per_day":10,
         "near_ai_configured":false,"claude_root_configured":true,
         "codex_root_configured":false\(admissionEvidence)}
        """)
    }

    func testAdmissionPreparationIsOfferedOnlyOnAnExplicitYes() throws {
        XCTAssertTrue(try settings(",\"admission_evidence_required\":true").admissionEvidenceOffered)
    }

    func testAdmissionPreparationIsWithheldFromEveryOtherAnswer() throws {
        for answer in [
            ",\"admission_evidence_required\":false",
            // The daemon could not read the config.
            ",\"admission_evidence_required\":null",
            // A daemon that predates the key.
            "",
        ] {
            XCTAssertFalse(
                try settings(answer).admissionEvidenceOffered,
                "\(answer.isEmpty ? "an absent key" : answer) was read as eligibility")
        }
    }

    // MARK: - Eligibility: an absent key is not a state

    /// The distinction the eligibility contract turns on, at the decoder.
    ///
    /// An entry with no `eligibility` key belongs to a contributor who was
    /// invited rather than admitted on evidence, or came from a daemon
    /// predating the field. Neither has an eligibility question, and reading
    /// the absence as `unknown` would put a caveat on work that carries none.
    func testAQueueEntryWithNoEligibilityKeyCarriesNoEligibility() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """)
        XCTAssertNil(entry.eligibility)
        XCTAssertNil(entry.eligibilityReason)
        XCTAssertNil(entry.contributionEligibility)
    }

    /// `unknown` really arrives on the wire -- a row this build never
    /// evaluated, or one whose submission failed transiently -- and it must
    /// survive the decoder as a state rather than collapsing into silence.
    func testUnknownOnTheWireSurvivesAsAState() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0,"eligibility":"unknown",
         "eligibility_reason":"receipt_unavailable"}
        """)
        XCTAssertEqual(entry.eligibility, "unknown")
        XCTAssertEqual(entry.contributionEligibility?.state, "unknown")
        XCTAssertEqual(entry.contributionEligibility?.reason, "receipt_unavailable")
    }

    /// An ineligible row still decodes whole. Nothing is filtered here: the
    /// queue shows every session on the contributor's computer and offers
    /// only the eligible ones.
    func testAnIneligibleEntryDecodesWithItsReason() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0,
         "eligibility":"ineligible_permanent",
         "eligibility_reason":"no_inference_call"}
        """)
        XCTAssertEqual(entry.contributionEligibility?.state, "ineligible_permanent")
        XCTAssertEqual(entry.contributionEligibility?.reason, "no_inference_call")
    }

    /// An `eligible` row carries no reason, and that is not a decoding
    /// failure -- there is nothing to explain.
    func testAnEligibleEntryCarriesNoReason() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0,"eligibility":"eligible"}
        """)
        XCTAssertEqual(entry.contributionEligibility?.state, "eligible")
        XCTAssertNil(entry.contributionEligibility?.reason)
    }

    func testPreviewSummaryDecodesDistinctCounts() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "redactions_distinct":{"local_path":12},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertEqual(summary.redactions["local_path"], 185)
        XCTAssertEqual(summary.redactionsDistinct["local_path"], 12)
    }

    func testPreviewSummaryFromAnOlderDaemonHasNoDistinctCounts() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertTrue(summary.redactionsDistinct.isEmpty)
    }

    func testPreviewSummaryDecodesTheEnrollment() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{},"enrolled":true,
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertTrue(summary.enrolled)
    }

    /// The one field on this summary that gates a button.
    ///
    /// Absent means false, not "assume yes". A daemon predating the field
    /// cannot say whether the preview pinned a real identity, and
    /// `PreviewSheet` arms `Contribute` on exactly this -- so the missing
    /// answer has to be the refusing one. Windows already fails closed the
    /// same way, by `System.Text.Json`'s default rather than by choice.
    func testPreviewSummaryFromAnOlderDaemonIsNotEnrolled() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertFalse(summary.enrolled)
    }

    func testHistoryRecordDecodesProjectID() throws {
        let record = try decode(HistoryRecord.self, """
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z","project_id":"proj_abc",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """)
        XCTAssertEqual(record.projectID, "proj_abc")
    }

    func testHistoryRecordFromBeforeTheUpgradeHasNoProjectID() throws {
        let record = try decode(HistoryRecord.self, """
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """)
        XCTAssertEqual(record.projectID, "")
    }
}
