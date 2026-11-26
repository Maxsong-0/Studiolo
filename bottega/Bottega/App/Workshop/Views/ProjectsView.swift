// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// The repositories agents may work on (LMAS section 11, `add_project`, `remove_project`). A line straight to an agent
/// gets its own copy of one; registering and removing both take Touch ID.
struct ProjectsView: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("A registered folder is where a bot you start a line with gets its own copy and writes code.")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                RegisterProject()
                ForEach(model.state.projectList) { ProjectCard(project: $0) }
                if model.state.projectList.isEmpty {
                    Text("No project is registered yet.").font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: 560, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .scrollEdgeEffectStyle(.soft, for: .top)
    }
}

private struct RegisterProject: View {
    @Environment(WorkshopModel.self) private var model
    @State private var folder: URL?
    @State private var name = ""
    @State private var isChoosing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label("Register a project", systemImage: "folder.badge.plus").font(.system(size: 12, weight: .semibold))
                Spacer()
                Text("Asks for Touch ID").font(.system(size: 11)).foregroundStyle(.secondary)
            }
            if let folder {
                TextField("Name", text: $name, prompt: Text("Name"))
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 14, weight: .semibold))
                Fact(name: "Folder", value: folder.path)
                Fact(name: "LMAS checks", value: "Repository root · nothing in it runs on checkout")
                Text("A bot you start a line with gets its own copy of this repository and can write code in it. Nothing is committed, merged or pushed for you.")
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 10) {
                if let folder {
                    Button { model.addProject(name: name, path: folder.path) } label: {
                        Label("Register with Touch ID", systemImage: "touchid")
                    }
                    .buttonStyle(.glassProminent)
                    .disabled(!model.isLive || name.isEmpty || model.isSending("add_project", about: name))
                    .help(model.isLive ? "LMAS looks at the folder when it gets this" : Wording.notLive(model.status))
                }
                Button(folder == nil ? "Choose a Folder…" : "Choose Another Folder…") { isChoosing = true }
                    .buttonStyle(.borderless)
            }
        }
        .padding(16)
        .vetroSurface(cornerRadius: 14)
        .fileImporter(isPresented: $isChoosing, allowedContentTypes: [.folder]) { result in
            guard case .success(let url) = result else { return }
            folder = url
            name = String(url.lastPathComponent.prefix(64))
        }
        // Registered: the form is done with.
        .onChange(of: model.state.projectList.map(\.name)) {
            if model.state.projects[name] != nil {
                folder = nil
                name = ""
            }
        }
    }
}

private struct ProjectCard: View {
    @Environment(WorkshopModel.self) private var model
    let project: BottegaKit.Project

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(verbatim: UntrustedText.line(project.name, limit: 64)).font(.system(size: 14, weight: .semibold))
                Spacer()
                Text(verbatim: lineCount).font(.system(size: 11)).foregroundStyle(.secondary)
            }
            Fact(name: "Folder", value: project.path)
            Fact(name: "Registered", value: Date(timeIntervalSince1970: Double(project.addedAt) / 1000).formatted(date: .abbreviated, time: .omitted))
            HStack(spacing: 10) {
                Button { model.removeProject(project.name) } label: {
                    Label("Remove with Touch ID", systemImage: "touchid")
                }
                .buttonStyle(.glass)
                .disabled(!model.isLive || model.isSending("remove_project", about: project.name))
                Text("Its lines are archived. Worktrees and branches stay.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(16)
        .vetroSurface(cornerRadius: 14)
    }

    private var lineCount: String {
        let lines = model.state.directLineList.filter { $0.project == project.name }
        return lines.count == 1 ? "1 line" : "\(lines.count) lines"
    }
}

private struct Fact: View {
    let name: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(verbatim: name).foregroundStyle(.secondary)
            Spacer(minLength: 12)
            Text(verbatim: UntrustedText.line(value, limit: 240))
                .font(.system(size: 11.5, design: .monospaced))
                .lineLimit(1)
                .truncationMode(.head)
                .textSelection(.enabled)
        }
        .font(.system(size: 12))
    }
}
