// swift-tools-version: 6.2
import PackageDescription
let frameworks = ["CoreAudio", "ScreenCaptureKit", "UniformTypeIdentifiers", "AVFoundation", "CoreMedia", "AudioToolbox", "Carbon", "IOKit", "ApplicationServices", "CoreServices", "CoreVideo", "CoreData", "CoreText", "CoreImage", "Metal", "CloudKit", "QuartzCore"]
let package = Package(
  name: "WisprFree",
  platforms: [.macOS(.v26)],
  targets: [
    .binaryTarget(name: "WfBridge", path: "WfBridge.xcframework"),
    .target(name: "WfCore", dependencies: ["WfBridge"], swiftSettings: [.swiftLanguageMode(.v6)],
            linkerSettings: frameworks.map { .linkedFramework($0) }),
    .executableTarget(name: "WisprFree", dependencies: ["WfCore"],
                      swiftSettings: [.swiftLanguageMode(.v6), .defaultIsolation(MainActor.self)]),
  ])
