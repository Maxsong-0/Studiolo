import SwiftUI

/// The context window and, where the provider reports them, the plan's usage limits or,
/// for a pay-as-you-go API key, how much credit is left on it. The limits and credits are
/// the same views Settings, Providers shows for every signed-in provider at once. When a
/// Hydra pair sends its heads out on another provider, both providers' limits stack: the
/// lead's first, then the heads'.
struct UsagePanel: View {
    @Environment(AppModel.self) private var model
    let usage: ContextUsage?
    let provider: ProviderKind
    /// The pair's heads provider, when it differs from the lead's. Nil keeps the panel
    /// to the chat's own provider, exactly as before.
    let headsProvider: ProviderKind?
    /// Inside a floating panel the content takes the panel's width instead of the
    /// popover's 360 points.
    let fillsWidth: Bool

    init(usage: ContextUsage?, provider: ProviderKind, headsProvider: ProviderKind? = nil, fillsWidth: Bool = false) {
        self.usage = usage
        self.provider = provider
        self.headsProvider = headsProvider
        self.fillsWidth = fillsWidth
    }

    var body: some View {
        let registry = model.providers
        // A second provider only when the pair's heads go out on one of their own.
        let heads: ProviderKind? = (headsProvider != nil && headsProvider != provider) ? headsProvider : nil
        let showsLimits = PlanLimitsReader.exposesLimits(provider)
        let showsCredits = CreditsReader.exposesCredits(provider)
        let showsContext = usage?.fraction != nil && usage?.windowTokens != nil
        VStack(alignment: .leading, spacing: 0) {
            if let usage, let fraction = usage.fraction, let window = usage.windowTokens {
                HStack {
                    Text("Context window")
                        .foregroundStyle(Chrome.secondaryText)
                    Spacer(minLength: 12)
                    Text(verbatim: "\(Self.tokens(usage.usedTokens)) / \(Self.tokens(window)) (\(Int((fraction * 100).rounded()))%)")
                        .foregroundStyle(Chrome.secondaryText)
                        .monospacedDigit()
                }
                .font(.system(size: 13))
                UsageBar(fraction: fraction, tint: Chrome.accent, label: "Context window")
                    .padding(.top, 10)
            }

            if let heads {
                providerSection(provider, role: "Lead", needsDivider: showsContext)
                // No leading divider when neither the context nor the lead showed anything.
                providerSection(heads, role: "Heads", needsDivider: showsContext || showsLimits || showsCredits)
            } else {
                if showsLimits {
                    if showsContext {
                        Divider().padding(.vertical, 14)
                    }
                    PlanLimitsView(provider: provider)
                }

                if showsCredits {
                    if showsContext {
                        Divider().padding(.vertical, 14)
                    }
                    CreditsView(provider: provider)
                }
            }
        }
        .padding(16)
        .frame(width: fillsWidth ? nil : 360)
        .frame(maxWidth: fillsWidth ? .infinity : nil, alignment: .leading)
        .onAppear {
            registry.refreshPlanLimits(provider)
            registry.refreshCredits(provider)
            if let heads {
                registry.refreshPlanLimits(heads)
                registry.refreshCredits(heads)
            }
        }
    }

    /// One provider's limits and credits under a small Lead/Heads header. A provider
    /// that reports neither gets no section at all.
    @ViewBuilder
    private func providerSection(_ p: ProviderKind, role: String, needsDivider: Bool) -> some View {
        let showsLimits = PlanLimitsReader.exposesLimits(p)
        let showsCredits = CreditsReader.exposesCredits(p)
        if showsLimits || showsCredits {
            if needsDivider {
                Divider().padding(.vertical, 14)
            }
            HStack(spacing: 6) {
                ProviderIcon(provider: p, size: 13)
                Text(verbatim: "\(role) · \(p.displayName)")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
            }
            .padding(.bottom, 8)
            if showsLimits {
                PlanLimitsView(provider: p)
            }
            if showsLimits && showsCredits {
                Divider().padding(.vertical, 14)
            }
            if showsCredits {
                CreditsView(provider: p)
            }
        }
    }

    static func tokens(_ count: Int) -> String {
        func compact(_ value: Double, _ suffix: String) -> String {
            let text = String(format: "%.1f", value)
            return (text.hasSuffix(".0") ? String(text.dropLast(2)) : text) + suffix
        }
        if count >= 1_000_000 { return compact(Double(count) / 1_000_000, "M") }
        if count >= 1_000 { return compact(Double(count) / 1_000, "k") }
        return "\(count)"
    }
}
