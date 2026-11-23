// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import SwiftUI

private enum TokenActivityStyle {
    /// Gap between cells, both axes.
    static let cellGap: CGFloat = 4
    static let cellRadius: CGFloat = 2
    /// Height of the month label strip.
    static let labelHeight: CGFloat = 18
    /// Narrowest a month label may squeeze before its neighbors hide it.
    static let minLabelAdvance: CGFloat = 30

    /// Five-step intensity ladder: empty, then four steps of the theme's
    /// accent (the system accent for System/Light/Dark), so the grid wears
    /// whatever the rest of the window wears. A day still to come is fainter
    /// than a quiet one, so the year keeps its shape without reading as data.
    static func color(for value: Int, maxValue: Int, future: Bool) -> Color {
        if future { return Chrome.overlay(0.035) }
        guard value > 0, maxValue > 0 else { return Chrome.overlay(0.07) }
        let accent = Chrome.accent
        let fraction = Double(value) / Double(maxValue)
        switch fraction {
        case ..<0.25: return accent.opacity(0.32)
        case ..<0.5: return accent.opacity(0.55)
        case ..<0.75: return accent.opacity(0.8)
        default: return accent
        }
    }
}

/// The heatmap section at the top of Settings, General. Plain page content,
/// not a ChromeCard: the grid needs the full column width, and a card would
/// inset it into a padded strip the mockup does not have. The mode is
/// view-local and the totals publish from the shared ledger, so switching
/// modes never touches the rest of General.
struct TokenActivitySection: View {
    @State private var mode: TokenActivityMode = .daily
    @State private var ledger = TokenLedger.shared
    /// The grid the last mode showed and a counter the canvas animates across, so a
    /// mode switch blends every cell from its old colour to its new one.
    @State private var previousGrid: TokenActivityGrid?
    @State private var switchCount = 0
    /// Built once per (totals, mode): a year of calendar arithmetic that every usage
    /// event of every running session would otherwise redo while General is open.
    @State private var grid = TokenActivityGrid.build(daily: [:], mode: .daily)

    private struct GridKey: Equatable {
        var daily: [String: Int]
        var mode: TokenActivityMode
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(verbatim: "Token activity")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(1)
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 12)
                ForEach(TokenActivityMode.allCases, id: \.self) { option in
                    Button {
                        guard option != mode else { return }
                        // The old mode's grid is the blend's starting point and stays until
                        // the blend has settled; the canvas shows it as-is for the first frame
                        // and the new grid as-is for the last.
                        previousGrid = grid
                        let switchIndex = switchCount + 1
                        withAnimation(.smooth(duration: 0.45)) {
                            mode = option
                            switchCount = switchIndex
                        } completion: {
                            if switchCount == switchIndex { previousGrid = nil }
                        }
                    } label: {
                        Text(verbatim: option.title)
                            .font(.system(size: 13, weight: mode == option ? .medium : .regular))
                    }
                    // A capsule chip, like every other small switch in the app: these
                    // were bare words with no hover, no hit target and no button trait.
                    .buttonStyle(.chip(active: mode == option))
                    .help(Self.modeHelp(option))
                    .accessibilityLabel(Text(verbatim: option.title))
                    .accessibilityAddTraits(mode == option ? [.isButton, .isSelected] : .isButton)
                }
            }
            .animation(.smooth(duration: 0.25), value: mode)

            heatmap(grid: grid)
            monthStrip(grid: grid)
            legend(total: ledger.dailyTotals.values.reduce(0, +), uncertainTokens: ledger.uncertainTokensThisYear)
        }
        .onChange(of: GridKey(daily: ledger.dailyTotals, mode: mode), initial: true) { _, key in
            grid = TokenActivityGrid.build(daily: key.daily, mode: key.mode)
        }
        .task {
            await ledger.refreshIfNeeded()
        }
    }

    /// Equal-width week columns of square cells. Each column takes an equal
    /// share of the width and each cell squares off to its column, so the grid
    /// fills any Settings width without clipping or scrolling. One canvas draws
    /// every cell: as a stack of some 360 shape views this was the heaviest
    /// layout in Settings, felt each time General opened.
    private func heatmap(grid: TokenActivityGrid) -> some View {
        HeatmapCanvas(grid: grid, previous: previousGrid, blend: Double(switchCount), switchIndex: switchCount)
            .accessibilityHidden(true)
    }

    /// Month names pinned to the column holding each month's first day.
    /// Labels that would land closer than a readable advance are dropped,
    /// so a narrow window shows fewer months instead of overlapping them.
    private func monthStrip(grid: TokenActivityGrid) -> some View {
        ZStack(alignment: .topLeading) {
            Color.clear.frame(height: TokenActivityStyle.labelHeight)
            GeometryReader { proxy in
                let columnCount = max(grid.columns.count, 1)
                let stride = (proxy.size.width + TokenActivityStyle.cellGap) / CGFloat(columnCount)
                let placed = placedTicks(grid.monthTicks, stride: stride)
                ForEach(placed.indices, id: \.self) { index in
                    Text(verbatim: placed[index].label)
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                        .fixedSize()
                        .position(
                            x: min(
                                max(CGFloat(placed[index].column) * stride, 12),
                                proxy.size.width - 12
                            ),
                            y: TokenActivityStyle.labelHeight / 2
                        )
                }
            }
        }
    }

    /// The line under the grid: what the ledger adds up to on the left, and the intensity
    /// ladder on the right. The canvas itself is hidden from VoiceOver (365 cells say
    /// nothing one at a time), so this is where the grid gets a readable value.
    /// Historical overlap stays available in help and VoiceOver without crowding the footer.
    private func legend(total: Int, uncertainTokens: Int) -> some View {
        let summary = "\(UsagePanel.tokens(total)) tokens"
        let detail = uncertainTokens > 0
            ? "\(UsagePanel.tokens(uncertainTokens)) tokens have an unknown source and may overlap imported history."
            : "Total tokens recorded this year."

        return HStack(spacing: 8) {
            Text(verbatim: summary)
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
                .help(detail)
            Spacer(minLength: 12)
            Text(verbatim: "Less")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
            ForEach(Array(Self.ladder.enumerated()), id: \.offset) { _, step in
                RoundedRectangle(cornerRadius: TokenActivityStyle.cellRadius, style: .continuous)
                    .fill(step)
                    .frame(width: 10, height: 10)
            }
            Text(verbatim: "More")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("Token activity"))
        .accessibilityValue(Text(uncertainTokens > 0
            ? "\(summary). \(detail)"
            : summary))
    }

    /// The five steps `TokenActivityStyle.color(for:maxValue:future:)` paints, as a ladder.
    private static var ladder: [Color] {
        [0, 1, 3, 5, 7].map { TokenActivityStyle.color(for: $0, maxValue: 8, future: false) }
    }

    private static func modeHelp(_ mode: TokenActivityMode) -> String {
        switch mode {
        case .daily: "Each day on its own, against the busiest day of the year"
        case .weekly: "Each week as one shade, against the busiest week"
        case .cumulative: "The running total, so the year fills as it goes"
        }
    }

    private func placedTicks(_ ticks: [(label: String, column: Int)], stride: CGFloat) -> [(label: String, column: Int)] {
        var placed: [(label: String, column: Int)] = []
        var lastX = -TokenActivityStyle.minLabelAdvance
        for tick in ticks {
            let x = CGFloat(tick.column) * stride
            guard x - lastX >= TokenActivityStyle.minLabelAdvance else { continue }
            placed.append(tick)
            lastX = x
        }
        return placed
    }
}

