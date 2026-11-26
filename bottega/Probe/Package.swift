// swift-tools-version: 6.2
// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import PackageDescription

// Talks to a real LMAS the way the app does, from a terminal, with keys that need no finger. A package of its own:
// nothing of it can end up in the app. A test could not do this: LMAS refuses Apple's own test runner.
let package = Package(
    name: "Probe",
    platforms: [.macOS(.v26)],
    dependencies: [.package(path: "../BottegaKit")],
    targets: [
        .executableTarget(
            name: "bottega-probe",
            dependencies: [
                .product(name: "BottegaKit", package: "BottegaKit"), .product(name: "BottegaLink", package: "BottegaKit"),
                .product(name: "BottegaTestKeys", package: "BottegaKit"),
            ])
    ],
    swiftLanguageModes: [.v6]
)
