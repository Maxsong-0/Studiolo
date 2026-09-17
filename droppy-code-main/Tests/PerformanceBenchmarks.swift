import Foundation

@main struct PerformanceBenchmarks {
    static func main() async throws {
        func countFDs() -> Int { (0..<4096).reduce(0) { $0 + (fcntl(Int32($1), F_GETFD) == -1 ? 0 : 1) } }
        let before = countFDs()
        let start = ContinuousClock.now
        for _ in 0..<100 {
            _ = try await Shell.run(URL(fileURLWithPath: "/usr/bin/true"), [], environment: [:])
        }
        print("100 processes: \(start.duration(to: .now)), open descriptors before=\(before) after=\(countFDs())")
        for size in [65_536, 262_144] {
            let bytes = JSONValue.object(["text": .string(String(repeating: "x", count: size))]).data() + Data([10])
            let process = StdioProcess(executable: URL(fileURLWithPath: "/usr/bin/true"), arguments: [], directory: FileManager.default.temporaryDirectory, environment: [:])
            let start = ContinuousClock.now
            for offset in stride(from: 0, to: bytes.count, by: 128) {
                process.consume(Data(bytes[offset..<min(offset + 128, bytes.count)]))
            }
            var iterator = process.messages.makeAsyncIterator()
            let message = await iterator.next()
            let decoded = message?["text"]?.string?.utf8.count ?? 0
            precondition(decoded == size)
            print("fragmented line bytes=\(size) duration=\(start.duration(to: .now)) decoded=\(decoded)")
        }
        let text = String(repeating: "Plain model text and code with no replacements.\n", count: 1000)
        var checksum = 0
        var textStart = ContinuousClock.now
        for _ in 0..<100 { checksum += TextCleanup.withoutEmDashes(text).utf8.count }
        print("100 plain text cleanups: \(textStart.duration(to: .now)) bytes=\(checksum)")
        textStart = .now
        for _ in 0..<100 { checksum += TextCleanup.singleLine(text).utf8.count }
        print("100 first-line summaries: \(textStart.duration(to: .now)) bytes=\(checksum)")
    }
}
