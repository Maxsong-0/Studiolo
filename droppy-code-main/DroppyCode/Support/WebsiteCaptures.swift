import AppKit
import AVFoundation
import ScreenCaptureKit
import SwiftUI

/// Renders the real app with mock data into stills and films for the marketing site:
///
///     open -n -W "Droppy Code.app" --args --website-captures <folder>
///
/// Nothing in here runs otherwise. The run keeps to its own storage folder and its own
/// defaults suite, never reads the Keychain, posts no notifications, and quits when the
/// last file is written, so the library, settings and permissions on this Mac are untouched.
///
/// The app window is put over a backdrop window showing this Mac's own desktop picture, and
/// every capture is the composite of the two as the window server shows it: the real Liquid
/// Glass over the real wallpaper. ScreenCaptureKit composites a process's own windows without
/// screen recording permission, which is all a capture ever includes. Stills are PNG at the
/// display's scale; films are recorded by a 60 fps stream straight into a high-bitrate HEVC
/// master, for `scripts/website_captures.py` to cut and encode.
@MainActor
enum WebsiteCaptures {
    nonisolated static let outputDirectory: URL? = {
        let arguments = CommandLine.arguments
        for flag in ["--website-captures", "--tour-captures"] {
            if let index = arguments.firstIndex(of: flag),
               arguments.indices.contains(index + 1) {
                return URL(fileURLWithPath: arguments[index + 1], isDirectory: true)
            }
        }
        return nil
    }()

    nonisolated static var isTourRun: Bool {
        let arguments = CommandLine.arguments
        guard let index = arguments.firstIndex(of: "--tour-captures") else { return false }
        return arguments.indices.contains(index + 1)
    }

    nonisolated static var isEnabled: Bool { outputDirectory != nil }

    /// The library and thread files the run reads, under the output folder.
    nonisolated static var storageRoot: URL? {
        outputDirectory?.appendingPathComponent("storage", isDirectory: true)
    }

    /// The defaults the run reads and writes instead of the app's own.
    nonisolated static var defaults: UserDefaults? {
        isEnabled ? UserDefaults(suiteName: suiteName) : nil
    }

    nonisolated private static let suiteName = "iordv.droppycode.website-captures"

    /// The composer's model chip, captured by the chip itself, so the slider and the
    /// switcher popovers hang from the real control.
    static var modelChipAnchor: WeakView?
    /// Where each thread's composer chip sits in the window (SwiftUI's global space, which
    /// is the hosting view's flipped coordinates), for a popover hung from the window itself.
    static var modelChipFrames: [UUID: CGRect] = [:]
    /// Where the toolbar's Hydra mark is, in the window, for the intro scene: the intro
    /// hangs from the mark in the app, so its capture hangs it there too.
    static var hydraMarkFrame: CGRect?
    /// Where the toolbar's Threads button is (the hidden sidebar's list hangs from it).
    static var threadsButtonFrame: CGRect?
    /// Where each lead's docked Hydra panel sits in the window (SwiftUI's global space),
    /// so the tour's Hydra page can zoom on the team's panel.
    static var hydraPanelFrames: [UUID: CGRect] = [:]

    /// Wallpaper around the window in every capture, in points.
    static let margin: CGFloat = 72

    /// One line per step into <output>/run.log, so a stalled run shows where it stopped.
    nonisolated static func log(_ line: String) {
        guard let output = outputDirectory else { return }
        let stamp = String(format: "%.1f", Date.now.timeIntervalSince1970.truncatingRemainder(dividingBy: 10_000))
        let text = "\(stamp) \(line)\n"
        let url = output.appendingPathComponent("run.log")
        if let handle = try? FileHandle(forWritingTo: url) {
            handle.seekToEndOfFile()
            handle.write(Data(text.utf8))
            try? handle.close()
        } else {
            try? text.write(to: url, atomically: true, encoding: .utf8)
        }
    }

    // MARK: - Mock data

    enum ID {
        static let droppyCode = UUID(uuidString: "A0000000-0000-4000-8000-000000000001")!
        static let site = UUID(uuidString: "A0000000-0000-4000-8000-000000000002")!
        static let ios = UUID(uuidString: "A0000000-0000-4000-8000-000000000003")!
        static let composer = UUID(uuidString: "B0000000-0000-4000-8000-000000000001")!
        static let diffPopover = UUID(uuidString: "B0000000-0000-4000-8000-000000000002")!
        static let archive = UUID(uuidString: "B0000000-0000-4000-8000-000000000003")!
        static let finalAnswer = UUID(uuidString: "B0000000-0000-4000-8000-000000000004")!
        static let pricing = UUID(uuidString: "B0000000-0000-4000-8000-000000000005")!
        static let footer = UUID(uuidString: "B0000000-0000-4000-8000-000000000006")!
        static let rail = UUID(uuidString: "B0000000-0000-4000-8000-000000000007")!
        static let liveActivity = UUID(uuidString: "B0000000-0000-4000-8000-000000000008")!
        static let widgets = UUID(uuidString: "B0000000-0000-4000-8000-000000000009")!
        static let fresh = UUID(uuidString: "B0000000-0000-4000-8000-000000000010")!
        static let freshPlan = UUID(uuidString: "B0000000-0000-4000-8000-000000000011")!
    }

