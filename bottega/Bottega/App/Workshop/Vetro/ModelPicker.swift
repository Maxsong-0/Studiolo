// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// The model in the box (Vetro, ADR-047): what the agent uses, and what it will use from its next turn on. Opens the
/// picker.
struct ModelPill: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    let vendor: String
    /// In use, as far as LMAS has said.
    let current: ModelChoice?
    /// Asked for and not in use yet.
    var pending: ModelChoice?
    /// The last change asked for that did not come about.
    var refusal: Why?
    var footnote = "Takes effect on the next turn."
    /// Another maker's tile in the rail: nil when it cannot be switched to here.
    var switchVendor: ((String) -> Void)?
    let pick: (ModelChoice) -> Void
    @State private var isOpen = Self.opensOnce()

    #if DEBUG
    /// `BOTTEGA_OPEN_PICKER=1`: the first pill shown opens its picker, for a screenshot.
    @MainActor private static var isWaitingToOpen = ProcessInfo.processInfo.environment["BOTTEGA_OPEN_PICKER"] != nil
    @MainActor private static func opensOnce() -> Bool {
        defer { isWaitingToOpen = false }
        return isWaitingToOpen
    }
    #else
    private static func opensOnce() -> Bool { false }
    #endif

    var body: some View {
        Button { isOpen.toggle() } label: {
            HStack(spacing: 6) {
                if let maker = Maker(vendor: vendor) { MakerTile(maker: maker, size: 16) }
                Text(verbatim: label).lineLimit(1)
                if pending != nil {
                    Text("next turn")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(look.selection, in: .capsule)
                }
                if refusal != nil {
                    Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange)
                }
                Image(systemName: "chevron.up.chevron.down").font(.system(size: 9, weight: .semibold)).foregroundStyle(.secondary)
            }
            .font(.system(size: 12.5, weight: .medium))
            .padding(.leading, 7)
            .padding(.trailing, 9)
            .frame(height: 34)
            .overlay { Capsule().strokeBorder(.separator) }
            .contentShape(.capsule)
        }
        .buttonStyle(.plain)
        .fixedSize()
        .help(help)
        .accessibilityLabel("Model: \(label)")
        .accessibilityHint(help)
        .popover(isPresented: $isOpen, arrowEdge: .top) {
            ModelPicker(
                vendor: vendor, selection: pending ?? current, footnote: footnote, switchVendor: switchVendor.map { go in
                    { vendor in
                        isOpen = false
                        go(vendor)
                    }
                }, pick: pick)
        }
    }

    private var shown: ModelChoice? { pending ?? current }

    private var label: String {
        guard let shown else { return "Default model" }
        let name = model.state.catalogs[vendor]?.listed?.models.first { $0.model == shown.model }?.name ?? shown.model
        return UntrustedText.line(name, limit: 40) + (shown.effort.map { " · " + Wording.effort($0) } ?? "")
    }

    private var help: String {
        if let refusal { return "Not changed: \(UntrustedText.line(refusal.text, limit: 200))" }
        if let pending, let current, pending != current {
            return "Uses \(Wording.model(current.model, current.effort)) until this turn is over"
        }
        return "Model and reasoning"
    }
}

