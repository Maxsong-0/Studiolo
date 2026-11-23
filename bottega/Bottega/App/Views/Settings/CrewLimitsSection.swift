// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

// Settings › Crew › Limits: the numbers the team runs to (`CrewLimits`). The first card
// bounds what a lead sends out, the second what a hand Bottega runs may take in one
// turn, the third when the watchdog calls such a hand thinking or stalled. Every choice
// writes straight to `AppSettings.crewLimits`.

/// The Limits section of the Crew page, with a way back to the defaults in its header.
struct CrewLimitsSection: View {
    var body: some View {
        ChromeSection(title: "Limits", trailing: AnyView(CrewLimitsRestoreButton())) {
            CrewLimitsDelegationCard()
            CrewLimitsBudgetCard()
            CrewLimitsWatchdogCard()
            Text("These cover the hands Bottega runs as threads of their own; Claude, Codex and Copilot keep the hands they run themselves to their own limits. The nudge, the tool counts and the word to wrap up reach hands on DeepSeek, Meta and Z.ai, which Bottega paces from inside their turn; every other hand is stopped by the clock.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .padding(.horizontal, 4)
        }
    }
}

/// Puts every limit back to its default; off while they already are.
private struct CrewLimitsRestoreButton: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Button {
            model.settings.crewLimits = .default
        } label: {
            Label("Restore defaults", systemImage: "arrow.counterclockwise")
        }
        .buttonStyle(.glass)
        .controlSize(.small)
        .disabled(model.settings.crewLimits == .default)
    }
}

/// What a lead may send out: rounds per request, tasks per block, hands at once with no
/// cap, and how often a block that sent no one is answered.
private struct CrewLimitsDelegationCard: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        let limits = settings.crewLimits
        ChromeCard {
            CrewLimitRow(
                title: "Rounds per request",
                detail: "How many times the lead may send hands out for one message of yours. Past that it finishes the work itself.",
                options: CrewLimitChoices.counts([1, 2, 3, 4, 6, 8, 10], current: limits.maxDelegationRounds, one: "round", many: "rounds"),
                selection: $settings.crewLimits.maxDelegationRounds
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Tasks per block",
                detail: "The most tasks one crew block sends out. The lead hears which ones did not go.",
                options: CrewLimitChoices.counts([4, 8, 16, 32, 64], current: limits.maxDelegatedTasks, one: "task", many: "tasks"),
                selection: $settings.crewLimits.maxDelegatedTasks
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Hands at once with no cap",
                detail: "When neither Hands at once nor the chat's pair sets a cap, this many go out at a time and the rest wait their turn.",
                options: CrewLimitChoices.counts(Array(CrewPair.maxHandsRange), current: limits.uncappedConcurrency, one: "hand", many: "hands"),
                selection: $settings.crewLimits.uncappedConcurrency
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Refused blocks per request",
                detail: "How many times the lead hears that its block was unreadable or held back before such a block is simply dropped.",
                options: CrewLimitChoices.counts([1, 2, 3, 5], current: limits.maxRefusedBlocks, one: "time", many: "times"),
                selection: $settings.crewLimits.maxRefusedBlocks
            )
        }
    }
}

/// What a hand may take in one turn: a nudge when it only reads, a word to wrap up and a
/// stop, by tools and by the clock, and the time it gets for its report.
private struct CrewLimitsBudgetCard: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        let limits = settings.crewLimits
        ChromeCard {
            CrewLimitRow(
                title: "Nudge to act after",
                detail: "Tools in a row without a change before a hand is told to act or report.",
                options: CrewLimitChoices.counts([12, 24, 48, 96], current: limits.pacingTools, one: "tool", many: "tools", off: "Never"),
                selection: $settings.crewLimits.pacingTools
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Tools before wrap-up",
                detail: "A hand this far in is told its budget is nearly spent and to finish.",
                options: CrewLimitChoices.counts([60, 120, 240, 480], current: limits.wrapUpTools, one: "tool", many: "tools", off: "No limit"),
                selection: $settings.crewLimits.wrapUpTools
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Tools before stop",
                detail: "Its tools go and it writes its report.",
                options: CrewLimitChoices.counts([80, 160, 320, 600, 1_000], current: limits.maxTools, one: "tool", many: "tools", off: "No limit"),
                selection: $settings.crewLimits.maxTools
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Time before wrap-up",
                detail: "The same word to finish, by the clock.",
                options: CrewLimitChoices.durations([15, 25, 45, 60, 90, 100, 120, 180, 240], current: limits.wrapUpSeconds, off: "No limit"),
                selection: $settings.crewLimits.wrapUpSeconds
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Time before stop",
                detail: "Whichever stop comes first ends the work. A model thinking hard takes minutes between tools.",
                options: CrewLimitChoices.durations([20, 35, 60, 90, 120, 180, 240, 480], current: limits.maxSeconds, off: "No limit"),
                selection: $settings.crewLimits.maxSeconds
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Time to report",
                detail: "What a stopped hand gets to write its report in.",
                options: CrewLimitChoices.durations([1, 2, 3, 5, 10], current: limits.reportSeconds),
                selection: $settings.crewLimits.reportSeconds
            )
        }
    }
}