    /// Writes the library, the thread histories and the project folders the run opens, and
    /// resets the run's defaults. Called before anything reads settings or the library.
    static func prepare() {
        guard let output = outputDirectory, let root = storageRoot, let defaults else { return }
        let files = FileManager.default
        try? files.removeItem(at: root)
        try? files.createDirectory(at: root.appendingPathComponent("threads"), withIntermediateDirectories: true)
        try? files.createDirectory(at: output.appendingPathComponent("films"), withIntermediateDirectories: true)

        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("dark", forKey: "appTheme")
        defaults.set(false, forKey: "notifyWhenFinished")
        defaults.set(false, forKey: "chimeWhenFinished")
        defaults.set(true, forKey: "showReasoning")
        defaults.set(true, forKey: "sidebarActivityView")
        defaults.set("supervised", forKey: "defaultRuntimeMode")
        defaults.set("claude", forKey: "defaultProvider")
        defaults.set(ID.droppyCode.uuidString, forKey: "lastProjectID")
        // The switcher shows a curated list, as a set-up Mac would.
        let pins = [
            ModelPin(provider: .claude, modelID: "opus"),
            ModelPin(provider: .claude, modelID: "sonnet"),
            ModelPin(provider: .antigravity, modelID: "gemini-3.8-flash"),
            ModelPin(provider: .deepseek, modelID: "deepseek-v4-pro"),
            ModelPin(provider: .meta, modelID: "muse-spark-1.3"),
        ]
        if let data = try? JSONEncoder().encode(pins) { defaults.set(data, forKey: "modelList") }
        defaults.synchronize()

        if isTourRun {
            defaults.set(true, forKey: "hydraEnabled")
            // With Hydra on, a queued follow-up would go out as a real head (see
            // `ThreadRuntime.enqueueFollowUp`): the queue scene must queue, nothing more.
            defaults.set(false, forKey: "hydraQueueHeads")
            defaults.set(false, forKey: "hydraAlwaysHeads")
        }
        let repos = output.appendingPathComponent("repos", isDirectory: true)
        var library = Library()
        library.projects = [
            project(ID.droppyCode, "Droppy Code", repos, ["DroppyCode/Views/Composer/ComposerView.swift": composerSource]),
            project(ID.site, "getdroppy.app", repos, ["docs/index.html": "<!doctype html>\n<html lang=\"en\">\n</html>\n"]),
            project(ID.ios, "droppy-ios", repos, ["Droppy/LiveActivity.swift": "import ActivityKit\n"]),
        ]
        library.projects[0].scripts = [
            ProjectScript(name: "Quick run", command: "scripts/quick_run.sh", symbol: "hammer"),
            ProjectScript(name: "Release", command: "scripts/release.sh", symbol: "play"),
        ]
        let now = Date.now
        let day: TimeInterval = 86_400
        library.threads = [
            thread(ID.composer, ID.droppyCode, "Composer: draft photo spacing", .claude, "opus", "high", age: 120, status: .completed, now),
            thread(ID.fresh, ID.droppyCode, "Attachment strip insets", .claude, "opus", "high", age: 300, status: nil, now),
            thread(ID.finalAnswer, ID.droppyCode, "Timeline final answer", .claude, "opus", "high", age: 600, status: .completed, now, plan: true),
            thread(ID.pricing, ID.site, "Regional pricing claim expiry", .codex, "gpt-5.5", nil, age: 900, status: .running, now),
            thread(ID.freshPlan, ID.droppyCode, "Final answer card", .claude, "opus", "high", age: 1_500, status: nil, now, plan: true),
            thread(ID.archive, ID.droppyCode, "Archive: delete all", .codex, "gpt-5.5", nil, age: 7_200, status: .completed, now),
            thread(ID.diffPopover, ID.droppyCode, "Diff popover row patch", .claude, "opus", "high", age: day + 3_600, status: .completed, now),
            thread(ID.footer, ID.site, "Footer dragon scrub", .cursor, nil, nil, age: day + 5_400, status: .completed, now, unread: true),
            thread(ID.rail, ID.site, "Highlight rail poster blend", .claude, "sonnet", "medium", age: day + 9_000, status: .completed, now),
            thread(ID.liveActivity, ID.ios, "Live Activity for Pomodoro", .claude, "opus", "high", age: 3 * day, status: .completed, now),
            thread(ID.widgets, ID.ios, "Widget snapshots", .antigravity, "gemini-3.8-flash", "high", age: 6 * day, status: .completed, now),
        ]
        if isTourRun {
            TourCaptures.addHydraMocks(to: &library, storageRoot: root, repos: repos, now: now)
        }
        write(library, to: root.appendingPathComponent("library.json"))

        write(composerHistory(now), to: root.appendingPathComponent("threads/\(ID.composer.uuidString).json"))
        write(finalAnswerHistory(now), to: root.appendingPathComponent("threads/\(ID.finalAnswer.uuidString).json"))
        write(pricingHistory(now), to: root.appendingPathComponent("threads/\(ID.pricing.uuidString).json"))
    }

    private static func write<Value: Encodable>(_ value: Value, to url: URL) {
        guard let data = try? JSONEncoder.storage.encode(value) else { return }
        try? data.write(to: url, options: .atomic)
    }

    private static func project(_ id: UUID, _ name: String, _ repos: URL, _ files: [String: String]) -> Project {
        let folder = repos.appendingPathComponent(name, isDirectory: true)
        for (path, contents) in files {
            let url = folder.appendingPathComponent(path)
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? contents.write(to: url, atomically: true, encoding: .utf8)
        }
        // A real repository on `main`, so the chrome row has a branch to show.
        let git = Process()
        git.executableURL = URL(fileURLWithPath: "/bin/sh")
        git.arguments = ["-c", "git init -q -b main && git -c user.name=Droppy -c user.email=hi@getdroppy.app add -A && git -c user.name=Droppy -c user.email=hi@getdroppy.app commit -q -m 'Initial import' >/dev/null 2>&1"]
        git.currentDirectoryURL = folder
        try? git.run()
        git.waitUntilExit()
        var project = Project(name: name, path: folder.path)
        project.id = id
        return project
    }

    static let composerThreadID = ID.composer
    static let freshThreadID = ID.fresh

    static func thread(
        _ id: UUID, _ projectID: UUID, _ title: String, _ provider: ProviderKind, _ model: String?, _ effort: String?,
        age: TimeInterval, status: TurnStatus?, _ now: Date, plan: Bool = false, unread: Bool = false
    ) -> ChatThread {
        var thread = ChatThread(projectID: projectID, provider: provider, model: model, effort: effort, runtimeMode: .supervised)
        thread.id = id
        thread.title = title
        thread.hasCustomTitle = true
        thread.createdAt = now.addingTimeInterval(-age - 1_800)
        thread.updatedAt = now.addingTimeInterval(-age)
        thread.lastStatus = status
        thread.hasUnread = unread
        thread.interactionMode = plan ? .plan : .build
        thread.branch = "main"
        return thread
    }

    /// The composer thread's earlier turn: a question about the strip, answered.
    private static func composerHistory(_ now: Date) -> ThreadDocument {
        var document = ThreadDocument(threadID: ID.composer)
        var turn = TurnRecord(index: 0)
        turn.startedAt = now.addingTimeInterval(-1_500)
        turn.completedAt = now.addingTimeInterval(-1_440)
        turn.status = .completed
        let user = TimelineItem(turnID: turn.id, date: turn.startedAt, content: .user(UserMessage(
            text: "Where does the composer decide how far the draft photo sits from the edges?")))
        turn.userItemID = user.id
        let reasoning = TimelineItem(turnID: turn.id, date: turn.startedAt.addingTimeInterval(2), content: .reasoning(ReasoningBlock(
            text: "The attachment strip lives in ComposerView.swift. Its insets come from a single constant, so I should read that first.")))
        var read = ToolCall(kind: .read, title: "Read", detail: "DroppyCode/Views/Composer/ComposerView.swift")
        read.status = .completed
        read.output = composerSource
        read.startedAt = turn.startedAt.addingTimeInterval(4)
        read.finishedAt = turn.startedAt.addingTimeInterval(5)
        let tool = TimelineItem(turnID: turn.id, date: read.startedAt, content: .tool(read))
        let answer = TimelineItem(turnID: turn.id, date: turn.startedAt.addingTimeInterval(9), content: .assistant(AssistantMessage(
            text: "In `DraftAttachments`, inside **ComposerView.swift**. The strip pads its leading edge with `attachmentInset` (14pt) but leaves the top on the stack's default spacing, which is why the photo hugs the top of the pill.\n\nWant me to give the top the same 14pt?")))
        let end = TimelineItem(turnID: turn.id, date: turn.completedAt!, content: .turnEnd(TurnSummary(
            turnID: turn.id, status: .completed, duration: 58, filesChanged: 0, additions: 0, deletions: 0)))
        document.turns = [turn]
        document.items = [user, reasoning, tool, answer, end]
        document.usage = ContextUsage(usedTokens: 18_400, windowTokens: 200_000)
        return document
    }

