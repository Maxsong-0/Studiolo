// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import CryptoKit
import Foundation
import Network

enum MCPOAuthError: LocalizedError, Sendable {
    case discoveryFailed(String)
    case metadataFailed(String)
    case registrationUnsupported
    case registrationFailed(String)
    case tokenFailed(String)
    case callbackFailed(String)
    case stateMismatch
    case timedOut
    case cancelled
    case signedOut
    case network(String)

    var errorDescription: String? {
        switch self {
        case .discoveryFailed(let detail):
            detail.isEmpty ? "Couldn't find the sign-in page." : "Couldn't find the sign-in page. \(detail)"
        case .metadataFailed(let detail):
            detail.isEmpty ? "The server's sign-in details didn't make sense." : "The server's sign-in details didn't make sense. \(detail)"
        case .registrationUnsupported:
            "This server doesn't let apps sign in on their own."
        case .registrationFailed(let detail):
            detail.isEmpty ? "Sign-in setup failed." : "Sign-in setup failed. \(detail)"
        case .tokenFailed(let detail):
            detail.isEmpty ? "Couldn't finish signing in." : "Couldn't finish signing in. \(detail)"
        case .callbackFailed(let detail):
            detail.isEmpty ? "Sign-in didn't finish." : detail
        case .stateMismatch:
            "Sign-in didn't finish. Try again."
        case .timedOut:
            "Sign-in timed out. Try again and complete it in the browser."
        case .cancelled:
            "Sign-in was cancelled."
        case .signedOut:
            "You're signed out. Sign in again to continue."
        case .network(let detail):
            detail.isEmpty ? "The network didn't answer." : detail
        }
    }
}

private struct AuthServerMetadata: Sendable {
    var issuer: String
    var authorizationEndpoint: String
    var tokenEndpoint: String
    var registrationEndpoint: String?
    var resource: String?
    var resourceScopes: [String]?
}

private struct RegisteredClient: Sendable {
    var clientID: String
    var clientSecret: String?
}

private actor OAuthRefreshGate {
    private var flights: [String: Task<String, Error>] = [:]

    func token(for serverID: String, refresh: @Sendable @escaping () async throws -> String) async throws -> String {
        if let flight = flights[serverID] {
            return try await flight.value
        }
        let flight = Task.detached { try await refresh() }
        flights[serverID] = flight
        defer { flights[serverID] = nil }
        return try await flight.value
    }
}

private struct CallbackResult: Sendable {
    var code: String
    var redirectURI: String
    var client: RegisteredClient
}

private final class CallbackState: @unchecked Sendable {
    private let lock = NSLock()
    private var redirectURI = ""
    private var client: RegisteredClient?

    func store(redirectURI: String, client: RegisteredClient) {
        lock.lock()
        self.redirectURI = redirectURI
        self.client = client
        lock.unlock()
    }

    func current() -> (redirectURI: String, client: RegisteredClient?) {
        lock.lock()
        defer { lock.unlock() }
        return (redirectURI, client)
    }
}

private final class CallbackWait: @unchecked Sendable {
    private let lock = NSLock()
    private var finished = false
    private let continuation: CheckedContinuation<CallbackResult, Error>
    private let listener: NWListener

    init(continuation: CheckedContinuation<CallbackResult, Error>, listener: NWListener) {
        self.continuation = continuation
        self.listener = listener
    }

    func finish(_ result: Result<CallbackResult, Error>) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        finished = true
        lock.unlock()
        listener.cancel()
        continuation.resume(with: result)
    }

    var isFinished: Bool {
        lock.lock()
        defer { lock.unlock() }
        return finished
    }
}

enum MCPOAuth {
    private static let refreshGate = OAuthRefreshGate()
    private static let callbackTimeout: TimeInterval = 300

    // MARK: - Sign in

