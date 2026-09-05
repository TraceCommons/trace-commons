import Foundation

/// Navigation state used directly by the onboarding coordinator.
struct OnboardingNavigation {
    enum Step: Equatable {
        case welcome
        /// Only on a fresh install: the daemon is refusing to start until
        /// the session roots are declared. Never resumed to -- a daemon that
        /// is running already has its roots.
        case roots
        case connect
        case consent
        case privacyScan
        case projects
        case done

        static func afterWelcome(needsRoots: Bool) -> Self {
            needsRoots ? .roots : .connect
        }

        func previous(privacyScanConfigured: Bool) -> Self? {
            switch self {
            case .welcome: return nil
            case .roots, .connect: return .welcome
            case .consent: return .connect
            case .privacyScan: return .consent
            case .projects: return privacyScanConfigured ? .privacyScan : .consent
            case .done: return .projects
            }
        }
    }

    private(set) var step: Step
    private(set) var connectVisit = 0
    private(set) var scanIncluded = false
    private(set) var consentSaveInProgress = false

    init(step: Step = .welcome) { self.step = step }

    mutating func enter(_ next: Step) {
        guard !consentSaveInProgress else { return }
        if next == .connect { connectVisit += 1 }
        step = next
    }

    mutating func enrolled(visit: Int) {
        guard step == .connect, visit == connectVisit else { return }
        enter(.consent)
    }

    mutating func beginConsentSave(scanConfigured: Bool?) -> Bool {
        guard step == .consent, !consentSaveInProgress, let scanConfigured else { return false }
        scanIncluded = scanConfigured
        consentSaveInProgress = true
        return true
    }

    mutating func finishConsentSave(succeeded: Bool) {
        guard consentSaveInProgress else { return }
        consentSaveInProgress = false
        if succeeded { enter(scanIncluded ? .privacyScan : .projects) }
    }
}
