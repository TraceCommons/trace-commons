import SwiftUI

/// "About credit." -- shown on first run and again in History, per the
/// shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Credit, framed honestly") and `design-import/DESIGN-SPEC.md` section
/// 5.9.1. Copy is verbatim.
///
/// Credit is a **record**, not a currency: no currency symbol, no fiat
/// estimate, no projection, no date, no streaks, no leaderboards, no
/// progress rings. The audience is developers giving away work product;
/// gamifying it insults them and makes the speculative framing look like
/// manipulation. `lastRefreshedAt == nil` renders as "Not synced yet", never
/// a confident `0.0`.
///
/// The card carries the community site's coin as its emblem -- mint face,
/// ink linework, `#00b894` rim -- because the coin is what a contributor has
/// already seen on the site, and the card's job is to say plainly that the
/// thing behind that emblem is a receipt. It is held STILL here: the
/// one-motion rule (spec section 4.5) gives the desktop app exactly one
/// animation, the mark's assembly, and "the coin only turns on the website".
/// The face carries no `$`. Section 5.9.1's frame draws one, but section
/// 7.3's standing rule is that no currency symbol appears in the native UI
/// and the `$` lives only on the website's coin -- which is what the card's
/// own disclaimer says out loud.
///
/// A single reusable view rather than two copies, since the two call sites
/// (onboarding and `HistoryView`) must never drift on this wording.
struct CreditRecordView: View {
    let creditFinal: Double
    let creditPending: Double
    let lastRefreshedAt: Date?

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            VStack(alignment: .leading, spacing: TC.Space.l) {
                HStack(alignment: .center, spacing: TC.Space.l) {
                    CreditCoin(size: 64)

                    VStack(alignment: .leading, spacing: TC.Space.s) {
                        Text("About credit.").font(TC.Font_.cardTitle)
                        Text("""
                        Contributions earn credit points, scored on how novel and \
                        information-rich a trace is. Today credit is a record, not a \
                        currency: there is no payout, no token, no exchange rate, and no \
                        date. The intent is that credit eventually settles to something \
                        real, and if it does it will settle from this record. Contribute \
                        because you want the commons to exist.
                        """)
                        .font(TC.Font_.body)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 560, alignment: .leading)
                    }
                }

                if lastRefreshedAt == nil {
                    // Never a confident 0.0 for a number that was never fetched.
                    TCTag(text: "Not synced yet", tone: .neutral, symbol: "arrow.triangle.2.circlepath")
                } else {
                    // Same label-over-figure shape as a queue card's manifest.
                    // No currency symbol, no ring, no streak: it is a record.
                    //
                    // The labels are the app's, not section 5.9.1's ("Recorded"
                    // and "Pending review"). "Review" already means one
                    // specific thing in this product -- "Held for privacy
                    // review", which appears on this same History screen, in
                    // the menu bar and on every quarantined row -- so a credit
                    // figure labelled "Pending review" reads as credit held by
                    // a privacy reviewer rather than credit that has not
                    // finished scoring. "Still being scored" says which of the
                    // two it is.
                    HStack(alignment: .top, spacing: TC.Space.figure) {
                        figure("Final", creditFinal, tone: .primary)
                        figure("Still being scored", creditPending, tone: .secondary)
                    }
                }

                Text("""
                A credit is a signed record that a contribution was accepted. It is not \
                currency — the $ on the coin is the website's joke, and the app keeps the \
                disclaimer.
                """)
                .font(TC.Font_.caption)
                .foregroundStyle(TC.inkSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 560, alignment: .leading)
            }
            .padding(.vertical, TC.Space.md)
            .padding(.horizontal, TC.Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()

            // Outside the card, in the smallest type on the screen: the
            // stillness is a stated decision, not an oversight.
            Text("Still, always. The coin only turns on the website.")
                .font(TC.Font_.captionSmall)
                .foregroundStyle(TC.inkTertiary)
        }
    }

    /// A label over a figure. `Pending review` is set in `ink.secondary`
    /// because it is a number that is still moving; `Recorded` is settled.
    private func figure(_ label: String, _ value: Double, tone: HierarchicalShapeStyle) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xxs) {
            TCFieldLabel(label)
            Text(String(format: "%.1f", value))
                .font(TC.Font_.metricValueMono)
                .foregroundStyle(tone)
                .monospacedDigit()
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(label): \(String(format: "%.1f", value))")
    }
}

// MARK: - The coin

/// The community site's coin, spec section 5.9.1: two stacked discs, the
/// `#00b894` rim offset 3 right and 2 down behind a mint face with a 2px ink
/// border.
///
/// It is a brand emblem sitting on a native card, which is the point -- the
/// coin is the thing a contributor recognises from the site, and the card
/// around it is the tool telling them what it actually is. It does not turn,
/// and its face carries no glyph: see `CreditRecordView`'s note on the `$`.
private struct CreditCoin: View {
    var size: CGFloat = 64

