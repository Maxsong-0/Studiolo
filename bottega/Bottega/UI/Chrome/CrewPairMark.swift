// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// A pair as its two provider marks, the lead's larger and the hands' smaller after an
/// arrow, so every place a pair is drawn says the same thing at the same proportions.
struct CrewPairMark: View {
    let lead: ProviderKind
    let hands: ProviderKind
    var leadSize: CGFloat = 16

    var body: some View {
        HStack(spacing: (leadSize * 0.25).rounded()) {
            ProviderIcon(provider: lead, size: leadSize)
                .foregroundStyle(Chrome.primaryText)
            Image(systemName: "arrow.right")
                .font(.system(size: (leadSize * 0.5).rounded(), weight: .semibold))
                .foregroundStyle(Chrome.secondaryText)
            ProviderIcon(provider: hands, size: (leadSize * 0.75).rounded())
                .foregroundStyle(Chrome.primaryText.opacity(0.8))
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("\(lead.displayName) lead, \(hands.displayName) hands"))
    }
}
