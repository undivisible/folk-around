// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "FolkAround",
    platforms: [
        .macOS(.v14)
    ],
    targets: [
        .executableTarget(
            name: "FolkAround",
            path: ".",
            exclude: [
                "folk-around",
                "src",
                "scripts",
                "README.md",
                "AGENTS.md"
            ]
        )
    ]
)
