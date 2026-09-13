// INTEGRATION: provides the daemon boundary double shared by tested-skill app
// tests while production decoding continues through the real DaemonClient.

import Foundation
import TCBridge

@testable import TraceCommonsApp

final class SkillLearningDaemon: DaemonCalling, @unchecked Sendable {
    struct Call {
        let method: String
        let params: String
    }

    private let lock = NSLock()
    private var recorded: [Call] = []
    private var failures: [String: String] = [:]
    private var installCommitReplyFailure: String?
    private var installed = false
    private var installedName = "repair-generated-sources"
    private var installedSourceSubmissionID = "22222222-2222-4222-8222-222222222222"
    private var lastSourceSubmissionID = "22222222-2222-4222-8222-222222222222"
    private var gatePasses = true
    private var installOccupied = false
    private var reportedInstallStatus: Bool?
    private var reportedInstallSkillPresence: Bool?
    private var recoveredDraft: [String: Any]?
    private var recoveredReviewID: String?
    private var responseFieldOverrides: [String: [String: String]] = [:]

    var calls: [Call] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func setFailure(_ method: String, label: String?) {
        lock.lock()
        defer { lock.unlock() }
        failures[method] = label
    }

    /// Simulates a daemon that commits the skill to disk, then loses the
    /// response before the client receives its receipt.
    func setInstallCommitReplyFailure(_ label: String?) {
        lock.lock()
        defer { lock.unlock() }
        installCommitReplyFailure = label
    }

    func setGatePasses(_ passes: Bool) {
        lock.lock()
        defer { lock.unlock() }
        gatePasses = passes
    }

    func setInstallOccupied(_ occupied: Bool) {
        lock.lock()
        defer { lock.unlock() }
        installOccupied = occupied
    }

    func setResponseField(
        _ method: String,
        field: String,
        value: String?
    ) {
        lock.lock()
        defer { lock.unlock() }
        if let value {
            responseFieldOverrides[method, default: [:]][field] = value
        } else {
            responseFieldOverrides[method]?[field] = nil
            if responseFieldOverrides[method]?.isEmpty == true {
                responseFieldOverrides[method] = nil
            }
        }
    }

    func setInstalled(_ value: Bool) {
        lock.lock()
        defer { lock.unlock() }
        installed = value
    }

    func setInstallStatusShape(installed: Bool?, includesSkill: Bool?) {
        lock.lock()
        defer { lock.unlock() }
        reportedInstallStatus = installed
        reportedInstallSkillPresence = includesSkill
    }

    func seedInstalledSkill(name: String, sourceSubmissionID: String) {
        lock.lock()
        defer { lock.unlock() }
        installed = true
        installedName = name
        installedSourceSubmissionID = sourceSubmissionID
    }

    func seedRecoveredReview(draft: SkillDraft, reviewID: String) {
        lock.lock()
        defer { lock.unlock() }
        recoveredDraft = [
            "name": draft.name,
            "description": draft.description,
            "procedure": draft.procedure,
        ]
        recoveredReviewID = reviewID
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append(Call(method: method, params: paramsJSON))
        if let label = failures[method] {
            return encode(["id": 1, "error": ["code": "unavailable", "message": label]])
        }
        let params = (try? JSONSerialization.jsonObject(with: Data(paramsJSON.utf8)))
            as? [String: Any] ?? [:]

        var result: Any
        switch method {
        case "skill_candidate":
            let source = params["submission_id"] as? String ?? lastSourceSubmissionID
            lastSourceSubmissionID = source
            result = candidate(sourceSubmissionID: source)
        case "skill_install_status":
            let source = params["source_submission_id"] as? String
            let matches = installed && source == installedSourceSubmissionID
            let includesSkill = reportedInstallSkillPresence ?? matches
            let skill: Any = includesSkill ? installedSkill : NSNull()
            result = ["installed": reportedInstallStatus ?? matches, "skill": skill]
        case "skill_review":
            var value = review
            if let submittedDraft = params["draft"] as? [String: Any] {
                value["draft"] = submittedDraft
            }
            result = value
        case "skill_evaluate":
            result = evaluation
        case "skill_install_plan":
            result = installPlan
        case "skill_install_commit":
            installed = true
            installedSourceSubmissionID = lastSourceSubmissionID
            if let label = installCommitReplyFailure {
                return encode(["id": 1, "error": ["code": "unavailable", "message": label]])
            }
            result = installedSkill
        case "skill_install_rollback":
            let matches = (params["source_submission_id"] as? String)
                == installedSourceSubmissionID
            if matches { installed = false }
            result = ["removed": matches, "retained_directory": false]
        default:
            return encode([
                "id": 1,
                "error": ["code": "unavailable", "message": "unexpected-test-method"],
            ])
        }
        if let overrides = responseFieldOverrides[method],
           var object = result as? [String: Any]
        {
            for (field, value) in overrides {
                object[field] = value
            }
            result = object
        }
        return encode(["id": 1, "result": result])
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }

