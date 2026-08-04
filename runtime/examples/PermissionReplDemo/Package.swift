// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "PermissionReplDemo",
  platforms: [.macOS(.v14)],
  products: [
    .executable(name: "PermissionReplDemo", targets: ["PermissionReplDemo"]),
    .library(name: "PermissionReplDemoSupport", targets: ["PermissionReplDemoSupport"]),
  ],
  dependencies: [
    .package(path: "../../swift")
  ],
  targets: [
    .target(
      name: "PermissionReplDemoSupport",
      dependencies: [
        .product(name: "RuntimeEmbed", package: "swift"),
        .product(name: "RuntimeJSC", package: "swift"),
      ]
    ),
    .executableTarget(
      name: "PermissionReplDemo",
      dependencies: [
        "PermissionReplDemoSupport",
        .product(name: "RuntimeJSC", package: "swift"),
      ]
    ),
    .testTarget(
      name: "PermissionReplDemoTests",
      dependencies: [
        "PermissionReplDemoSupport",
        .product(name: "RuntimeJSC", package: "swift"),
      ]
    ),
  ]
)
