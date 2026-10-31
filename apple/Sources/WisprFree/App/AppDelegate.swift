import AppKit
import WfCore

/// The app's life: start the shell, put up the pill and the menus, and let the shell put things
/// back before going away. It never activates, so starting it can't take the focus from the user.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let options = LaunchOptions(arguments: CommandLine.arguments)
    private var shell: Shell?
    private var pill: PillController?
    private var statusItem: StatusItem?
    private var editor: SettingsEditor?
    private var pages: SettingsPages?
    private var walk: SetupWalk?
    private var windows: Windows?
    /// Picks run in the order they were made.
    private var picking: Task<Void, Never>?
    private var terminateSignal: DispatchSourceSignal?
    private let events = EventStreams()

    func applicationDidFinishLaunching(_ notification: Notification) {
        quitOnTerminateSignal()
        NSApp.mainMenu = MainMenu.make { [weak self] in self?.pick(.openSettings) }
        Task { await start() }
    }

    /// Only a person can want this. A check that sees this line has taken the focus.
    func applicationDidBecomeActive(_ notification: Notification) {
        log("took the focus")
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
            Task { await followLevels() }
            try show(on: shell)
            try applyOptions(to: shell)
        } catch {
            log("could not start: \(error)")
            NSApp.terminate(nil)
        }
    }

    private func show(on shell: Shell) throws {
        let editor = try SettingsEditor(core: shell)
        self.editor = editor
        let pages = SettingsPages(core: shell)
        self.pages = pages
        let walk = SetupWalk(core: shell)
        self.walk = walk
        windows = Windows(
            core: shell, editor: editor, pages: pages, walk: walk,
            stage: options.behind ? .behind : .front)
        if options.byHand, editor.settings.general.showInDock {
            showInDock(true)
        }
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
        } else if options.watchesSetup {
            try shell.watchSetup()
        }
        if let name = options.pillState, let state = pillStateNamed(name: name) {
            try shell.stage(scene: .state(state: state))
        }
        for title in options.picks where statusItem?.press(title) != true {
            log("no item \(title)")
        }
        if let folder = options.shots, let pill {
            Task {
                await Shots(shell: shell, pill: pill, folder: folder).take()
                NSApp.terminate(nil)
            }
        }
        if let folder = options.windowShots, let windows {
            Task {
                await WindowShots(shell: shell, windows: windows, folder: folder).take()
                NSApp.terminate(nil)
            }
        }
        if let rounds = options.tour, let windows {
            Task { await WindowShots(shell: shell, windows: windows).tour(rounds) }
        }
        if options.walkMeeting, let windows {
            Task {
                await MeetingWalk(shell: shell, windows: windows).take()
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
                windows?.open(window)
            case .openQuietly(let window):
                windows?.openQuietly(window)
            case .raise(let window):
                windows?.raise(window)
            case .close(let window):
                windows?.close(window)
            case .setup(let page):
                walk?.adopt(page)
            case .meetings(let changes):
                windows?.book?.adopt(changes)
            case .settingsChanged(let current):
                editor?.adopt(current)
            case .pages(let state):
                pages?.adopt(state)
            case .pickFolder(let start):
                pickFolder(from: start)
            case .showInDock(let visible):
                showInDock(visible)
            default:
                break
            }
        }
    }

    private func followLevels() async {
        for await level in events.levels {
            pages?.hear(level: level)
            walk?.hear(level: level)
        }
    }

    /// A panel takes the focus, so it's only for a person. Anything else gets its answer ready.
    private func pickFolder(from start: String) {
        let answer: (String) -> Void = { [weak self] folder in
            self?.pages?.ask { try $0.folderPicked(folder: folder) }
        }
        if options.byHand {
            // The models step asks too, and that step closed Settings.
            let over = windows?.window(.settings) ?? windows?.window(.onboarding)
            FolderPanel.ask(from: start, over: over, answer: answer)
        } else if let folder = options.pickFolder {
            answer(folder)
        } else {
            log("would ask for a folder from \(start)")
        }
    }

    /// Only a run a person started may change how it sits on their Mac.
    private func showInDock(_ visible: Bool) {
        guard options.byHand else {
            log("would show in Dock: \(visible)")
            return
        }
        NSApp.setActivationPolicy(visible ? .regular : .accessory)
        NSApp.activate()
    }

    private func pick(_ action: MenuAction) {
        guard let shell else { return }
        let previous = picking
        picking = Task {
            await previous?.value
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
