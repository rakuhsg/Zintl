// swift-tools-version: 6.2

import PackageDescription

let package = Package(
  name: "WebGPUByowMacOS",
  platforms: [
    .macOS(.v10_15)
  ],
  products: [
    .library(
      name: "webgpu_byow_macos",
      type: .dynamic,
      targets: ["WebGPUByowMacOS"]
    )
  ],
  targets: [
    .target(name: "WebGPUByowMacOS")
  ]
)
