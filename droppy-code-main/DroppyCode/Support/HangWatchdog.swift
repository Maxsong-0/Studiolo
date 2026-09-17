import Foundation
import os

/// Catches the main thread standing still, and keeps what it was doing.
///
/// A freeze that ends in a force quit leaves nothing behind: the Dock asks spindump for a
/// hang report, spindump declines for a development build, and the crash reporter never
/// sees a process that was killed rather than crashed. So the app watches itself. A thread
/// of its own pings the main thread twice a second; when a ping goes unanswered for
/// `stallThreshold`, it runs `sample` on the process and writes every thread's stack to
/// `~/Library/Logs/Droppy Code/hang-<date>.txt`. `sample` reads the stacks from outside,
/// so a main thread stuck in a loop or a lock is exactly what it captures. One report per
/// stall; the next stall gets one of its own.
///
/// This costs a timer on a background thread and a block on the main queue every half
/// second, nothing more, so it stays on in every build.
enum HangWatchdog {
    private static let log = Logger(subsystem: "iordv.droppycode", category: "hang")

    /// How long the main thread may go without answering before it counts as stuck. Long
    /// enough that a heavy but finite piece of work (a big diff, a slow layout pass) does
    /// not set off a report, short enough that a real freeze is caught well before the
    /// user reaches for Force Quit.
    private static let stallThreshold: TimeInterval = 4

    /// How long `sample` watches the process. Short: a stuck thread does not move, and a
    /// long sample only delays the write.
    private static let sampleSeconds = 2

    /// Reports kept before the oldest is dropped, so a machine that freezes often does not
    /// fill its logs folder.
    private static let keptReports = 10

    private static let state = OSAllocatedUnfairLock(initialState: State())

    private struct State {
        var lastPong = Date()
        var reportedThisStall = false
        var started = false
    }

    static func start() {
        let shouldStart = state.withLock { state -> Bool in
            if state.started { return false }
            state.started = true
            state.lastPong = Date()
            return true
        }
        guard shouldStart else { return }
        let thread = Thread {
            while true {
                Thread.sleep(forTimeInterval: 0.5)
                ping()
                check()
            }
        }
        thread.name = "hang-watchdog"
        thread.qualityOfService = .utility
        thread.start()
    }

    private static func ping() {
        DispatchQueue.main.async {
            state.withLock { state in
                state.lastPong = Date()
                state.reportedThisStall = false
            }
        }
    }

    private static func check() {
        let stalledFor = state.withLock { state -> TimeInterval? in
            let stalled = Date().timeIntervalSince(state.lastPong)
            guard stalled >= stallThreshold, !state.reportedThisStall else { return nil }
            state.reportedThisStall = true
            return stalled
        }
        guard let stalledFor else { return }
        log.error("Main thread unresponsive for \(stalledFor, format: .fixed(precision: 1))s, sampling")
        writeReport(stalledFor: stalledFor)
    }

    /// Runs `sample` against this process and writes what it saw. Runs on the watchdog's
    /// own thread, never the main one, so a stuck main thread cannot stop the report.
    private static func writeReport(stalledFor: TimeInterval) {
        let folder = reportsFolder()
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let stamp = ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-")
        let file = folder.appendingPathComponent("hang-\(stamp).txt")

        var header = "Droppy Code main thread unresponsive for \(String(format: "%.1f", stalledFor))s\n"
        header += "Version \(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?") (\(Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "?"))\n"
        header += "Sampled at \(Date())\n\n"

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/sample")
        process.arguments = [String(ProcessInfo.processInfo.processIdentifier), String(sampleSeconds), "-mayDie"]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = output
        var body = ""
        do {
            try process.run()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            body = String(decoding: data, as: UTF8.self)
            if process.terminationStatus != 0 {
                body = "sample exited with status \(process.terminationStatus)\n\n" + body
            }
        } catch {
            body = "sample could not run: \(error.localizedDescription)\n"
        }
        try? (header + body).write(to: file, atomically: true, encoding: .utf8)
        log.error("Hang report written to \(file.path, privacy: .public)")
        trimReports(in: folder)
    }

    private static func reportsFolder() -> URL {
        let library = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask).first ?? URL(fileURLWithPath: NSHomeDirectory() + "/Library")
        return library.appendingPathComponent("Logs/Droppy Code", isDirectory: true)
    }

    private static func trimReports(in folder: URL) {
        guard let names = try? FileManager.default.contentsOfDirectory(atPath: folder.path) else { return }
        let reports = names.filter { $0.hasPrefix("hang-") && $0.hasSuffix(".txt") }.sorted()
        guard reports.count > keptReports else { return }
        for name in reports.prefix(reports.count - keptReports) {
            try? FileManager.default.removeItem(at: folder.appendingPathComponent(name))
        }
    }
}