    static func signIn(serverID: String, serverName: String, url: String) async throws {
        let metadata = try await discover(url: url)
        let verifier = randomToken(byteCount: 32)
        let challenge = pkceChallenge(for: verifier)
        let state = randomToken(byteCount: 24)
        let callback = try await runCallbackServer(serverName: serverName, expectedState: state) { redirectURI in
            let client = try await clientRegistration(serverID: serverID, metadata: metadata, redirectURI: redirectURI)
            guard let authorizationURL = authorizationURL(
                metadata: metadata,
                clientID: client.clientID,
                redirectURI: redirectURI,
                state: state,
                challenge: challenge
            ) else {
                throw MCPOAuthError.metadataFailed("The authorization address isn't valid.")
            }
            return (authorizationURL, client)
        }
        try await exchangeCode(
            serverID: serverID,
            metadata: metadata,
            client: callback.client,
            code: callback.code,
            redirectURI: callback.redirectURI,
            verifier: verifier
        )
    }

    // MARK: - Tokens

    static func accessToken(serverID: String) async throws -> String {
        guard let stored = MCPKeychain.value(server: serverID, field: "oauth"),
              let json = JSONValue.parse(stored),
              let accessToken = json["access_token"]?.string,
              !accessToken.isEmpty
        else {
            throw MCPOAuthError.signedOut
        }
        let needsRefresh: Bool = {
            guard let expiresAt = json["expires_at"]?.double else { return false }
            return Date().timeIntervalSince1970 >= expiresAt - 90
        }()
        guard needsRefresh else { return accessToken }
        guard let refreshToken = json["refresh_token"]?.string, !refreshToken.isEmpty else {
            throw MCPOAuthError.signedOut
        }
        return try await refreshGate.token(for: serverID) {
            try await performRefresh(serverID: serverID, stored: json, refreshToken: refreshToken)
        }
    }

    static func isSignedIn(serverID: String) -> Bool {
        MCPKeychain.value(server: serverID, field: "oauth") != nil
    }

    static func signOut(serverID: String) {
        MCPKeychain.delete(server: serverID, field: "oauth")
        MCPKeychain.delete(server: serverID, field: "oauth-client")
    }

    static func refreshIfNeeded(serverID: String) async {
        _ = try? await accessToken(serverID: serverID)
    }

    // MARK: - Discovery

    private static func discover(url: String) async throws -> AuthServerMetadata {
        guard URL(string: url) != nil else {
            throw MCPOAuthError.discoveryFailed("That address doesn't look like a URL.")
        }
        var issuer: String?
        var resource: String?
        var resourceScopes: [String]?
        let challenge = await protectedResourceMetadataURL(baseURL: url)
        if let metadataURL = challenge.metadataURL {
            if let json = try? await fetchJSON(url: metadataURL) {
                issuer = json["authorization_servers"]?.array?.first?.string
                resource = json["resource"]?.string
                if let listed = json["scopes_supported"]?.array {
                    let names = listed.compactMap(\.string).filter { !$0.isEmpty }
                    resourceScopes = names.isEmpty ? nil : names
                }
            }
        } else if let requestURL = URL(string: url) {
            let origin = try origin(of: url)
            let path = requestURL.path.isEmpty ? "" : requestURL.path
            let candidates = [
                origin + "/.well-known/oauth-protected-resource" + path,
                origin + "/.well-known/oauth-protected-resource",
            ]
            for candidate in candidates {
                if let json = try? await fetchJSON(url: candidate) {
                    let hasServers = json["authorization_servers"]?.array != nil
                    let hasResource = json["resource"]?.string != nil
                    guard hasServers || hasResource else { continue }
                    issuer = json["authorization_servers"]?.array?.first?.string
                    resource = json["resource"]?.string
                    if let listed = json["scopes_supported"]?.array {
                        let names = listed.compactMap(\.string).filter { !$0.isEmpty }
                        resourceScopes = names.isEmpty ? nil : names
                    }
                    break
                }
            }
        }
        if resourceScopes == nil {
            resourceScopes = challenge.challengeScopes
        }
        let fallbackIssuer = try origin(of: url)
        let normalizedIssuer = (issuer ?? fallbackIssuer).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard !normalizedIssuer.isEmpty else {
            throw MCPOAuthError.discoveryFailed("That address doesn't look like a URL.")
        }
        return try await fetchAuthorizationServerMetadata(issuer: normalizedIssuer, resource: resource, resourceScopes: resourceScopes)
    }

