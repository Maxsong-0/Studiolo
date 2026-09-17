import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

@Test func revertSummaryPreservesPathsAndCountsBinaryFiles() throws {
    let preview = try RevertPreview(numstat: "12\t3\tSources/main.swift\0-\t-\timage.png\00\t4\ta\tfile\nname.txt\0")
    #expect(preview.files.count == 3)
    #expect(preview.additions == 12)
    #expect(preview.deletions == 7)
    #expect(preview.files[1].isBinary)
    #expect(preview.files[2].path == "a\tfile\nname.txt")
    #expect(throws: (any Error).self) { try RevertPreview(numstat: "not a diff\0") }
}

@Test func commandListIncludesEveryCommandAndRanksRecentUse() {
    let supplied = (0..<20).map { SlashCommand(name: "command-\($0)", detail: "") }
    let commands = SlashCommand.suggestions(supplied + [supplied[0]], matching: "", recent: ["command-18", "command-2"])
    #expect(commands.count == 20)
    #expect(commands.prefix(2).map(\.name) == ["command-18", "command-2"])
    #expect(SlashCommand.suggestions(supplied, matching: "COMMAND-1", recent: []).count == 11)
}

@Test func codexSkillsUseExplicitSkillInputs() {
    let command = SlashCommand(name: "review", detail: "Review code", skillPath: "/skills/review/SKILL.md")
    let input = command.codexInput(for: "/review check this change")
    #expect(input.first?["text"]?.string == "$review check this change")
    #expect(input.last?["type"]?.string == "skill")
    #expect(input.last?["path"]?.string == "/skills/review/SKILL.md")
}

@Test func wideThumbnailsKeepRetinaDetail() async throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".png")
    defer { try? FileManager.default.removeItem(at: url) }
    let context = try #require(CGContext(data: nil, width: 1600, height: 200, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    let source = try #require(context.makeImage())
    let destination = try #require(CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
    CGImageDestinationAddImage(destination, source, nil)
    #expect(CGImageDestinationFinalize(destination))
    let cache = ThumbnailCache()
    let thumbnail = try #require(await cache.thumbnail(for: url.path, pointSize: 48, fillingSquare: true))
    #expect(min(thumbnail.image.width, thumbnail.image.height) >= 96)
    #expect(max(thumbnail.image.width, thumbnail.image.height) <= 1600)
    let fitted = try #require(await cache.thumbnail(for: url.path, pointSize: 48))
    #expect(max(fitted.image.width, fitted.image.height) == 96)
}

private func writePNG(width: Int, height: Int) throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".png")
    let context = try #require(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    // Noise, so the PNG is not a run of identical rows the decoder can skip through.
    if let data = context.data {
        let bytes = data.bindMemory(to: UInt32.self, capacity: context.bytesPerRow / 4 * height)
        var state: UInt32 = 0x9E37_79B9
        for index in 0..<(context.bytesPerRow / 4 * height) {
            state = state &* 1_664_525 &+ 1_013_904_223
            bytes[index] = state | 0xFF00_0000
        }
    }
    let source = try #require(context.makeImage())
    let destination = try #require(CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
    CGImageDestinationAddImage(destination, source, nil)
    #expect(CGImageDestinationFinalize(destination))
    return url
}

@Test func concurrentThumbnailRequestsShareOneDecodeAndDifferentOnesOverlap() async throws {
    let first = try writePNG(width: 3000, height: 3000)
    let second = try writePNG(width: 3000, height: 3000)
    defer { try? FileManager.default.removeItem(at: first); try? FileManager.default.removeItem(at: second) }
    let cache = ThumbnailCache()
    let clock = ContinuousClock()
    // One decode for two callers asking at once.
    async let a = cache.thumbnail(for: first.path, pointSize: 48)
    async let b = cache.thumbnail(for: first.path, pointSize: 48)
    let (imageA, imageB) = try await (#require(a), #require(b))
    #expect(imageA.image === imageB.image)
    #expect(await cache.decodes == 1)
    // Two different images decode side by side: together they take well under twice one alone.
    let alone = try await clock.measure { _ = try #require(await ThumbnailCache().thumbnail(for: second.path, pointSize: 48)) }
    let fresh = ThumbnailCache()
    let together = try await clock.measure {
        async let x = fresh.thumbnail(for: first.path, pointSize: 48)
        async let y = fresh.thumbnail(for: second.path, pointSize: 48)
        _ = try await (#require(x), #require(y))
    }
    #expect(together < alone * 1.6, "two decodes took \(together) against \(alone) for one")
    #expect(await fresh.decodes == 2)
}

@main struct RegressionTests {
    static func main() async {
        let result: CInt = await Testing.__swiftPMEntryPoint()
        exit(result)
    }
}

@Test func providerCatalogsDecodeInvocableCommands() {
    let skills: JSONValue = ["data": [["skills": [
        ["name": "enabled", "path": "/skills/enabled", "description": "Long", "enabled": true, "interface": ["shortDescription": "Short"]],
        ["name": "disabled", "path": "/skills/disabled", "enabled": false],
    ]]]]
    let commands = SlashCommand.codexSkills(skills)
    #expect(commands.map(\.name) == ["enabled"])
    #expect(commands.first?.detail == "Short")
    #expect(SlashCommand.claudeCommands(["compact", ["name": "plugin:review", "description": "Review"]]).map(\.name) == ["compact", "plugin:review"])
}

@Test func commandSearchPrioritizesPrefixesAndDeduplicatesBuiltIns() {
    let local = SlashCommand(name: "plan", detail: "Local", isBuiltIn: true)
    let commands = [local, SlashCommand(name: "plan", detail: "Provider"), SlashCommand(name: "explain", detail: "")]
    let results = SlashCommand.suggestions(commands, matching: "pla", recent: ["explain", "explain"])
    #expect(results.map(\.name) == ["plan", "explain"])
    #expect(results.first?.isBuiltIn == true)
}

@Test func frequentlyUsedThumbnailSurvivesCacheTurnover() async throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".png")
    defer { try? FileManager.default.removeItem(at: url) }
    let context = try #require(CGContext(data: nil, width: 4, height: 4, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    let source = try #require(context.makeImage())
    let destination = try #require(CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
    CGImageDestinationAddImage(destination, source, nil)
    #expect(CGImageDestinationFinalize(destination))
    let cache = ThumbnailCache()
    let first = try #require(await cache.thumbnail(for: url.path, pointSize: 1))
    for pointSize in 2...260 {
        _ = await cache.thumbnail(for: url.path, pointSize: CGFloat(pointSize))
        let hot = try #require(await cache.thumbnail(for: url.path, pointSize: 1))
        if hot.image !== first.image {
            Issue.record("Frequently used thumbnail was decoded again at entry \(pointSize)")
            return
        }
    }
}
