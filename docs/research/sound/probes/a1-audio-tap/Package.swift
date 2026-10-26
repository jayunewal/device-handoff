// swift-tools-version:6.0
// SPDX-License-Identifier: Apache-2.0
import PackageDescription
let package = Package(
    name: "a1audiotap",
    platforms: [.macOS(.v15)],
    targets: [
        .executableTarget(name: "tonegen", path: "Sources/tonegen"),
        .executableTarget(name: "tapprobe", path: "Sources/tapprobe"),
    ],
    swiftLanguageModes: [.v5]
)
