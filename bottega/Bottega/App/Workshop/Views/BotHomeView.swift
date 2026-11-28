// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// A bot's home (Vetro, ADR-047): open a line straight to it (LMAS `open_line`), and see the lines it already has.
struct BotHomeView: View {
    @Environment(WorkshopModel.self) private var model
    let vendor: String

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                StartLineCard(vendor: vendor)
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 12, alignment: .top), GridItem(.flexible(), alignment: .top)], spacing: 12) {
                    BotLines(vendor: vendor)
                    BotModels(vendor: vendor)
                    BotQuota(vendor: vendor)
                }
            }
            .frame(maxWidth: 640, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .scrollEdgeEffectStyle(.soft, for: .top)
    }

    private var header: some View {
        HStack(spacing: 12) {
            if let maker = Maker(vendor: vendor) { MakerTile(maker: maker, size: 40) }
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: Maker(vendor: vendor)?.title ?? UntrustedText.line(vendor, limit: 40))
                    .font(.system(size: 20, weight: .semibold))
                let roles = Wording.roles(model.state.catalogs[vendor]?.listed?.roles ?? [])
                if !roles.isEmpty { Text(verbatim: roles).font(.system(size: 12)).foregroundStyle(.secondary) }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// The box at the top: where the line's copy comes from, what the bot is on it, its model, and the first thing said.
private struct StartLineCard: View {
    @Environment(WorkshopModel.self) private var model
    let vendor: String
    @FocusState private var isFocused: Bool

    var body: some View {
        @Bindable var model = model
        let draft = model[lineDraft: vendor]
        let name = Maker(vendor: vendor)?.title ?? UntrustedText.line(vendor, limit: 40)
        VStack(alignment: .leading, spacing: 10) {
            if let problem = draft.problem {
                Label {
                    Text(verbatim: UntrustedText.line(problem.text, limit: 400))
                } icon: {
                    Image(systemName: "arrow.uturn.backward.circle")
                }
                .font(.callout)
                .foregroundStyle(.trouble)
            }
            TextField("", text: $model[lineDraft: vendor].text, prompt: Text("Start a line with \(name)"), axis: .vertical)
                .textFieldStyle(.plain)
                .font(.system(size: 14))
                .lineLimit(1...8)
                .focused($isFocused)
            HStack(spacing: 6) {
                projectMenu(draft)
                roleMenu(draft)
                Spacer(minLength: 4)
                ModePill()
                ModelPill(
                    vendor: vendor, current: draft.choice, footnote: "Used when the line opens.",
                    switchVendor: { model.page = .bot($0) }, pick: { model[lineDraft: vendor].choice = $0 })
                Button { model.openLine(for: vendor) } label: {
                    Image(systemName: "arrow.up")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(.background)
                        .frame(width: 34, height: 34)
                        .background(.primary, in: .circle)
                        .opacity(canOpen ? 1 : 0.3)
                        .contentShape(.circle)
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(!canOpen)
                .help(whyNot ?? "Open the line (⌘Return)")
                .accessibilityLabel("Open the line")
            }
            if let whyNot { Text(verbatim: whyNot).font(.system(size: 11.5)).foregroundStyle(.secondary) }
        }
        .padding(14)
        .vetroSurface(cornerRadius: 18)
    }

    private var canOpen: Bool { whyNot == nil && model.lineToOpen(for: vendor) != nil }

    private var whyNot: String? {
        if case .recording = model.source { return "A recording: nothing can be sent." }
        if !model.isLive { return Wording.notLive(model.status) + "." }
        if model.overview != nil, model.overview?.lines == nil { return "This LMAS was started without lines straight to an agent (--lines)." }
        if model.state.projectList.isEmpty { return "Register a project first: the line works on its own copy of one." }
        if model.state.catalogs[vendor]?.listed == nil { return "LMAS has not said what this bot can be on a line yet." }
        return nil
    }

    private func projectMenu(_ draft: LineDraft) -> some View {
        Menu {
            ForEach(model.state.projectList) { project in
                Button(UntrustedText.line(project.name, limit: 64)) { model[lineDraft: vendor].project = project.name }
            }
            if !model.state.projectList.isEmpty { Divider() }
            Button("Register a Project…") { model.page = .projects }
        } label: {
            Chip(symbol: "folder", text: (draft.project ?? model.state.projectList.first?.name).map { UntrustedText.line($0, limit: 40) } ?? "No project")
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Which repository the line gets its own copy of")
    }

    private func roleMenu(_ draft: LineDraft) -> some View {
        let roles = model.state.catalogs[vendor]?.listed?.roles ?? []
        return Menu {
            ForEach(roles, id: \.self) { role in
                Button(Wording.role(role)) { model[lineDraft: vendor].role = role }
            }
        } label: {
            Chip(symbol: "person", text: (draft.role ?? roles.first).map(Wording.role) ?? "Role")
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .disabled(roles.isEmpty)
        .help("What the bot is on this line")
    }
}

private struct Chip: View {
    let symbol: String
    let text: String

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: symbol).font(.system(size: 11))
            Text(verbatim: text).lineLimit(1)
            Image(systemName: "chevron.up.chevron.down").font(.system(size: 8, weight: .semibold)).foregroundStyle(.secondary)
        }
        .font(.system(size: 12, weight: .medium))
        .padding(.horizontal, 10)
        .frame(height: 28)
        .vetroEdge(in: Capsule())
        .contentShape(.capsule)
    }
}

private struct Panel<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(verbatim: title).font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
            content
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .vetroSurface(cornerRadius: 14)
    }
}