    /// The plan-mode thread: one exchange already on screen.
    private static func finalAnswerHistory(_ now: Date) -> ThreadDocument {
        var document = ThreadDocument(threadID: ID.finalAnswer)
        var turn = TurnRecord(index: 0)
        turn.startedAt = now.addingTimeInterval(-700)
        turn.completedAt = now.addingTimeInterval(-640)
        turn.status = .completed
        let user = TimelineItem(turnID: turn.id, date: turn.startedAt, content: .user(UserMessage(
            text: "The last reply of a turn gets lost between the tool calls. What would you change?")))
        turn.userItemID = user.id
        let answer = TimelineItem(turnID: turn.id, date: turn.startedAt.addingTimeInterval(6), content: .assistant(AssistantMessage(
            text: "Three things, and I would keep them small: tag the final text so the timeline knows which block is the answer, give that block the glass card the plan cards already use, and leave streaming text plain so nothing flickers while it arrives.")))
        let end = TimelineItem(turnID: turn.id, date: turn.completedAt!, content: .turnEnd(TurnSummary(
            turnID: turn.id, status: .completed, duration: 41, filesChanged: 0, additions: 0, deletions: 0)))
        document.turns = [turn]
        document.items = [user, answer, end]
        document.usage = ContextUsage(usedTokens: 9_800, windowTokens: 200_000)
        return document
    }

    /// A thread on another project, so the sidebar has a second pulse.
    private static func pricingHistory(_ now: Date) -> ThreadDocument {
        var document = ThreadDocument(threadID: ID.pricing)
        var turn = TurnRecord(index: 0)
        turn.startedAt = now.addingTimeInterval(-900)
        let user = TimelineItem(turnID: turn.id, date: turn.startedAt, content: .user(UserMessage(
            text: "Why does the site quote the India price and charge the global one?")))
        turn.userItemID = user.id
        document.turns = [turn]
        document.items = [user]
        return document
    }

    private static let composerSource = """
    import SwiftUI

    struct DraftAttachments: View {
        @Binding var attachments: [Attachment]
        private let attachmentInset: CGFloat = 14

        var body: some View {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    ForEach(attachments) { attachment in
                        AttachmentPreview(attachment: attachment) {
                            attachments.removeAll { $0.id == attachment.id }
                        }
                    }
                }
                .padding(.leading, attachmentInset)
                .transition(.scale.combined(with: .opacity))
            }
        }
    }

    """

    private static let composerPatch = """
    diff --git a/DroppyCode/Views/Composer/ComposerView.swift b/DroppyCode/Views/Composer/ComposerView.swift
    --- a/DroppyCode/Views/Composer/ComposerView.swift
    +++ b/DroppyCode/Views/Composer/ComposerView.swift
    @@ -12,8 +12,10 @@ struct DraftAttachments: View {
                     }
                 }
    -            .padding(.leading, attachmentInset)
    +            .padding(.top, attachmentInset)
    +            .padding(.leading, attachmentInset)
    +            .accessibilityLabel("Draft photo")
                 .transition(.scale.combined(with: .opacity))
             }
         }
     }

    """

    // MARK: - Run

    static func run(model: AppModel) async {
        if isTourRun { await TourCaptures.run(model: model); return }
        guard let output = outputDirectory else { return }
        // A capture call that never returns must not leave a recording process behind: the
        // run quits after its budget no matter where it is.
        Task {
            try? await Task.sleep(for: .seconds(110))
            log("out of time, quitting")
            exit(0)
        }
        let stage = Stage(model: model)
        stage.show(size: Stage.wide)
        await stage.ensureActive()
        let recorder = Recorder(output: output, stage: stage)
        try? await Task.sleep(for: .milliseconds(1_400))

        await blankScene(model, stage, recorder, name: "blank-wide")
        await editingScene(model, stage, recorder)
        await themesScene(model, stage, recorder)
        await diffScene(model, stage, recorder)
        await queueScene(model, stage, recorder)
        await paletteScene(model, stage, recorder)
        await plansScene(model, stage, recorder)
        await questionScene(model, stage, recorder)

        // The detail scenes: a narrow window without the sidebar, on an empty thread, so the
        // composer and its popovers sit on clean glass and can be shown large.
        stage.resize(to: Stage.narrow)
        if model.sidebar.isVisible { model.sidebar.toggle() }
        model.selectedThreadID = ID.fresh
        try? await Task.sleep(for: .milliseconds(1_200))
        await recorder.still("blank-narrow", stage.captureRect)
        await sliderScene(model, stage, recorder)
        await switcherScene(model, stage, recorder)

        await recorder.finish()
        log("done")
        exit(0)
    }

