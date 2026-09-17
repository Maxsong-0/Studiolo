import Foundation
import Testing
@testable import DroppyCode

@MainActor private func sessionFixture(mode: String) throws -> (CodexSession, URL) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("codex-revert-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let fixture: [String: Any] = ["mode": mode, "turns": ["keep", "remove", "external-turn", "latest"]]
    try JSONSerialization.data(withJSONObject: fixture).write(to: directory.appendingPathComponent("rewind-fixture.json"))
    let executable = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("Tests/Fixtures/codex-rewind.py")
    let session = CodexSession(configuration: SessionConfiguration(provider: .codex, executable: executable, workingDirectory: directory, environment: [:], resumeID: "existing-session", resumeAt: nil, model: nil, effort: nil, runtimeMode: .supervised, interactionMode: .build))
    return (session, directory)
}

@Test @MainActor func codexRevertUsesNativeTurnsAcrossPages() async throws {
    let (session, directory) = try sessionFixture(mode: "success")
    defer { session.stop(); try? FileManager.default.removeItem(at: directory) }
    _ = try await session.start()
    _ = try await session.rollback(from: "remove")
    let state = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("rewind-fixture.json"))) as? [String: Any]
    #expect(state?["turns"] as? [String] == ["keep"])
    #expect(state?["rollbackCount"] as? Int == 3)
}

@Test @MainActor func paginatedCodexRevertForksOnlyRetainedHistory() async throws {
    let (session, directory) = try sessionFixture(mode: "paginated")
    defer { session.stop(); try? FileManager.default.removeItem(at: directory) }
    _ = try await session.start()
    #expect(try await session.rollback(from: "remove") == "reverted-session")
    let state = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("rewind-fixture.json"))) as? [String: Any]
    #expect(state?["forkTurns"] as? [String] == ["keep"])
    #expect(state?["archived"] as? [String] == ["existing-session"])
}

@Test @MainActor func paginatedCodexRevertOfFirstTurnClearsTheSession() async throws {
    let (session, directory) = try sessionFixture(mode: "paginated")
    defer { session.stop(); try? FileManager.default.removeItem(at: directory) }
    _ = try await session.start()
    #expect(try await session.rollback(from: "keep") == nil)
    let state = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("rewind-fixture.json"))) as? [String: Any]
    #expect(state?["forkTurns"] == nil)
    #expect(state?["archived"] as? [String] == ["existing-session"])
}

@Test @MainActor func paginatedCodexRevertRejectsWrongForkBoundary() async throws {
    let (session, directory) = try sessionFixture(mode: "forkUnchanged")
    defer { session.stop(); try? FileManager.default.removeItem(at: directory) }
    _ = try await session.start()
    await #expect(throws: (any Error).self) { try await session.rollback(from: "remove") }
    let state = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("rewind-fixture.json"))) as? [String: Any]
    #expect(state?["archived"] as? [String] == ["reverted-session"])
}

@Test @MainActor func paginatedCodexRevertKeepsOriginalWhenArchiveFails() async throws {
    let (session, directory) = try sessionFixture(mode: "archiveError")
    defer { session.stop(); try? FileManager.default.removeItem(at: directory) }
    _ = try await session.start()
    await #expect(throws: (any Error).self) { try await session.rollback(from: "remove") }
    let state = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("rewind-fixture.json"))) as? [String: Any]
    #expect(state?["archived"] as? [String] == ["reverted-session"])
    #expect(state?["turns"] as? [String] == ["keep", "remove", "external-turn", "latest"])
}

@Test @MainActor func codexRevertRejectsUnchangedProviderHistory() async throws {
    let (session, directory) = try sessionFixture(mode: "unchanged")
    defer { session.stop(); try? FileManager.default.removeItem(at: directory) }
    _ = try await session.start()
    await #expect(throws: (any Error).self) { try await session.rollback(from: "remove") }
}

@Test @MainActor func codexResumeFailureDoesNotCreateAnotherConversation() async throws {
    let (session, directory) = try sessionFixture(mode: "resumeError")
    defer { session.stop(); try? FileManager.default.removeItem(at: directory) }
    await #expect(throws: (any Error).self) { try await session.start() }
    #expect(!session.isRunning)
    let state = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("rewind-fixture.json"))) as? [String: Any]
    #expect((state?["methods"] as? [String])?.contains("thread/start") == false)
}
