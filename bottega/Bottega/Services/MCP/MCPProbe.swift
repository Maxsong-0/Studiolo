// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

enum MCPProbeError: LocalizedError, Sendable {
    case commandMissing(String)
    case exited(code: Int32, stderr: String)
    case timedOut
    case http(status: Int, body: String)
    case unauthorizedOAuth
    /// The sign-in is fine but the server needs a step on its side first (GitLab answers 403
    /// "MCP server disabled", or 404 on GitLab 19.4 and earlier, until the top-level group allows MCP clients).
    case needsSetup(MCPSetupGuide)
    /// A signed-in server answered 404 at its own MCP address: a hiccup on their side.
    case unavailable(host: String)
    case malformed(String)

    var errorDescription: String? {
        switch self {
        case .commandMissing(let name):
            switch name {
            case "npx": return "npx isn't installed. Install Node.js from nodejs.org, then try again."
            case "uvx": return "uvx isn't installed. Run `brew install uv` in Terminal, then try again."
            default: return "\(name) isn't installed. Install it, then try again."
            }
        case .exited(let code, let stderr):
            let last = stderr
                .components(separatedBy: .newlines)
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                .last(where: { !$0.isEmpty })
            if let last { return "The server quit (exit \(code)). \(last)" }
            return "The server quit (exit \(code))."
        case .timedOut:
            return "The server didn't answer in time."
        case .http(let status, _):
            switch status {
            case 401, 403: return "That key was refused. Press Try again and paste a new one."
            case 404: return "Nothing answered at that address. Check the address and try again."
            default: return "The server answered \(status)."
            }
        case .unauthorizedOAuth:
            return "Sign-in was rejected. Sign in again in your browser."
        case .needsSetup(let guide):
            return guide.reason
        case .unavailable(let host):
            return "\(host) didn't answer (404). If the address is right, try again in a few minutes."
        case .malformed(let detail):
            return "The server sent something unexpected: \(detail)"
        }
    }
}

enum MCPProbe {
    @concurrent
    static func probe(_ server: MCPResolvedServer) async throws -> MCPProbeResult {
        if server.command != nil {
            return try await probeStdio(server)
        }
        if let url = server.url {
            return try await probeRemote(server, url)
        }
        throw MCPProbeError.malformed("missing command or URL")
    }

    // MARK: - Shared message builders

    private static func initializeParams() -> JSONValue {
        .object([
            "protocolVersion": .string("2025-06-18"),
            "capabilities": .object([:]),
            "clientInfo": .object(["name": .string("Bottega"), "version": .string("1.0")]),
        ])
    }

    private static func request(id: Int, method: String, params: JSONValue?) -> JSONValue {
        var object: [String: JSONValue] = [
            "jsonrpc": .string("2.0"),
            "id": .int(id),
            "method": .string(method),
        ]
        if let params { object["params"] = params }
        return .object(object)
    }

    private static func notification(method: String) -> JSONValue {
        .object(["jsonrpc": .string("2.0"), "method": .string(method)])
    }

    private static func checkReply(_ value: JSONValue) throws {
        if let error = value["error"], error.object != nil {
            throw MCPProbeError.malformed(error["message"]?.string ?? "unknown error")
        }
    }

    private static func firstLine(_ text: String?) -> String? {
        guard let text else { return nil }
        let line = text.components(separatedBy: .newlines).first?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return line.isEmpty ? nil : line
    }

    private static func toolSummaries(from result: JSONValue?) -> [MCPToolSummary] {
        guard let tools = result?["tools"]?.array else { return [] }
        return tools.compactMap { tool in
            guard let name = tool["name"]?.string else { return nil }
            return MCPToolSummary(name: name, summary: firstLine(tool["description"]?.string))
        }
    }

    // MARK: - Local servers over stdio