    /// The empty thread, so the encoder knows what untouched glass looks like.
    private static func blankScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, name: String) async {
        log("scene blankScene")
        model.selectedThreadID = ID.fresh
        try? await Task.sleep(for: .milliseconds(900))
        await recorder.still(name, stage.captureRect)
    }

    /// The composer thread mid-turn: reasoning, a read, an edit, the to-dos, a build running
    /// and the reply streaming in. Ends still running, so the working line stays.
    static func editingScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = true) async {
        log("scene editingScene")
        model.selectedThreadID = ID.composer
        // The sidebar's other pulse.
        let pricing = model.runtime(for: ID.pricing)
        pricing.rehearseTurn(nil)
        pricing.rehearse(.reasoningDelta(id: "pricing-r", text: "Tracing claimExpiryFromString: the expiry is field 2, not the last field."))
        try? await Task.sleep(for: .milliseconds(900))

        let runtime = model.runtime(for: ID.composer)
        if film { await recorder.startFilm("editing", stage.captureRect) }
        try? await Task.sleep(for: .milliseconds(600))
        runtime.rehearseTurn(
            "The draft photo in the composer should sit as far from the top as it does from the left.",
            touchedPaths: ["DroppyCode/Views/Composer/ComposerView.swift"],
            providerDiff: composerPatch
        )
        try? await Task.sleep(for: .milliseconds(500))
        await stream(runtime, id: "r1", reasoning: true, "Reading ComposerView.swift and the attachment strip. The leading edge pads with attachmentInset, 14pt, while the top keeps the stack's default spacing, so the photo hugs the top of the pill. Matching the top to the leading inset is a one-line change, plus a label the strip is missing.", step: 45)
        runtime.rehearse(.reasoningCompleted(id: "r1", text: ""))
        try? await Task.sleep(for: .milliseconds(250))

        runtime.rehearse(.toolStarted(id: "t-read", call: ToolCall(kind: .read, title: "Read", detail: "DroppyCode/Views/Composer/ComposerView.swift")))
        try? await Task.sleep(for: .milliseconds(650))
        runtime.rehearse(.toolUpdated(id: "t-read", update: ToolUpdate(output: composerSource, status: .completed)))
        try? await Task.sleep(for: .milliseconds(300))

        runtime.rehearse(.toolStarted(id: "t-edit", call: ToolCall(kind: .edit, title: "Edit", detail: "ComposerView.swift · attachment padding")))
        try? await Task.sleep(for: .milliseconds(800))
        runtime.rehearse(.toolUpdated(id: "t-edit", update: ToolUpdate(
            status: .completed,
            edits: [FileEdit(path: "DroppyCode/Views/Composer/ComposerView.swift", diff: composerPatch, additions: 3, deletions: 1)]
        )))
        runtime.rehearse(.diff(composerPatch))
        try? await Task.sleep(for: .milliseconds(250))
        runtime.rehearse(.todos([
            TodoStep(text: "Find the attachment strip's insets", status: .done),
            TodoStep(text: "Match the top inset to the leading one", status: .done),
            TodoStep(text: "Build and confirm nothing else moved", status: .active),
        ]))
        try? await Task.sleep(for: .milliseconds(350))
        runtime.rehearse(.toolStarted(id: "t-build", call: ToolCall(kind: .command, title: "xcodebuild -scheme DroppyCode build", detail: nil)))
        try? await Task.sleep(for: .milliseconds(400))
        await stream(runtime, id: "m1", reasoning: false, "Done. The attachment strip now uses the same 14pt inset on both axes, so the draft photo sits as far from the top as from the left. The build is running to confirm nothing else moved.", step: 38)
        runtime.rehearse(.messageCompleted(id: "m1", text: ""))
        runtime.noteDiffChanged()
        try? await Task.sleep(for: .milliseconds(1_400))
        if film { await recorder.stopFilm() }
        await recorder.still("editing", stage.captureRect)
    }

    /// The same window in every theme, for the theme picker.
    private static func themesScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder) async {
        log("scene themesScene")
        let current = model.settings.theme
        for theme in AppTheme.allCases {
            model.settings.theme = theme
            try? await Task.sleep(for: .milliseconds(500))
            await recorder.still("theme-\(theme.rawValue)", stage.captureRect)
        }
        model.settings.theme = current
        try? await Task.sleep(for: .milliseconds(800))
    }

    /// The changes popover, open on the turn's diff.
    static func diffScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = true, stillName: String? = nil) async {
        log("scene diffScene")
        let runtime = model.runtime(for: ID.composer)
        runtime.clearDiffFocus()
        runtime.diffAnchor = nil
        runtime.isDiffVisible = true
        try? await Task.sleep(for: .milliseconds(1_200))
        await recorder.still(stillName ?? "diff", film ? stage.captureRect : stage.tourCaptureRect)
        runtime.isDiffVisible = false
        try? await Task.sleep(for: .milliseconds(600))
    }

    /// Three follow-ups queued behind the running turn.
    static func queueScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = true, stillName: String? = nil) async {
        log("scene queueScene")
        let runtime = model.runtime(for: ID.composer)
        if film { await recorder.startFilm("queue", stage.captureRect) }
        try? await Task.sleep(for: .milliseconds(700))
        for text in [
            "Also match the trailing inset, and keep the corner radius at 10.",
            "Then run the composer's UI tests.",
            "Commit with a message that names the attachment strip.",
        ] {
            runtime.enqueueFollowUp(text: text, attachments: [])
            try? await Task.sleep(for: .milliseconds(800))
        }
        try? await Task.sleep(for: .milliseconds(1_200))
        if film { await recorder.stopFilm() }
        await recorder.still(stillName ?? "queue", film ? stage.captureRect : stage.tourCaptureRect)
        for prompt in runtime.followUps { runtime.removeFollowUp(prompt.id) }
        try? await Task.sleep(for: .milliseconds(600))
    }

    /// The command palette over the thread.
    static func paletteScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = true, stillName: String? = nil) async {
        log("scene paletteScene")
        model.isCommandPalettePresented = true
        try? await Task.sleep(for: .milliseconds(800))
        await recorder.still(stillName ?? "palette", film ? stage.captureRect : stage.tourCaptureRect)
        model.isCommandPalettePresented = false
        try? await Task.sleep(for: .milliseconds(500))
    }

    /// A fresh plan-mode thread: one ask, a proposed plan, the to-dos. Nothing else on the
    /// glass, so the encoder can cut the whole exchange out cleanly.
    static func plansScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = true, stillName: String? = nil) async {
        log("scene plansScene")
        model.selectedThreadID = ID.freshPlan
        try? await Task.sleep(for: .milliseconds(900))
        let runtime = model.runtime(for: ID.freshPlan)
        runtime.rehearseTurn("Give the final answer of a turn its own card.")
        try? await Task.sleep(for: .milliseconds(400))
        runtime.rehearse(.planCompleted(id: "p0", markdown: "1. Tag the last assistant text of a turn as the final answer\n2. Give it the glass card the plan cards use\n3. Keep streaming text plain until the turn ends"))
        try? await Task.sleep(for: .milliseconds(300))
        runtime.rehearse(.todos([
            TodoStep(text: "Tag the final answer in the timeline", status: .done),
            TodoStep(text: "Draw it on the plan cards' glass", status: .active),
            TodoStep(text: "Leave streaming text plain", status: .pending),
        ]))
        try? await Task.sleep(for: .milliseconds(1_200))
        await recorder.still(stillName ?? "plans", film ? stage.captureRect : stage.tourCaptureRect)
    }

    /// The plan-mode thread: a plan proposed, then the agent asks a question, then it asks
    /// before pushing. Lands the thread under "Needs attention" in the sidebar.
    static func questionScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = true, stillName: String? = nil) async {
        log("scene questionScene")
        model.selectedThreadID = ID.finalAnswer
        try? await Task.sleep(for: .milliseconds(1_000))
        let runtime = model.runtime(for: ID.finalAnswer)
        if film { await recorder.startFilm("question", stage.captureRect) }
        try? await Task.sleep(for: .milliseconds(600))
        runtime.rehearseTurn("Make the final answer of a turn stand out from the tool chatter above it.")
        try? await Task.sleep(for: .milliseconds(400))
        await stream(runtime, id: "r2", reasoning: true, "Comparing the last text block against tool results in TimelineView. The answer needs its own marker before it can get its own card.", step: 40)
        runtime.rehearse(.reasoningCompleted(id: "r2", text: ""))
        try? await Task.sleep(for: .milliseconds(200))
        runtime.rehearse(.toolStarted(id: "t-tl", call: ToolCall(kind: .read, title: "Read", detail: "DroppyCode/Views/Chat/TimelineRows.swift")))
        try? await Task.sleep(for: .milliseconds(500))
        runtime.rehearse(.toolUpdated(id: "t-tl", update: ToolUpdate(output: "struct TimelineRows: View {\n", status: .completed)))
        try? await Task.sleep(for: .milliseconds(300))
        runtime.rehearse(.planCompleted(id: "p1", markdown: "1. Tag the last assistant text of a turn as the final answer\n2. Give it the glass card the plan cards use\n3. Keep streaming text plain until the turn ends"))
        try? await Task.sleep(for: .milliseconds(900))
        runtime.rehearse(.question(QuestionRequest(id: "q1", questions: [
            QuestionRequest.Question(
                id: "q1-a",
                header: "Tool calls",
                prompt: "Should the final answer also collapse the tool calls above it by default?",
                choices: [
                    QuestionRequest.Choice(label: "Yes, collapse them", detail: "The reply stays in view; the steps are one click away"),
                    QuestionRequest.Choice(label: "No, keep them open", detail: "Every step stays visible above the answer"),
                ],
                allowsMultiple: false,
                allowsOther: true,
                isSecret: false
            ),
        ])))
        try? await Task.sleep(for: .milliseconds(2_600))
        await recorder.still(stillName ?? "question", film ? stage.captureRect : stage.tourCaptureRect)
        runtime.rehearse(.requestResolved(id: "q1"))
        // Over a gradient only the question itself is photographed; the approval half of
        // the film stays in the website run, so no approval is left behind.
        guard film else {
            try? await Task.sleep(for: .milliseconds(600))
            return
        }
        try? await Task.sleep(for: .milliseconds(800))
        runtime.rehearse(.toolStarted(id: "t-edit2", call: ToolCall(kind: .edit, title: "Edit", detail: "TimelineRows.swift · final answer card")))
        try? await Task.sleep(for: .milliseconds(700))
        runtime.rehearse(.toolUpdated(id: "t-edit2", update: ToolUpdate(status: .completed, edits: [FileEdit(path: "DroppyCode/Views/Chat/TimelineRows.swift", additions: 18, deletions: 3)])))
        try? await Task.sleep(for: .milliseconds(500))
        runtime.rehearse(.approval(ApprovalRequest(
            id: "a1",
            kind: .command,
            title: "git push origin feat/timeline-final-answer",
            detail: "Pushes the branch so the merge request can be opened.",
            reason: nil,
            options: [
                ApprovalRequest.Option(id: "yes", title: "Allow", role: .approve),
                ApprovalRequest.Option(id: "always", title: "Allow for this thread", role: .approveAlways),
                ApprovalRequest.Option(id: "no", title: "Deny", role: .decline),
            ],
            toolItemID: nil
        )))
        try? await Task.sleep(for: .milliseconds(2_400))
        if film { await recorder.stopFilm() }
        await recorder.still("approval", stage.captureRect)
    }

    /// The reasoning slider, from the chip: high, then maximum with its particles, then fast
    /// mode fused in, then back down.
    private static func sliderScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder) async {
        log("scene sliderScene")
        guard let thread = model.thread(ID.fresh) else { return }
        let option = model.providers.model(thread.model, for: thread.provider)
        let card = EffortSliderCard(
            modelName: option?.shortName ?? thread.model ?? thread.provider.displayName,
            provider: thread.provider,
            efforts: option?.efforts ?? [],
            defaultEffort: option?.defaultEffort,
            supportsFast: option?.supportsFast ?? false,
            effort: Binding(get: { model.thread(ID.fresh)?.effort }, set: { effort in model.updateThread(ID.fresh) { $0.effort = effort } }),
            fastMode: Binding(get: { model.thread(ID.fresh)?.fastMode ?? false }, set: { on in model.updateThread(ID.fresh) { $0.fastMode = on } }),
            onTitleTap: {},
            onReset: {}
        )
        let popover = stage.presentPopover(card.environment(model), width: 330)
        try? await Task.sleep(for: .milliseconds(900))
        await recorder.startFilm("slider", stage.captureRect)
        try? await Task.sleep(for: .milliseconds(900))
        withAnimation(.snappy(duration: 0.2)) { model.updateThread(ID.fresh) { $0.effort = "xhigh" } }
        try? await Task.sleep(for: .milliseconds(800))
        withAnimation(.snappy(duration: 0.2)) { model.updateThread(ID.fresh) { $0.effort = "max" } }
        try? await Task.sleep(for: .milliseconds(2_000))
        await recorder.still("slider", stage.captureRect)
        withAnimation(.snappy(duration: 0.2)) { model.updateThread(ID.fresh) { $0.fastMode = true } }
        try? await Task.sleep(for: .milliseconds(2_200))
        withAnimation(.snappy(duration: 0.2)) { model.updateThread(ID.fresh) { $0.fastMode = false; $0.effort = "medium" } }
        try? await Task.sleep(for: .milliseconds(1_000))
        withAnimation(.snappy(duration: 0.2)) { model.updateThread(ID.fresh) { $0.effort = "high" } }
        try? await Task.sleep(for: .milliseconds(1_100))
        await recorder.stopFilm()
        popover.close()
        try? await Task.sleep(for: .milliseconds(500))
    }

    /// The model switcher behind the slider's title.
    private static func switcherScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder) async {
        log("scene switcherScene")
        guard let thread = model.thread(ID.fresh) else { return }
        let list = ModelList(thread: thread, hasHistory: false, onChoose: { _ in }, onBack: {})
        let popover = stage.presentPopover(list.environment(model), width: 330)
        try? await Task.sleep(for: .milliseconds(900))
        await recorder.still("switcher", stage.captureRect)
        popover.close()
        try? await Task.sleep(for: .milliseconds(400))
    }

    /// The composer with a slash command menu open: the draft holds a leading slash,
    /// so the caret-anchored suggestions list shows the registry's commands for the
    /// thread's provider and directory, seeded here since the run never launches CLIs.
    static func slashScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = false, stillName: String? = nil) async {
        log("scene slashScene")
        model.selectedThreadID = ID.composer
        try? await Task.sleep(for: .milliseconds(900))
        let runtime = model.runtime(for: ID.composer)
        runtime.draft = ComposerDraft()
        if let thread = model.thread(ID.composer), let project = model.project(thread.projectID) {
            let directory = thread.worktreePath ?? project.path
            model.providers.updateCommands([
                SlashCommand(name: "compact", detail: "Summarize the conversation to free up context", isBuiltIn: true),
                SlashCommand(name: "review", detail: "Review the working tree for issues"),
                SlashCommand(name: "commit", detail: "Commit the staged changes"),
                SlashCommand(name: "test", detail: "Run the project's tests"),
                SlashCommand(name: "plan", detail: "Turn plan mode on or off", isBuiltIn: true),
            ], for: thread.provider, directory: directory)
        }
        runtime.draft.text = "/"
        try? await Task.sleep(for: .milliseconds(900))
        await recorder.still(stillName ?? "slash", film ? stage.captureRect : stage.tourCaptureRect)
        runtime.draft = ComposerDraft()
        try? await Task.sleep(for: .milliseconds(500))
    }

    /// A reply quote in the composer: one line of the assistant's reply as the quoted
    /// chip, with a short draft under it.
    static func quoteScene(_ model: AppModel, _ stage: Stage, _ recorder: Recorder, film: Bool = false, stillName: String? = nil) async {
        log("scene quoteScene")
        model.selectedThreadID = ID.composer
        try? await Task.sleep(for: .milliseconds(900))
        let runtime = model.runtime(for: ID.composer)
        var draft = ComposerDraft()
        draft.quotes = [ReplyQuote(text: "In `DraftAttachments`, inside **ComposerView.swift**.")]
        draft.text = "Do that for the sidebar too."
        runtime.draft = draft
        try? await Task.sleep(for: .milliseconds(900))
        await recorder.still(stillName ?? "quote", film ? stage.captureRect : stage.tourCaptureRect)
        runtime.draft = ComposerDraft()
        try? await Task.sleep(for: .milliseconds(500))
    }

    /// Streams `text` into the timeline a few characters at a time, as a provider does.
    private static func stream(_ runtime: ThreadRuntime, id: String, reasoning: Bool, _ text: String, step: Int) async {
        var index = text.startIndex
        while index < text.endIndex {
            let end = text.index(index, offsetBy: 4, limitedBy: text.endIndex) ?? text.endIndex
            let chunk = String(text[index..<end])
            runtime.rehearse(reasoning ? .reasoningDelta(id: id, text: chunk) : .messageDelta(id: id, text: chunk))
            index = end
            try? await Task.sleep(for: .milliseconds(step))
        }
    }
}

