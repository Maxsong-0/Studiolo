import AppKit
import SwiftUI

/// What a thread shows before its first message: the app icon and a question about the project.
/// The project name opens a popover to move the empty thread to another project.
struct NewThreadPrompt: View {
    @Environment(AppModel.self) private var model
    let threadID: UUID
    let projectName: String?

    @State private var isVisible = false
    @State private var isChoosingProject = false
    @State private var isHoveringName = false

    var body: some View {
        VStack(spacing: 26) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .interpolation(.high)
                .frame(width: 72, height: 72)
                .accessibilityHidden(true)
            HStack(spacing: 0) {
                if let projectName, !projectName.isEmpty {
                    Text("What should we build in ")
                    Button {
                        isChoosingProject.toggle()
                    } label: {
                        Text(verbatim: projectName)
                            .underline(true, pattern: .dot, color: isHoveringName ? Chrome.primaryText : Chrome.secondaryText)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .onHover { isHoveringName = $0 }
                    .help("Choose another project")
                    .popover(isPresented: $isChoosingProject, arrowEdge: .bottom) {
                        projectChoices
                    }
                    Text("?")
                } else {
                    Text("What should we build?")
                }
            }
            .font(.system(size: 28, weight: .regular))
            .foregroundStyle(Chrome.primaryText)
            .lineLimit(1)
            .minimumScaleFactor(0.6)
        }
        .padding(.horizontal, 40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .opacity(isVisible ? 1 : 0)
        .scaleEffect(isVisible ? 1 : 0.97)
        .onAppear {
            withAnimation(.easeOut(duration: 0.3)) { isVisible = true }
        }
    }

    private var projectChoices: some View {
        let current = model.thread(threadID)?.projectID
        return PopoverMenu {
            PopoverSectionHeader("Projects")
            ForEach(model.projects) { project in
                PopoverItem(project.name, symbol: "folder", isChecked: project.id == current) {
                    move(to: project)
                }
            }
            PopoverDivider()
            PopoverItem("Add project…", symbol: "plus") {
                model.chooseProjectFolder()
            }
        }
    }

    private func move(to project: Project) {
        guard let thread = model.thread(threadID), thread.projectID != project.id else { return }
        model.existingRuntime(for: threadID)?.stopSession()
        model.updateThread(threadID) { thread in
            thread.projectID = project.id
            thread.worktreePath = nil
            thread.branch = nil
            thread.providerSessionID = nil
            thread.providerResumeAt = nil
        }
        model.updateProject(project.id) { $0.isExpanded = true }
    }
}
