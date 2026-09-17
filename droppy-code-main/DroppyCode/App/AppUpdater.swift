import AppKit
import Foundation
import Security

/// Where the update story's slider is between its phases. The phase boundaries are the
/// milestones the slider ticks past: download done, verified, handed off, installed.
enum UpdateProgressMilestones {
    /// What the head creeps toward while the connection is being made.
    static let preparingCeiling = 0.05
    static let downloadEnd = 0.55
    static let verifyEnd = 0.70
    /// What the head creeps toward while the image is mounted and the app staged.
    static let stagingCeiling = 0.82
    /// The installer has the update; the app quits.
    static let handoffEnd = 0.86
    /// Crossed by the post-relaunch sweep to 100%.
    static let installDot = 0.96
    static let dots = [downloadEnd, verifyEnd, handoffEnd, installDot]
}

/// The one source of truth for the update story the About page tells: the installer reports
/// real phases and byte progress here, and the version pill renders it as the slider.
///
/// The track only ever moves forward. Phases without byte progress (connecting, staging) creep
/// toward their ceiling so the head never freezes, and never reach the next milestone on their own.
@MainActor
@Observable
final class UpdateInstallProgress {
    static let shared = UpdateInstallProgress()

    /// The raw target, in its own observable so a download's many reports a second re-render
    /// only the slider that chases it, not the page around it.
    @MainActor
    @Observable
    final class FractionTarget {
        fileprivate(set) var value = 0.0
    }

    enum Phase: Equatable {
        case idle
        /// Connecting; no bytes yet.
        case preparing
        case downloading
        /// The image is mounted and the app in it checked.
        case verifying
        /// The verified app is copied next to the installed one.
        case staging
        /// The installer owns the update; the app is about to quit.
        case handoff
        /// After the relaunch: the sweep to 100%.
        case celebrating
        /// Briefly, before the pill returns to its ready face.
        case failed
    }

    private(set) var phase: Phase = .idle
    let fractionTarget = FractionTarget()
    var fraction: Double { fractionTarget.value }

    /// Set at launch when this process is the relaunch after an install; the About page
    /// consumes it and plays the finishing sweep.
    private(set) var hasPendingCelebration = false

    @ObservationIgnored private var creepTask: Task<Void, Never>?
    @ObservationIgnored private var creepCeiling = 0.0
    @ObservationIgnored private var creepRate = 0.0
    @ObservationIgnored private var failureReset: Task<Void, Never>?

    private init() {}

    /// While the pill shows the slider rather than a face.
    var showsProgressUI: Bool {
        switch phase {
        case .idle, .failed: false
        default: true
        }
    }

    var isInstallRunning: Bool {
        switch phase {
        case .preparing, .downloading, .verifying, .staging, .handoff: true
        default: false
        }
    }

    // MARK: - Reported by the installer

    func begin() {
        cancelFailureReset()
        phase = .preparing
        fractionTarget.value = 0.004
        Haptics.perform(.generic)
        startCreep(toward: UpdateProgressMilestones.preparingCeiling, rate: 0.05)
    }

    /// Byte progress in 0...1. Zero or less means the size is unknown; the head then creeps
    /// toward mid-download so it keeps visibly moving.
    func noteDownloadProgress(_ downloadFraction: Double) {
        if phase == .preparing {
            phase = .downloading
        } else if phase != .downloading {
            return
        }
        guard downloadFraction > 0 else {
            startCreep(toward: 0.50, rate: 0.012)
            return
        }
        stopCreep()
        let start = UpdateProgressMilestones.preparingCeiling
        let end = UpdateProgressMilestones.downloadEnd
        apply(start + (end - start) * min(downloadFraction, 1))
    }

    func beginVerification() {
        guard isInstallRunning else { return }
        stopCreep()
        phase = .verifying
        apply(UpdateProgressMilestones.downloadEnd)
        startCreep(toward: UpdateProgressMilestones.verifyEnd - 0.02, rate: 0.12)
    }

    func beginStaging() {
        guard isInstallRunning else { return }
        stopCreep()
        phase = .staging
        apply(UpdateProgressMilestones.verifyEnd)
        startCreep(toward: UpdateProgressMilestones.stagingCeiling, rate: 0.10)
    }

    func noteHandoff() {
        guard isInstallRunning else { return }
        stopCreep()
        phase = .handoff
        apply(UpdateProgressMilestones.handoffEnd)
    }

