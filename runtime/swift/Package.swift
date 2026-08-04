// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "EmbeddedRuntime",
  platforms: [.macOS(.v14)],
  products: [
    .library(name: "RuntimeJSCFFI", type: .static, targets: ["RuntimeJSCFFI"])
  ],
  targets: [
    .target(
      name: "RuntimeJSCShim",
      linkerSettings: [.linkedFramework("JavaScriptCore")]
    ),
    .target(
      name: "RuntimeJSCFFI",
      dependencies: ["RuntimeJSCShim"],
      linkerSettings: [.linkedFramework("JavaScriptCore")]
    ),
    .testTarget(name: "RuntimeJSCFFITests", dependencies: ["RuntimeJSCFFI"]),
  ]
)
