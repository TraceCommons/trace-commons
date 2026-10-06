import XCTest

/// Rules every glass surface in the app target obeys, checked by reading the
/// source. The list grows as screens move to TCDesign; a file on it may not
/// paint its own colours, its own materials or its own motion, because those
/// are what `TCDesign` adapts for Reduce Transparency, Increase Contrast and
/// Reduce Motion, and a screen that paints its own bypasses all three.
final class GlassSurfaceRulesTests: XCTestCase {
    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp")

    /// Glass files, relative to `Sources/TraceCommonsApp`. Append, never remove.
    static let files: [String] = [
        "Views/Settings/GlassSettingsContent.swift",
        "Views/Settings/ConnectionSection.swift",
        "Views/Settings/StartupSection.swift",
        "Views/Settings/WatchingSection.swift",
        "Views/Settings/ConsentSection.swift",
        "Views/Settings/PublicProfileSection.swift",
        "Views/Settings/ChangesSection.swift",
        "Views/Settings/SettingsStateRow.swift",
        "Views/Settings/GlassSourceRow.swift",
        "Views/Settings/WatchedFoldersSection.swift",
        "Views/Settings/PrivateAISection.swift",
        "Views/Settings/ProjectsSection.swift",
        "Views/Settings/ToolsSection.swift",
        "Views/Settings/WitnessSection.swift",
        // Ron's first run (#1030) and its hosts.
        "Views/FirstRun/FirstRunFrame.swift",
        "Views/FirstRun/JoinScreen.swift",
        "Views/FirstRun/FoldersScreen.swift",
        "Views/FirstRun/ToolAnswerRow.swift",
        "Views/FirstRun/ToolsScreen.swift",
        "Views/FirstRun/RulesScreen.swift",
        "Views/FirstRun/UsesScreen.swift",
        "Views/FirstRun/SharingDisclosures.swift",
        "Views/FirstRun/PasskeySheets.swift",
        "Views/OnboardingCoordinatorView.swift",
        "Views/Monitor/FirstRunViews.swift",
        "Views/ComputeView.swift",
        "Views/SkillLearningView.swift",
        "Views/Monitor/HistoryInspector.swift",
        "Views/Monitor/TracesHealth.swift",
        "Views/Monitor/TracesViews.swift",
        "Views/ScrubbingCaveat.swift",
        "Views/CertificateSection.swift",
        "Views/SessionSendDisclosureView.swift",
        "Views/PreviewSheet.swift",
        "Views/Monitor/TracesOffers.swift",
        "Views/SessionContributionOverview.swift",
        "Views/SessionDetailView.swift",
        "Views/PrivateInferenceView.swift",
        "Views/CredentialSection.swift",
        "Views/BalanceRow.swift",
        "Views/FundingRow.swift",
        "Views/NearAiJoinView.swift",
        "Views/HarnessListView.swift",
        "Views/Monitor/InferenceAccount.swift",
        "Views/Monitor/InferenceViews.swift",
        // Ron's inspector host and its prompts (#1146, Task 3 of #1241).
        "Views/Monitor/TracesInspectorHost.swift",
        "Views/Monitor/InspectorPrompts.swift",
        // Ron's summary inspector (#1146, Task 4 of #1241).
        "Views/Monitor/SummaryInspector.swift",
        // Ron's folder inspector (#1146, Task 5 of #1241).
        "Views/Monitor/ToolFolderInspectors.swift",
        // History's left pane (#1241 Task 8) now lives here: its stat cards,
        // credit card, community card and list.
        "Views/Monitor/HomeViews.swift",
        // Ron's session review card (#1146, Task 6 of #1241).
        "Views/Monitor/SessionReviewCard.swift",
    ]

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// No legacy palette: the glass tokens carry the contrast floors and the
    /// Increase Contrast values; `TC.` and `CommunityBrand` carry neither.
    func test_noLegacyPalette() throws {
        for rel in Self.files {
            let source = try Self.text(rel)
            XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression), "\(rel) reads TC.")
            XCTAssertFalse(source.contains("CommunityBrand"), "\(rel) reads CommunityBrand")
        }
    }

    /// No colour of its own: a hex literal or a system colour name bypasses
    /// the 4.5:1 and 3:1 floors `TextContrastTests` and
    /// `SelectionContrastTests` check on the tokens.
    func test_noColourLiterals() throws {
        let banned = [#"0x[0-9A-Fa-f]{6}"#, #"Color\(red:"#, #"\.foregroundStyle\(\.red\)"#,
                      #"\.foregroundStyle\(\.green\)"#, #"\.foregroundStyle\(Color\.(red|green|orange|yellow)"#]
        for rel in Self.files {
            let source = try Self.text(rel)
            for pattern in banned {
                XCTAssertNil(source.range(of: pattern, options: .regularExpression), "\(rel) paints \(pattern)")
            }
        }
    }

    /// No material of its own: Reduce Transparency is honoured inside
    /// `glassSurface` and the pane backdrop, nowhere else.
    func test_noMaterialOfItsOwn() throws {
        let banned = [".ultraThinMaterial", ".thinMaterial", ".regularMaterial", ".thickMaterial",
                      "NSVisualEffectView", ".glassEffect("]
        for rel in Self.files {
            let source = try Self.text(rel)
            for token in banned { XCTAssertFalse(source.contains(token), "\(rel) uses \(token)") }
        }
    }

    /// Every animation on a glass file is gated on Reduce Motion, the way
    /// `MenuBarGlassPanel` gates its slide.
    func test_everyAnimationHonoursReduceMotion() throws {
        for rel in Self.files {
            let source = try Self.text(rel)
            for line in source.split(separator: "\n") where line.contains(".animation(") || line.contains("withAnimation(") {
                XCTAssertTrue(line.contains("reduceMotion") || line.contains("GlassMotion"),
                              "\(rel) animates without Reduce Motion: \(line.trimmingCharacters(in: .whitespaces))")
            }
        }
    }

    /// No fixed point size: type follows the macOS text-size setting through
    /// `glassType`; a literal size does not.
    func test_noFixedPointType() throws {
        for rel in Self.files {
            let source = try Self.text(rel)
            XCTAssertNil(source.range(of: #"\.font\(\.system\(size:"#, options: .regularExpression), "\(rel) fixes a point size")
        }
    }

    /// Every status dot and status label carries words: a colour alone is
    /// not a state (HIG, and the owner's fail-closed rule).
    /// A dot drawn directly is paired with its sentence: the very next line
    /// is the `Text` it sits beside (Home's watching line draws a ringed
    /// dot that `GlassStatusLabel` cannot).
    func test_everyDotHasWords() throws {
        for rel in Self.files {
            for line in Self.bareDots(in: try Self.text(rel)) {
                XCTFail("\(rel):\(line) draws a bare dot; use GlassStatusLabel or pair the dot with its sentence")
            }
        }
    }

    /// The pairing rule still refuses a dot with no sentence beside it, and
    /// a dot that ends the file.
    func test_aBareDotIsStillRefused() {
        XCTAssertEqual(Self.bareDots(in: "GlassStatusDot(.on)\n    Spacer()\nGlassStatusDot(.ask)"), [1, 3])
        XCTAssertEqual(Self.bareDots(in: "GlassStatusDot(.on, ring: true)\n    Text(words)"), [])
    }

    /// The 1-based lines that draw `GlassStatusDot(` without a `Text(` on
    /// the next line.
    static func bareDots(in source: String) -> [Int] {
        let lines = source.components(separatedBy: "\n")
        return lines.indices.filter { index in
            guard lines[index].contains("GlassStatusDot(") else { return false }
            let next = index + 1 < lines.count ? lines[index + 1].trimmingCharacters(in: .whitespaces) : ""
            return !next.hasPrefix("Text(")
        }.map { $0 + 1 }
    }
}
