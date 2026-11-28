// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI
import UniformTypeIdentifiers

/// The capsule floating at the bottom of a conversation (Vetro): attach, the words, who it goes to, send.
///
/// On a line with a question to the user it answers that; choosing someone else interjects instead. Return makes a
/// new line, ⌘Return sends; ⌘Return in an empty box opens the ask that has waited longest.
struct WorkshopComposer: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    let key: DraftKey
    @State private var isAttaching = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(model.sending(in: key)) { SendingRow(item: $0) }
            // A recording shows the box as it would be, with nothing to send to: for looking at the interface.
            if canSend || !model[draft: key].isBlank || isRecording {
                box
            } else {
                Text(verbatim: whyNot)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity)
                    .glassBehind(.regular, in: .capsule)
                    .vetroEdge(in: Capsule())
            }
        }
        .frame(maxWidth: 600)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 28)
        .padding(.bottom, 16)
        .padding(.top, 4)
        .fileImporter(isPresented: $isAttaching, allowedContentTypes: [.text, .sourceCode, .json, .log, .data], allowsMultipleSelection: true) { result in
            if case .success(let urls) = result { model.attach(urls, to: key) }
        }
    }

    private var box: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !canSend {
                Text(verbatim: whyNot).font(.callout).foregroundStyle(.secondary).padding(.horizontal, 10)
            }
            // The words get a row of their own; what goes with them sits under them, so nothing is squeezed.
            HStack(alignment: .top, spacing: 2) {
                Button { isAttaching = true } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 15, weight: .medium))
                        .foregroundStyle(.secondary)
                        .frame(width: 34, height: 34)
                        .contentShape(.circle)
                }
                .buttonStyle(.plain)
                .help("Attach text files")
                .accessibilityLabel("Attach text files")
                DraftField(key: key, placeholder: placeholder, isPlain: true)
                    .font(.system(size: 14))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack(spacing: 2) {
                if case .line = key, isInterjecting {
                    @Bindable var model = model
                    Toggle("Needs a reply", isOn: $model[draft: key].wantsReply)
                        .toggleStyle(.button)
                        .buttonStyle(.plain)
                        .font(.system(size: 12, weight: .medium))
                        .lineLimit(1)
                        .fixedSize()
                        .padding(.horizontal, 10)
                        .frame(height: 30)
                        .background(model[draft: key].wantsReply ? look.selection : .clear, in: .capsule)
                        .vetroEdge(in: Capsule())
                }
                RecipientMenu(key: key)
                Spacer(minLength: 4)
                ModePill()
                if let (line, session) = model.modelTarget(of: key), let vendor = model.state.vendors[session] {
                    let uses = model.state.models[session]
                    ModelPill(vendor: vendor, current: uses?.current, pending: uses?.requested, refusal: uses?.refusal?.why) {
                        model.setModel($0, for: session, on: line)
                    }
                    // A model is changed for how things are now: not while LMAS cannot be told.
                    .disabled(!model.isLive)
                    .help(Wording.notLive(model.status))
                }
                ContextPill()
                DictateButton()
                // With nothing typed and nothing to read, Send would do nothing: Stop takes its place while a turn runs.
                if model[draft: key].isBlank, !model.hasAskToRead, let turn = model.runningTurn(for: key) {
                    StopTurnButton(turn: turn)
                } else {
                    SendDraftButton(key: key, title: "Send", isRound: true)
                }
            }
        }
        .padding(EdgeInsets(top: 8, leading: 10, bottom: 8, trailing: 8))
        .glassBehind(.regular, in: .rect(cornerRadius: 25, style: .continuous))
        .vetroEdge(in: RoundedRectangle(cornerRadius: 25, style: .continuous))
    }

    private var canSend: Bool { model.hasLink && model.target(of: key) != nil }

    private var isRecording: Bool { if case .recording = model.source { model.target(of: key) != nil } else { false } }

    private var whyNot: String {
        if case .recording = model.source { return "A recording: nothing can be sent." }
        if !model.isLive { return Wording.notLive(model.status) + "." }
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
            HStack(spacing: 7) {
                if let name = recipient { MemberGlyph(name: name, size: 20) }
                Text(verbatim: label).lineLimit(1)
                Image(systemName: "chevron.up.chevron.down").font(.system(size: 9, weight: .semibold)).foregroundStyle(.secondary)
            }
            .font(.system(size: 12.5, weight: .medium))
            .padding(.leading, recipient == nil ? 12 : 6)
            .padding(.trailing, 9)
            .frame(height: 34)
            .vetroEdge(in: Capsule())
            .contentShape(.capsule)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Who it goes to")
    }

    private var recipient: String? {
        switch model.target(of: key) {
        case .answer(let question): question.from
        case .interject(let name), .start(let name): name
        case nil: nil
        }
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
    /// The round button at the end of the capsule.
    var isRound = false

    var body: some View {
        button
            .keyboardShortcut(hasShortcut ? KeyboardShortcut(.return, modifiers: .command) : nil)
            .disabled(model[draft: key].isBlank ? !model.hasAskToRead : !model.hasLink || model.target(of: key) == nil)
            .help("⌘Return")
    }

    @ViewBuilder private var button: some View {
        if isRound {
            Button { model.send(key) } label: { SendCircle() }
            .buttonStyle(.plain)
            .padding(.leading, 4)
            .accessibilityLabel(title)
        } else {
            Button(title, systemImage: "arrow.up") { model.send(key) }
                .labelStyle(.titleAndIcon)
                .buttonStyle(.glassProminent)
        }
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
    /// Inside the composer, which is the box already. On its own (an answer on the waiting page) it has its own.
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
                .foregroundStyle(.trouble)
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
                .modifier(AnswerBox(isOn: !isPlain, isTypedIn: isFocused) { isFocused = true })
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
        #if DEBUG
        // `BOTTEGA_ANSWER=one\ntwo`: an answer already typed, for a screenshot of the box with several lines in it.
        .onAppear {
            if case .question = key, model[draft: key].text.isEmpty, let typed = ProcessInfo.processInfo.environment["BOTTEGA_ANSWER"] {
                model[draft: key].text = typed.replacingOccurrences(of: "\\n", with: "\n")
            }
        }
        #endif
    }
}

/// The box of a field that is not in the composer: a capsule while the answer is one line, as every other field
/// here is, and a rounded box once it is more.
private struct AnswerBox: ViewModifier {
    let isOn: Bool
    let isTypedIn: Bool
    let typeHere: () -> Void

    func body(content: Content) -> some View {
        if isOn {
            content
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .glassFieldBox(in: RoundedRectangle(cornerRadius: 16, style: .continuous), isTypedIn: isTypedIn, typeHere: typeHere)
        } else {
            content
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
        .vetroSurface(cornerRadius: 10)
    }
}

/// Dark on light glass, light on dark; faint while there is nothing to send.
private struct SendCircle: View {
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        Image(systemName: "arrow.up")
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(.background)
            .frame(width: 34, height: 34)
            .background(.primary, in: .circle)
            .opacity(isEnabled ? 1 : 0.3)
            .contentShape(.circle)
    }
}
