// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

enum MCPHubError: LocalizedError, Sendable {
    case unknownTool(String)
    case server(String)

    var errorDescription: String? {
        switch self {
        case .unknownTool(let name):
            return "Unknown tool: \(name)."
        case .server(let message):
            return message
        }
    }
}

actor MCPHub {
    static let shared = MCPHub()

    /// Every server process started this run, so the app can end them as it quits: a
    /// `Process` child outlives its parent, and an `npx` server left behind would keep
    /// running until the Mac restarts.
    private static let liveProcesses = MCPLiveProcesses()

    nonisolated static func track(_ process: Process) { liveProcesses.add(process) }

    /// Ends every running server at once. Called from `applicationWillTerminate`, where
    /// nothing asynchronous would get to run.
    nonisolated static func terminateAll() { liveProcesses.terminateAll() }

    struct Tool: Sendable, Hashable {
        let server: String
        let name: String
        let description: String
        let inputSchema: JSONValue

        var callName: String { "mcp__" + server + "__" + name }
    }

    struct Result: Sendable {
        let text: String
        let isError: Bool
    }

    private var specs: [String: MCPServerSpec]?
    private var clients: [String: MCPClient] = [:]
    private var toolCache: [String: [Tool]] = [:]
    private var failed: Set<String> = []

    func tools() async -> [Tool] {
        if specs == nil { specs = Self.readSpecs() }
        guard let current = specs else { return [] }
        var all: [Tool] = []
        for id in current.keys.sorted() {
            if let cached = toolCache[id] {
                all += cached
                continue
            }
            if failed.contains(id) { continue }
            guard let spec = current[id] else { continue }
            let client = self.client(for: id, spec: spec)
            do {
                let listed = try await client.listTools()
                toolCache[id] = listed
                all += listed
            } catch {
                print("MCPHub: \(id): \(error.localizedDescription)")
                failed.insert(id)
                await client.stop()
            }
        }
        return all.sorted {
            if $0.server != $1.server { return $0.server < $1.server }
            return $0.name < $1.name
        }
    }

    func call(_ callName: String, arguments: JSONValue) async throws -> Result {
        if specs == nil { specs = Self.readSpecs() }
        guard let parsed = Self.parse(callName: callName),
              let spec = specs?[parsed.server]
        else { throw MCPHubError.unknownTool(callName) }
        if let cached = toolCache[parsed.server],
           !cached.contains(where: { $0.name == parsed.tool }) {
            throw MCPHubError.unknownTool(callName)
        }
        let client = self.client(for: parsed.server, spec: spec)
        do {
            return try await client.callTool(name: parsed.tool, arguments: arguments)
        } catch let error as MCPHubError {
            throw error
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            throw MCPHubError.server(error.localizedDescription)
        }
    }

    func reload() {
        let existing = clients
        clients = [:]
        toolCache = [:]
        failed = []
        specs = nil
        for client in existing.values { Task { await client.stop() } }
    }

    nonisolated static func parse(callName: String) -> (server: String, tool: String)? {
        guard callName.hasPrefix("mcp__") else { return nil }
        let rest = callName.dropFirst("mcp__".count)
        guard let separator = rest.range(of: "__") else { return nil }
        let server = String(rest[..<separator.lowerBound])
        let tool = String(rest[separator.upperBound...])
        guard !server.isEmpty, !tool.isEmpty else { return nil }
        return (server, tool)
    }

    private func client(for id: String, spec: MCPServerSpec) -> MCPClient {
        if let existing = clients[id] { return existing }
        let created = MCPClient(id: id, spec: spec)
        clients[id] = created
        return created
    }

    nonisolated private static func readSpecs() -> [String: MCPServerSpec] {
        guard let data = try? Data(contentsOf: MCPPaths.claudeConfigURL),
              let parsed = JSONValue.parse(data),
              let servers = parsed["mcpServers"]?.object
        else { return [:] }
        var out: [String: MCPServerSpec] = [:]
        for (id, entry) in servers {
            if let command = entry["command"]?.string, !command.isEmpty {
                let args = entry["args"]?.array?.compactMap(\.string) ?? []
                var env: [String: String] = [:]
                for (key, value) in entry["env"]?.object ?? [:] {
                    if let string = value.string { env[key] = string }
                }
                out[id] = .stdio(command: command, args: args, env: env)
            } else if let url = entry["url"]?.string, !url.isEmpty {
                var headers: [String: String] = [:]
                for (key, value) in entry["headers"]?.object ?? [:] {
                    if let string = value.string { headers[key] = string }
                }
                out[id] = .http(url: url, headers: headers)
            }
        }
        return out
    }
}

