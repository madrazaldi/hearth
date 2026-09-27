// swift-tools-version: 6.0
// The native macOS frontend. It links the Rust core (crates/hearth-ffi) as a
// static library; build it with ./build.sh, which compiles the Rust side
// first. `swift build` alone works once target/debug/libhearth_ffi.a exists.

import Foundation
import PackageDescription

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().path

let package = Package(
  name: "Hearth",
  platforms: [.macOS(.v14)],
  targets: [
    .target(name: "CHearth", path: "Sources/CHearth"),
    .executableTarget(
      name: "Hearth",
      dependencies: ["CHearth"],
      path: "Sources/Hearth",
      swiftSettings: [.swiftLanguageMode(.v5), .enableUpcomingFeature("BareSlashRegexLiterals")],
      linkerSettings: [
        .unsafeFlags(["-L", "\(root)/target/debug"], .when(configuration: .debug)),
        // build.sh lipos the per-architecture release builds into one archive here.
        .unsafeFlags(["-L", "\(root)/target/universal/release"], .when(configuration: .release)),
        .linkedLibrary("hearth_ffi"),
        .linkedLibrary("iconv"),
        .linkedFramework("Security"),
        .linkedFramework("CoreFoundation"),
      ]
    ),
    .testTarget(
      name: "HearthTests",
      dependencies: ["Hearth"],
      path: "Tests/HearthTests",
      swiftSettings: [.swiftLanguageMode(.v5), .enableUpcomingFeature("BareSlashRegexLiterals")]
    ),
  ]
)
