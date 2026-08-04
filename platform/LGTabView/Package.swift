// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "AppKitSandbox",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .library(name: "LiquidGlassTabs", targets: ["LiquidGlassTabs"]),
        .executable(name: "AppKitSandbox", targets: ["AppKitSandbox"]),
        .executable(name: "ComplexApp", targets: ["ComplexApp"])
    ],
    targets: [
        .target(
            name: "LiquidGlassTabs",
            path: "Library/Sources/LiquidGlassTabs"
        ),
        .executableTarget(
            name: "AppKitSandbox",
            dependencies: ["LiquidGlassTabs"],
            path: "DemoApps/AppKitSandbox"
        ),
        .executableTarget(
            name: "ComplexApp",
            dependencies: ["LiquidGlassTabs"],
            path: "DemoApps/ComplexApp"
        )
    ]
)
