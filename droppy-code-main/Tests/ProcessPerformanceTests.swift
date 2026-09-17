import Foundation
import Testing

@Test func cancelledShellStopsWork() async throws {
    let started = ContinuousClock.now
    let task = Task { try await Shell.run(URL(fileURLWithPath: "/bin/sleep"), ["2"], environment: [:], timeout: 5) }
    try await Task.sleep(for: .milliseconds(100))
    task.cancel()
    await #expect(throws: CancellationError.self) { try await task.value }
    #expect(started.duration(to: .now) < .seconds(1.5))
}

@Test func preCancelledShellDoesNotStart() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let task = Task {
        withUnsafeCurrentTask { $0?.cancel() }
        return try await Shell.run(URL(fileURLWithPath: "/usr/bin/touch"), [directory.path], environment: [:])
    }
    await #expect(throws: CancellationError.self) { try await task.value }
    #expect(!FileManager.default.fileExists(atPath: directory.path))
}

@Test func shellDrainsBothPipes() async throws {
    let result = try await Shell.run(URL(fileURLWithPath: "/bin/sh"), ["-c", "printf stdout; printf stderr >&2"], environment: [:])
    #expect(result.status == 0)
    #expect(result.output == "stdout")
    #expect(result.errorOutput == "stderr")
}

@Test @MainActor func rpcCancellationReleasesPendingRequest() async throws {
    let process = StdioProcess(executable: URL(fileURLWithPath: "/bin/cat"), arguments: [], directory: FileManager.default.temporaryDirectory, environment: [:])
    let connection = JSONRPCConnection(process: process, sendsVersion: true)
    connection.onRequest = { _, _, _ in }
    try connection.start()
    defer { connection.close() }
    let request = Task { try await connection.request("test/wait") }
    try await Task.sleep(for: .milliseconds(20))
    request.cancel()
    let deadline = Task {
        try? await Task.sleep(for: .milliseconds(300))
        if !Task.isCancelled { connection.close() }
    }
    defer { deadline.cancel() }
    await #expect(throws: CancellationError.self) { try await request.value }
}

@Test(arguments: [StdioProcess.Framing.lines, .contentLength]) func stdioPreservesMessageOrder(framing: StdioProcess.Framing) async throws {
    let process = StdioProcess(executable: URL(fileURLWithPath: "/bin/cat"), arguments: [], directory: FileManager.default.temporaryDirectory, environment: [:], framing: framing)
    try process.start()
    defer { process.terminate() }
    for value in 0..<100 { process.send(["sequence": .int(value), "text": "α👩🏽‍💻"]) }
    var iterator = process.messages.makeAsyncIterator()
    for value in 0..<100 {
        let message = await iterator.next()
        #expect(message?["sequence"]?.int == value)
        #expect(message?["text"]?.string == "α👩🏽‍💻")
    }
}

@Test(arguments: [false, true]) func cancelledShellStopsItsChildProcess(ignoresTermination: Bool) async throws {
    let marker = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: marker) }
    let script = (ignoresTermination ? "trap '' TERM; " : "") + "sleep 5 & echo $! > \"$1\"; wait"
    let task = Task { try await Shell.run(URL(fileURLWithPath: "/bin/sh"), ["-c", script, "test", marker.path], environment: [:]) }
    let deadline = ContinuousClock.now.advanced(by: .seconds(2))
    while !FileManager.default.fileExists(atPath: marker.path), ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(5))
    }
    let pidText = try String(contentsOf: marker, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
    let pid = try #require(Int32(pidText))
    defer { if kill(pid, 0) == 0 { kill(pid, SIGKILL) } }
    task.cancel()
    await #expect(throws: CancellationError.self) { try await task.value }
    let stopped = ContinuousClock.now.advanced(by: .milliseconds(300))
    while kill(pid, 0) == 0, ContinuousClock.now < stopped {
        try await Task.sleep(for: .milliseconds(5))
    }
    #expect(kill(pid, 0) != 0)
}

@Test func cancellationDuringOutputDrainThrowsPromptly() async throws {
    let marker = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: marker) }
    let task = Task { try await Shell.run(URL(fileURLWithPath: "/bin/sh"), ["-c", "sleep 5 & echo $! > \"$1\"; exit 0", "test", marker.path], environment: [:]) }
    let deadline = ContinuousClock.now.advanced(by: .seconds(2))
    while !FileManager.default.fileExists(atPath: marker.path), ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(5))
    }
    let pidText = try String(contentsOf: marker, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
    let pid = try #require(Int32(pidText))
    defer { if kill(pid, 0) == 0 { kill(pid, SIGKILL) } }
    try await Task.sleep(for: .milliseconds(100))
    let start = ContinuousClock.now
    task.cancel()
    await #expect(throws: CancellationError.self) { try await task.value }
    #expect(start.duration(to: .now) < .milliseconds(500))
}

@Test func shellTimeoutEscalatesForIgnoredTermination() async throws {
    let start = ContinuousClock.now
    let result = try await Shell.run(URL(fileURLWithPath: "/bin/sh"), ["-c", "trap '' TERM; exec sleep 5"], environment: [:], timeout: 0.05)
    #expect(result.status != 0)
    #expect(start.duration(to: .now) < .seconds(2))
}

@Test func zeroShellTimeoutStillArmsAfterLaunch() async throws {
    let start = ContinuousClock.now
    let result = try await Shell.run(URL(fileURLWithPath: "/bin/sleep"), ["2"], environment: [:], timeout: 0)
    #expect(result.status != 0)
    #expect(start.duration(to: .now) < .seconds(1.5))
}

@main struct ProcessPerformanceTests {
    static func main() async {
        let result: CInt = await Testing.__swiftPMEntryPoint()
        exit(result)
    }
}

@Test func boundedShellOutputKeepsTheTail() async throws {
    let result = try await Shell.run(URL(fileURLWithPath: "/bin/sh"), ["-c", "i=0; while [ $i -lt 20000 ]; do echo line-$i; i=$((i+1)); done; echo tail-marker >&2"], environment: [:], outputLimit: 4_000)
    #expect(result.succeeded)
    #expect(result.stdout.count <= 8_000)
    #expect(result.output.hasSuffix("line-19999\n"))
    #expect(result.errorOutput == "tail-marker\n")
    let whole = try await Shell.run(URL(fileURLWithPath: "/bin/sh"), ["-c", "i=0; while [ $i -lt 3000 ]; do echo line-$i; i=$((i+1)); done"], environment: [:])
    #expect(whole.output.split(separator: "\n").count == 3000)
}