    private func encode(_ object: Any) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        else {
            return #"{"id":1,"error":{"code":"unavailable","message":"test-encoding-failed"}}"#
        }
        return String(decoding: data, as: UTF8.self)
    }

    private var skillMD: String {
        "---\nname: repair-generated-sources\ndescription: test\n---\n\nEdit the source."
    }

    private var draft: [String: Any] {
        [
            "name": "repair-generated-sources",
            "description": "Use when a generated file has an authoritative source.",
            "procedure": "# Procedure\n\nEdit the source, regenerate, and verify drift.",
        ]
    }

    private var evaluationContract: [String: Any] {
        [
            "task_count": 8,
            "plan_task_count": 6,
            "fixture_pool_count": 7,
            "cluster_count": 7,
            "arm_count": 3,
            "total_requests": 24,
            "output_token_limit": 900,
            "request_timeout_seconds": 90,
            "required_model_owner": "nearai",
            "fixture_scope": "Six source-excluded client-shipped public repository incidents plus two metadata-only applicability probes; no session task, correction, or evidence is sent.",
            "selection_policy": "Select six incidents from the fixed public fixture pool after excluding source-task overlap.",
        ]
    }

    private func candidate(sourceSubmissionID: String) -> [String: Any] {
        var value: [String: Any] = [
            "candidate_id": "11111111-1111-4111-8111-111111111111",
            "family": "generated-source-repair",
            "source_submission_id": sourceSubmissionID,
            "source_correction": "Change the source schema and regenerate the generated file.",
            "source_evidence": [[
                "event_id": "33333333-3333-4333-8333-333333333333",
                "kind": "feedback",
                "excerpt": "The source schema owns this generated output.",
            ]],
            "draft": draft,
            "manual_control_instruction": "Edit the source and regenerate generated files.",
            "evaluation_contract": evaluationContract,
        ]
        if let recoveredDraft {
            value["draft"] = recoveredDraft
        }
        if let recoveredReviewID {
            value["replaces_review_id"] = recoveredReviewID
        }
        return value
    }

    private var review: [String: Any] {
        [
            "review_id": "44444444-4444-4444-8444-444444444444",
            "candidate_id": "11111111-1111-4111-8111-111111111111",
            "family": "generated-source-repair",
            "source_submission_id": "22222222-2222-4222-8222-222222222222",
            "source_evidence_ids": ["33333333-3333-4333-8333-333333333333"],
            "draft": draft,
            "skill_md": skillMD,
            "skill_sha256": String(repeating: "a", count: 64),
        ]
    }

    private var evaluation: [String: Any] {
        let trials = trialRows
        let arms = [
            (arm: "baseline", label: "Baseline"),
            (arm: "manual_instruction", label: "Simple instruction"),
            (arm: "candidate_skill", label: "Candidate skill"),
        ]
        let summarize: ([[String: Any]]) -> [[String: Any]] = { selectedTrials in
            arms.map { arm in
                let matching = selectedTrials.filter { ($0["arm"] as? String) == arm.arm }
                return [
                    "arm": arm.arm,
                    "label": arm.label,
                    "passed": matching.filter { ($0["passed"] as? Bool) == true }.count,
                    "total": matching.count,
                ]
            }
        }
        var value = evaluationContract
        value.merge([
            "evaluation_id": "55555555-5555-4555-8555-555555555555",
            "review_id": "44444444-4444-4444-8444-444444444444",
            "skill_sha256": String(repeating: "a", count: 64),
            "requested_model": "deepseek-ai/DeepSeek-V4-Flash",
            "served_model": "deepseek-ai/DeepSeek-V4-Flash",
            "model_owned_by": "nearai",
            "source_task_fingerprint_sha256": String(repeating: "e", count: 64),
            "selected_plan_task_ids": trials
                .filter { ($0["cluster"] as? String) != "metadata-applicability" }
                .compactMap { $0["task_id"] as? String }
                .reduce(into: [String]()) { ids, id in
                    if !ids.contains(id) { ids.append(id) }
                },
            "excluded_source_overlap_task_ids": [],
            "reserve_plan_task_ids": ["contributor-release-version-stamp"],
            "ambient_context": [
                "public immutable repository evidence",
                "metadata-only applicability probes",
            ],
            "summaries": summarize(trials),
            "plan_summaries": summarize(
                trials.filter { ($0["cluster"] as? String) != "metadata-applicability" }
            ),
            "applicability_summaries": summarize(
                trials.filter { ($0["cluster"] as? String) == "metadata-applicability" }
            ),
            "trials": trials,
            "regressions": [],
            "install_allowed": gatePasses,
            "gate_reason": gatePasses
                ? "skill-improved-over-both-controls"
                : "skill-did-not-beat-both-controls",
        ]) { _, responseValue in responseValue }
        return value
    }

    private var trialRows: [[String: Any]] {
        let fixtures = trialFixtures
        return fixtures.flatMap { fixture in
            ["baseline", "manual_instruction", "candidate_skill"].map { arm in
                let passed: Bool
                switch arm {
                case "baseline":
                    passed = fixture.baselinePasses
                case "manual_instruction":
                    passed = fixture.manualPasses
                default:
                    passed = gatePasses || fixture.candidatePassesWhenGateFails
                }

                let answer: Any
                let rawOutput: String
                if let guidanceShouldApply = fixture.guidanceShouldApply {
                    answer = NSNull()
                    let applicable = arm == "baseline" ? false : guidanceShouldApply
                    rawOutput = encode(["applicable": applicable])
                } else {
                    let plan = passed ? fixture.passingAnswer : fixture.failingAnswer
                    answer = plan as Any
                    rawOutput = encode(plan)
                }

                return [
                    "task_id": fixture.taskID,
                    "cluster": fixture.cluster,
                    "task": fixture.task,
                    "source_url": fixture.sourceURL,
                    "arm": arm,
                    "passed": passed,
                    "failure_reasons": passed
                        ? []
                        : ["The plan omitted the authoritative source or a required regeneration step."],
                    "answer": answer,
                    "raw_output": rawOutput,
                    "request_id": "request-\(fixture.taskID)-\(arm)",
                    "served_model": "deepseek-ai/DeepSeek-V4-Flash",
                    "finish_reason": "stop",
                    "usage": [
                        "prompt_tokens": 200,
                        "completion_tokens": 100,
                        "reasoning_tokens": 0,
                        "total_tokens": 300,
                    ],
                ] as [String: Any]
            }
        }
    }

    private struct TrialFixture {
        let taskID: String
        let cluster: String
        let task: String
        let sourceURL: String
        let guidanceShouldApply: Bool?
        let passingAnswer: [String: Any]
        let failingAnswer: [String: Any]
        let baselinePasses: Bool
        let manualPasses: Bool
        let candidatePassesWhenGateFails: Bool
    }

    private var trialFixtures: [TrialFixture] {
        let noPlan: [String: Any] = [:]
        let incompletePlan: [String: Any] = [
            "diagnosis": "Edit the checked-in output directly.",
            "edit_paths": ["generated/output"],
            "commands": [],
            "verification": [],
        ]
        return [
            TrialFixture(
                taskID: "discovery-positive-generated-source",
                cluster: "metadata-applicability",
                task: "Agent Skill metadata applicability: A checked-in Windows package image is generated from shared Rust source; change its background and regenerate every scale variant.",
                sourceURL: "https://github.com/TraceCommons/trace-commons",
                guidanceShouldApply: true,
                passingAnswer: noPlan,
                failingAnswer: noPlan,
                baselinePasses: true,
                manualPasses: true,
                candidatePassesWhenGateFails: true
            ),
            TrialFixture(
                taskID: "discovery-negative-handwritten-docs",
                cluster: "metadata-applicability",
                task: "Agent Skill metadata applicability: Correct punctuation in a hand-written README paragraph. The file is not generated and no derived output changes.",
                sourceURL: "https://github.com/TraceCommons/trace-commons",
                guidanceShouldApply: false,
                passingAnswer: noPlan,
                failingAnswer: noPlan,
                baselinePasses: true,
                manualPasses: true,
                candidatePassesWhenGateFails: true
            ),
            TrialFixture(
                taskID: "mark-msix-scale-ladder",
                cluster: "mark-assets",
                task: "Change the Windows package tile background while keeping all three base assets and their four display-scale variants consistent.",
                sourceURL: "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5",
                guidanceShouldApply: nil,
                passingAnswer: [
                    "diagnosis": "The shared mark library generates every committed tile variant.",
                    "edit_paths": ["crates/trace-commons-mark/src/lib.rs"],
                    "commands": ["cargo run -p trace-commons-mark --bin mark-export -- assets/mark --repo-root ."],
                    "verification": ["scripts/mark/check-drift.sh"],
                ],
                failingAnswer: incompletePlan,
                baselinePasses: false,
                manualPasses: true,
                candidatePassesWhenGateFails: true
            ),
            TrialFixture(
                taskID: "witness-compose-image-pin",
                cluster: "witness-compose",
                task: "Replace the witness image digest and keep the measured app-compose deployment document current.",
                sourceURL: "https://github.com/TraceCommons/trace-commons/commit/c443326564264b27f5c65cd385b89762e7a81ea1",
                guidanceShouldApply: nil,
                passingAnswer: [
                    "diagnosis": "The compose file owns the image pin and generates app-compose.json.",
                    "edit_paths": ["deploy/witness/docker-compose.yml"],
                    "commands": ["deploy/witness/build-app-compose.sh"],
                    "verification": ["deploy/witness/build-app-compose.sh --check"],
                ],
                failingAnswer: incompletePlan,
                baselinePasses: false,
                manualPasses: true,
                candidatePassesWhenGateFails: true
            ),
            TrialFixture(
                taskID: "flatpak-ironwire-pin",
                cluster: "flatpak-sources",
                task: "Advance the pinned IronWire proxy dependency to a reviewed revision and keep both lockfiles and the offline Flatpak vendor source set synchronized.",
                sourceURL: "https://github.com/TraceCommons/trace-commons/commit/e89a628bf4be8b1e31dd278793b00391e9781f1c",
                guidanceShouldApply: nil,
                passingAnswer: [
                    "diagnosis": "The contributor dependency pin owns two lockfiles and the generated offline source set.",
                    "edit_paths": ["crates/trace-commons-contributor/Cargo.toml"],
                    "commands": [
                        "cargo update -p ironwire_proxy",
                        "cargo update --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml -p ironwire_proxy",
                        "python3 flatpak-cargo-generator.py crates/trace-commons-contributor-gtk/Cargo.lock -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json",
                    ],
                    "verification": ["cargo test -p trace-commons-contributor --test release_pipeline"],
                ],
                failingAnswer: incompletePlan,
                baselinePasses: false,
                manualPasses: false,
                candidatePassesWhenGateFails: false
            ),
            TrialFixture(
                taskID: "winget-publisher",
                cluster: "winget-manifests",
                task: "Change the publisher shown in the generated WinGet release manifests without allowing the singleton, version, and locale documents to drift apart.",
                sourceURL: "https://github.com/TraceCommons/trace-commons/commit/c5786be30bf4135ca0d5e63e5b8ee04f02acbc04",
                guidanceShouldApply: nil,
                passingAnswer: [
                    "diagnosis": "The generator owns all three versioned WinGet manifests.",
                    "edit_paths": ["scripts/winget/generate-manifests.sh"],
                    "commands": ["scripts/winget/generate-manifests.sh 0.12.2"],
                    "verification": ["winget validate --manifest manifests/t/TraceCommons/Contributor/0.12.2"],
                ],
                failingAnswer: incompletePlan,
                baselinePasses: false,
                manualPasses: false,
                candidatePassesWhenGateFails: false
            ),
            TrialFixture(
                taskID: "png-encoder-foreign-decoder",
                cluster: "png-encoding",
                task: "Reduce the generated Windows tile PNG size by changing the shared encoder, regenerate all committed base tiles, and verify the bytes with an independent decoder.",
                sourceURL: "https://github.com/TraceCommons/trace-commons/commit/13f53d4e2c453a0ba15de2ce7d8c6f77eb33a77b",
                guidanceShouldApply: nil,
                passingAnswer: [
                    "diagnosis": "The shared raster encoder owns every committed base tile.",
                    "edit_paths": ["crates/trace-commons-mark/src/raster.rs"],
                    "commands": ["cargo run -p trace-commons-mark --bin mark-export -- assets/mark --repo-root ."],
                    "verification": ["python3 scripts/mark/verify-png.py target/mark-verify"],
                ],
                failingAnswer: incompletePlan,
                baselinePasses: false,
                manualPasses: true,
                candidatePassesWhenGateFails: false
            ),
            TrialFixture(
                taskID: "update-conformance-platform",
                cluster: "update-conformance",
                task: "Change the artifact URL origin in the deterministic update-conformance generator and regenerate every signed manifest fixture without hand-editing its signatures.",
                sourceURL: "https://github.com/TraceCommons/trace-commons/commit/ddbfd593338541cf67af6cbd50d3af50de492ee2",
                guidanceShouldApply: nil,
                passingAnswer: [
                    "diagnosis": "The conformance generator owns each manifest and signature fixture.",
                    "edit_paths": ["tests/fixtures/update-conformance/regenerate.sh"],
                    "commands": ["tests/fixtures/update-conformance/regenerate.sh"],
                    "verification": ["cargo test -p trace-commons-contributor --test update_conformance"],
                ],
                failingAnswer: incompletePlan,
                baselinePasses: false,
                manualPasses: false,
                candidatePassesWhenGateFails: false
            ),
        ]
    }

    private var installPlan: [String: Any] {
        let target = "$CODEX_HOME/skills/repair-generated-sources"
        return [
            "plan_id": "66666666-6666-4666-8666-666666666666",
            "evaluation_id": "55555555-5555-4555-8555-555555555555",
            "tool": "Codex",
            "target_location": target,
            "skill_location": target + "/SKILL.md",
            "marker_location": target + "/.trace-commons-install.json",
            "occupied": installOccupied,
            "can_install": !installOccupied,
            "skill_md": skillMD,
            "skill_sha256": String(repeating: "a", count: 64),
            "marker_json": markerJSON,
            "marker_file_sha256": String(repeating: "b", count: 64),
        ]
    }

    private var markerJSON: String {
        let skillDigest = String(repeating: "a", count: 64)
        let markerDigest = String(repeating: "c", count: 64)
        return """
        {
          "schema_version": 2,
          "install_id": "77777777-7777-4777-8777-777777777777",
          "evaluation_id": "55555555-5555-4555-8555-555555555555",
          "tool": "Codex",
          "name": "repair-generated-sources",
          "skill_sha256": "\(skillDigest)",
          "source_submission_id": "\(lastSourceSubmissionID)",
          "source_evidence_ids": ["33333333-3333-4333-8333-333333333333"],
          "owner_scope_sha256": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
          "device_key_id": "device-key-test",
          "installed_at": "2026-09-11T05:00:00Z",
          "marker_sha256": "\(markerDigest)",
          "device_signature_b64": "synthetic-signature"
        }
        """
    }

    private var installedSkill: [String: Any] {
        [
            "install_id": "77777777-7777-4777-8777-777777777777",
            "evaluation_id": "55555555-5555-4555-8555-555555555555",
            "source_submission_id": installedSourceSubmissionID,
            "tool": "Codex",
            "name": installedName,
            "target_location": "$CODEX_HOME/skills/\(installedName)",
            "skill_sha256": String(repeating: "a", count: 64),
            "marker_sha256": String(repeating: "c", count: 64),
            "installed_at": "2026-09-11T05:00:00Z",
        ]
    }
}