private enum MCPServerSpec: Sendable {
    case stdio(command: String, args: [String], env: [String: String])
    case http(url: String, headers: [String: String])
}

private actor MCPClient {
    let id: String
    let spec: MCPServerSpec

    private var process: Process?
    private var conversation: HubConversation?
    private var stdinHandle: FileHandle?
    private var nextID = 1
    private var initialized = false
    private var didRestart = false
    private var dead = false
    private var sessionID: String?
    /// Replies read off stdout for a request other than the one being waited on: two
    /// calls in flight on one server each get their own, rather than the first reader
    /// dropping the other's.
    private var replies: [Int: JSONValue] = [:]

    init(id: String, spec: MCPServerSpec) {
        self.id = id
        self.spec = spec
    }

    func stop() {
        if let process {
            (process.standardOutput as? Pipe)?.fileHandleForReading.readabilityHandler = nil
            (process.standardError as? Pipe)?.fileHandleForReading.readabilityHandler = nil
            Self.terminate(process)
        }
        process = nil
        conversation = nil
        stdinHandle = nil
        initialized = false
        sessionID = nil
        replies = [:]
    }

    func listTools() async throws -> [MCPHub.Tool] {
        var out: [MCPHub.Tool] = []
        var cursor: String?
        for _ in 0..<10 {
            var params: [String: JSONValue] = [:]
            if let cursor { params["cursor"] = .string(cursor) }
            let reply = try await rpc(method: "tools/list", params: .object(params), timeout: 60)
            let list = reply["result"]
            for item in list?["tools"]?.array ?? [] {
                guard let name = item["name"]?.string else { continue }
                let rawDescription = item["description"]?.string ?? ""
                let schema: JSONValue
                if let found = item["inputSchema"], found.object != nil {
                    schema = found
                } else {
                    schema = .object(["type": .string("object"), "properties": .object([:])])
                }
                out.append(MCPHub.Tool(
                    server: id,
                    name: name,
                    description: String(rawDescription.prefix(400)),
                    inputSchema: schema
                ))
            }
            guard let next = list?["nextCursor"]?.string, !next.isEmpty else { break }
            cursor = next
        }
        return out
    }

    func callTool(name: String, arguments: JSONValue) async throws -> MCPHub.Result {
        let args: JSONValue = arguments.isNull ? .object([:]) : arguments
        let reply = try await rpc(
            method: "tools/call",
            params: .object(["name": .string(name), "arguments": args]),
            timeout: 120
        )
        let result = reply["result"]
        var lines: [String] = []
        for item in result?["content"]?.array ?? [] {
            if let text = item["text"]?.string {
                lines.append(text)
            } else if item["type"]?.string == "image" {
                lines.append("[image]")
            } else if item["type"]?.string == "resource" {
                lines.append("[resource: \(item["resource"]?["uri"]?.string ?? "")]")
            }
        }
        return MCPHub.Result(
            text: lines.joined(separator: "\n"),
            isError: result?["isError"]?.bool ?? false
        )
    }

    private func rpc(method: String, params: JSONValue?, timeout: TimeInterval) async throws -> JSONValue {
        switch spec {
        case .stdio(let command, let args, let env):
            return try await stdioRPC(command: command, args: args, env: env, method: method, params: params, timeout: timeout)
        case .http(let url, let headers):
            return try await httpRPC(urlString: url, headers: headers, method: method, params: params, timeout: timeout)
        }
    }

    // MARK: - Shared message builders

    private func initializeParams() -> JSONValue {
        .object([
            "protocolVersion": .string("2025-06-18"),
            "capabilities": .object([:]),
            "clientInfo": .object(["name": .string("Bottega"), "version": .string("1.0")]),
        ])
    }

    private func request(id: Int, method: String, params: JSONValue?) -> JSONValue {
        var object: [String: JSONValue] = [
            "jsonrpc": .string("2.0"),
            "id": .int(id),
            "method": .string(method),
        ]
        if let params { object["params"] = params }
        return .object(object)
    }

    private func notification(method: String) -> JSONValue {
        .object(["jsonrpc": .string("2.0"), "method": .string(method)])
    }

    private func checkReply(_ value: JSONValue) throws {
        if let error = value["error"], error.object != nil {
            throw MCPHubError.server(error["message"]?.string ?? "unknown error")
        }
    }

    private static func terminate(_ process: Process) {
        guard process.isRunning else { return }
        process.terminate()
        let process = process
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 1) {
            guard process.isRunning else { return }
            let pid = process.processIdentifier
            kill(getpgid(pid) == pid ? -pid : pid, SIGKILL)
        }
    }

    // MARK: - Local servers over stdio

    private func stdioRPC(command: String, args: [String], env: [String: String], method: String, params: JSONValue?, timeout: TimeInterval) async throws -> JSONValue {
        try await ensureStdio(command: command, args: args, env: env)
        guard let proc = process, let stdin = stdinHandle else {
            throw MCPHubError.server("The server quit.")
        }
        let id = nextID
        nextID += 1
        var line = request(id: id, method: method, params: params).data()
        line.append(0x0A)
        try? stdin.write(contentsOf: line)
        return try await waitStdio(process: proc, id: id, timeout: timeout)
    }

    private func ensureStdio(command: String, args: [String], env: [String: String]) async throws {
        if dead { throw MCPHubError.server("The server quit.") }
        if let proc = process, proc.isRunning, initialized { return }
        if process != nil {
            if didRestart {
                dead = true
                throw MCPHubError.server("The server quit.")
            }
            didRestart = true
        }
        try await startStdio(command: command, args: args, env: env)
    }

    private func startStdio(command: String, args: [String], env: [String: String]) async throws {
        guard let executable = LoginEnvironment.which(command, in: LoginEnvironment.current) else {
            throw MCPHubError.server("\(command) isn't installed.")
        }
        var environment = LoginEnvironment.current
        for (key, value) in env { environment[key] = value }

        let proc = Process()
        proc.executableURL = executable
        proc.arguments = args
        proc.environment = environment
        proc.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
        let stdinPipe = Pipe()
        let stdoutPipe = Pipe()
        let stderrPipe = Pipe()
        proc.standardInput = stdinPipe
        proc.standardOutput = stdoutPipe
        proc.standardError = stderrPipe

        let conv = HubConversation()
        stdoutPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty {
                handle.readabilityHandler = nil
            } else {
                conv.appendStdout(data)
            }
        }
        stderrPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty { handle.readabilityHandler = nil }
        }
        proc.terminationHandler = { [conv] finished in
            conv.setExit(finished.terminationStatus)
        }
        do {
            try proc.run()
        } catch {
            stdoutPipe.fileHandleForReading.readabilityHandler = nil
            stderrPipe.fileHandleForReading.readabilityHandler = nil
            Self.terminate(proc)
            throw MCPHubError.server(error.localizedDescription)
        }
        process = proc
        conversation = conv
        stdinHandle = stdinPipe.fileHandleForWriting
        MCPHub.track(proc)
        do {
            let id = nextID
            nextID += 1
            var line = request(id: id, method: "initialize", params: initializeParams()).data()
            line.append(0x0A)
            try? stdinPipe.fileHandleForWriting.write(contentsOf: line)
            let reply = try await waitStdio(process: proc, id: id, timeout: 60)
            guard reply["result"]?.object != nil else {
                throw MCPHubError.server("The server sent something unexpected: missing result.")
            }
            var initializedLine = notification(method: "notifications/initialized").data()
            initializedLine.append(0x0A)
            try? stdinPipe.fileHandleForWriting.write(contentsOf: initializedLine)
            initialized = true
        } catch {
            stop()
            if error is MCPHubError || error is CancellationError { throw error }
            throw MCPHubError.server(error.localizedDescription)
        }
    }

    private func waitStdio(process proc: Process, id: Int, timeout: TimeInterval) async throws -> JSONValue {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if let conv = conversation {
                for line in conv.takeLines() {
                    guard let value = JSONValue.parse(line), value.object != nil, let replyID = value["id"]?.int else { continue }
                    replies[replyID] = value
                }
                if let value = replies.removeValue(forKey: id) {
                    try checkReply(value)
                    return value
                }
                if let code = conv.exitCode() {
                    throw MCPHubError.server("The server quit (exit \(code)).")
                }
            }
            if !proc.isRunning {
                throw MCPHubError.server("The server quit.")
            }
            guard Date() < deadline else {
                replies[id] = nil
                throw MCPHubError.server("The server didn't answer in time.")
            }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    // MARK: - Remote servers over streamable HTTP

    private func httpRPC(urlString: String, headers: [String: String], method: String, params: JSONValue?, timeout: TimeInterval) async throws -> JSONValue {
        guard let url = URL(string: urlString) else {
            throw MCPHubError.server("The server address is invalid.")
        }
        if !initialized {
            let id = nextID
            nextID += 1
            let posted = try await post(url: url, headers: headers, payload: request(id: id, method: "initialize", params: initializeParams()), timeout: 60)
            guard let body = posted.body else {
                throw MCPHubError.server("The server sent something unexpected: empty reply.")
            }
            try checkReply(body)
            guard body["result"]?.object != nil else {
                throw MCPHubError.server("The server sent something unexpected: missing result.")
            }
            sessionID = posted.sessionID ?? sessionID
            initialized = true
            do {
                _ = try await post(url: url, headers: headers, payload: notification(method: "notifications/initialized"), timeout: 30)
            } catch {
                if error is CancellationError { throw error }
            }
        }
        let id = nextID
        nextID += 1
        let posted = try await post(url: url, headers: headers, payload: request(id: id, method: method, params: params), timeout: timeout)
        guard let body = posted.body else {
            throw MCPHubError.server("The server sent something unexpected: empty reply.")
        }
        try checkReply(body)
        return body
    }

    private func post(url: URL, headers: [String: String], payload: JSONValue, timeout: TimeInterval) async throws -> (sessionID: String?, body: JSONValue?) {
        var urlRequest = URLRequest(url: url, timeoutInterval: timeout)
        urlRequest.httpMethod = "POST"
        urlRequest.httpBody = payload.data()
        for (field, value) in headers { urlRequest.setValue(value, forHTTPHeaderField: field) }
        urlRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        urlRequest.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
        urlRequest.setValue("2025-06-18", forHTTPHeaderField: "MCP-Protocol-Version")
        if let sessionID { urlRequest.setValue(sessionID, forHTTPHeaderField: "Mcp-Session-Id") }

        let (data, response) = try await URLSession.shared.data(for: urlRequest)
        guard let http = response as? HTTPURLResponse else {
            throw MCPHubError.server("The server sent something unexpected: invalid reply.")
        }
        let session = http.value(forHTTPHeaderField: "Mcp-Session-Id")
        guard (200..<300).contains(http.statusCode) else {
            throw MCPHubError.server("The server answered \(http.statusCode).")
        }
        let contentType = http.value(forHTTPHeaderField: "Content-Type") ?? ""
        if contentType.contains("text/event-stream") {
            return (session, Self.firstSSEMessage(data))
        }
        guard !data.isEmpty else { return (session, nil) }
        return (session, JSONValue.parse(data))
    }

    private static func firstSSEMessage(_ data: Data) -> JSONValue? {
        let text = String(decoding: data, as: UTF8.self)
        for line in text.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard trimmed.hasPrefix("data:") else { continue }
            let payload = trimmed.dropFirst(5).trimmingCharacters(in: .whitespaces)
            guard let value = JSONValue.parse(String(payload)),
                  value.object != nil, value["id"] != nil
            else { continue }
            return value
        }
        return nil
    }
}