// MARK: - Stage

/// The two windows a capture composites: the app window, and this run's own backdrop under
/// it, showing this Mac's desktop picture.
@MainActor
final class Stage {
    static let wide = NSSize(width: 1320, height: 860)
    static let narrow = NSSize(width: 860, height: 560)

    let model: AppModel
    private var backdrop: NSWindow?
    private var window: NSWindow?

    init(model: AppModel) {
        self.model = model
    }

    /// The app window's frame in screen coordinates, for a still's sidecar.
    var windowFrame: NSRect? { window?.frame }

    /// The app window with its wallpaper margin, in screen coordinates.
    var captureRect: NSRect {
        (window?.frame ?? .zero).insetBy(dx: -WebsiteCaptures.margin, dy: -WebsiteCaptures.margin)
    }

    /// The app window with the tour's 72-point surround, widened or heightened
    /// symmetrically around the window's centre until exactly 16:10.
    var tourCaptureRect: NSRect {
        Self.tourRect(around: window?.frame ?? .zero)
    }

    /// The tour rect around the window plus an open popover, so a slider or model
    /// switcher hanging past the window edge stays centered and uncropped at 16:10.
    /// Falls back to the window-only rect when the popover is closed or headless.
    func tourCaptureRect(including popover: NSPopover?) -> NSRect {
        guard let frame = window?.frame,
              let popover,
              popover.isShown,
              let panel = popover.contentViewController?.view.window?.frame
        else { return tourCaptureRect }
        return Self.tourRect(around: frame.union(panel))
    }

