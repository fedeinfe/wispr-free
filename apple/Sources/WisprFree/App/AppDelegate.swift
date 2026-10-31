import AppKit
import WfCore

/// The app's life: start the shell, put up the pill and the menus, and let the shell put things
/// back before going away. It never activates, so starting it can't take the focus from the user.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let options = LaunchOptions(arguments: CommandLine.arguments)
    private var shell: Shell?
    private var pill: PillController?
    private var statusItem: StatusItem?
    private var terminateSignal: DispatchSourceSignal?
    private let events = EventStreams()

    func applicationDidFinishLaunching(_ notification: Notification) {
        quitOnTerminateSignal()
        NSApp.mainMenu = MainMenu.make { [weak self] in self?.pick(.openSettings) }
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
            let paths = try devPaths()
            let shell =
                options.rehearse
                ? try await Shell.rehearse(paths: paths, sink: events.sink)
                : try await Shell.start(paths: paths, sink: events.sink)
            self.shell = shell
            log("shell started")
            Task { await follow() }
            show(on: shell)
            try applyOptions(to: shell)
        } catch {
            log("could not start: \(error)")
            NSApp.terminate(nil)
        }
    }

    private func show(on shell: Shell) {
        let pick: (MenuAction) -> Void = { [weak self] action in self?.pick(action) }
        let pill = PillController(
            shell: shell, surroundings: pillSurroundings, catchesMouse: options.byHand,
            onMenuAction: pick)
        self.pill = pill
        let debug = options.debugPill
        statusItem = StatusItem(
            entries: { (try? shell.statusMenu(debug: debug)) ?? [] }, pick: pick)
        pill.start()
    }

    /// Pictures are taken with the cursor out of the way, wherever the user's is.
    private var pillSurroundings: PillSurroundings {
        options.shots == nil ? RealSurroundings() : Shots.Surroundings()
    }

    private func applyOptions(to shell: Shell) throws {
        if options.byHand {
            try shell.listenToKeys()
        }
        if let name = options.pillState, let state = pillStateNamed(name: name) {
            try shell.stage(scene: .state(state: state))
        }
        if let title = options.pick, statusItem?.press(title) != true {
            log("no item \(title)")
        }
        if let folder = options.shots, let pill {
            Task {
                await Shots(shell: shell, pill: pill, folder: folder).take()
                NSApp.terminate(nil)
            }
        }
    }

    private func follow() async {
        for await event in events.states {
            switch event {
            case .quitAsked:
                NSApp.terminate(nil)
            case .open(let window):
                // Their windows arrive with tickets 11 to 14.
                log("no \(window) window yet")
            default:
                break
            }
        }
    }

    private func pick(_ action: MenuAction) {
        guard let shell else { return }
        Task {
            do {
                try await shell.menuAction(action: action)
            } catch {
                log("the menu failed: \(error)")
            }
        }
    }

    /// With SIGTERM, anything outside can ask the app to quit the way the menu does.
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

nonisolated func log(_ text: String) {
    FileHandle.standardError.write(Data("\(text)\n".utf8))
}