    /// A step failed. The pill goes back to its ready face; the installer explains why.
    func fail() {
        guard isInstallRunning else { return }
        stopCreep()
        phase = .failed
        Haptics.perform(.levelChange)
        failureReset = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(600))
            guard let self, !Task.isCancelled, phase == .failed else { return }
            resetToIdle()
        }
    }

    // MARK: - After the relaunch

    func armCelebration() {
        hasPendingCelebration = true
    }

    /// Mounts the slider at the handoff position; the page then sweeps it home.
    func consumePendingCelebration() -> Bool {
        guard hasPendingCelebration else { return false }
        hasPendingCelebration = false
        stopCreep()
        cancelFailureReset()
        phase = .celebrating
        fractionTarget.value = UpdateProgressMilestones.handoffEnd
        return true
    }

    func finishCelebrationSweep() {
        guard phase == .celebrating else { return }
        apply(1, firesHaptics: false)
    }

    func completeCelebration() {
        guard phase == .celebrating else { return }
        resetToIdle()
    }

    // MARK: - Internals

    private func resetToIdle() {
        stopCreep()
        cancelFailureReset()
        phase = .idle
        fractionTarget.value = 0
    }

    private func cancelFailureReset() {
        failureReset?.cancel()
        failureReset = nil
    }

    /// Moves the head forward, never back, and ticks once per milestone crossed.
    private func apply(_ newValue: Double, firesHaptics: Bool = true) {
        let clamped = min(max(newValue, 0), 1)
        guard clamped > fraction else { return }
        let previous = fraction
        fractionTarget.value = clamped
        guard firesHaptics else { return }
        for dot in UpdateProgressMilestones.dots where previous < dot && clamped >= dot {
            Haptics.perform(dot == UpdateProgressMilestones.handoffEnd ? .levelChange : .alignment)
        }
    }

    private func startCreep(toward ceiling: Double, rate: Double) {
        creepCeiling = ceiling
        creepRate = rate
        guard creepTask == nil else { return }
        creepTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(125))
                guard let self, !Task.isCancelled else { return }
                let remaining = creepCeiling - fraction
                guard remaining > 0.0005 else {
                    creepTask = nil
                    return
                }
                apply(fraction + remaining * creepRate)
            }
        }
    }

    private func stopCreep() {
        creepTask?.cancel()
        creepTask = nil
    }
}

/// Downloads a release, checks it is Droppy's, and hands it to a small installer that swaps
/// it in once the app has quit and relaunches it.
///
/// The image is mounted and its app copied out; the copy is what gets checked (its Developer
/// ID signature must be the team's and its identifier this app's, its version the one the
/// user accepted, and Gatekeeper must accept it) and what gets installed. The running app
/// cannot replace itself, so a shell script waits for it to quit, moves the old bundle aside,
/// moves the new one into its place, clears the quarantine the check already covered, and
/// opens it. A marker in the defaults lets the relaunched build finish the story.
@MainActor
final class AppUpdater {
    static let shared = AppUpdater()

    nonisolated static let teamID = "NARHG44L48"
    private static let relaunchVersionKey = "appUpdater_pendingRelaunchVersion"
    /// How long the app may stay alive after the handoff before the installer is called off.
    private static let handoffTimeout: TimeInterval = 25

    private var installTask: Task<Void, Never>?
    private var progress: UpdateInstallProgress { .shared }

    private init() {}

    /// The relaunch after an install: true once, when the running version is the one the
    /// install promised. A stale marker (the swap failed) is dropped silently; the About page
    /// then simply offers the update again.
    func consumeRelaunchMarker() -> Bool {
        let defaults = UserDefaults.standard
        guard let pending = defaults.string(forKey: Self.relaunchVersionKey) else { return false }
        defaults.removeObject(forKey: Self.relaunchVersionKey)
        return pending == AppInfo.version
    }

    func install(_ update: AvailableUpdate) {
        guard installTask == nil, !AppInfo.isDevelopment else { return }
        installTask = Task { [self] in
            defer { installTask = nil }
            progress.begin()
            do {
                let image = try await download(update.downloadURL)
                progress.beginVerification()
                let staged = try await stage(image: image, expectedVersion: update.version)
                progress.beginStaging()
                try handOff(staged: staged, version: update.version)
            } catch {
                progress.fail()
                await MainActor.run {
                    let alert = NSAlert()
                    alert.messageText = "Update Failed"
                    alert.informativeText = error.localizedDescription
                    alert.alertStyle = .warning
                    alert.runModal()
                }
            }
        }
    }

    // MARK: - Download

    private func download(_ url: URL) async throws -> URL {
        let destination = FileManager.default.temporaryDirectory
            .appending(path: "DroppyCodeUpdate-\(UUID().uuidString).dmg")
        let downloader = Downloader { fraction in
            Task { @MainActor in UpdateInstallProgress.shared.noteDownloadProgress(fraction) }
        }
        try await downloader.download(url, to: destination)
        return destination
    }

