// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct MCPGlyph: View {
    let entry: MCPCatalogEntry
    var size: CGFloat = 20
    var isRunning = false
    var status: ToolCall.Status?

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        ZStack {
            if isRunning {
                CrewBreathRing(color: NSColor(entry.color.opacity(0.55)).cgColor, lineWidth: 1.5)
                    .frame(width: size, height: size)
                    .allowsHitTesting(false)
            }
            Image(entry.asset)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .foregroundStyle(entry.isMonochrome ? Chrome.primaryText : entry.color)
                .frame(width: size, height: size)
        }
        .frame(width: size, height: size)
        .overlay(alignment: .bottomTrailing) {
            if let badge = badgeSymbol {
                badgeView(symbol: badge.symbol, color: badge.color)
            }
        }
        .accessibilityLabel(Text("\(entry.name) tool"))
    }

    private var badgeSymbol: (symbol: String, color: Color)? {
        switch status {
        case .failed: ("xmark", Chrome.danger)
        case .declined: ("hand.raised.slash", Chrome.warning)
        default: nil
        }
    }

    private func badgeView(symbol: String, color: Color) -> some View {
        let diameter = max(8, size * 0.5)
        return ZStack {
            Circle()
                .fill(color)
            Image(systemName: symbol)
                .font(.system(size: diameter * 0.55, weight: .heavy))
                .foregroundStyle(.white)
        }
        .frame(width: diameter, height: diameter)
        .overlay {
            Circle().strokeBorder(colorScheme == .dark ? Color.black.opacity(0.55) : Color.white.opacity(0.95), lineWidth: max(1, diameter * 0.12))
        }
        .offset(x: diameter * 0.3, y: diameter * 0.3)
        .transition(.scale.combined(with: .opacity))
    }
}
