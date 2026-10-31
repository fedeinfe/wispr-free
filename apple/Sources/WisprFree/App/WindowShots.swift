import AppKit
import WfCore

/// `cargo xtask shots`: every window in light and dark, each over its own backdrop, opened the way
/// a menu opens it. As with the pill, both sit behind everything else and the capture asks for
/// those two alone, so the user sees nothing and nothing activates.
final class WindowShots {
    /// Long enough for a window to draw in a new appearance.
    private static let settle: Duration = .milliseconds(450)
    private static let poll: Duration = .milliseconds(20)
    private static let attempts = 250
    /// Backdrop around the window, so its corners and shadow show.
    private static let margin: CGFloat = 28

    /// Settings is shot once per page. Tall, a long page shows whole.
    private static let tallHeight: CGFloat = 1500

    private static let scenes: [(String, AppWindow, MenuAction)] = [
        ("meetings", .meetings, .openMeetings),
        ("onboarding", .onboarding, .openOnboarding),
    ]

    private let shell: Shell
    private let windows: Windows
    private let folder: URL

    /// A tour takes no pictures, so it needs no `folder`.
    init(shell: Shell, windows: Windows, folder: String = "") {
        self.shell = shell
        self.windows = windows
        self.folder = URL(filePath: folder, directoryHint: .isDirectory)
    }

    func take() async {
        do {
            // Asking ScreenCaptureKit without the grant would bring up a prompt.
            guard CGPreflightScreenCaptureAccess() else {
                log("shots need Screen Recording")
                return
            }
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try fillInThePages()
            for row in settingsPages() {
                try await shell.setSettingsPage(page: row.page)
                try await shell.menuAction(action: .openSettings)
                let name = "settings-\(row.page)"
                guard let window = await open(.settings) else {
                    throw Shots.ShotError.noPicture(name)
                }
                try await shoot(name, window)
                window.setContentSize(NSSize(width: SettingsView.shape.size.width, height: Self.tallHeight))
                try await shoot("\(name)-tall", window)
                window.close()
                await closed(.settings)
            }
            for (name, kind, action) in Self.scenes {
                try await shell.menuAction(action: action)
                guard let window = await open(kind) else { throw Shots.ShotError.noPicture(name) }
                try await shoot(name, window)
                window.close()
            }
            log("shots taken")
        } catch {
            log("shots failed: \(error)")
        }
    }

    /// So the pages show their lists open, with something in them.
    private func fillInThePages() throws {
        try shell.changeSetting(change: .tier(tier: .balanced))
        try shell.setMicrophoneAuto(on: false)
        try shell.setLanguagesAuto(on: false)
        for language: Int32 in [0, 1] { try shell.toggleLanguage(index: language) }
        try shell.addWord(word: "Whisper")
        try shell.addReplacement()
        try shell.editReplacement(index: 0, row: Replacement(wrong: "wisper", right: "Whisper"))
        try shell.addSnippet()
        try shell.editSnippet(
            index: 0, row: Snippet(trigger: "my address", text: "Via Roma 1, Milano"))
    }

    /// `cargo xtask footprint --settings`: every page, drawn, `rounds` times over.
    func tour(_ rounds: Int) async {
        do {
            for _ in 0..<rounds {
                for row in settingsPages() {
                    try await shell.setSettingsPage(page: row.page)
                    try await shell.menuAction(action: .openSettings)
                    guard let window = await open(.settings) else {
                        throw Shots.ShotError.noPicture("settings")
                    }
                    try await Task.sleep(for: Self.settle)
                    window.close()
                    await closed(.settings)
                }
            }
            log("toured")
        } catch {
            log("the tour failed: \(error)")
        }
    }

    private func closed(_ kind: AppWindow) async {
        for _ in 0..<Self.attempts where windows.window(kind) != nil {
            try? await Task.sleep(for: Self.poll)
        }
    }

    /// The event is on its way, so the window is too.
    private func open(_ kind: AppWindow) async -> NSWindow? {
        for _ in 0..<Self.attempts {
            if let window = windows.window(kind) { return window }
            try? await Task.sleep(for: Self.poll)
        }
        return nil
    }

    private func shoot(_ name: String, _ window: NSWindow) async throws {
        let frame = window.frame.insetBy(dx: -Self.margin, dy: -Self.margin)
        let backdrop = NSWindow(
            contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
        backdrop.isReleasedWhenClosed = false
        backdrop.orderBack(nil)
        for look in Shots.backdrops.prefix(2) {
            NSApp.appearance = NSAppearance(named: look.appearance)
            backdrop.backgroundColor = look.color
            try await Task.sleep(for: Self.settle)
            let image = try await Shots.capture(
                [backdrop, window], frame: frame, scale: window.backingScaleFactor)
            try Shots.write(image, to: folder.appending(path: "\(name)-\(look.name).png"))
        }
        backdrop.close()
    }
}