    // MARK: - Verify and stage

    private func stage(image: URL, expectedVersion: String) async throws -> URL {
        let staging = FileManager.default.temporaryDirectory
            .appending(path: "DroppyCodeUpdate-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let mount = staging.appending(path: "image", directoryHint: .isDirectory)
        let attach = try await Shell.run(
            URL(filePath: "/usr/bin/hdiutil"),
            ["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint", mount.path, image.path],
            timeout: 120
        )
        guard attach.succeeded else {
            throw UpdateInstallError("The downloaded disk image could not be opened. \(attach.failureMessage)")
        }
        // The copy is what gets checked and installed; the image is let go as soon as it is made.
        let staged = staging.appending(path: Bundle.main.bundleURL.lastPathComponent, directoryHint: .isDirectory)
        let copy: ShellResult
        do {
            let contents = (try? FileManager.default.contentsOfDirectory(at: mount, includingPropertiesForKeys: nil)) ?? []
            guard let source = contents.first(where: { $0.pathExtension == "app" }) else {
                throw UpdateInstallError("The disk image holds no app.")
            }
            copy = try await Shell.run(URL(filePath: "/usr/bin/ditto"), [source.path, staged.path], timeout: 300)
            _ = try? await Shell.run(URL(filePath: "/usr/bin/hdiutil"), ["detach", mount.path, "-force"], timeout: 60)
            try? FileManager.default.removeItem(at: image)
        } catch {
            _ = try? await Shell.run(URL(filePath: "/usr/bin/hdiutil"), ["detach", mount.path, "-force"], timeout: 60)
            try? FileManager.default.removeItem(at: image)
            throw error
        }
        guard copy.succeeded else {
            throw UpdateInstallError("The update could not be copied. \(copy.failureMessage)")
        }
        try Self.verifySignature(of: staged)
        let bundle = Bundle(url: staged)
        let version = bundle?.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
        guard let version, let actual = AppVersion(version), let expected = AppVersion(expectedVersion), actual == expected else {
            throw UpdateInstallError("The downloaded app is version \(version ?? "unknown"), not \(expectedVersion).")
        }
        let gatekeeper = try await Shell.run(URL(filePath: "/usr/sbin/spctl"), ["--assess", "--type", "execute", staged.path], timeout: 120)
        guard gatekeeper.succeeded else {
            throw UpdateInstallError("Gatekeeper did not accept the downloaded app. \(gatekeeper.failureMessage)")
        }
        return staged
    }

    /// The app must be signed with the team's Developer ID certificate and carry this app's
    /// identifier: anything else is not a Droppy Code release, whatever it is called.
    nonisolated static func verifySignature(of appURL: URL) throws {
        let bundleID = Bundle.main.bundleIdentifier ?? "iordv.droppycode"
        var staticCode: SecStaticCode?
        guard SecStaticCodeCreateWithPath(appURL as CFURL, [], &staticCode) == errSecSuccess, let code = staticCode else {
            throw UpdateInstallError("The downloaded app is not signed.")
        }
        let text = """
        anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] \
        and certificate leaf[field.1.2.840.113635.100.6.1.13] \
        and certificate leaf[subject.OU] = "\(teamID)" and identifier "\(bundleID)"
        """
        var requirement: SecRequirement?
        guard SecRequirementCreateWithString(text as CFString, [], &requirement) == errSecSuccess, let requirement else {
            throw UpdateInstallError("The signature requirement could not be built.")
        }
        var error: Unmanaged<CFError>?
        let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSCheckNestedCode | kSecCSStrictValidate)
        let status = SecStaticCodeCheckValidityWithErrors(code, flags, requirement, &error)
        guard status == errSecSuccess else {
            let reason = error?.takeRetainedValue().localizedDescription ?? "OSStatus \(status)"
            throw UpdateInstallError("The downloaded app is not signed by Droppy. \(reason)")
        }
    }

    // MARK: - Hand off

