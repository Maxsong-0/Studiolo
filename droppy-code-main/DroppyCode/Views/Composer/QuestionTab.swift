import SwiftUI

/// The agent's questions as a badge at the end of the conversation, answered in
/// its popover; it leaves with the answer.
struct QuestionBadgeRow: View {
    let request: QuestionRequest
    let runtime: ThreadRuntime
    @State private var isShowingQuestions = false

    var body: some View {
        let count = request.questions.count
        ChatBadge(
            title: count == 1 ? "1 question" : "\(count) questions",
            caption: "to answer",
            needsAttention: true,
            isPresented: $isShowingQuestions,
            glyph: { Image(systemName: "questionmark.bubble").foregroundStyle(.tint) },
            detail: { QuestionSheet(request: request, runtime: runtime) }
        )
    }
}

/// The agent's questions, one per page, answered in the badge's popover. It stays open
/// until the questions are answered or skipped. Past a screenful the page scrolls.
///
/// The popover freezes the sheet at the size it opens with (see `BadgePopoverCoordinator`),
/// so the sheet knows its own height before it opens: the largest page is measured once,
/// here, and nothing in it changes size afterwards; picking, typing and scrolling leave
/// the popover where it is.
struct QuestionSheet: View {
    /// The most the questions take before they scroll.
    private static let maxHeight: CGFloat = 360
    private static let width: CGFloat = 460

    let request: QuestionRequest
    let runtime: ThreadRuntime
    /// The tallest page's natural height at the sheet's width, capped: a scroll view offered
    /// no height collapses, so the popover's measuring pass needs it given outright.
    private let pageHeight: CGFloat

    @State private var selections: [String: [String]]
    @State private var written: [String: String] = [:]
    @State private var page = 0
    /// The popover's own close (see `BadgePopoverCoordinator`): the sheet closes it before
    /// it answers, so the popover leaves on its own animation from the badge, rather than
    /// hanging over the badge's departure and being torn down after it.
    @Environment(\.closePopover) private var closePopover

    init(request: QuestionRequest, runtime: ThreadRuntime) {
        self.request = request
        self.runtime = runtime
        var initial: [String: [String]] = [:]
        for question in request.questions {
            let labels = Self.recommended(question)
            if !labels.isEmpty { initial[question.id] = labels }
        }
        _selections = State(initialValue: initial)
        var largest: CGFloat = 0
        for question in request.questions {
            let probe = QuestionPage(question: question, selections: .constant([:]), written: .constant([:]), onSubmit: {}, onPick: {})
                .frame(width: Self.width)
            let natural = NSHostingView(rootView: probe).fittingSize.height
            largest = max(largest, natural)
        }
        pageHeight = min(max(largest, 120), Self.maxHeight)
    }

    /// Labels mentioning recommended, in order; a single-choice question offers only the first.
    private static func recommended(_ question: QuestionRequest.Question) -> [String] {
        let matches = question.choices.map(\.label).filter { $0.localizedCaseInsensitiveContains("recommended") }
        if matches.isEmpty { return [] }
        if question.allowsMultiple { return matches }
        return [matches[0]]
    }

    var body: some View {
        let count = request.questions.count
        let answers = QuestionAnswers.collect(request, selections: selections, written: written)
        let unanswered = QuestionAnswers.unansweredCount(request, answers: answers)
        let isComplete = unanswered == 0
        VStack(alignment: .leading, spacing: 0) {
            if count > 1 {
                HStack(spacing: 5) {
                    ForEach(Array(request.questions.enumerated()), id: \.element.id) { index, question in
                        let answered = !(answers[question.id] ?? []).isEmpty
                        Button {
                            withAnimation(Chrome.panelSlide) { page = index }
                        } label: {
                            Capsule().fill(index == page ? Chrome.accent : answered ? Chrome.accent.opacity(0.35) : Chrome.overlay(0.18))
                                .frame(width: index == page ? 18 : 6, height: 6)
                        }
                        .buttonStyle(.plain)
                    }
                    Spacer()
                    Text(verbatim: "\(page + 1) of \(count)")
                        .font(.system(size: 11)).monospacedDigit()
                        .foregroundStyle(Chrome.secondaryText)
                }
                .animation(Chrome.panelSlide, value: page)
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .padding(.bottom, 6)
            }

            HStack(spacing: 0) {
                ForEach(Array(request.questions.enumerated()), id: \.element.id) { index, question in
                    ScrollView(.vertical) {
                        QuestionPage(question: question, selections: $selections, written: $written, onSubmit: { advance() }, onPick: { if !question.allowsMultiple, index < count - 1 { advance(after: 0.18) } })
                    }
                    .scrollBounceBehavior(.basedOnSize)
                    .frame(width: Self.width, height: pageHeight)
                }
            }
            .offset(x: -CGFloat(page) * Self.width)
            .frame(width: Self.width, height: pageHeight, alignment: .leading)
            .clipped()
            .animation(Chrome.panelSlide, value: page)

            Divider()

            HStack(spacing: 8) {
                if count > 1 {
                    Button {
                        withAnimation(Chrome.panelSlide) { page = max(0, page - 1) }
                    } label: {
                        Image(systemName: "chevron.left")
                    }
                    .buttonStyle(.bordered)
                    .disabled(page == 0)
                    .help("Previous question")
                }
                if unanswered > 0 {
                    Text(verbatim: unanswered == 1 ? "1 question still to answer" : "\(unanswered) questions still to answer")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer()
                Button("Skip") { send([:]) }
                    .buttonStyle(.bordered)
                    .help("Sends no answers and lets the agent carry on")
                // Return moves on while an answer is still missing; once every question has
                // one (the recommended picks come preselected) Return sends from any page,
                // and Next stays beside it for reading the rest first.
                if page < count - 1, !isComplete {
                    Button("Next") { advance() }
                        .buttonStyle(.borderedProminent)
                        .keyboardShortcut(.defaultAction)
                        .help("Goes to the next question")
                } else {
                    if page < count - 1 {
                        Button("Next") { advance() }
                            .buttonStyle(.bordered)
                            .help("Goes to the next question")
                    }
                    Button("Send") { send(answers) }
                        .buttonStyle(.borderedProminent)
                        .keyboardShortcut(.defaultAction)
                        .disabled(!isComplete)
                        .help(isComplete
                            ? "Sends your answers"
                            : (count == 1 ? "Answer the question to send" : "Answer all \(count) questions to send"))
                }
            }
            .controlSize(.regular)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .frame(width: Self.width)
    }

    private func advance(after delay: Double = 0) {
        let count = request.questions.count
        guard page < count - 1 else {
            let answers = QuestionAnswers.collect(request, selections: selections, written: written)
            if QuestionAnswers.unansweredCount(request, answers: answers) == 0 { send(answers) }
            return
        }
        if delay > 0 {
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(delay))
                withAnimation(Chrome.panelSlide) { page += 1 }
            }
        } else {
            withAnimation(Chrome.panelSlide) { page += 1 }
        }
    }