    private static func probeStdio(_ server: MCPResolvedServer) async throws -> MCPProbeResult {
        let command = server.command ?? ""
        guard let executable = LoginEnvironment.which(command, in: LoginEnvironment.current) else {
            throw MCPProbeError.commandMissing(command)
        }
        let isOAuth = server.args.contains("mcp-remote")
        let deadline = Date().addingTimeInterval(isOAuth ? 240 : 90)

        var environment = LoginEnvironment.current
        for (key, value) in server.env { environment[key] = value }

        let process = Process()
        process.executableURL = executable
        process.arguments = server.args
        process.environment = environment
        process.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
        let stdinPipe = Pipe()
        let stdoutPipe = Pipe()
        let stderrPipe = Pipe()
        process.standardInput = stdinPipe
        process.standardOutput = stdoutPipe
        process.standardError = stderrPipe

        let conversation = StdioConversation()
        stdoutPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty {
                handle.readabilityHandler = nil
            } else {
                conversation.appendStdout(data)
            }
        }
        stderrPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty {
                handle.readabilityHandler = nil
            } else {
                conversation.appendStderr(data)
            }
        }
        process.terminationHandler = { [conversation] finished in
            conversation.setExit(finished.terminationStatus)
        }
        do {
            try process.run()
        } catch {
            stdoutPipe.fileHandleForReading.readabilityHandler = nil
            stderrPipe.fileHandleForReading.readabilityHandler = nil
            terminate(process)
            throw error
        }
        defer {
            stdoutPipe.fileHandleForReading.readabilityHandler = nil
            stderrPipe.fileHandleForReading.readabilityHandler = nil
            terminate(process)
            try? stdinPipe.fileHandleForWriting.close()
        }

        func send(_ message: JSONValue) {
            var line = message.data()
            line.append(0x0A)
            try? stdinPipe.fileHandleForWriting.write(contentsOf: line)
        }

        func nextReply(id: Int) async throws -> JSONValue {
            while true {
                for line in conversation.takeLines() {
                    guard let value = JSONValue.parse(line), value.object != nil else { continue }
                    guard value["id"]?.int == id else { continue }
                    try checkReply(value)
                    return value
                }
                if let code = conversation.exitCode() {
                    throw exitError(code: code, isOAuth: isOAuth, stderr: conversation.stderrText())
                }
                guard Date() < deadline else {
                    throw exitError(code: nil, isOAuth: isOAuth, stderr: conversation.stderrText())
                }
                try await Task.sleep(for: .milliseconds(5))
            }
        }

        send(request(id: 1, method: "initialize", params: initializeParams()))
        let initReply = try await nextReply(id: 1)
        let initResult = initReply["result"]
        guard initResult?.object != nil else { throw MCPProbeError.malformed("missing result") }
        let serverName = initResult?["serverInfo"]?["name"]?.string
        let serverVersion = initResult?["serverInfo"]?["version"]?.string

        send(notification(method: "notifications/initialized"))

        var tools: [MCPToolSummary] = []
        var cursor: String?
        var nextID = 2
        for _ in 0..<10 {
            var params: [String: JSONValue] = [:]
            if let cursor { params["cursor"] = .string(cursor) }
            send(request(id: nextID, method: "tools/list", params: .object(params)))
            let reply = try await nextReply(id: nextID)
            nextID += 1
            let list = reply["result"]
            tools += toolSummaries(from: list)
            guard let next = list?["nextCursor"]?.string, !next.isEmpty else { break }
            cursor = next
        }
        return MCPProbeResult(serverName: serverName, serverVersion: serverVersion, tools: tools)
    }

    private static func exitError(code: Int32?, isOAuth: Bool, stderr: String) -> MCPProbeError {
        if isOAuth && (stderr.contains("Unauthorized") || stderr.contains("invalid_grant")) {
            return .unauthorizedOAuth
        }
        if let code { return .exited(code: code, stderr: stderr) }
        return .timedOut
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

    // MARK: - Remote servers over streamable HTTP

    private static func probeRemote(_ server: MCPResolvedServer, _ urlString: String) async throws -> MCPProbeResult {
        guard let url = URL(string: urlString) else {
            throw MCPProbeError.malformed("invalid URL")
        }
        var sessionID: String?

        func post(_ payload: JSONValue) async throws -> JSONValue? {
            do {
                let posted = try await postJSON(to: url, headers: server.headers, sessionID: sessionID, payload: payload)
                sessionID = posted.sessionID ?? sessionID
                return posted.body
            } catch MCPProbeError.http(let status, let body) where (status == 401 || status == 403) && server.oauthUpstream != nil {
                if status == 403, body.localizedCaseInsensitiveContains("MCP server disabled") {
                    throw MCPProbeError.needsSetup(Self.gitLabServerDisabled(upstream: server.oauthUpstream ?? urlString))
                }
                throw MCPProbeError.unauthorizedOAuth
            } catch MCPProbeError.http(let status, _) where status == 404 && server.oauthUpstream != nil {
                let upstream = server.oauthUpstream ?? urlString
                // GitLab 19.4 and earlier answer a namespace that does not allow its MCP server with 404
                // instead of 403 'MCP server disabled'; both statuses share every cause, so a 404 from
                // its MCP endpoint is the setup case rather than a hiccup on GitLab's side.
                if Self.isGitLabMCP(upstream) {
                    throw MCPProbeError.needsSetup(Self.gitLabServerDisabled(upstream: upstream))
                }
                throw MCPProbeError.unavailable(host: URL(string: upstream)?.host ?? "The server")
            }
        }

        func nearest(_ value: JSONValue?) throws -> JSONValue {
            guard let body = value else { throw MCPProbeError.malformed("empty reply") }
            try checkReply(body)
            guard body.object != nil else { throw MCPProbeError.malformed("invalid reply") }
            return body
        }

        let initReply = try await nearest(post(request(id: 1, method: "initialize", params: initializeParams())))
        let initResult = initReply["result"]
        guard initResult?.object != nil else { throw MCPProbeError.malformed("missing result") }
        let serverName = initResult?["serverInfo"]?["name"]?.string
        let serverVersion = initResult?["serverInfo"]?["version"]?.string

        do {
            _ = try await post(notification(method: "notifications/initialized"))
        } catch {
            if error is CancellationError { throw error }
        }

        var tools: [MCPToolSummary] = []
        var cursor: String?
        var nextID = 2
        for _ in 0..<10 {
            var params: [String: JSONValue] = [:]
            if let cursor { params["cursor"] = .string(cursor) }
            let reply = try await nearest(post(request(id: nextID, method: "tools/list", params: .object(params))))
            nextID += 1
            let list = reply["result"]
            tools += toolSummaries(from: list)
            guard let next = list?["nextCursor"]?.string, !next.isEmpty else { break }
            cursor = next
        }
        return MCPProbeResult(serverName: serverName, serverVersion: serverVersion, tools: tools)
    }

    private static func postJSON(
        to url: URL,
        headers: [String: String],
        sessionID: String?,
        payload: JSONValue
    ) async throws -> (sessionID: String?, body: JSONValue?) {
        var request = URLRequest(url: url, timeoutInterval: 30)
        request.httpMethod = "POST"
        request.httpBody = payload.data()
        for (field, value) in headers { request.setValue(value, forHTTPHeaderField: field) }
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
        request.setValue("2025-06-18", forHTTPHeaderField: "MCP-Protocol-Version")
        if let sessionID { request.setValue(sessionID, forHTTPHeaderField: "Mcp-Session-Id") }

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw MCPProbeError.malformed("invalid reply")
        }
        let session = http.value(forHTTPHeaderField: "Mcp-Session-Id")
        guard (200..<300).contains(http.statusCode) else {
            let body = String(String(decoding: data, as: UTF8.self).prefix(2000))
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw MCPProbeError.http(status: http.statusCode, body: body)
        }
        let contentType = http.value(forHTTPHeaderField: "Content-Type") ?? ""
        if contentType.contains("text/event-stream") {
            return (session, firstSSEMessage(data))
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

    // MARK: - Setup guides

    /// Whether an OAuth upstream is a GitLab instance's MCP endpoint. GitLab.com and a
    /// self-managed instance both answer at <origin>/api/v4/mcp, behind a relative URL root too.
    static func isGitLabMCP(_ upstream: String) -> Bool {
        guard let url = URL(string: upstream) else { return false }
        return url.path.lowercased().hasSuffix("/api/v4/mcp")
    }

    /// GitLab keeps its MCP server off until an Owner allows it on the top-level group.
    static func gitLabServerDisabled(upstream: String) -> MCPSetupGuide {
        let origin: String = {
            guard let url = URL(string: upstream), let host = url.host else { return "https://gitlab.com" }
            return "\(url.scheme ?? "https")://\(host)"
        }()
        return MCPSetupGuide(
            title: "Turn on GitLab's MCP server",
            reason: "You're signed in, but GitLab keeps its MCP server off until a group owner turns it on. Until then, the address answers not found.",
            steps: [
                "Open your groups and pick the top-level group your projects live in",
                "Go to Settings › General and expand Permissions and group features",
                "Under MCP client access, tick Allow connection to GitLab and save",
                "Come back here and press Check again",
            ],
            pageLabel: "Open my groups",
            pageURL: origin + "/dashboard/groups",
            note: "GitLab can take a minute to apply the change."
        )
    }
}

/// Lock-guarded buffers fed by the stdio readability handlers: stdout bytes are
/// split on newlines into complete lines, stderr and the exit status are kept
/// for failure reporting.
private final class StdioConversation: @unchecked Sendable {
    private let lock = NSLock()
    private var buffer = Data()
    private var pending: [Data] = []
    private var stderr = Data()
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

    func appendStderr(_ data: Data) {
        lock.withLock {
            stderr.append(data)
            if stderr.count > 64_000 { stderr = Data(stderr.suffix(16_000)) }
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

    func stderrText() -> String {
        lock.withLock { String(decoding: stderr, as: UTF8.self) }
    }

    func exitCode() -> Int32? {
        lock.withLock { exitStatus }
    }
}
