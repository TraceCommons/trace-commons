import SwiftUI
import TCDesign
import TCShellCore

/// The Referrals section (referral spec section 6): the account's codes
/// with Copy and Turn off, how many people joined, the credit from
/// referrals, and a field to enter a code someone shared while the 30 days
/// after joining last. Every sentence is the core's draft copy
/// (`ReferralCopy`, for owner review); this only lays them out. There is no
/// banner, notification or one-time message: the lines here are headings,
/// labels, the account's state and the answer to the person's own click.
struct ReferralsSection: View {
    @EnvironmentObject private var app: AppModel
    @StateObject private var model = ReferralModel()

    private var read: SettingsRead {
        SettingsRead.resolve(answered: model.answered, failed: model.failed, startup: app.startup)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if read != .answered {
                SettingsReadNotice(read, retry: { Task { await model.refresh() } })
            } else if let surface = model.surface {
                if model.sessionEnded {
                    GlassEyebrowCard(surface.copy.title) {
                        // The session ended: the line is the action, as the
                        // payout section has it.
                        Button(surface.copy.sessionEnded) {
                            Task {
                                await app.signInToAccount()
                                await model.refresh()
                            }
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(app.accountSigningIn && !app.canRestartNearAiBrowserSignIn)
                    }
                } else {
                    codesCard(surface)
                    if surface.showsEnterField(model.status) || model.status?.applied == true {
                        enterCard(surface)
                    }
                }
            } else {
                SettingsUnavailable(read: .failed, retry: { Task { await model.refresh() } })
            }
        }
        .task(id: app.payoutLinkDaemonClient.map(ObjectIdentifier.init)) {
            model.attach(app.payoutLinkDaemonClient)
            await model.refresh()
        }
        .onChange(of: app.accountSessionGeneration) {
            Task { await model.refresh() }
        }
    }

    private func codesCard(_ surface: ReferralSurface) -> some View {
        GlassEyebrowCard(surface.copy.title) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(surface.copy.body)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let listing = model.listing {
                    totals(surface, listing)
                    ForEach(listing.codes) { row in
                        codeRow(surface, row)
                    }
                    if surface.showsDisableScope(listing) {
                        Text(surface.copy.disableScope)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let limit = surface.limitLine(listing) {
                        Text(limit)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    } else {
                        Button(surface.copy.createButton) { Task { await model.create() } }
                            .buttonStyle(GlassButtonStyle(.glass))
                            .accessibilityLabel(surface.copy.createAccessibilityName)
                            .disabled(model.busy)
                    }
                }
                if let refusal = model.refusal, refusal != .notApplied, refusal != .invalidCode {
                    refusalLine(surface.refusalLine(refusal))
                }
            }
        }
    }

    private func totals(_ surface: ReferralSurface, _ listing: ReferralCodesListing) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            InsightsUsageRow(label: surface.copy.peopleJoined, value: String(listing.peopleJoined))
            if let credit = surface.creditLine(listing, money: ReferralMoney.format) {
                InsightsUsageRow(label: surface.copy.credit, value: credit)
            }
        }
    }

    private func codeRow(_ surface: ReferralSurface, _ row: ReferralCodeRow) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(surface.shownCode(row))
                        .glassType(row.code == nil ? GlassTokens.TypeScale.caption : GlassTokens.TypeScale.mono)
                        .textSelection(.enabled)
                    if let state = surface.stateLabel(row) {
                        GlassChip(glass: state, muted: true)
                    }
                }
                Text(surface.joinedLine(row.refereeCount))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            Spacer(minLength: 0)
            if surface.canCopy(row) {
                Button(model.copiedCodeId == row.codeId ? surface.copy.copied : surface.copy.copyButton) {
                    model.copy(row)
                }
                .buttonStyle(GlassButtonStyle(.glass))
                .accessibilityLabel(surface.copyAccessibilityName(row))
            }
            if surface.canDisable(row) {
                Button(surface.copy.disableButton) { Task { await model.disable(row) } }
                    .buttonStyle(GlassButtonStyle(.link))
                    .accessibilityLabel(surface.disableAccessibilityName(row))
                    .disabled(model.busy)
            }
        }
    }

    /// The code someone shared: the field while one may be entered, the
    /// account's state once one is applied.
    private func enterCard(_ surface: ReferralSurface) -> some View {
        GlassEyebrowCard(surface.copy.enterTitle) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if model.status?.applied == true {
                    Text(surface.copy.appliedStatus)
                        .glassType(GlassTokens.TypeScale.body)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    if let until = model.status?.applyUntil.flatMap(Self.date) {
                        Text(surface.enterScope(date: until))
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    HStack(alignment: .bottom, spacing: GlassTokens.Space.s3) {
                        GlassTextField(surface.copy.fieldLabel, text: $model.draft,
                                       prompt: surface.copy.fieldPlaceholder,
                                       invalid: model.fieldProblem != nil)
                            .onSubmit { Task { await model.apply() } }
                        Button(surface.copy.applyButton) { Task { await model.apply() } }
                            .buttonStyle(GlassButtonStyle(.glass))
                            .accessibilityLabel(surface.copy.applyAccessibilityName)
                            .disabled(!model.canApply)
                    }
                    if let problem = model.fieldProblem {
                        refusalLine(problem)
                    } else if let refusal = model.refusal, refusal == .notApplied || refusal == .invalidCode {
                        refusalLine(surface.refusalLine(refusal))
                    }
                }
            }
        }
    }

    private func refusalLine(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassStatus.outside.textColor)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// `apply_until`, as the date the person reads.
    static func date(_ rfc3339: String) -> String? {
        let parse = Date.ISO8601FormatStyle(includingFractionalSeconds: true)
        let date = (try? parse.parse(rfc3339)) ?? (try? Date.ISO8601FormatStyle().parse(rfc3339))
        return date?.formatted(date: .long, time: .omitted)
    }
}