    /// A SwiftUI global-space frame (the hosting view's flipped coordinates) as a screen
    /// rect, for a still's focus.
    func screenRect(fromGlobal frame: CGRect) -> NSRect? {
        guard let window, let content = window.contentView else { return nil }
        let local = content.isFlipped ? frame : NSRect(x: frame.minX, y: content.bounds.height - frame.maxY, width: frame.width, height: frame.height)
        return window.convertToScreen(content.convert(local, to: nil))
    }

    /// What a popover scene is about, in screen coordinates: the popover's panel and the
    /// chip it hangs from, so the tour page can zoom on both. Nil when the popover is not
    /// showing.
    func popoverFocus(_ popover: NSPopover?) -> NSRect? {
        guard let popover, popover.isShown, let panel = popover.contentViewController?.view.window?.frame else { return nil }
        guard let anchor = popoverAnchor, let window = anchor.view.window else { return panel }
        let chip = window.convertToScreen(anchor.view.convert(anchor.rect, to: nil))
        return panel.union(chip)
    }

    /// Whether the app window is out of the way (see `setWindowHidden`); `ensureActive`
    /// leaves it there.
    private var isWindowHidden = false

    /// The app window out of the way, for a shot of another window over the backdrop alone.
    func setWindowHidden(_ hidden: Bool) {
        guard let window else { return }
        isWindowHidden = hidden
        if hidden {
            window.orderOut(nil)
        } else {
            window.makeKeyAndOrderFront(nil)
            backdrop?.order(.below, relativeTo: window.windowNumber)
        }
    }

    /// A 16:10 rect around `frame` with at least the wallpaper margin on every side.
    static func tourRect(around frame: NSRect) -> NSRect {
        var rect = frame.insetBy(dx: -WebsiteCaptures.margin, dy: -WebsiteCaptures.margin)
        let target: CGFloat = 1.6
        let ratio = rect.width / max(rect.height, 1)
        if ratio < target {
            let want = (rect.height * target - rect.width) / 2
            rect = rect.insetBy(dx: -want, dy: 0)
        } else if ratio > target {
            let want = (rect.width / target - rect.height) / 2
            rect = rect.insetBy(dx: 0, dy: -want)
        }
        return NSRect(
            x: (frame.midX - rect.width / 2).rounded(),
            y: (frame.midY - rect.height / 2).rounded(),
            width: rect.width.rounded(),
            height: rect.height.rounded()
        )
    }

    /// Replaces the backdrop window's content with a gradient view, full-screen.
    func setBackdrop<Content: View>(_ content: Content) {
        guard let backdrop, let screen = NSScreen.main else { return }
        backdrop.contentView = NSHostingView(rootView: content.ignoresSafeArea())
        backdrop.contentView?.frame = NSRect(origin: .zero, size: screen.frame.size)
    }

    func show(size: NSSize) {
        guard let screen = NSScreen.main else { return }
        let backdrop = NSWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
        backdrop.isOpaque = true
        backdrop.hasShadow = false
        backdrop.level = .normal
        backdrop.isReleasedWhenClosed = false
        backdrop.contentView = NSHostingView(rootView: BackdropWallpaper(screen: screen).ignoresSafeArea())
        self.backdrop = backdrop

        let window = WindowManager.shared.makeCaptureWindow(size: size)
        self.window = window
        place(window, size: size, on: screen)

        backdrop.orderFront(nil)
        window.makeKeyAndOrderFront(nil)
        backdrop.order(.below, relativeTo: window.windowNumber)
    }

    /// The traffic lights and every control draw active only while the app is active and the
    /// window is key, and macOS refuses activation while the person at the Mac is typing in
    /// another app. So before every capture: activate, and wait for it to have taken, for up
    /// to a few seconds.
    func ensureActive() async {
        guard let window else { return }
        // With the app window out of the way, the app only needs to be active: another
        // window of its own is key then, and ordering this one front would undo the shot.
        if isWindowHidden {
            for _ in 0..<24 where !NSApp.isActive {
                NSApp.activate(ignoringOtherApps: true)
                try? await Task.sleep(for: .milliseconds(250))
            }
            return
        }
        var activated = false
        for _ in 0..<24 {
            if NSApp.isActive, window.isKeyWindow { break }
            activated = true
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
            backdrop?.order(.below, relativeTo: window.windowNumber)
            try? await Task.sleep(for: .milliseconds(250))
        }
        if !(NSApp.isActive && window.isKeyWindow) { WebsiteCaptures.log("could not become the active app") }
        // The traffic lights and controls redraw a beat after activation lands.
        if activated { try? await Task.sleep(for: .milliseconds(450)) }
    }



