// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// One line the reader wrote for themselves on the task page. Kept for the app rather
/// than a thread, and never written by an agent: it is a list of what to do next, not a
/// record of what one chat did.
struct UserTask: Codable, Hashable, Identifiable {
    var id = UUID()
    var text: String
    var isDone = false
}

/// The task page of the right sidebar, in two halves. Above: what the agent is working
/// through on this thread — the checklist it keeps and the plan it proposed, live as they
/// stream, read-only because they are its own. Below: the reader's list, typed here and
/// kept across launches.
struct TaskPane: View {
    @Environment(AppModel.self) private var model
    /// The window's text size: the symbols' column grows with the type beside it.
    @Environment(\.chatZoom) private var zoom
    let runtime: ThreadRuntime

    @State private var draft = ""
    @State private var showsPlan = false

    var body: some View {
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 16) {
                agentHalf
                Rectangle()
                    .fill(Chrome.overlay(0.1))
                    .frame(height: 1)
                readerHalf
            }
            .padding(.horizontal, Chrome.listInset + 2)
            .padding(.bottom, 16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: - The agent's half

    @ViewBuilder
    private var agentHalf: some View {
        let steps = Self.latestTodos(of: runtime)
        let plan = Self.latestPlan(of: runtime)
        VStack(alignment: .leading, spacing: 8) {
            TaskSectionHeader(title: "On this thread", detail: Self.progress(of: steps))
            if steps.isEmpty, plan == nil {
                Text("The agent has not written a checklist for this thread yet.")
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText.opacity(0.8))
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(Array(steps.enumerated()), id: \.offset) { _, step in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: Self.symbol(for: step.status))
                        .zoomedFont(size: 11, weight: .semibold)
                        .foregroundStyle(Self.tint(for: step.status))
                        .frame(width: ChatZoom.scaled(14, by: zoom))
                    Text(verbatim: step.text)
                        .zoomedFont(size: 12)
                        .foregroundStyle(step.status == .done ? Chrome.secondaryText : Chrome.primaryText)
                        .strikethrough(step.status == .done, color: Chrome.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
            }
            if let plan {
                Button {
                    withAnimation(.snappy(duration: 0.22)) { showsPlan.toggle() }
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "list.bullet.rectangle")
                            .zoomedFont(size: 11, weight: .semibold)
                            .frame(width: ChatZoom.scaled(14, by: zoom))
                        Text(verbatim: Self.planTitle(plan.state))
                            .zoomedFont(size: 12, weight: .medium)
                        Image(systemName: "chevron.right")
                            .chromeChevronFont()
                            .rotationEffect(.degrees(showsPlan ? 90 : 0))
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(Chrome.secondaryText)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                if showsPlan {
                    MarkdownView(text: plan.markdown).equatable()
                        .environment(\.markdownPointSize, 12)
                        .padding(.leading, 22)
                        .transition(.opacity)
                }
            }
        }
    }

    // MARK: - The reader's half

    @ViewBuilder
    private var readerHalf: some View {
        let tasks = model.settings.userTasks
        let done = tasks.count { $0.isDone }
        VStack(alignment: .leading, spacing: 8) {
            TaskSectionHeader(
                title: "My list",
                detail: tasks.isEmpty ? nil : "\(done)/\(tasks.count)",
                trailing: done > 0 ? ("Clear done", { model.settings.userTasks.removeAll(where: \.isDone) }) : nil
            )
            TextField("Add a task", text: $draft)
                .textFieldStyle(.plain)
                .zoomedFont(size: 12)
                .padding(.horizontal, 10)
                .frame(height: Chrome.capsuleContentHeight)
                .background {
                    Capsule(style: .continuous)
                        .fill(Chrome.overlay(0.08))
                }
                .onSubmit(add)
            ForEach(tasks) { task in
                UserTaskRow(
                    task: task,
                    toggle: {
                        guard let index = model.settings.userTasks.firstIndex(where: { $0.id == task.id }) else { return }
                        model.settings.userTasks[index].isDone.toggle()
                    },
                    remove: {
                        withAnimation(.snappy(duration: 0.2)) {
                            model.settings.userTasks.removeAll { $0.id == task.id }
                        }
                    }
                )
            }
            if tasks.isEmpty {
                Text("Nothing here yet. Type above and press return.")
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText.opacity(0.8))
            }
        }
    }