    var body: some View {
        ZStack(alignment: .topLeading) {
            Circle()
                .fill(CommunityBrand.rim)
                .frame(width: disc, height: disc)
                .offset(x: 3 * unit, y: 2 * unit)
            Circle()
                .fill(CommunityBrand.accent)
                .frame(width: disc, height: disc)
                .overlay {
                    Circle().strokeBorder(CommunityBrand.ink, lineWidth: 2 * unit)
                }
        }
        .frame(width: size, height: size, alignment: .topLeading)
        .accessibilityHidden(true)
    }

    /// One unit of the 64pt positioning box the spec states the coin in.
    private var unit: CGFloat { size / 64 }
    private var disc: CGFloat { 58 * unit }
}

// MARK: - The manifesto takeover

/// Onboarding's privacy stanza -- the one black screen in the flow, spec
/// section 5.9.2.
///
/// The site inverts a page to black and sets its headline in the single
/// yellow it owns; that inversion becomes this screen. It is the only place
/// in the product `#f5c91f` appears, and section 7.3 says it appears exactly
/// once, so nothing else may use it.
///
/// Under Reduce Motion it does NOT invert: it renders ink-on-white, which is
/// the accessibility requirement stated in 5.9.2 and repeated in 7.3. The
/// inversion is not an animation -- there is nothing to animate under the
/// one-motion rule -- but it IS the kind of full-field contrast flip that
/// Reduce Motion is the system's signal for, so the setting is honoured
/// rather than argued with.
///
/// Not yet wired into `OnboardingCoordinatorView`: placing it in the flow
/// would change the flow's steps, which this pass does not do. It is drawn,
/// verbatim, ready for the pass that does.
struct PrivacyManifestoView: View {
    var onContinue: () -> Void
    var onBack: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: CommunityBrand.Metric.stanzaGap) {
            HStack(alignment: .firstTextBaseline) {
                Text("The promise")
                Spacer(minLength: TC.Space.m)
                // The manifesto is step 3 of the six-step flow.
                Text("03 — 06")
            }
            .font(CommunityBrand.Font_.labelMono)
            .textCase(.uppercase)
            .tracking(CommunityBrand.Font_.monoTracking)
            .foregroundStyle(mutedInk)

            Text("Nothing is sent unless you say so.".uppercased())
                .font(CommunityBrand.Font_.displayManifesto)
                .tracking(CommunityBrand.Font_.displayManifestoTracking)
                .lineSpacing(-6)
                .foregroundStyle(CommunityBrand.yellow)
                .frame(maxWidth: 460, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)

            Text("""
            Every trace is scrubbed on this machine, shown to you first, and sent only \
            when you press the one button that sends it. There is no other path out.
            """)
            .font(CommunityBrand.Font_.lede)
            .lineSpacing(TC.Font_.LineHeight.spacing(for: 18, 1.3))
            .foregroundStyle(primaryInk)
            .frame(maxWidth: 520, alignment: .leading)
            .fixedSize(horizontal: false, vertical: true)

            HStack(spacing: TC.Space.m) {
                Button("Continue", action: onContinue)
                    .buttonStyle(
                        CommunityBrandButtonStyle(
                            fill: CommunityBrand.accent,
                            ink: CommunityBrand.ink,
                            border: borderInk,
                            size: .onboarding
                        )
                    )
                    .keyboardShortcut(.defaultAction)
                Button("Back", action: onBack)
                    .buttonStyle(
                        CommunityBrandButtonStyle(
                            fill: ground,
                            ink: primaryInk,
                            border: borderInk,
                            size: .onboarding
                        )
                    )
            }
            .padding(.top, TC.Space.xs)

            Text("""
            The site's yellow (#f5c91f) appears exactly once there and exactly once here. \
            With reduced motion the site never inverts; this screen would follow the same \
            setting and render ink-on-white.
            """)
            .font(CommunityBrand.Font_.footnote)
            .foregroundStyle(mutedInk)
            .frame(maxWidth: 520, alignment: .leading)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, TC.Space.lg)
        .padding(.horizontal, CommunityBrand.Metric.stanzaPadding)
        .padding(.bottom, TC.Space.xxxl - TC.Space.xs)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(ground)
        // Whichever way it renders, it renders in the brand's light-only
        // vocabulary; nothing here follows the system appearance.
        .environment(\.colorScheme, .light)
    }

    private var ground: Color { reduceMotion ? CommunityBrand.paper : CommunityBrand.ink }
    private var primaryInk: Color { reduceMotion ? CommunityBrand.ink : CommunityBrand.paper }
    private var borderInk: Color { reduceMotion ? CommunityBrand.ink : CommunityBrand.paper }
    /// `brand.muted` on white, `brand.muted.onblack` on black.
    private var mutedInk: Color {
        reduceMotion ? CommunityBrand.muted : CommunityBrand.mutedOnBlack
    }
}

#Preview("Credit record") {
    CreditRecordView(creditFinal: 1240, creditPending: 180, lastRefreshedAt: Date())
        .padding(TC.Space.xlg)
        .frame(width: 560)
        .background(TC.ground)
}

#Preview("Privacy stanza") {
    PrivacyManifestoView(onContinue: {}, onBack: {})
        .frame(width: 640, height: 380)
}
