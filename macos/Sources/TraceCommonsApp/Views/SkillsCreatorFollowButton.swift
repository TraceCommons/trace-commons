import SwiftUI
import TCDesign
import TCShellCore

/// Shared creator control used by Home and Following. Its label always reflects
/// the confirmed snapshot; only this creator shows the pending IPC spinner.
struct SkillsCreatorFollowButton: View {
    let publisher: DaemonData.SkillsPublisher
    let model: InsightsSkillsModel

    private var followed: Bool { model.shown?.followedPublishers.contains(publisher.id) == true }
    private var pending: Bool { model.pendingFollowPublisherID == publisher.id }
    private var failed: Bool { model.followFailurePublisherID == publisher.id }
    private var publisherValues: [String: String] { ["publisher": publisher.name] }

    var body: some View {
        VStack(alignment: .trailing, spacing: InsightsLayout.lineGap) {
            control
                .disabled(!model.canFollow)
                .fixedSize()
                .accessibilityValue(pending ? model.text("skills_follow_saving_accessibility", values: publisherValues) : "")
            if failed {
                Text(model.text("skills_follow_failed"))
                    .insightsError()
                    .multilineTextAlignment(.trailing)
                    .accessibilityLabel(model.text("skills_follow_failed_accessibility", values: publisherValues))
            }
        }
    }

    @ViewBuilder
    private var control: some View {
        if followed {
            Menu {
                Button(model.text("skills_unfollow")) { save(false) }
                    .accessibilityLabel(model.text("skills_unfollow_accessibility", values: publisherValues))
            } label: {
                label("skills_following", symbol: "checkmark")
            }
            .menuStyle(.button)
            .menuIndicator(.hidden)
            .buttonStyle(GlassButtonStyle(.glass, selected: true))
            .accessibilityLabel(model.text("skills_following_accessibility", values: publisherValues))
        } else {
            Button { save(true) } label: {
                label("skills_follow", symbol: "plus")
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .accessibilityLabel(model.text("skills_follow_accessibility", values: publisherValues))
        }
    }

    private func label(_ key: String, symbol: String) -> some View {
        HStack(spacing: GlassTokens.Space.s2) {
            if pending { GlassSpinner() }
            else { Image(systemName: symbol).accessibilityHidden(true) }
            Text(model.text(key))
        }
    }

    private func save(_ followed: Bool) {
        Task { await model.setFollow(publisherID: publisher.id, followed: followed) }
    }
}