    private func add() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        withAnimation(.snappy(duration: 0.2)) {
            model.settings.userTasks.append(UserTask(text: text))
        }
        draft = ""
    }

    // MARK: - Reading the thread

    /// The checklist the agent last wrote on this thread. Walked back from the end, so a
    /// streaming thread is never scanned from the top.
    @MainActor
    private static func latestTodos(of runtime: ThreadRuntime) -> [TodoStep] {
        for entry in runtime.entries.reversed() {
            guard entry.kind == .todos, case .todos(let steps) = entry.item.content, !steps.isEmpty else { continue }
            return steps
        }
        return []
    }

    /// The plan the agent last proposed, unless it was dismissed.
    @MainActor
    private static func latestPlan(of runtime: ThreadRuntime) -> ProposedPlan? {
        for entry in runtime.entries.reversed() {
            guard entry.kind == .plan, case .plan(let plan) = entry.item.content else { continue }
            return plan.state == .dismissed ? nil : plan
        }
        return nil
    }

    private static func progress(of steps: [TodoStep]) -> String? {
        guard !steps.isEmpty else { return nil }
        return "\(steps.count { $0.status == .done })/\(steps.count)"
    }

    private static func symbol(for status: TodoStep.Status) -> String {
        switch status {
        case .pending: "circle"
        case .active: "circle.dotted.circle"
        case .done: "checkmark.circle.fill"
        }
    }

    private static func tint(for status: TodoStep.Status) -> Color {
        switch status {
        case .pending: Chrome.secondaryText.opacity(0.7)
        case .active: Chrome.accent
        case .done: Chrome.success
        }
    }

    private static func planTitle(_ state: ProposedPlan.State) -> String {
        switch state {
        case .drafting: "Plan, being written"
        case .proposed: "Plan, waiting on you"
        case .accepted: "Plan, accepted"
        case .dismissed: "Plan"
        }
    }
}

/// A half's heading: its name, a count, and the one action it offers.
private struct TaskSectionHeader: View {
    let title: String
    var detail: String? = nil
    var trailing: (String, () -> Void)? = nil

    var body: some View {
        HStack(spacing: 6) {
            Text(verbatim: title)
                .zoomedFont(size: 11, weight: .semibold)
                .foregroundStyle(Chrome.secondaryText)
            if let detail {
                Text(verbatim: detail)
                    .zoomedFont(size: 11, monospacedDigit: true)
                    .foregroundStyle(Chrome.secondaryText.opacity(0.7))
            }
            Spacer(minLength: 0)
            if let trailing {
                Button(trailing.0, action: trailing.1)
                    .buttonStyle(.plain)
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
    }
}

/// One line of the reader's own list: a box to tick, the words, and a cross that appears
/// under the pointer.
private struct UserTaskRow: View {
    let task: UserTask
    let toggle: () -> Void
    let remove: () -> Void

    @Environment(\.chatZoom) private var zoom
    @State private var isHovering = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Button(action: toggle) {
                Image(systemName: task.isDone ? "checkmark.circle.fill" : "circle")
                    .zoomedFont(size: 12, weight: .semibold)
                    .foregroundStyle(task.isDone ? Chrome.success : Chrome.secondaryText.opacity(0.7))
                    .frame(width: ChatZoom.scaled(14, by: zoom))
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            Text(verbatim: task.text)
                .zoomedFont(size: 12)
                .foregroundStyle(task.isDone ? Chrome.secondaryText : Chrome.primaryText)
                .strikethrough(task.isDone, color: Chrome.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 4)
            if isHovering {
                Button(action: remove) {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(Chrome.secondaryText)
                        .frame(width: 16, height: 16)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Remove")
                .transition(.opacity)
            }
        }
        .padding(.vertical, 1)
        .contentShape(.rect)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
    }
}
