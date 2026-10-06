import SwiftUI
import TCDesign

struct WatchingSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassEyebrowCard(SettingsWords.watching) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if model.daemonSettings == nil {
                    SettingsReadNotice(model.settingsRead, retry: model.refreshSettings)
                } else if let settings = model.daemonSettings {
                    line(SettingsLegacyWords.sessionFinishedAfter(settings.quiescenceSecs))
                    line(SettingsLegacyWords.atMostOneNotification(settings.digestIntervalSecs / 3600))
                    line(SettingsLegacyWords.undecidedDropped(settings.queueTtlDays))
                    SettingsStateRow(title: SettingsLegacyWords.notificationsRenderedHere,
                                     isOn: !settings.localNotifications)
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
