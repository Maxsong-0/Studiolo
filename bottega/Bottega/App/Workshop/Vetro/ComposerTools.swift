// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

// What sits in the box besides the words (Vetro, ADR-047). Three of them wait on LMAS: they are there, say so, and
// do nothing, rather than show made-up numbers.

/// The five ways an agent may be let act. LMAS sets its sandbox itself and takes no mode from Bottega yet.
struct ModePill: View {
    private static let modes: [(name: String, symbol: String, about: String)] = [
        ("Ask", "shield", "Asks before it edits a file or runs a command."),
        ("Accept edits", "pencil.line", "Edits files without asking; asks before commands."),
        ("Plan", "list.bullet", "Reads only. Writes a plan and waits for your go-ahead."),
        ("Jev decides", "sparkle", "Jev answers its asks, and brings you the ones that matter."),
        ("Full access", "bolt", "Does what it needs without asking."),
    ]

    var body: some View {
        Menu {
            Section("LMAS does not take a mode yet. The sandbox is set by LMAS.") {
                ForEach(Self.modes, id: \.name) { mode in
                    Button {} label: { Label(mode.name, systemImage: mode.symbol) }
                        .disabled(true)
                        .help(mode.about)
                }
            }
        } label: {
            Label("Mode", systemImage: "shield")
                .labelStyle(.titleAndIcon)
                .font(.system(size: 12.5, weight: .medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 9)
                .frame(height: 34)
                .contentShape(.capsule)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Permission mode: LMAS does not take one yet")
    }
}

/// How full the agent's context is. Empty until LMAS reports it.
struct ContextPill: View {
    var body: some View {
        HStack(spacing: 5) {
            Circle()
                .strokeBorder(.tertiary, style: StrokeStyle(lineWidth: 2, dash: [1.5, 2]))
                .frame(width: 14, height: 14)
            Text(verbatim: "—")
        }
        .font(.system(size: 12, weight: .medium))
        .foregroundStyle(.secondary)
        .padding(.horizontal, 6)
        .frame(height: 34)
        .help("Context used: LMAS does not report it yet")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Context used: LMAS does not report it yet")
    }
}

/// Dictation. Not in Bottega yet.
struct DictateButton: View {
    var body: some View {
        Button {} label: {
            Image(systemName: "mic")
                .font(.system(size: 14))
                .frame(width: 30, height: 34)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(true)
        .help("Dictation is not in Bottega yet")
        .accessibilityLabel("Dictate: not in Bottega yet")
    }
}

/// In place of Send while the agent is in the middle of a turn and the box is empty. ⌘. does the same.
struct StopTurnButton: View {
    @Environment(WorkshopModel.self) private var model
    let turn: Turn

    var body: some View {
        let canStop = model.canStop(turn)
        Button { model.stop(turn) } label: {
            ZStack {
                Circle().fill(.primary)
                if isStopping {
                    ProgressView().controlSize(.small).tint(Color(nsColor: .windowBackgroundColor))
                } else {
                    RoundedRectangle(cornerRadius: 2.5, style: .continuous)
                        .fill(.background)
                        .frame(width: 11, height: 11)
                }
            }
            .frame(width: 34, height: 34)
            .opacity(canStop && !isStopping ? 1 : 0.3)
            .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .padding(.leading, 4)
        .disabled(!canStop || isStopping || !model.isLive)
        .help(help(canStop))
        .accessibilityLabel(isStopping ? "Stopping this turn" : "Stop this turn")
        .accessibilityHint(help(canStop))
    }

    private var isStopping: Bool { if case .requested = turn.stop { true } else { false } }

    private func help(_ canStop: Bool) -> String {
        if !canStop { return "A card's own turn cannot be stopped from here yet" }
        if isStopping { return "Asked to stop. LMAS says within 15 seconds whether it did." }
        if case .failed = turn.stop { return "It did not stop last time. ⌘. tries again." }
        return "Stop this turn (⌘.). The agent and the line stay."
    }
}
