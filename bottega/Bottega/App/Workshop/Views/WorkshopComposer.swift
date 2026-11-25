// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI
import UniformTypeIdentifiers

/// The floating box at the bottom of a conversation.
///
/// On a line with a question to the user it answers that; choosing someone else interjects instead. Return makes a
/// new line, ⌘Return sends; ⌘Return in an empty box opens the ask that has waited longest.
struct WorkshopComposer: View {
    @Environment(WorkshopModel.self) private var model
    let key: DraftKey
    @State private var isAttaching = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(model.sending(in: key)) { SendingRow(item: $0) }
            if canSend || !model[draft: key].isBlank {
                box
            } else {
                Text(verbatim: whyNot)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity)
                    .glassEffect(.regular, in: .capsule)
            }
        }
        .frame(maxWidth: 780)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 20)
        .padding(.bottom, 16)
        .padding(.top, 4)
        .fileImporter(isPresented: $isAttaching, allowedContentTypes: [.text, .sourceCode, .json, .log, .data], allowsMultipleSelection: true) { result in
            if case .success(let urls) = result { model.attach(urls, to: key) }
        }
    }

    private var box: some View {
        VStack(alignment: .leading, spacing: 6) {
            DraftField(key: key, placeholder: placeholder, isPlain: true)
            if !canSend {
                Text(verbatim: whyNot).font(.callout).foregroundStyle(.secondary)
            }
            HStack(spacing: 10) {
                RecipientMenu(key: key)
                if case .line = key, isInterjecting {
                    @Bindable var model = model
                    Toggle("Needs a reply", isOn: $model[draft: key].wantsReply).toggleStyle(.checkbox).controlSize(.small)
                }
                Spacer()
                Button("Attach Text Files", systemImage: "paperclip") { isAttaching = true }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                SendDraftButton(key: key, title: "Send")
            }
        }
        .padding(12)
        .glassEffect(.regular, in: .rect(cornerRadius: 20))
    }

    private var canSend: Bool { model.hasLink && model.target(of: key) != nil }

    private var whyNot: String {
        if case .recording = model.source { return "A recording: nothing can be sent." }
        if !model.isLive { return "Not connected to LMAS." }
        return switch key {
        case .dispatcher: "Nobody is here to talk to yet."
        case .line: "Everyone on this line has left."
        case .question: "This question is closed."
        }
    }

    private var isInterjecting: Bool {
        if case .interject = model.target(of: key) { true } else { false }
    }

    private var placeholder: String {
        switch model.target(of: key) {
        case .answer(let question): "Answer \(UntrustedText.line(question.from, limit: 40))"
        case .interject(let name): "Interject to \(UntrustedText.line(name, limit: 40))"
        case .start(let name): "Say something to \(UntrustedText.line(name, limit: 40))"
        case nil: ""
        }
    }
}

/// Who it goes to, and the choice to interject instead of answering.
private struct RecipientMenu: View {
    @Environment(WorkshopModel.self) private var model
    let key: DraftKey

    var body: some View {
        Menu {
            if let question = model.state.questionHere(key) {
                Button("Answer \(UntrustedText.line(question.from, limit: 40))") {
                    model[draft: key].interjects = false
                    model[draft: key].recipient = nil
                }
                Divider()
            }
            ForEach(model.state.recipients(for: key)) { session in
                Button(prefix + UntrustedText.line(session.name, limit: 40)) {
                    model[draft: key].interjects = true
                    model[draft: key].recipient = session.name
                }
            }
        } label: {
            Text(verbatim: label)
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .font(.callout)
    }

    private var prefix: String {
        if case .line = key { "Interject to " } else { "" }
    }

    private var label: String {
        switch model.target(of: key) {
        case .answer(let question): "Answers \(UntrustedText.line(question.from, limit: 40))"
        case .interject(let name), .start(let name): "To \(UntrustedText.line(name, limit: 40))"
        case nil: "To nobody"
        }
    }
}

struct SendDraftButton: View {
    @Environment(WorkshopModel.self) private var model
    let key: DraftKey
    let title: String

    var body: some View {
        Button(title, systemImage: "arrow.up") { model.send(key) }
            .labelStyle(.titleAndIcon)
            .buttonStyle(.glassProminent)
            .keyboardShortcut(hasShortcut ? KeyboardShortcut(.return, modifiers: .command) : nil)
            .disabled(model[draft: key].isBlank ? !model.hasAskToRead : !model.hasLink || model.target(of: key) == nil)
            .help("⌘Return")
    }

    /// Several questions can be listed at once; only the one being answered takes ⌘Return.
    private var hasShortcut: Bool {
        if case .question = key { model.focusedDraft == key } else { true }
    }
}

/// The text, its files, and why it came back if it did.
struct DraftField: View {
    @Environment(WorkshopModel.self) private var model
    let key: DraftKey
    let placeholder: String
    var isPlain = false
    @FocusState private var isFocused: Bool

    var body: some View {
        @Bindable var model = model
        let draft = model[draft: key]
        VStack(alignment: .leading, spacing: 6) {
            if let problem = draft.problem {
                Label {
                    Text(verbatim: UntrustedText.line(problem.text, limit: 400))
                } icon: {
                    Image(systemName: "arrow.uturn.backward.circle")
                }
                .font(.callout)
                .foregroundStyle(.orange)
            }
            TextField("", text: $model[draft: key].text, prompt: Text(verbatim: placeholder), axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...10)
                .modifier(ReturnMakesALine())
                .focused($isFocused)
                .onChange(of: isFocused) {
                    if isFocused {
                        model.focusedDraft = key
                    } else if model.focusedDraft == key {
                        model.focusedDraft = nil
                    }
                }
                .padding(isPlain ? 0 : 8)
                .background {
                    if !isPlain { RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.5)) }
                }
            ForEach(Array(draft.files.enumerated()), id: \.offset) { index, file in
                HStack(spacing: 6) {
                    Image(systemName: "doc.text")
                    Text(verbatim: UntrustedText.line(file.name, limit: 80))
                    Button("Remove", systemImage: "xmark.circle.fill") { model[draft: key].files.remove(at: index) }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                }
                .font(.callout)
            }
        }
    }
}

/// A text field ends editing on Return; here it starts a new line, where the cursor is. (Return that an input method
/// is using to take a candidate never gets this far.)
private struct ReturnMakesALine: ViewModifier {
    func body(content: Content) -> some View {
        content.onKeyPress(keys: [.return]) { press in
            guard press.modifiers.isEmpty || press.modifiers == .shift else { return .ignored }
            guard let editor = NSApp.keyWindow?.firstResponder as? NSTextView, !editor.hasMarkedText() else { return .ignored }
            editor.insertNewlineIgnoringFieldEditor(nil)
            return .handled
        }
    }
}

/// Words on their way.
private struct SendingRow: View {
    @Environment(WorkshopModel.self) private var model
    let item: Outbox.Item

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "clock")
            Text(verbatim: UntrustedText.line(item.draft.typed?.text ?? "", limit: 120)).lineLimit(1)
            Spacer()
            if item.signed == nil {
                Button("Take Back") { model.withdraw(item.id) }.controlSize(.small)
            }
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(.background.secondary, in: .rect(cornerRadius: 10))
    }
}
