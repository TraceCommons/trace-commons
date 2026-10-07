import SwiftUI
import TCDesign

struct WatchingSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        // #1146's watcher card: drawn once the core's status is known, so
        // a chip and its buttons never guess at a state nobody read.
        if model.statusRead == .answered {
            watcher(paused: model.status.paused)
        }
        GlassEyebrowCard(SettingsLegacyWords.discoveryEyebrow, title: SettingsLegacyWords.discoveryTitle) {
            Button(SettingsLegacyWords.refresh, action: model.refreshSettings)
                .buttonStyle(GlassButtonStyle(.link))
                .fixedSize()
        } content: {
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

    /// #1146's `DAEMON / Contribution watcher`: Watching or Paused as a
    /// glass chip, what pausing does, then Pause watcher and Resume watcher,
    /// each enabled only in the state it changes. Pausing here is until it
    /// is resumed, as #1146's is; the menu bar keeps the timed pauses.
    private func watcher(paused: Bool) -> some View {
        GlassEyebrowCard(SettingsLegacyWords.watcherEyebrow, title: SettingsLegacyWords.watcherTitle) {
            GlassChip(glass: paused ? SettingsLegacyWords.watcherPaused : SettingsLegacyWords.watcherWatching,
                      muted: paused)
        } content: {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                Text(SettingsLegacyWords.watcherCaption)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: GlassTokens.Space.s4) {
                    Button(SettingsLegacyWords.pauseWatcher) { model.pause(until: nil) }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(paused)
                    Button(SettingsLegacyWords.resumeWatcher) { model.resume() }
                        .buttonStyle(GlassButtonStyle(.primary, small: true))
                        .disabled(!paused)
                }
            }
        }
    }

    private func line(_ sentence: String) -> some View {
        Text(sentence).glassType(GlassTokens.TypeScale.body)
    }
}