    func resize(to size: NSSize) {
        guard let window, let screen = window.screen ?? NSScreen.main else { return }
        place(window, size: size, on: screen)
    }

    /// Centred, unless the pointer would sit inside the window: a hovered row or a tooltip
    /// must never end up in a capture, so the window moves out from under it.
    private func place(_ window: NSWindow, size: NSSize, on screen: NSScreen) {
        let visible = screen.visibleFrame
        var frame = NSRect(origin: .zero, size: size)
        frame.origin.x = (screen.frame.midX - size.width / 2).rounded()
        frame.origin.y = (screen.frame.midY - size.height / 2).rounded()
        let pointer = NSEvent.mouseLocation
        if frame.insetBy(dx: -24, dy: -24).contains(pointer) {
            // Wherever it goes, the whole capture, wallpaper margin included, stays on screen.
            let margin = WebsiteCaptures.margin
            let room = visible.insetBy(dx: margin, dy: margin)
            let candidates = [
                NSRect(x: room.minX, y: frame.minY, width: size.width, height: size.height),
                NSRect(x: room.maxX - size.width, y: frame.minY, width: size.width, height: size.height),
                NSRect(x: frame.minX, y: room.minY, width: size.width, height: size.height),
                NSRect(x: frame.minX, y: room.maxY - size.height, width: size.width, height: size.height),
                NSRect(x: room.minX, y: room.minY, width: size.width, height: size.height),
                NSRect(x: room.maxX - size.width, y: room.maxY - size.height, width: size.width, height: size.height),
            ]
            if let clear = candidates.first(where: { room.contains($0) && !$0.insetBy(dx: -24, dy: -24).contains(pointer) }) {
                frame = clear
            }
        }
        window.setFrame(frame, display: true)
    }

    /// Shows `content` in a popover hanging from the composer's model chip, the way the chip
    /// shows the slider. Sized to the content, as the chip's popover is.
    /// Where the last popover was hung, so `holdPopover` can hang it there again.
    private var popoverAnchor: (view: NSView, rect: NSRect)?

    /// Keeps a presented popover up for `seconds`: one that closes on its own meanwhile (a
    /// re-rendered anchor takes its popover down with it) is shown again where it was.
    func holdPopover(_ popover: NSPopover, seconds: Double) async {
        let deadline = Date.now.addingTimeInterval(seconds)
        while Date.now < deadline {
            try? await Task.sleep(for: .milliseconds(200))
            guard !popover.isShown, let anchor = popoverAnchor else { continue }
            WebsiteCaptures.log("popover closed on its own; showing it again")
            popover.show(relativeTo: anchor.rect, of: anchor.view, preferredEdge: anchor.view.isFlipped ? .minY : .maxY)
        }
    }

    func presentPopover<Content: View>(_ content: Content, width: CGFloat, chipOf threadID: UUID? = nil, at frame: CGRect? = nil) -> NSPopover {
        let popover = NSPopover()
        popover.behavior = .applicationDefined
        let probe = NSHostingView(rootView: content.frame(width: width))
        probe.frame = NSRect(x: 0, y: 0, width: width, height: 10)
        let height = probe.fittingSize.height
        popover.setFixedContent(content, size: NSSize(width: width, height: height))
        // A rect of the window's content given outright (the Hydra mark's), the same way
        // the chip's is used below.
        if let frame, let content = window?.contentView {
            let rect = content.isFlipped ? frame : NSRect(x: frame.minX, y: content.bounds.height - frame.maxY, width: frame.width, height: frame.height)
            WebsiteCaptures.log("popover \(width)x\(height) at the given frame \(rect)")
            popoverAnchor = (content, rect)
            popover.show(relativeTo: rect, of: content, preferredEdge: content.isFlipped ? .minY : .maxY)
        }
        // The chip's view is SwiftUI's to make and remake (the chip turns compact and back
        // with the composer's width, and a remade view takes a popover hung from it down),
        // so the popover hangs from the window's own content view instead, at the rect
        // where the thread's chip is right now.
        if !popover.isShown, let threadID, let frame = WebsiteCaptures.modelChipFrames[threadID], let content = window?.contentView {
            let rect = content.isFlipped ? frame : NSRect(x: frame.minX, y: content.bounds.height - frame.maxY, width: frame.width, height: frame.height)
            WebsiteCaptures.log("popover \(width)x\(height) at the chip's frame \(rect)")
            popoverAnchor = (content, rect)
            popover.show(relativeTo: rect, of: content, preferredEdge: content.isFlipped ? .minY : .maxY)
        } else if !popover.isShown, let anchor = WebsiteCaptures.modelChipAnchor?.value, anchor.window === window, !anchor.isHiddenOrHasHiddenAncestor,
           let content = window?.contentView {
            let rect = anchor.convert(anchor.bounds, to: content)
            WebsiteCaptures.log("popover \(width)x\(height) at the chip's rect \(rect)")
            popoverAnchor = (content, rect)
            popover.show(relativeTo: rect, of: content, preferredEdge: content.isFlipped ? .minY : .maxY)
        }
        // A chip that is no longer on screen refuses the popover: it hangs from where the
        // composer's chip sits instead, at the right end of the chat box.
        if !popover.isShown, let view = window?.contentView {
            WebsiteCaptures.log("popover \(width)x\(height) without a chip anchor")
            let rect = NSRect(x: view.bounds.maxX - 150, y: view.isFlipped ? view.bounds.maxY - 44 : 44, width: 10, height: 10)
            popoverAnchor = (view, rect)
            popover.show(relativeTo: rect, of: view, preferredEdge: view.isFlipped ? .minY : .maxY)
        }
        return popover
    }
}

/// What the glass blurs: this Mac's desktop picture, filling the screen the way the desktop
/// shows it. Falls back to the system's default wallpaper.
struct BackdropWallpaper: View {
    let screen: NSScreen
    var alignment: Alignment = .center

    private static let systemDefault = URL(fileURLWithPath: "/System/Library/Wallpapers/.default/DefaultAerial.heic")

    private var image: NSImage? {
        if let url = NSWorkspace.shared.desktopImageURL(for: screen), let image = NSImage(contentsOf: url) { return image }
        return NSImage(contentsOf: Self.systemDefault)
    }

    var body: some View {
        GeometryReader { proxy in
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: proxy.size.width, height: proxy.size.height, alignment: alignment)
                    .clipped()
            } else {
                Color.black
            }
        }
    }
}

// MARK: - Recorder

/// Stills and films of this process's windows as the window server shows them: the run's
/// backdrop, the app window and any popover above it, and nothing of anyone else's.
///
/// Films are single screenshots taken on a 60 Hz clock and encoded as they arrive, not a
/// ScreenCaptureKit stream: while a stream runs, the window server draws the captured app
/// as if it were inactive, gray traffic lights and all, and screenshots never do that.
@MainActor
final class Recorder {
    private let output: URL
    private weak var stage: Stage?
    private var film: FilmRecorder?
    private var loop: Task<Void, Never>?

