import AppKit
import WfCore

/// The app's life: start the shell, and let it put things back before going away. It never
/// activates and shows nothing, so starting it can't take the focus from the user.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var shell: Shell?
    private var terminateSignal: DispatchSourceSignal?
    private let events = EventStreams()

    func applicationDidFinishLaunching(_ notification: Notification) {
        quitOnTerminateSignal()
        Task { await start() }
    }

    /// Replies through the run loop, not the main actor: when `terminate` is called from a block on
    /// the main queue, its modal loop runs inside that block, and the queue never gets to anything
    /// else.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let shell else { return .terminateNow }
        Task.detached {
            do {
                try await shell.quit()
                log("shell closed")
            } catch {
                log("quitting failed: \(error)")
            }
            RunLoop.main.perform(inModes: [.common]) {
                MainActor.assumeIsolated {
                    NSApp.reply(toApplicationShouldTerminate: true)
                }
            }
        }
        return .terminateLater
    }

    private func start() async {
        do {
            shell = try await Shell.start(paths: try devPaths(), sink: events.sink)
            log("shell started")
        } catch {
            log("could not start: \(error)")
            NSApp.terminate(nil)
        }
    }

    /// With no menu yet, this is how anything outside asks the app to quit.
    private func quitOnTerminateSignal() {
        signal(SIGTERM, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        source.setEventHandler {
            MainActor.assumeIsolated { NSApp.terminate(nil) }
        }
        source.resume()
        terminateSignal = source
    }
}

private nonisolated func log(_ text: String) {
    FileHandle.standardError.write(Data("\(text)\n".utf8))
}