/// When the watchdog calls a quiet hand thinking, and when stalled.
private struct CrewLimitsWatchdogCard: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        let limits = settings.crewLimits
        ChromeCard {
            CrewLimitRow(
                title: "Thinking after",
                detail: "A hand quiet this long, with no tool and no word, says it is thinking in its row.",
                options: CrewLimitChoices.durations([1, 2, 3, 5, 10], current: limits.watchdogThinkingAfter),
                selection: $settings.crewLimits.watchdogThinkingAfter
            )
            ChromeRowDivider()
            CrewLimitRow(
                title: "Stalled after",
                detail: "A hand quiet this long with no new tool is stopped if it waits on an approval or a question nobody answers, or has written no file this turn. Its report tells the lead why.",
                options: CrewLimitChoices.durations([10, 15, 20, 30, 60, 120], current: limits.watchdogStalledAfter, off: "Never"),
                selection: $settings.crewLimits.watchdogStalledAfter
            )
        }
    }
}

/// One limit: its words, and a capsule that opens its choices.
private struct CrewLimitRow<Value: Hashable>: View {
    let title: String
    let detail: String
    let options: [(value: Value, title: String)]
    @Binding var selection: Value

    var body: some View {
        ChromeRow(title: title, detail: detail) {
            GlassPickerButton(options: options, selection: $selection)
        }
    }
}

/// The choices a limit's capsule offers: the presets, and the value in force when it is
/// none of them (a save from an older build, say), so the capsule always names it.
private enum CrewLimitChoices {
    static func counts(_ presets: [Int], current: Int, one: String, many: String) -> [(value: Int, title: String)] {
        values(presets, current: current).map { (value: $0, title: countTitle($0, one: one, many: many)) }
    }

    /// Counts that can be switched off: `off` first, then the numbers.
    static func counts(_ presets: [Int], current: Int?, one: String, many: String, off: String) -> [(value: Int?, title: String)] {
        let numbers: [(value: Int?, title: String)] = values(presets, current: current).map { (value: Optional($0), title: countTitle($0, one: one, many: many)) }
        return [(value: nil, title: off)] + numbers
    }

    /// Durations offered in minutes and kept in seconds.
    static func durations(_ minutes: [Int], current: TimeInterval) -> [(value: TimeInterval, title: String)] {
        values(minutes.map { TimeInterval($0 * 60) }, current: current).map { (value: $0, title: durationTitle($0)) }
    }

    /// Durations that can be switched off: `off` first, then the spans.
    static func durations(_ minutes: [Int], current: TimeInterval?, off: String) -> [(value: TimeInterval?, title: String)] {
        let spans: [(value: TimeInterval?, title: String)] = values(minutes.map { TimeInterval($0 * 60) }, current: current).map { (value: Optional($0), title: durationTitle($0)) }
        return [(value: nil, title: off)] + spans
    }

    private static func values<Value: Comparable>(_ presets: [Value], current: Value?) -> [Value] {
        guard let current, !presets.contains(current) else { return presets }
        return (presets + [current]).sorted()
    }

    private static func countTitle(_ value: Int, one: String, many: String) -> String {
        "\(value) \(value == 1 ? one : many)"
    }

    /// "45 min", "1 h", "1 h 30 min".
    private static func durationTitle(_ seconds: TimeInterval) -> String {
        let minutes = Int((seconds / 60).rounded())
        guard minutes >= 60 else { return "\(minutes) min" }
        let rest = minutes % 60
        return rest == 0 ? "\(minutes / 60) h" : "\(minutes / 60) h \(rest) min"
    }
}
