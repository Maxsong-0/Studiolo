import Foundation

/// A child process that exchanges JSON messages over stdio, one per line or
/// behind a `Content-Length` header.
///
/// Reading, parsing and writing happen on private queues. Parsed messages are
/// delivered through `messages`, which finishes once stdout closes or the
/// process exits.
final class StdioProcess: @unchecked Sendable {
    typealias Framing = StdioFramer.Framing

    let messages: AsyncStream<JSONValue>

    private let framing: Framing
    private let continuation: AsyncStream<JSONValue>.Continuation
    private let process = Process()
    private let stdin = Pipe()
    private let stdout = Pipe()
    private let stderr = Pipe()
    private let writeQueue = DispatchQueue(label: "droppycode.stdio.write")
    private let lock = NSLock()
    private var framer: StdioFramer
    private var errorBuffer = Data()
    private var exitStatus: Int32?
    private var exitWaiters: [CheckedContinuation<Int32, Never>] = []

    init(executable: URL, arguments: [String], directory: URL, environment: [String: String], framing: Framing = .lines) {
        let stream = AsyncStream.makeStream(of: JSONValue.self)
        messages = stream.stream
        continuation = stream.continuation
        framer = StdioFramer(framing: framing)
        self.framing = framing
        process.executableURL = executable
        process.arguments = arguments
        process.currentDirectoryURL = directory
        process.environment = environment
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = stderr
    }

    var isRunning: Bool { process.isRunning }

    /// The last few kilobytes written to stderr, used when a provider fails.
    var errorTail: String {
        lock.withLock { String(decoding: errorBuffer.suffix(4_000), as: UTF8.self) }
    }

    func start() throws {
        stdout.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard let self else {
                handle.readabilityHandler = nil
                return
            }
            if data.isEmpty {
                handle.readabilityHandler = nil
                self.finishOutput()
            } else {
                self.consume(data)
            }
        }
        stderr.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard let self, !data.isEmpty else {
                handle.readabilityHandler = nil
                return
            }
            self.appendError(data)
        }
        process.terminationHandler = { [weak self] process in
            self?.didExit(process.terminationStatus)
        }
        do {
            try process.run()
        } catch {
            stdout.fileHandleForReading.readabilityHandler = nil
            stderr.fileHandleForReading.readabilityHandler = nil
            continuation.finish()
            throw error
        }
    }

    func send(_ message: JSONValue) {
        let handle = stdin.fileHandleForWriting
        let framing = framing
        writeQueue.async {
            let body = message.data()
            let payload: Data
            switch framing {
            case .lines:
                payload = body + Data([0x0A])
            case .contentLength:
                payload = Data("Content-Length: \(body.count)\r\n\r\n".utf8) + body
            }
            try? handle.write(contentsOf: payload)
        }
    }

    /// Writes raw bytes to the process's stdin: a prompt piped to a one-shot CLI.
    func write(_ data: Data) {
        let handle = stdin.fileHandleForWriting
        writeQueue.async {
            try? handle.write(contentsOf: data)
        }
    }

    /// Closes stdin, for a process that reads its input to the end before it starts.
    func closeInput() {
        let handle = stdin.fileHandleForWriting
        writeQueue.async { try? handle.close() }
    }

    func terminate() {
        let handle = stdin.fileHandleForWriting
        writeQueue.async { try? handle.close() }
        guard process.isRunning else { return }
        process.terminate()
        let process = process
        DispatchQueue.global().asyncAfter(deadline: .now() + 3) {
            if process.isRunning { kill(process.processIdentifier, SIGKILL) }
        }
    }

    func waitForExit() async -> Int32 {
        await withCheckedContinuation { continuation in
            lock.lock()
            if let exitStatus {
                lock.unlock()
                continuation.resume(returning: exitStatus)
            } else {
                exitWaiters.append(continuation)
                lock.unlock()
            }
        }
    }

    private func consume(_ data: Data) {
        let messages = lock.withLock { framer.append(data) }
        for message in messages { deliver(message) }
    }

    private func deliver(_ line: Data) {
        var line = line
        while let last = line.last, last == 0x0D || last == 0x20 { line.removeLast() }
        guard !line.isEmpty, let message = JSONValue.parse(line) else { return }
        continuation.yield(message)
    }

    private func finishOutput() {
        if let remainder = lock.withLock({ framer.finish() }) { deliver(remainder) }
        continuation.finish()
    }

    private func appendError(_ data: Data) {
        lock.withLock {
            errorBuffer.append(data)
            if errorBuffer.count > 64_000 {
                errorBuffer = Data(errorBuffer.suffix(16_000))
            }
        }
    }

    private func didExit(_ status: Int32) {
        let waiters = lock.withLock {
            exitStatus = status
            let waiters = exitWaiters
            exitWaiters.removeAll()
            return waiters
        }
        for waiter in waiters { waiter.resume(returning: status) }

        // A grandchild can inherit stdout and keep it open after the provider exits.
        let handle = stdout.fileHandleForReading
        DispatchQueue.global().asyncAfter(deadline: .now() + 1) { [weak self] in
            handle.readabilityHandler = nil
            self?.finishOutput()
        }
    }
}
