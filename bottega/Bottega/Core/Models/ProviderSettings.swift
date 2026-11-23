// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// What the provider registry needs from the settings: which providers are switched on,
/// where their binaries and keys are, and the model each last ran. The app's settings
/// object conforms; the registry never sees the rest of it.
@MainActor
protocol ProviderSettings: AnyObject {
    func isEnabled(_ provider: ProviderKind) -> Bool
    func binaryPath(for provider: ProviderKind) -> String
    func lastModel(for provider: ProviderKind) -> String?
    func apiKey(for provider: ProviderKind) -> String
}