    private static func protectedResourceMetadataURL(baseURL: String) async -> (metadataURL: String?, challengeScopes: [String]?) {
        guard let requestURL = URL(string: baseURL) else { return (nil, nil) }
        var request = URLRequest(url: requestURL, timeoutInterval: 30)
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              let http = response as? HTTPURLResponse,
              http.statusCode == 401
        else { return (nil, nil) }
        var challenges: [String] = []
        for (key, value) in http.allHeaderFields {
            guard let name = key as? String,
                  name.caseInsensitiveCompare("WWW-Authenticate") == .orderedSame
            else { continue }
            if let text = value as? String {
                challenges.append(text)
            } else if let parts = value as? [String] {
                challenges.append(contentsOf: parts)
            }
        }
        var metadataURL: String?
        var scopesFromChallenge: [String]?
        for challenge in challenges {
            if metadataURL == nil, let found = resourceMetadataURL(from: challenge) {
                metadataURL = found
            }
            if scopesFromChallenge == nil, let found = challengeScopes(from: challenge) {
                scopesFromChallenge = found
            }
            if metadataURL != nil, scopesFromChallenge != nil { break }
        }
        _ = data
        return (metadataURL, scopesFromChallenge)
    }

    private static func resourceMetadataURL(from header: String) -> String? {
        challengeParameter("resource_metadata", in: header)
    }

    /// The value of one parameter of a WWW-Authenticate challenge, quoted or (as Stripe
    /// sends `resource_metadata`) bare up to the next comma or space; nil when absent or empty.
    private static func challengeParameter(_ name: String, in header: String) -> String? {
        guard let range = header.range(of: name + "=") else { return nil }
        let rest = header[range.upperBound...].trimmingCharacters(in: .whitespaces)
        let value: String
        if rest.hasPrefix("\"") {
            let inner = rest.dropFirst()
            guard let end = inner.firstIndex(of: "\"") else { return nil }
            value = String(inner[..<end])
        } else {
            value = String(rest.prefix { $0 != "," && $0 != " " })
        }
        return value.isEmpty ? nil : value
    }

    private static func challengeScopes(from header: String) -> [String]? {
        guard let value = challengeParameter("scope", in: header) else { return nil }
        let names = value.split(separator: " ").map(String.init).filter { !$0.isEmpty }
        return names.isEmpty ? nil : names
    }

    private static func fetchAuthorizationServerMetadata(issuer: String, resource: String?, resourceScopes: [String]?) async throws -> AuthServerMetadata {
        let base = issuer.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        var candidates: [String] = []
        // An issuer with a path (Stripe's is https://access.stripe.com/mcp) keeps its
        // metadata where RFC 8414 puts it, the path after the well-known segment; the
        // path-first forms below are what the rest answer to.
        if let components = URLComponents(string: base), let origin = try? origin(of: base), !components.path.isEmpty, components.path != "/" {
            candidates += [
                origin + "/.well-known/oauth-authorization-server" + components.path,
                origin + "/.well-known/openid-configuration" + components.path,
            ]
        }
        candidates += [
            base + "/.well-known/oauth-authorization-server",
            base + "/.well-known/openid-configuration",
        ]
        var lastError = ""
        for candidate in candidates {
            do {
                let json = try await fetchJSON(url: candidate)
                guard let authorizationEndpoint = json["authorization_endpoint"]?.string,
                      !authorizationEndpoint.isEmpty,
                      let tokenEndpoint = json["token_endpoint"]?.string,
                      !tokenEndpoint.isEmpty
                else {
                    lastError = "It's missing the addresses needed to sign in."
                    continue
                }
                return AuthServerMetadata(
                    issuer: base,
                    authorizationEndpoint: authorizationEndpoint,
                    tokenEndpoint: tokenEndpoint,
                    registrationEndpoint: json["registration_endpoint"]?.string,
                    resource: resource,
                    resourceScopes: resourceScopes
                )
            } catch {
                lastError = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            }
        }
        throw MCPOAuthError.metadataFailed(lastError)
    }

    // MARK: - Client registration