/// Lock-guarded buffer fed by the stdio readability handler: stdout bytes are
/// split on newlines into complete lines, and the exit status is kept so a
/// reply loop can tell the process went away. Stderr is drained and discarded.
private final class HubConversation: @unchecked Sendable {
    private let lock = NSLock()
    private var buffer = Data()
    private var pending: [Data] = []
    private var exitStatus: Int32?

    func appendStdout(_ data: Data) {
        lock.withLock {
            buffer.append(data)
            while let newline = buffer.firstIndex(of: 0x0A) {
                pending.append(Data(buffer[..<newline]))
                buffer.removeSubrange(...newline)
            }
        }
    }

    func setExit(_ status: Int32) {
        lock.withLock { exitStatus = status }
    }

    func takeLines() -> [Data] {
        lock.withLock {
            let lines = pending
            pending.removeAll()
            return lines
        }
    }

    func exitCode() -> Int32? {
        lock.withLock { exitStatus }
    }
}

/// The server processes alive this run, behind a lock so the quit path can reach them
/// from any thread without an actor hop.
private final class MCPLiveProcesses: @unchecked Sendable {
    private let lock = NSLock()
    private var processes: [Process] = []

    func add(_ process: Process) {
        lock.withLock {
            processes.removeAll { !$0.isRunning }
            processes.append(process)
        }
    }

    func terminateAll() {
        let running = lock.withLock { processes.filter(\.isRunning) }
        for process in running {
            process.terminate()
        }
        for process in running where process.isRunning {
            let pid = process.processIdentifier
            kill(getpgid(pid) == pid ? -pid : pid, SIGKILL)
        }
    }
}
