import Foundation

/// A pay-as-you-go API provider's prepaid balance: the number its dashboard shows,
/// where a subscription provider shows usage windows. DeepSeek is the only one read
/// here so far — its key is billed per token out of this credit.
struct ProviderCredits: Sendable, Equatable {
    /// What is left to spend.
    var total: Double
    /// The account's currency as the provider reports it ("USD", "CNY"); empty when it reports none.
    var currency: String
    /// The share of the total that came from a top-up, when the provider splits it.
    var toppedUp: Double?
    /// The share of the total that came from promotional grants, when the provider splits it.
    var granted: Double?
    /// The share of the total that the subscription grants each month, when the provider splits it.
    var monthly: Double?
    /// False once the provider says the account can no longer be charged: every request fails from then on.
    var isUsable: Bool

    /// "$12.34": the amount with whatever currency the provider reported.
    var amountText: String { Self.money(total, currency: currency) }

    /// "Topped up $10.00 · Granted $2.34", or nil when the provider reports one number.
    var breakdownText: String? {
        var parts: [String] = []
        if let monthly, monthly > 0 { parts.append("Monthly \(Self.money(monthly, currency: currency))") }
        if let toppedUp, toppedUp > 0 { parts.append("Topped up \(Self.money(toppedUp, currency: currency))") }
        if let granted, granted > 0 { parts.append("Granted \(Self.money(granted, currency: currency))") }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    var isDepleted: Bool { !isUsable || total <= 0 }

    /// Money wears the currency the provider reported, pinned to English formatting:
    /// the Mac's own locale would read "US$ 0,43" where DeepSeek's dashboard, and the
    /// rest of this app, write "$0.43". An unknown or missing code falls back to a
    /// plain number so the panel never shows a wrong symbol.
    static func money(_ value: Double, currency: String) -> String {
        let code = currency.uppercased()
        guard code.count == 3, code.allSatisfy(\.isLetter) else {
            let number = plain(value)
            return currency.isEmpty ? number : "\(number) \(currency)"
        }
        return value.formatted(.currency(code: code).locale(Locale(identifier: "en_US")))
    }

    private static func plain(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(2)).locale(Locale(identifier: "en_US")))
    }
}

/// Reading a provider's prepaid balance, the counterpart to `PlanLimitsReader`.
/// An API-key provider is billed from a credit the user tops up, so where a
/// subscription shows usage windows, the balance is what is worth watching.
/// Command Code shows both: its plan's rolling windows and the credits behind them.
@MainActor
enum CreditsReader {
    static func exposesCredits(_ provider: ProviderKind) -> Bool { provider == .deepseek || provider == .commandcode }

    static func read(_ provider: ProviderKind, apiKey: String) async -> ProviderCredits? {
        switch provider {
        case .deepseek: await DeepSeekAPI.balance(apiKey: apiKey)
        case .commandcode: await CommandCodeAPI.credits(apiKey: apiKey)
        default: nil
        }
    }

    /// Where the provider's dashboard tops the balance up.
    static func topUpURL(_ provider: ProviderKind) -> URL? {
        switch provider {
        case .deepseek: URL(string: "https://platform.deepseek.com/top_up")
        case .commandcode: CommandCodeAPI.billingURL
        default: nil
        }
    }
}
