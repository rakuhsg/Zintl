// swift-tools-version: 6.0

import Foundation
import PackageDescription

let runtimeLibraryPath = URL(fileURLWithPath: #filePath)
  .deletingLastPathComponent()
  .deletingLastPathComponent()
  .appendingPathComponent("target/debug")
  .path

let package = Package(
  name: "EmbeddedRuntime",
  platforms: [.macOS(.v14)],
  products: [
    .library(name: "RuntimeJSC", targets: ["RuntimeJSC"]),
    .library(name: "RuntimeEmbed", targets: ["RuntimeEmbed"]),
    .executable(name: "EmbeddingLifecycleExample", targets: ["EmbeddingLifecycleExample"]),
  ],
  targets: [
    .systemLibrary(name: "CRuntimeFFI"),
    .target(
      name: "RuntimeJSCShim",
      linkerSettings: [.linkedFramework("JavaScriptCore")]
    ),
    .target(
      name: "RuntimeJSC",
      dependencies: ["CRuntimeFFI", "RuntimeJSCShim"],
      linkerSettings: [
        .linkedFramework("JavaScriptCore"),
        .unsafeFlags(["-L", runtimeLibraryPath, "-lruntime_ffi"]),
      ]
    ),
    .target(name: "RuntimeEmbed", dependencies: ["RuntimeJSC"]),
    .executableTarget(
      name: "EmbeddingLifecycleExample",
      dependencies: ["RuntimeEmbed", "RuntimeJSC"]
    ),
    .testTarget(name: "RuntimeJSCTests", dependencies: ["RuntimeJSC", "RuntimeEmbed"]),
  ]
)