    private static func clientRegistration(serverID: String, metadata: AuthServerMetadata, redirectURI: String) async throws -> RegisteredClient {
        if let stored = MCPKeychain.value(server: serverID, field: "oauth-client"),
           let json = JSONValue.parse(stored),
           let clientID = json["client_id"]?.string,
           !clientID.isEmpty,
           json["issuer"]?.string == metadata.issuer
        {
            return RegisteredClient(clientID: clientID, clientSecret: json["client_secret"]?.string)
        }
        guard let registrationEndpoint = metadata.registrationEndpoint, !registrationEndpoint.isEmpty else {
            throw MCPOAuthError.registrationUnsupported
        }
        var body: [String: Any] = [
            "client_name": "Bottega",
            "redirect_uris": [redirectURI],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        ]
        if let resourceScopes = metadata.resourceScopes, !resourceScopes.isEmpty {
            body["scope"] = resourceScopes.joined(separator: " ")
        }
        let (data, response) = try await post(url: registrationEndpoint, body: body)
        guard (200..<300).contains(response.statusCode),
              let json = JSONValue.parse(data),
              let clientID = json["client_id"]?.string,
              !clientID.isEmpty
        else {
            let refusal = response.statusCode == 403
                ? "The server only signs in apps it has approved (403), so this one can't yet."
                : "The server refused to set things up (\(response.statusCode))."
            throw MCPOAuthError.registrationFailed(oauthErrorMessage(data: data) ?? refusal)
        }
        let clientSecret = json["client_secret"]?.string
        var stored: [String: Any] = ["client_id": clientID, "issuer": metadata.issuer]
        if let clientSecret, !clientSecret.isEmpty {
            stored["client_secret"] = clientSecret
        }
        if let storedData = try? JSONSerialization.data(withJSONObject: stored, options: [.sortedKeys]),
           let storedString = String(data: storedData, encoding: .utf8)
        {
            MCPKeychain.set(storedString, server: serverID, field: "oauth-client")
        }
        return RegisteredClient(clientID: clientID, clientSecret: clientSecret)
    }

    // MARK: - Callback server

