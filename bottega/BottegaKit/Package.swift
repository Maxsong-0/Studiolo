// swift-tools-version: 6.2
// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import PackageDescription

// The part of Bottega that knows LMAS and nothing about windows: BottegaKit is pure (wire format, what gets
// signed, events reduced to state), BottegaLink is the connection. The P3 iPhone app builds on BottegaKit too.
// BottegaTestKeys holds keys that sign without a finger: for the tests and the probe, never for an app.
let strict: [SwiftSetting] = [
    .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
    .enableUpcomingFeature("InferIsolatedConformances"),
    .enableUpcomingFeature("MemberImportVisibility"),
]

let package = Package(
    name: "BottegaKit",
    platforms: [.macOS(.v26), .iOS(.v26)],
    products: [
        .library(name: "BottegaKit", targets: ["BottegaKit"]),
        .library(name: "BottegaLink", targets: ["BottegaLink"]),
        .library(name: "BottegaTestKeys", targets: ["BottegaTestKeys"]),
    ],
    targets: [
        .target(name: "BottegaKit", swiftSettings: strict),
        .target(name: "BottegaLink", dependencies: ["BottegaKit"], swiftSettings: strict),
        .target(name: "BottegaTestKeys", dependencies: ["BottegaKit", "BottegaLink"], swiftSettings: strict),
        .testTarget(
            name: "BottegaKitTests",
            dependencies: ["BottegaKit"],
            resources: [.copy("Fixtures")],
            swiftSettings: strict
        ),
        .testTarget(
            name: "BottegaLinkTests",
            dependencies: ["BottegaKit", "BottegaLink", "BottegaTestKeys"],
            swiftSettings: strict
        ),
    ],
    swiftLanguageModes: [.v6]
)
