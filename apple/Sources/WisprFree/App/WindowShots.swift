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

    private static let scenes: [(String, AppWindow, MenuAction)] = [
        ("settings", .settings, .openSettings),
        ("meetings", .meetings, .openMeetings),
        ("onboarding", .onboarding, .openOnboarding),
    ]

    private let shell: Shell
    private let windows: Windows
    private let folder: URL

    init(shell: Shell, windows: Windows, folder: String) {
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