    /// Starts the installer and quits. The installer waits for this process to end before it
    /// touches the installed app.
    private func handOff(staged: URL, version: String) throws {
        let destination = Bundle.main.bundleURL
        let parent = destination.deletingLastPathComponent()
        guard FileManager.default.isWritableFile(atPath: parent.path) else {
            throw UpdateInstallError("Droppy Code cannot replace itself in \(parent.path). Move it to your Applications folder and try again.")
        }
        let script = staged.deletingLastPathComponent().appending(path: "install.sh")
        try Self.installerScript.write(to: script, atomically: true, encoding: .utf8)
        let log = staged.deletingLastPathComponent().appending(path: "install.log")
        FileManager.default.createFile(atPath: log.path, contents: nil)
        let process = Process()
        process.executableURL = URL(filePath: "/bin/sh")
        process.arguments = [
            script.path,
            String(ProcessInfo.processInfo.processIdentifier),
            staged.path,
            destination.path,
        ]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = try FileHandle(forWritingTo: log)
        process.standardError = process.standardOutput
        try process.run()
        UserDefaults.standard.set(version, forKey: Self.relaunchVersionKey)
        progress.noteHandoff()
        // If the quit is refused the installer gives up on its own; the pill goes back too.
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(Self.handoffTimeout))
            guard UpdateInstallProgress.shared.phase == .handoff else { return }
            UserDefaults.standard.removeObject(forKey: Self.relaunchVersionKey)
            UpdateInstallProgress.shared.fail()
        }
        Task { @MainActor in
            // Let the handoff tick land before the windows go.
            try? await Task.sleep(for: .milliseconds(350))
            NSApp.terminate(nil)
        }
    }

    /// Waits for the app to quit, swaps the bundles, and opens the new one. A swap that fails
    /// puts the old app back. Everything it was given is cleaned up at the end.
    private static let installerScript = """
    #!/bin/sh
    # Droppy Code update installer: waits for the app to quit, swaps in the update, relaunches.
    trap '' HUP
    PID="$1"; NEW="$2"; DEST="$3"
    STAGING="$(dirname "$NEW")"
    i=0
    while kill -0 "$PID" 2>/dev/null; do
      sleep 0.25
      i=$((i + 1))
      if [ "$i" -gt 240 ]; then echo "The app did not quit"; rm -rf "$STAGING"; exit 1; fi
    done
    BACKUP="$(dirname "$DEST")/.$(basename "$DEST").previous"
    rm -rf "$BACKUP"
    if [ -e "$DEST" ]; then
      mv "$DEST" "$BACKUP" || { echo "Could not move the installed app aside"; rm -rf "$STAGING"; exit 1; }
    fi
    if mv "$NEW" "$DEST"; then
      rm -rf "$BACKUP"
    else
      echo "Could not move the update into place"
      [ -e "$BACKUP" ] && mv "$BACKUP" "$DEST"
      rm -rf "$STAGING"
      exit 1
    fi
    xattr -dr com.apple.quarantine "$DEST" 2>/dev/null
    open "$DEST"
    rm -rf "$STAGING"
    """
}

struct UpdateInstallError: LocalizedError {
    var message: String
    var errorDescription: String? { message }

    init(_ message: String) {
        self.message = message
    }
}

/// One download to a file, with byte progress. Kept as a delegate so a large image streams
/// to disk instead of through memory.
private final class Downloader: NSObject, URLSessionDownloadDelegate, @unchecked Sendable {
    private let onProgress: @Sendable (Double) -> Void
    private var continuation: CheckedContinuation<URL, Error>?
    private var destination: URL?
    private let lock = NSLock()

    init(onProgress: @escaping @Sendable (Double) -> Void) {
        self.onProgress = onProgress
    }

    func download(_ url: URL, to destination: URL) async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 30
        configuration.timeoutIntervalForResource = 1800
        configuration.waitsForConnectivity = false
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        defer { session.finishTasksAndInvalidate() }
        self.destination = destination
        var request = URLRequest(url: url)
        request.setValue("Droppy Code/\(AppInfo.version)", forHTTPHeaderField: "User-Agent")
        _ = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<URL, Error>) in
            lock.withLock { self.continuation = continuation }
            session.downloadTask(with: request).resume()
        }
    }

    private func finish(_ result: Result<URL, Error>) {
        let continuation = lock.withLock { () -> CheckedContinuation<URL, Error>? in
            defer { self.continuation = nil }
            return self.continuation
        }
        continuation?.resume(with: result)
    }

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64, totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        guard totalBytesExpectedToWrite > 0 else {
            onProgress(0)
            return
        }
        onProgress(Double(totalBytesWritten) / Double(totalBytesExpectedToWrite))
    }

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        // The file goes away when this returns, so it moves now.
        guard let destination else {
            finish(.failure(UpdateInstallError("The download had nowhere to go.")))
            return
        }
        if let http = downloadTask.response as? HTTPURLResponse, !(200...299).contains(http.statusCode) {
            finish(.failure(UpdateInstallError("The download failed with HTTP \(http.statusCode).")))
            return
        }
        do {
            try? FileManager.default.removeItem(at: destination)
            try FileManager.default.moveItem(at: location, to: destination)
            finish(.success(destination))
        } catch {
            finish(.failure(error))
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error {
            finish(.failure(UpdateInstallError("The update could not be downloaded. \(error.localizedDescription)")))
        }
    }
}
