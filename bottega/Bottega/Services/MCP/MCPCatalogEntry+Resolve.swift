// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A catalog entry made launchable. A server that signs in is reached through its route on
/// the app's proxy (`MCPProxy`), which lives in this layer, so this sits apart from the
/// entries themselves (`Core/Models/MCP.swift`), which stay plain values.
extension MCPCatalogEntry {
    /// The launchable form, with `{key}` placeholders replaced by the user's values (or the
    /// field's default). OAuth servers resolve to their route on the app's proxy at
    /// `proxyPort`: the port it has now, unless the caller read one of its own to write
    /// down beside the result (see `MCPStore.exportAll`).
    func resolve(values: [String: String], proxyPort: UInt16 = MCPProxy.port) -> MCPResolvedServer {
        func fill(_ text: String) -> String { self.fill(text, values: values) }
        var server = MCPResolvedServer(id: id, name: name)
        switch transport {
        case .stdio(let command, let args):
            server.command = command
            server.args = args.map(fill)
            // An optional field left empty drops its variable rather than passing "".
            server.env = env.mapValues(fill).filter { !$0.value.isEmpty }
        case .http(let url, let headers):
            server.url = fill(url)
            server.headers = headers.mapValues(fill).filter { !$0.value.isEmpty }
        case .oauth(let url):
            server.url = MCPProxy.url(for: id, port: proxyPort)
            server.oauthUpstream = fill(url)
        }
        return server
    }
}
