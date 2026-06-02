//  folk-around menu bar companion
//  SwiftUI app that lives in the menu bar, manages the folk-around daemon
//
//  Build: xcodebuild or just open in Xcode
//  Requires: macOS 14+

import SwiftUI
import ServiceManagement
import UniformTypeIdentifiers

@main
struct FolkAroundApp: App {
    @State private var daemon = DaemonController()

    var body: some Scene {
        MenuBarExtra {
            VStack(alignment: .leading, spacing: 8) {
                // Header
                HStack {
                    Circle()
                        .fill(daemon.isRunning ? Color.green : Color.red)
                        .frame(width: 8, height: 8)
                    Text("folk-around")
                        .font(.headline)
                    Spacer()
                }
                .padding(.bottom, 4)

                // Status
                Text(daemon.statusText)
                    .font(.caption)
                    .foregroundColor(.secondary)

                Divider()

                // Controls
                Button(daemon.isRunning ? "Stop Daemon" : "Start Daemon") {
                    daemon.toggle()
                }
                .keyboardShortcut("r")

                Button("Restart") {
                    daemon.restart()
                }
                .keyboardShortcut("r", modifiers: [.command, .option])
                .disabled(!daemon.isRunning)

                Divider()

                // Mode selector
                Picker("Mode", selection: $daemon.mode) {
                    Text("Full").tag("full")
                    Text("Limited").tag("limited")
                    Text("Sandbox").tag("sandbox")
                }
                .pickerStyle(.inline)
                .disabled(daemon.isRunning)

                Divider()

                // Transport info
                if let port = daemon.httpPort {
                    HStack {
                        Text("HTTP")
                            .font(.caption)
                        Spacer()
                        Text(":\(port)")
                            .font(.caption.monospaced())
                    }
                    HStack {
                        Text("SSE")
                            .font(.caption)
                        Spacer()
                        Text("http://localhost:\(port)/sse")
                            .font(.caption.monospaced())
                    }
                }

                Divider()

                // Tool list
                Text("9 tools registered")
                    .font(.caption)
                    .foregroundColor(.secondary)

                Divider()

                Button("Open Config (~/.folk-around.toml)") {
                    daemon.openConfig()
                }

                Button("Logs...") {
                    daemon.openLogs()
                }

                Divider()

                Button("Quit folk-around") {
                    daemon.quit()
                }
                .keyboardShortcut("q")
            }
            .padding()
            .frame(width: 240)
        } label: {
            Image(systemName: daemon.isRunning ? "circle.fill" : "circle")
                .foregroundColor(daemon.isRunning ? .green : .secondary)
        }
    }
}

@MainActor
class DaemonController: ObservableObject {
    @Published var isRunning = false
    @Published var mode = "full"
    @Published var httpPort: UInt16? = nil

    private var process: Process?

    var statusText: String {
        if isRunning {
            return "Running (mode: \(mode))"
        }
        return "Stopped"
    }

    func toggle() {
        if isRunning {
            stop()
        } else {
            start()
        }
    }

    func start() {
        guard process == nil else { return }

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/local/bin/folk-around")
        proc.arguments = ["--mode", mode, "--http", "8080", "--verbose"]

        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = pipe

        // Read output for status
        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            if !data.isEmpty, let line = String(data: data, encoding: .utf8) {
                DispatchQueue.main.async {
                    if line.contains("HTTP listening") {
                        self?.httpPort = 8080
                    }
                }
            }
        }

        proc.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async {
                self?.process = nil
                self?.isRunning = false
            }
        }

        do {
            try proc.run()
            process = proc
            isRunning = true
        } catch {
            print("Failed to start: \(error)")
        }
    }

    func stop() {
        process?.terminate()
        process = nil
        isRunning = false
        httpPort = nil
    }

    func restart() {
        stop()
        // Small delay to ensure cleanup
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.start()
        }
    }

    func quit() {
        stop()
        NSApplication.shared.terminate(nil)
    }

    func openConfig() {
        let path = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".folk-around.toml")
        NSWorkspace.shared.open(path)
    }

    func openLogs() {
        let path = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/folk-around.log")
        NSWorkspace.shared.open(path)
    }
}