/// Its lines, newest first: being set up, failed, up or down. Click one with letters on it to go there.
private struct BotLines: View {
    @Environment(WorkshopModel.self) private var model
    let vendor: String

    var body: some View {
        let lines = model.state.directLineList.filter { $0.vendor == vendor }
        Panel(title: title) {
            if lines.isEmpty {
                Text("No line straight to this bot yet.").font(.system(size: 12)).foregroundStyle(.secondary)
            }
            ForEach(lines) { line in
                let state = Wording.directLine(line)
                let thread = model.state.threads(with: line.id).last
                Button { if let thread { model.page = .line(thread.id) } } label: {
                    HStack(alignment: .top, spacing: 8) {
                        // A line that did not open is trouble, not something waiting on the user: orange.
                        Image(systemName: state.symbol).foregroundStyle(state.isTrouble ? AnyShapeStyle(.trouble) : AnyShapeStyle(.secondary))
                        VStack(alignment: .leading, spacing: 1) {
                            Text(verbatim: title(of: line, thread: thread)).font(.system(size: 12.5)).lineLimit(1)
                            Text(verbatim: "\(state.text) · \(line.project.map { UntrustedText.line($0, limit: 40) } ?? "")")
                                .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(2)
                        }
                        Spacer(minLength: 0)
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .disabled(thread == nil)
            }
        }
    }

    /// Section 11: being set up, or with its agent up, a line takes one of LMAS's places; across all bots.
    private var title: String {
        guard let max = model.overview?.lines?.max else { return "Lines" }
        let taken = model.state.directLineList.filter { line in
            line.failure == nil && !model.state.arrangement(of: line.id).isArchived && (line.isBeingSetUp || line.worker?.isUp == true)
        }.count
        return "Lines · \(taken) of \(max) up across all bots"
    }

    private func title(of line: DirectLine, thread: BottegaKit.Line?) -> String {
        if let thread { return model.state.title(of: thread) }
        return UntrustedText.line(line.id, limit: 40)
    }
}

/// What LMAS last heard from the CLI about its models.
private struct BotModels: View {
    @Environment(WorkshopModel.self) private var model
    let vendor: String

    var body: some View {
        Panel(title: "Models") {
            switch model.state.catalogs[vendor] {
            case .listed(let list):
                row("Default", defaultModel(list))
                row("To choose from", list.models.count == 1 ? "1 model" : "\(list.models.count) models")
                row("Can be", list.roles.map(Wording.role).joined(separator: ", "))
                if let version = list.cliVersion { row("CLI", UntrustedText.line(version, limit: 40)) }
            case .unavailable(let why, _):
                Text(verbatim: "Cannot be asked now. \(UntrustedText.line(why.text, limit: 200))")
                    .font(.system(size: 12)).foregroundStyle(.secondary)
            case nil:
                Text("LMAS has not asked it yet.").font(.system(size: 12)).foregroundStyle(.secondary)
            }
        }
    }

    /// By its name, as the CLI shows it. A vendor that does not say leaves it to its CLI (agy).
    private func defaultModel(_ list: Event.ModelsListed) -> String {
        guard let id = list.defaultModel else { return "Left to the CLI" }
        let name = UntrustedText.line(list.models.first { $0.model == id }?.name ?? id, limit: 40)
        return name + (list.defaultEffort.map { " · " + Wording.effort($0) } ?? "")
    }

    private func row(_ name: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(verbatim: name).foregroundStyle(.secondary)
            Spacer(minLength: 8)
            Text(verbatim: value).lineLimit(1)
        }
        .font(.system(size: 12))
    }
}

/// Its quota gates as LMAS last said them. How much is used, LMAS does not send.
private struct BotQuota: View {
    @Environment(WorkshopModel.self) private var model
    let vendor: String

    var body: some View {
        let shut = model.state.gatesNotOpen(of: vendor)
        Panel(title: "Quota") {
            if shut.isEmpty {
                Label("Open: it gets work as usual.", systemImage: Wording.gateSymbol("open"))
                    .font(.system(size: 12)).foregroundStyle(.secondary)
            }
            ForEach(shut, id: \.key) { gate in
                VStack(alignment: .leading, spacing: 2) {
                    Label {
                        Text(verbatim: gate.model.map { "\(UntrustedText.line($0, limit: 40)): \(Wording.gateLevel(gate.level))" }
                            ?? Wording.gateLevel(gate.level))
                    } icon: {
                        Image(systemName: Wording.gateSymbol(gate.level))
                    }
                    .font(.system(size: 12.5, weight: .medium))
                    // Orange whichever it is, shut or not quite; the gauge and the words say how shut.
                    .foregroundStyle(.trouble)
                    Text(verbatim: Wording.gateDetail(gate)).font(.system(size: 11)).foregroundStyle(.secondary)
                }
            }
            NotInLMASYet(text: "LMAS does not send how much is used.").padding(.horizontal, -10)
        }
    }
}