    init(output: URL, stage: Stage) {
        self.output = output
        self.stage = stage
    }

    /// Photographs `rect` as <name>.png. `focus`, in screen coordinates, names the part of
    /// the scene the tour page should zoom on; it goes into the sidecar.
    func still(_ name: String, _ rect: NSRect, focus: NSRect? = nil) async {
        await stage?.ensureActive()
        WebsiteCaptures.log("still \(name)")
        guard let filter = await Self.filter() else { WebsiteCaptures.log("still \(name): no filter"); return }
        let configuration = Self.configuration(for: rect)
        configuration.captureResolution = .best
        do {
            let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration)
            let representation = NSBitmapImageRep(cgImage: image)
            guard let data = representation.representation(using: .png, properties: [:]) else { return }
            try data.write(to: output.appendingPathComponent("\(name).png"), options: .atomic)
            writeSidecar(for: name, rect: rect, focus: focus)
        } catch {
            WebsiteCaptures.log("still \(name) failed: \(error.localizedDescription)")
        }
    }

    /// Where the app window sits inside a still, beside it as <name>.json: the still's
    /// rect and the window's frame, both in screen points, top-left origin; the script
    /// takes the pixel scale from the PNG's width over the rect's. A still framed around
    /// an open popover is wider than the window, and the tour script cuts the window's
    /// inside out of it; guessing that box from the window's size alone cut the
    /// sidebar's edge off the welcome page.
    ///
    /// `focus`, when given, is written the same way as `window`: the part of the still the
    /// tour page zooms on, top-left origin, points.
    private func writeSidecar(for name: String, rect: NSRect, focus: NSRect?) {
        guard let window = stage?.windowFrame else { return }
        var sidecar: [String: [Double]] = [
            "rect": [rect.minX, rect.minY, rect.width, rect.height],
            "window": [window.minX - rect.minX, rect.maxY - window.maxY, window.width, window.height],
        ]
        if let focus {
            sidecar["focus"] = [focus.minX - rect.minX, rect.maxY - focus.maxY, focus.width, focus.height]
        }
        guard let data = try? JSONSerialization.data(withJSONObject: sidecar, options: [.sortedKeys]) else { return }
        try? data.write(to: output.appendingPathComponent("\(name).json"), options: .atomic)
    }

    /// Starts filming `rect` into films/<name>.mov at 60 fps.
    func startFilm(_ name: String, _ rect: NSRect) async {
        await stopFilm()
        await stage?.ensureActive()
        WebsiteCaptures.log("film \(name): start")
        guard let filter = await Self.filter() else { return }
        let configuration = Self.configuration(for: rect)
        configuration.captureResolution = .best
        let url = output.appendingPathComponent("films/\(name).mov")
        try? FileManager.default.removeItem(at: url)
        let recorder: FilmRecorder
        do {
            recorder = try FilmRecorder(url: url, width: configuration.width, height: configuration.height)
        } catch {
            WebsiteCaptures.log("film \(name): failed to start: \(error.localizedDescription)")
            return
        }
        film = recorder
        let interval = 1.0 / 60
        loop = Task { [weak self] in
            let started = Date.now
            var next = started
            while let self, !Task.isCancelled {
                let shot = Date.now
                if let buffer = try? await SCScreenshotManager.captureSampleBuffer(contentFilter: filter, configuration: configuration) {
                    recorder.append(buffer, at: shot.timeIntervalSince(started))
                }
                _ = self
                next = next.addingTimeInterval(interval)
                let wait = next.timeIntervalSinceNow
                if wait > 0 {
                    try? await Task.sleep(for: .seconds(wait))
                } else {
                    next = Date.now
                }
            }
        }
    }

    func stopFilm() async {
        loop?.cancel()
        loop = nil
        guard let film else { return }
        self.film = nil
        WebsiteCaptures.log("film: stopping")
        await film.finish()
        WebsiteCaptures.log("film: stopped, \(film.frames) frames")
    }

    func finish() async {
        await stopFilm()
    }

    private static func filter() async -> SCContentFilter? {
        guard let content = try? await SCShareableContent.currentProcess,
              let display = content.displays.first(where: { $0.displayID == CGMainDisplayID() }) ?? content.displays.first
        else { return nil }
        return SCContentFilter(display: display, including: content.windows)
    }

    private static func configuration(for rect: NSRect) -> SCStreamConfiguration {
        let configuration = SCStreamConfiguration()
        let scale = NSScreen.main?.backingScaleFactor ?? 2
        let displayHeight = CGDisplayBounds(CGMainDisplayID()).height
        configuration.sourceRect = CGRect(x: rect.minX, y: displayHeight - rect.maxY, width: rect.width, height: rect.height)
        // Even dimensions, for the video encoders.
        configuration.width = Int(rect.width * scale) & ~1
        configuration.height = Int(rect.height * scale) & ~1
        configuration.scalesToFit = false
        configuration.showsCursor = false
        return configuration
    }
}

/// Encodes screenshots into an HEVC master as they arrive, on its own queue, stamped with
/// the moment each was taken.
private final class FilmRecorder: @unchecked Sendable {
    private let queue = DispatchQueue(label: "droppycode.website-captures.film", qos: .userInteractive)
    private let writer: AVAssetWriter
    private let input: AVAssetWriterInput
    private let adaptor: AVAssetWriterInputPixelBufferAdaptor
    private var started = false
    private var last = CMTime.zero
    private(set) var frames = 0

    init(url: URL, width: Int, height: Int) throws {
        writer = try AVAssetWriter(outputURL: url, fileType: .mov)
        input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.hevc,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: 80_000_000,
                AVVideoExpectedSourceFrameRateKey: 60,
                AVVideoMaxKeyFrameIntervalKey: 60,
                AVVideoAllowFrameReorderingKey: false,
            ],
        ])
        input.expectsMediaDataInRealTime = true
        adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
        ])
        writer.add(input)
        guard writer.startWriting() else { throw writer.error ?? CocoaError(.fileWriteUnknown) }
        writer.startSession(atSourceTime: .zero)
        started = true
    }

    func append(_ buffer: CMSampleBuffer, at seconds: TimeInterval) {
        let frame = Frame(buffer: buffer)
        queue.async { [self] in
            guard let pixelBuffer = CMSampleBufferGetImageBuffer(frame.buffer), input.isReadyForMoreMediaData else { return }
            let time = CMTime(seconds: seconds, preferredTimescale: 6_000)
            guard time > last else { return }
            if adaptor.append(pixelBuffer, withPresentationTime: time) {
                last = time
                frames += 1
            }
        }
    }

    func finish() async {
        await withCheckedContinuation { continuation in
            queue.async { [self] in
                input.markAsFinished()
                writer.finishWriting { continuation.resume() }
            }
        }
    }

    private struct Frame: @unchecked Sendable {
        let buffer: CMSampleBuffer
    }
}
