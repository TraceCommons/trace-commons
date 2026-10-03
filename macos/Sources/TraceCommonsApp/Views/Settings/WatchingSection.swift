import SwiftUI
import TCDesign

struct WatchingSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassEyebrowCard(SettingsWords.watching) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if let settings = model.daemonSettings {
                    line(SettingsLegacyWords.sessionFinishedAfter(settings.quiescenceSecs))
                    line(SettingsLegacyWords.atMostOneNotification(settings.digestIntervalSecs / 3600))
                    line(SettingsLegacyWords.undecidedDropped(settings.queueTtlDays))
                    GlassStatusLabel(SettingsLegacyWords.notificationsRenderedHere,
                                     status: settings.localNotifications ? .off : .on)
                }
                if model.status.paused {
                    line(SettingsLegacyWords.pausedNothingSent)
                }
            }
        }
    }

    private func line(_ sentence: String) -> some View {
        Text(sentence).glassType(GlassTokens.TypeScale.body)
    }
}