    /// The popover goes first, on its own animation; the answer follows, and the badge
    /// leaves with it.
    private func send(_ answers: [String: [String]]) {
        closePopover?()
        runtime.answer(request, answers: answers)
    }
}

/// One question: header, prompt, choices, a field for anything else. Its own view so the
/// sheet can measure each page before the popover opens.
private struct QuestionPage: View {
    let question: QuestionRequest.Question
    @Binding var selections: [String: [String]]
    @Binding var written: [String: String]
    /// Return in a field moves on, sending the answers on the last page when complete.
    let onSubmit: () -> Void
    /// A choice just became selected; single choice moves on.
    let onPick: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !question.header.isEmpty {
                Text(verbatim: question.header)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(Chrome.secondaryText)
                    .textCase(.uppercase)
            }
            Text(verbatim: question.prompt)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Chrome.primaryText)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            // Keyed by index, so two identically worded choices both work.
            ForEach(Array(question.choices.enumerated()), id: \.offset) { _, choice in
                choiceRow(question, choice)
            }
            if question.allowsOther || question.choices.isEmpty {
                let binding = Binding(
                    get: { written[question.id] ?? "" },
                    set: { written[question.id] = $0 }
                )
                Group {
                    if question.isSecret {
                        SecureField("Your answer", text: binding)
                    } else {
                        TextField(question.choices.isEmpty ? "Your answer" : "Something else", text: binding)
                    }
                }
                .textFieldStyle(.roundedBorder)
                .controlSize(.regular)
                .onSubmit(onSubmit)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The label without a trailing ' (Recommended)', and whether it was there.
    private static func bareLabel(_ label: String) -> (label: String, isRecommended: Bool) {
        let suffix = " (Recommended)"
        if label.count >= suffix.count, label.suffix(suffix.count).localizedCaseInsensitiveCompare(suffix) == .orderedSame {
            return (String(label.dropLast(suffix.count)), true)
        }
        return (label, label.localizedCaseInsensitiveContains("recommended"))
    }

    private func choiceRow(_ question: QuestionRequest.Question, _ choice: QuestionRequest.Choice) -> some View {
        let isSelected = selections[question.id, default: []].contains(choice.label)
        let bare = Self.bareLabel(choice.label)
        return Button {
            var current = selections[question.id, default: []]
            if question.allowsMultiple {
                if isSelected { current.removeAll { $0 == choice.label } } else { current.append(choice.label) }
            } else {
                current = isSelected ? [] : [choice.label]
            }
            selections[question.id] = current
            if current.contains(choice.label) { onPick() }
        } label: {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: symbol(selected: isSelected, multiple: question.allowsMultiple))
                    .font(.system(size: 14))
                    .foregroundStyle(isSelected ? AnyShapeStyle(Chrome.accent) : AnyShapeStyle(Chrome.secondaryText))
                    .frame(width: 18)
                    .padding(.top, 1)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(verbatim: bare.label)
                            .font(.system(size: 13, weight: isSelected ? .semibold : .regular))
                            .foregroundStyle(Chrome.primaryText)
                        if bare.isRecommended {
                            Text("Recommended")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundStyle(Chrome.accent)
                                .padding(.horizontal, 6).padding(.vertical, 2)
                                .background(Capsule().fill(Chrome.accent.opacity(0.14)))
                        }
                    }
                    if let detail = choice.detail, !detail.isEmpty {
                        Text(verbatim: detail)
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .padding(.horizontal, 12).padding(.vertical, 9)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(isSelected ? Chrome.accent.opacity(0.12) : Chrome.overlay(0.06)))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .animation(Chrome.hover, value: isSelected)
    }

    private func symbol(selected: Bool, multiple: Bool) -> String {
        switch (selected, multiple) {
        case (true, true): "checkmark.square.fill"
        case (false, true): "square"
        case (true, false): "checkmark.circle.fill"
        case (false, false): "circle"
        }
    }
}

/// What the sheet sends: per question, the picked choices and anything written.
private enum QuestionAnswers {
    static func collect(_ request: QuestionRequest, selections: [String: [String]], written: [String: String]) -> [String: [String]] {
        var result: [String: [String]] = [:]
        for question in request.questions {
            var values = selections[question.id, default: []]
            if let text = written[question.id]?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty {
                values.append(text)
            }
            result[question.id] = values
        }
        return result
    }

    static func unansweredCount(_ request: QuestionRequest, answers: [String: [String]]) -> Int {
        request.questions.filter { (answers[$0.id] ?? []).isEmpty }.count
    }
}