/// Three columns, as in the design: the makers, the models of the one in use, and how hard it reasons.
struct ModelPicker: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    let vendor: String
    let selection: ModelChoice?
    let footnote: String
    let switchVendor: ((String) -> Void)?
    let pick: (ModelChoice) -> Void

    @State private var query = ""
    /// The model whose levels are on the right: the one in use, or the one last clicked.
    @State private var focus: String?

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            rail
            Divider()
            list.frame(width: 260)
            Divider()
            side.frame(width: 190)
        }
        .frame(height: 320)
        .onAppear { focus = selection?.model ?? catalog?.defaultModel }
    }

    private var catalog: Event.ModelsListed? { model.state.catalogs[vendor]?.listed }

    // MARK: Makers

    private var rail: some View {
        VStack(spacing: 6) {
            ForEach(vendors, id: \.self) { other in
                let isThis = other == vendor
                Button { switchVendor?(other) } label: {
                    Group {
                        if let maker = Maker(vendor: other) {
                            MakerTile(maker: maker, size: 22)
                        } else {
                            Text(verbatim: String(other.prefix(1)).uppercased()).font(.system(size: 12, weight: .semibold))
                        }
                    }
                    .frame(width: 34, height: 34)
                    .background(isThis ? look.selection : .clear, in: .rect(cornerRadius: 9, style: .continuous))
                    .opacity(isThis || switchVendor != nil ? 1 : 0.4)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .disabled(isThis || switchVendor == nil)
                .help(isThis ? (Maker(vendor: other)?.title ?? other)
                    : switchVendor == nil ? "Another maker takes a line of its own: open one from its home" : "Go to \(Maker(vendor: other)?.title ?? other)")
                .accessibilityLabel(Maker(vendor: other)?.title ?? other)
                .accessibilityAddTraits(isThis ? .isSelected : [])
            }
            Spacer(minLength: 0)
        }
        .padding(8)
    }

    /// The makers LMAS has asked, in the design's order.
    private var vendors: [String] {
        let known = Maker.allCases.map(\.vendor).filter { model.state.catalogs[$0] != nil || $0 == vendor }
        let others = model.state.catalogs.keys.filter { Maker(vendor: $0) == nil }.sorted()
        return known + others
    }

    // MARK: Models

    private var list: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search models", text: $query).textFieldStyle(.plain)
            }
            .font(.system(size: 12.5))
            .padding(.horizontal, 12)
            .frame(height: 34)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 1) {
                    switch model.state.catalogs[vendor] {
                    case .listed(let list):
                        let shown = list.models.filter(matches)
                        ForEach(shown) { row($0, list: list) }
                        if shown.isEmpty {
                            Text("No model matches.").font(.system(size: 12)).foregroundStyle(.secondary).padding(10)
                        }
                    case .unavailable(let why, _):
                        VStack(alignment: .leading, spacing: 4) {
                            Text(verbatim: "\(Maker(vendor: vendor)?.title ?? UntrustedText.line(vendor, limit: 40)) cannot be asked for its models")
                                .font(.system(size: 12, weight: .semibold))
                            Text(verbatim: UntrustedText.line(why.text, limit: 200))
                            Text("LMAS asks again in 10 minutes. The other bots are not affected.")
                        }
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                        .padding(10)
                    case nil:
                        Text("LMAS has not said what this maker offers yet.")
                            .font(.system(size: 12)).foregroundStyle(.secondary).padding(10)
                    }
                }
                .padding(6)
            }
            Divider()
            HStack(spacing: 6) {
                Text(verbatim: footnote)
                Spacer(minLength: 0)
                NotInLMASYet(text: "Usage: not from LMAS yet").fixedSize()
            }
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 10)
            .frame(height: 30)
        }
    }

    private func matches(_ item: Event.ModelsListed.Model) -> Bool {
        let words = query.trimmingCharacters(in: .whitespaces)
        return words.isEmpty || item.name.localizedCaseInsensitiveContains(words) || item.model.localizedCaseInsensitiveContains(words)
    }

    private func row(_ item: Event.ModelsListed.Model, list: Event.ModelsListed) -> some View {
        let isUsed = selection?.model == item.model
        let isFocused = focus == item.model
        return Button {
            focus = item.model
            // A model whose level cannot be chosen is used as it is; otherwise the level picks it.
            if item.efforts?.isEmpty ?? true { pick(ModelChoice(model: item.model)) }
        } label: {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: UntrustedText.line(item.name, limit: 60)).font(.system(size: 12.5)).lineLimit(1)
                    if let about = item.about {
                        Text(verbatim: UntrustedText.line(about, limit: 120)).font(.system(size: 10.5)).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
                if list.defaultModel == item.model {
                    Text("Default").font(.system(size: 10.5)).foregroundStyle(.secondary)
                }
                if isUsed { Image(systemName: "checkmark").font(.system(size: 11, weight: .semibold)) }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(isFocused ? look.selection : .clear, in: .rect(cornerRadius: 7, style: .continuous))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isUsed ? .isSelected : [])
    }

    // MARK: Reasoning

    private var side: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text("Reasoning").font(.system(size: 12, weight: .semibold))
                Spacer(minLength: 0)
                if let effort = selection?.effort, selection?.model == focus {
                    Text(verbatim: Wording.effort(effort)).font(.system(size: 11)).foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 6)
            .frame(height: 34)
            if let focused {
                if let efforts = focused.efforts, !efforts.isEmpty {
                    ForEach(efforts, id: \.self) { effort in
                        let isUsed = selection?.model == focused.model && selection?.effort == effort
                        Button { pick(ModelChoice(model: focused.model, effort: effort)) } label: {
                            HStack {
                                Text(verbatim: Wording.effort(effort)).font(.system(size: 12.5))
                                Spacer(minLength: 0)
                                if focused.defaultEffort == effort {
                                    Text("Default").font(.system(size: 10.5)).foregroundStyle(.secondary)
                                }
                                Image(systemName: "checkmark").font(.system(size: 11, weight: .semibold)).opacity(isUsed ? 1 : 0)
                            }
                            .padding(.horizontal, 6)
                            .frame(height: 28)
                            .background(isUsed ? look.selection : .clear, in: .rect(cornerRadius: 7, style: .continuous))
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(isUsed ? .isSelected : [])
                    }
                    if selection?.model != focused.model {
                        Text("Pick a level to use \(UntrustedText.line(focused.name, limit: 40)).")
                            .font(.system(size: 11)).foregroundStyle(.secondary).padding(6)
                    }
                } else {
                    Text("This model's reasoning is not chosen here.").font(.system(size: 11)).foregroundStyle(.secondary).padding(6)
                }
            }
            Spacer(minLength: 0)
            Divider().padding(.vertical, 6)
            HStack {
                Text("Sandbox").foregroundStyle(.secondary)
                Spacer(minLength: 0)
                Text("Set by LMAS")
            }
            .font(.system(size: 11.5))
            .padding(.horizontal, 6)
        }
        .padding(8)
    }

    private var focused: Event.ModelsListed.Model? {
        catalog?.models.first { $0.model == focus }
    }
}