    private static func runCallbackServer(
        serverName: String,
        expectedState: String,
        onReady: @Sendable @escaping (String) async throws -> (url: URL, client: RegisteredClient)
    ) async throws -> CallbackResult {
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: .any)
        let listener: NWListener
        do {
            listener = try NWListener(using: parameters)
        } catch {
            throw MCPOAuthError.network("Couldn't listen for the sign-in reply.")
        }
        let shared = CallbackState()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let wait = CallbackWait(continuation: continuation, listener: listener)
                let queue = DispatchQueue(label: "MCPOAuth.callback")
                listener.stateUpdateHandler = { state in
                    switch state {
                    case .ready:
                        guard let port = listener.port else {
                            wait.finish(.failure(MCPOAuthError.network("Couldn't listen for the sign-in reply.")))
                            return
                        }
                        let redirectURI = "http://127.0.0.1:\(port)/callback"
                        Task {
                            do {
                                let ready = try await onReady(redirectURI)
                                shared.store(redirectURI: redirectURI, client: ready.client)
                                await MainActor.run { _ = NSWorkspace.shared.open(ready.url) }
                            } catch {
                                wait.finish(.failure(error))
                            }
                        }
                    case .failed:
                        wait.finish(.failure(MCPOAuthError.network("The connection to the browser broke.")))
                    case .cancelled:
                        wait.finish(.failure(MCPOAuthError.cancelled))
                    default:
                        break
                    }
                }
                listener.newConnectionHandler = { connection in
                    connection.start(queue: queue)
                    receiveRequest(wait: wait, shared: shared, serverName: serverName, expectedState: expectedState, connection: connection, buffer: Data(), queue: queue)
                }
                listener.start(queue: queue)
                queue.asyncAfter(deadline: .now() + callbackTimeout) {
                    wait.finish(.failure(MCPOAuthError.timedOut))
                }
            }
        } onCancel: {
            listener.cancel()
        }
    }

    private static func receiveRequest(
        wait: CallbackWait,
        shared: CallbackState,
        serverName: String,
        expectedState: String,
        connection: NWConnection,
        buffer: Data,
        queue: DispatchQueue
    ) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { data, _, isComplete, error in
            guard !wait.isFinished else {
                connection.cancel()
                return
            }
            var combined = buffer
            if let data, !data.isEmpty {
                combined.append(data)
            }
            guard let terminator = combined.range(of: Data("\r\n\r\n".utf8)) else {
                if isComplete || error != nil || combined.count > 1024 * 1024 {
                    connection.cancel()
                    return
                }
                receiveRequest(wait: wait, shared: shared, serverName: serverName, expectedState: expectedState, connection: connection, buffer: combined, queue: queue)
                return
            }
            handleRequest(
                wait: wait,
                shared: shared,
                serverName: serverName,
                expectedState: expectedState,
                connection: connection,
                head: Data(combined[..<terminator.upperBound])
            )
        }
    }

    private static func handleRequest(
        wait: CallbackWait,
        shared: CallbackState,
        serverName: String,
        expectedState: String,
        connection: NWConnection,
        head: Data
    ) {
        guard let requestLine = String(data: head, encoding: .utf8)?
            .components(separatedBy: "\r\n").first
        else {
            connection.cancel()
            return
        }
        let parts = requestLine.split(separator: " ")
        guard parts.count >= 2, parts[0] == "GET" else {
            sendHTML("<html><body>Method not allowed.</body></html>", status: 405, statusText: "Method Not Allowed", on: connection) {
                connection.cancel()
            }
            return
        }
        let target = String(parts[1])
        guard target == "/callback" || target.hasPrefix("/callback?") else {
            sendHTML("<html><body>Not found.</body></html>", status: 404, statusText: "Not Found", on: connection) {
                connection.cancel()
            }
            return
        }
        let queryItems = URLComponents(string: "http://127.0.0.1" + target)?.queryItems ?? []
        func value(_ name: String) -> String? {
            queryItems.first(where: { $0.name == name })?.value
        }
        if let providerError = value("error"), !providerError.isEmpty {
            let description = value("error_description")
            let message = (description?.isEmpty == false ? description! : "The sign-in was refused (\(providerError)).")
            let page = failurePage(serverName: serverName, message: message)
            sendHTML(page, status: 200, statusText: "OK", on: connection) {
                wait.finish(.failure(MCPOAuthError.callbackFailed(message)))
            }
            return
        }
        guard let code = value("code"), !code.isEmpty else {
            let page = failurePage(serverName: serverName, message: "The browser didn't send back a code.")
            sendHTML(page, status: 200, statusText: "OK", on: connection) {
                wait.finish(.failure(MCPOAuthError.callbackFailed("The browser didn't send back a code.")))
            }
            return
        }
        guard value("state") == expectedState else {
            let page = failurePage(serverName: serverName, message: "Sign-in didn't finish. Try again.")
            sendHTML(page, status: 200, statusText: "OK", on: connection) {
                wait.finish(.failure(MCPOAuthError.stateMismatch))
            }
            return
        }
        let page = successPage(serverName: serverName)
        sendHTML(page, status: 200, statusText: "OK", on: connection) {
            let state = shared.current()
            guard let client = state.client, !state.redirectURI.isEmpty else {
                wait.finish(.failure(MCPOAuthError.callbackFailed("Sign-in didn't finish.")))
                return
            }
            wait.finish(.success(CallbackResult(code: code, redirectURI: state.redirectURI, client: client)))
        }
    }

    private static func sendHTML(_ html: String, status: Int, statusText: String, on connection: NWConnection, done: @escaping @Sendable () -> Void) {
        let body = Data(html.utf8)
        let head = "HTTP/1.1 \(status) \(statusText)\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n"
        var payload = Data(head.utf8)
        payload.append(body)
        connection.send(content: payload, completion: .contentProcessed { _ in done() })
    }

    // MARK: - Authorization + token

    private static func authorizationURL(
        metadata: AuthServerMetadata,
        clientID: String,
        redirectURI: String,
        state: String,
        challenge: String
    ) -> URL? {
        guard var components = URLComponents(string: metadata.authorizationEndpoint) else { return nil }
        var items = components.queryItems ?? []
        items.append(contentsOf: [
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "client_id", value: clientID),
            URLQueryItem(name: "redirect_uri", value: redirectURI),
            URLQueryItem(name: "code_challenge", value: challenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
            URLQueryItem(name: "state", value: state),
        ])
        if let resource = metadata.resource, !resource.isEmpty {
            items.append(URLQueryItem(name: "resource", value: resource))
        }
        if let resourceScopes = metadata.resourceScopes, !resourceScopes.isEmpty {
            items.append(URLQueryItem(name: "scope", value: resourceScopes.joined(separator: " ")))
        }
        components.queryItems = items
        return components.url
    }

    private static func exchangeCode(
        serverID: String,
        metadata: AuthServerMetadata,
        client: RegisteredClient,
        code: String,
        redirectURI: String,
        verifier: String
    ) async throws {
        var fields: [(String, String)] = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirectURI),
            ("client_id", client.clientID),
            ("code_verifier", verifier),
        ]
        if let clientSecret = client.clientSecret, !clientSecret.isEmpty {
            fields.append(("client_secret", clientSecret))
        }
        if let resource = metadata.resource, !resource.isEmpty {
            fields.append(("resource", resource))
        }
        let (data, response) = try await postForm(url: metadata.tokenEndpoint, fields: fields)
        guard (200..<300).contains(response.statusCode),
              let json = JSONValue.parse(data),
              let accessToken = json["access_token"]?.string,
              !accessToken.isEmpty
        else {
            throw MCPOAuthError.tokenFailed(oauthErrorMessage(data: data) ?? "The server didn't hand back a token.")
        }
        storeSession(
            serverID: serverID,
            accessToken: accessToken,
            refreshToken: json["refresh_token"]?.string,
            expiresIn: json["expires_in"]?.int,
            tokenEndpoint: metadata.tokenEndpoint,
            clientID: client.clientID,
            clientSecret: client.clientSecret,
            resource: metadata.resource
        )
    }

    private static func performRefresh(serverID: String, stored: JSONValue, refreshToken: String) async throws -> String {
        guard let tokenEndpoint = stored["token_endpoint"]?.string,
              !tokenEndpoint.isEmpty,
              let clientID = stored["client_id"]?.string,
              !clientID.isEmpty
        else {
            throw MCPOAuthError.signedOut
        }
        var fields: [(String, String)] = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refreshToken),
            ("client_id", clientID),
        ]
        if let clientSecret = stored["client_secret"]?.string, !clientSecret.isEmpty {
            fields.append(("client_secret", clientSecret))
        }
        if let resource = stored["resource"]?.string, !resource.isEmpty {
            fields.append(("resource", resource))
        }
        let (data, response) = try await postForm(url: tokenEndpoint, fields: fields)
        guard (200..<300).contains(response.statusCode),
              let json = JSONValue.parse(data),
              let accessToken = json["access_token"]?.string,
              !accessToken.isEmpty
        else {
            throw MCPOAuthError.tokenFailed(oauthErrorMessage(data: data) ?? "The sign-in expired.")
        }
        storeSession(
            serverID: serverID,
            accessToken: accessToken,
            refreshToken: json["refresh_token"]?.string ?? refreshToken,
            expiresIn: json["expires_in"]?.int,
            tokenEndpoint: tokenEndpoint,
            clientID: clientID,
            clientSecret: stored["client_secret"]?.string,
            resource: stored["resource"]?.string
        )
        return accessToken
    }

    private static func storeSession(
        serverID: String,
        accessToken: String,
        refreshToken: String?,
        expiresIn: Int?,
        tokenEndpoint: String,
        clientID: String,
        clientSecret: String?,
        resource: String?
    ) {
        var session: [String: Any] = [
            "access_token": accessToken,
            "token_endpoint": tokenEndpoint,
            "client_id": clientID,
        ]
        if let refreshToken, !refreshToken.isEmpty {
            session["refresh_token"] = refreshToken
        }
        if let expiresIn {
            session["expires_at"] = Date().timeIntervalSince1970 + Double(expiresIn)
        }
        if let clientSecret, !clientSecret.isEmpty {
            session["client_secret"] = clientSecret
        }
        if let resource, !resource.isEmpty {
            session["resource"] = resource
        }
        guard let data = try? JSONSerialization.data(withJSONObject: session, options: [.sortedKeys]),
              let string = String(data: data, encoding: .utf8)
        else { return }
        MCPKeychain.set(string, server: serverID, field: "oauth")
    }

    // MARK: - HTTP helpers

    private static func fetchJSON(url: String) async throws -> JSONValue {
        guard let requestURL = URL(string: url) else {
            throw MCPOAuthError.network("That address isn't valid.")
        }
        var request = URLRequest(url: requestURL, timeoutInterval: 30)
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        let data: Data
        let response: HTTPURLResponse
        do {
            let (body, urlResponse) = try await URLSession.shared.data(for: request)
            data = body
            guard let http = urlResponse as? HTTPURLResponse else {
                throw MCPOAuthError.network("The server didn't answer properly.")
            }
            response = http
        } catch let error as MCPOAuthError {
            throw error
        } catch {
            throw MCPOAuthError.network("")
        }
        guard (200..<300).contains(response.statusCode), let json = JSONValue.parse(data) else {
            throw MCPOAuthError.metadataFailed("The server answered \(response.statusCode).")
        }
        return json
    }

    private static func post(url: String, body: [String: Any]) async throws -> (Data, HTTPURLResponse) {
        guard let requestURL = URL(string: url),
              let bodyData = try? JSONSerialization.data(withJSONObject: body, options: [])
        else {
            throw MCPOAuthError.network("That address isn't valid.")
        }
        var request = URLRequest(url: requestURL, timeoutInterval: 30)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = bodyData
        do {
            let (data, urlResponse) = try await URLSession.shared.data(for: request)
            guard let http = urlResponse as? HTTPURLResponse else {
                throw MCPOAuthError.network("The server didn't answer properly.")
            }
            return (data, http)
        } catch let error as MCPOAuthError {
            throw error
        } catch {
            throw MCPOAuthError.network("")
        }
    }

    private static func postForm(url: String, fields: [(String, String)]) async throws -> (Data, HTTPURLResponse) {
        guard let requestURL = URL(string: url) else {
            throw MCPOAuthError.network("That address isn't valid.")
        }
        var components = URLComponents()
        components.queryItems = fields.map { URLQueryItem(name: $0.0, value: $0.1) }
        var request = URLRequest(url: requestURL, timeoutInterval: 30)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = Data((components.percentEncodedQuery ?? "").utf8)
        do {
            let (data, urlResponse) = try await URLSession.shared.data(for: request)
            guard let http = urlResponse as? HTTPURLResponse else {
                throw MCPOAuthError.network("The server didn't answer properly.")
            }
            return (data, http)
        } catch let error as MCPOAuthError {
            throw error
        } catch {
            throw MCPOAuthError.network("")
        }
    }

    private static func oauthErrorMessage(data: Data) -> String? {
        guard let json = JSONValue.parse(data) else { return nil }
        if let description = json["error_description"]?.string, !description.isEmpty {
            return description
        }
        return json["error"]?.string
    }

    private static func origin(of url: String) throws -> String {
        guard let components = URLComponents(string: url),
              let scheme = components.scheme, !scheme.isEmpty,
              let host = components.host, !host.isEmpty
        else {
            throw MCPOAuthError.discoveryFailed("That address doesn't look like a URL.")
        }
        var origin = "\(scheme)://\(host)"
        if let port = components.port {
            origin += ":\(port)"
        }
        return origin
    }

    // MARK: - PKCE + randomness

    private static func randomToken(byteCount: Int) -> String {
        var bytes = [UInt8](repeating: 0, count: byteCount)
        for index in bytes.indices {
            bytes[index] = UInt8.random(in: UInt8.min...UInt8.max)
        }
        return base64url(Data(bytes))
    }

    private static func pkceChallenge(for verifier: String) -> String {
        let digest = SHA256.hash(data: Data(verifier.utf8))
        return base64url(Data(digest))
    }

    private static func base64url(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    // MARK: - Branded callback pages

    static func successPage(serverName: String) -> String {
        page(
            headTitle: "Signed in to \(escapeHTML(serverName))",
            heading: "Signed in to \(escapeHTML(serverName))",
            message: "You can close this tab and head back to Bottega."
        )
    }

    static func failurePage(serverName: String, message: String) -> String {
        page(
            headTitle: "Sign-in didn't finish \u{2014} \(escapeHTML(serverName))",
            heading: "Sign-in didn't finish",
            message: escapeHTML(message)
        )
    }

    private static func page(headTitle: String, heading: String, message: String) -> String {
        """
        <!DOCTYPE html>
        <html lang="en">
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <meta name="color-scheme" content="dark">
        <title>\(headTitle)</title>
        <style>
        * { box-sizing: border-box; }
        html, body { margin: 0; padding: 0; }
        body {
          background: #0B0B0F;
          color: #F5F5F7;
          font-family: -apple-system, "SF Pro Text", "Helvetica Neue", Helvetica, sans-serif;
          min-height: 100vh;
          display: flex;
          align-items: center;
          justify-content: center;
        }
        .card {
          width: 420px;
          max-width: calc(100vw - 48px);
          padding: 40px;
          border-radius: 22px;
          background: rgba(255, 255, 255, 0.06);
          text-align: center;
        }
        h1 {
          font-size: 22px;
          font-weight: 600;
          color: #F5F5F7;
          margin: 20px 0 8px;
        }
        p {
          font-size: 14px;
          color: rgba(255, 255, 255, 0.62);
          line-height: 1.5;
          margin: 0 0 24px;
        }
        .wordmark {
          font-size: 12px;
          color: rgba(255, 255, 255, 0.4);
          letter-spacing: 0.04em;
        }
        </style>
        </head>
        <body>
        <div class="card">
        <svg width="56" height="56" viewBox="0 0 100 100" opacity="0.9" xmlns="http://www.w3.org/2000/svg"><path fill="#FFFFFF" fill-rule="nonzero" d="M16.00 94.00 C16.00 94.00 16.17 80.33 17.00 74.00 C17.83 67.67 19.50 61.00 21.00 56.00 C22.50 51.00 25.50 47.33 26.00 44.00 C26.50 40.67 26.17 38.83 24.00 36.00 C21.83 33.17 15.83 30.17 13.00 27.00 C10.17 23.83 7.00 17.00 7.00 17.00 C7.00 17.00 16.50 24.50 20.00 27.00 C23.50 29.50 27.50 33.17 28.00 32.00 C28.50 30.83 24.33 24.50 23.00 20.00 C21.67 15.50 20.00 5.00 20.00 5.00 C20.00 5.00 27.50 12.67 31.00 16.00 C34.50 19.33 37.17 23.67 41.00 25.00 C44.83 26.33 50.17 23.33 54.00 24.00 C57.83 24.67 61.00 26.67 64.00 29.00 C67.00 31.33 68.67 34.50 72.00 38.00 C75.33 41.50 80.33 46.33 84.00 50.00 C87.67 53.67 92.17 57.33 94.00 60.00 C95.83 62.67 95.33 64.33 95.00 66.00 C94.67 67.67 93.83 69.00 92.00 70.00 C90.17 71.00 87.33 71.67 84.00 72.00 C80.67 72.33 75.33 71.50 72.00 72.00 C68.67 72.50 67.33 72.67 64.00 75.00 C60.67 77.33 55.33 83.17 52.00 86.00 C48.67 88.83 46.00 90.67 44.00 92.00 C42.00 93.33 40.00 94.00 40.00 94.00 C40.00 94.00 16.00 94.00 16.00 94.00 Z M63.00 38.00 C60.24 38.00 58.00 40.24 58.00 43.00 C58.00 45.76 60.24 48.00 63.00 48.00 C65.76 48.00 68.00 45.76 68.00 43.00 C68.00 40.24 65.76 38.00 63.00 38.00 Z M86.00 61.20 C85.01 61.20 84.20 62.01 84.20 63.00 C84.20 63.99 85.01 64.80 86.00 64.80 C86.99 64.80 87.80 63.99 87.80 63.00 C87.80 62.01 86.99 61.20 86.00 61.20 Z"/></svg>
        <h1>\(heading)</h1>
        <p>\(message)</p>
        <div class="wordmark">Bottega</div>
        </div>
        <script>setTimeout(() => window.close(), 1500);</script>
        </body>
        </html>
        """
    }

    private static func escapeHTML(_ string: String) -> String {
        var escaped = ""
        escaped.reserveCapacity(string.count)
        for character in string {
            switch character {
            case "&": escaped += "&amp;"
            case "<": escaped += "&lt;"
            case ">": escaped += "&gt;"
            case "\"": escaped += "&quot;"
            case "'": escaped += "&#39;"
            default: escaped.append(character)
            }
        }
        return escaped
    }
}