/// The heatmap's cells, drawn in one pass. The cell size follows the width the canvas is
/// given; the height follows from that, so the grid stays square-celled at any width.
///
/// A mode switch blends every cell from the colour it had under the previous mode to its
/// new one, so the grid melts between modes instead of snapping. The blend runs on an
/// animatable modifier (`HeatmapBlendModifier`): `switchIndex` lands at once, `blend`
/// travels to it over the animation, and their difference is how far the melt has come.
private struct HeatmapCanvas: View {
    let grid: TokenActivityGrid
    let previous: TokenActivityGrid?
    var blend: Double
    var switchIndex: Int

    @State private var width: CGFloat = 0

    var body: some View {
        let columns = CGFloat(max(grid.columns.count, 1))
        let rows = CGFloat(TokenActivityGrid.rows)
        let gap = TokenActivityStyle.cellGap
        let cell = max(0, (width - gap * (columns - 1)) / columns)
        Color.clear
            .modifier(HeatmapBlendModifier(grid: grid, previous: previous, blend: blend, base: Double(switchIndex - 1)))
            .frame(maxWidth: .infinity)
            .frame(height: max(1, cell * rows + gap * (rows - 1)))
            .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { width = $0 }
    }
}

/// Draws the grid with each cell mixed `progress` of the way from the previous mode's
/// colour to the current one. Animatable over `blend`, so SwiftUI redraws it every frame
/// of a mode switch with the interpolated value.
private struct HeatmapBlendModifier: ViewModifier, Animatable {
    let grid: TokenActivityGrid
    let previous: TokenActivityGrid?
    var blend: Double
    /// Where the latest switch started; `blend - base` is its progress, 0 to 1.
    let base: Double

    var animatableData: Double {
        get { blend }
        set { blend = newValue }
    }

    func body(content: Content) -> some View {
        let progress = min(max(blend - base, 0), 1)
        let gap = TokenActivityStyle.cellGap
        content.overlay {
            Canvas { context, size in
                let columns = grid.columns.count
                guard columns > 0 else { return }
                let cell = (size.width - gap * CGFloat(columns - 1)) / CGFloat(columns)
                guard cell > 0 else { return }
                let pitch = cell + gap
                for c in 0..<columns {
                    for r in 0..<TokenActivityGrid.rows {
                        guard let date = grid.columns[c][r] else { continue }
                        let future = date >= grid.tomorrow
                        var color = TokenActivityStyle.color(for: grid.values[c][r], maxValue: grid.maxValue, future: future)
                        if let previous, progress < 1, c < previous.values.count {
                            let old = TokenActivityStyle.color(for: previous.values[c][r], maxValue: previous.maxValue, future: future)
                            color = progress <= 0 ? old : old.mix(with: color, by: progress)
                        }
                        let rect = CGRect(x: CGFloat(c) * pitch, y: CGFloat(r) * pitch, width: cell, height: cell)
                        context.fill(
                            Path(roundedRect: rect, cornerRadius: TokenActivityStyle.cellRadius, style: .continuous),
                            with: .color(color)
                        )
                    }
                }
            }
        }
    }
}
