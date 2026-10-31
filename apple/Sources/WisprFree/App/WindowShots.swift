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

    /// Settings is shot once per page. Tall, a long page shows whole, up to the screen's height;
    /// scrolled to its end, the rest shows too.
    private static let tallHeight: CGFloat = 1500

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
            try await shootTheWalk()
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
                scrollToEnd(window)
                try await shoot("\(name)-end", window)
                window.close()
                await closed(.settings)
            }
            try await shootTheMeetings()
            try await shootTheMissingModels()
            log("shots taken")
        } catch {
            log("shots failed: \(error)")
        }
    }

    /// A first run, step by step. A rehearsal answers every "Allow…" at once.
    private func shootTheWalk() async throws {
        let window = try await openTheWalk()
        try await until { $0.step == .microphone && $0.microphoneMark == .passed }
        try await shoot("onboarding-microphone", window)
        try shell.askPermission(index: 0)
        try await until { $0.canContinue }
        try await shoot("onboarding-microphone-allowed", window)
        try shell.setupContinue()
        try await until { $0.step == .keys }
        try await shoot("onboarding-keys", window)
        try shell.askPermission(index: 2)
        try shell.askPermission(index: 1)
        try await until { $0.canContinue }
        try shell.setupContinue()
        try await until { $0.step == .models }
        try await shoot("onboarding-models", window)
        scrollToEnd(window)
        try await shoot("onboarding-models-end", window)
        try shell.setupContinue()
        try await until { $0.step == .trial }
        try await shoot("onboarding-trial", window)
        try shell.startTrial()
        try await until { $0.trial.canStop }
        try await shoot("onboarding-trial-listening", window)
        try shell.stopTrial()
        try await until { $0.trial.heard != nil && $0.trial.phase == .idle }
        try await shoot("onboarding-trial-heard", window)
        scrollToEnd(window)
        try await shoot("onboarding-trial-end", window)
        window.close()
        await closed(.onboarding)
    }

    /// With nothing picked, then each tab of the first meeting and the states that show only for a
    /// moment. The last picture is the smallest the window goes.
    private func shootTheMeetings() async throws {
        let (window, book) = try await openTheMeetings()
        try await shoot("meetings-nothing-picked", window)
        try await pickAMeeting(book)
        for tab in MeetingTab.allCases {
            book.tab = tab
            try await shoot("meetings-\(tab.title.lowercased())", window)
        }
        book.tab = .transcript
        try await Task.sleep(for: Self.settle)
        scrollToEnd(window)
        try await shoot("meetings-transcript-end", window)
        book.tab = .chat
        book.askAbout("What was decided?")
        try await waitFor("the answer") { book.streamingText.contains(" ") }
        try await shoot("meetings-chat-streaming", window)
        try await waitFor("the end of the answer") { !book.chatRunning }
        try await shoot("meetings-chat-answered", window)
        if let chip = book.chat.last?.blocks.first?.chips.first, chip.row >= 0 {
            book.show(row: chip.row)
            try await shoot("meetings-jumped", window)
        }
        try shell.exportMeeting()
        try await waitFor("the export") { !book.notice.isEmpty }
        book.tab = .summary
        try await shoot("meetings-exported", window)
        book.query = "budget"
        try await waitFor("the hits") { book.meetings.contains { !$0.snippet.isEmpty } }
        try await shoot("meetings-search", window)
        book.query = "no such words"
        try await waitFor("no hits") { book.meetings.isEmpty }
        try await shoot("meetings-no-hits", window)
        book.query = ""
        try await waitFor("the list") { !book.meetings.isEmpty }
        let first = book.title
        book.selection = book.meetings.last?.id
        try await waitFor("the other meeting") { !book.when.isEmpty && book.title != first }
        try await shoot("meetings-unfinished", window)
        window.setContentSize(MeetingsView.shape.minimum)
        try await shoot("meetings-smallest", window)
        window.close()
        await closed(.meetings)
    }

    private func openTheMeetings() async throws -> (NSWindow, MeetingsBook) {
        try await shell.menuAction(action: .openMeetings)
        guard let window = await open(.meetings), let book = windows.book else {
            throw Shots.ShotError.noPicture("meetings")
        }
        return (window, book)
    }

    private func pickAMeeting(_ book: MeetingsBook) async throws {
        try await waitFor("the list") { !book.meetings.isEmpty }
        book.selection = book.meetings.first?.id
        try await waitFor("the meeting") { !book.when.isEmpty && !book.transcript.isEmpty }
    }

    private func waitFor(_ what: String, _ done: () -> Bool) async throws {
        for _ in 0..<Self.attempts {
            if done() { return }
            try await Task.sleep(for: Self.poll)
        }
        throw Shots.ShotError.noPicture(what)
    }

    /// The pages' tier needs models that aren't there, so the walk comes back to that step.
    private func shootTheMissingModels() async throws {
        let window = try await openTheWalk()
        try await until { $0.step == .models }
        try await shoot("onboarding-models-missing", window)
        scrollToEnd(window)
        try await shoot("onboarding-models-missing-end", window)
        window.close()
        await closed(.onboarding)
    }

    private func openTheWalk() async throws -> NSWindow {
        try await shell.menuAction(action: .openOnboarding)
        guard let window = await open(.onboarding) else {
            throw Shots.ShotError.noPicture("onboarding")
        }
        return window
    }

    private func until(_ done: (SetupPage) -> Bool) async throws {
        for _ in 0..<Self.attempts {
            if let page = try await shell.steps(), done(page) { return }
            try await Task.sleep(for: Self.poll)
        }
        throw Shots.ShotError.noPicture("onboarding")
    }

    private func scrollToEnd(_ window: NSWindow) {
        guard let content = window.contentView else { return }
        let page = Self.scrollers(in: content)
            .compactMap(\.documentView)
            .max { $0.frame.height < $1.frame.height }
        guard let page else { return }
        page.scroll(NSPoint(x: 0, y: page.isFlipped ? page.bounds.maxY : page.bounds.minY))
    }

    static func scrollers(in view: NSView) -> [NSScrollView] {
        view.subviews.flatMap(scrollers) + [view as? NSScrollView].compactMap { $0 }
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

    /// `cargo xtask footprint`: every window, used and closed, `rounds` times over.
    func tour(_ rounds: Int) async {
        do {
            for _ in 0..<rounds {
                try await tourSettings()
                try await tourTheMeetings()
                try await tourTheWalk()
            }
            log("toured")
        } catch {
            log("the tour failed: \(error)")
        }
    }

    private func tourSettings() async throws {
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

    /// Every tab drawn, the transcript to its end, an answer asked for and a search run. A library
    /// with nothing in it still opens the window.
    private func tourTheMeetings() async throws {
        let (window, book) = try await openTheMeetings()
        try await Task.sleep(for: Self.settle)
        if try await pickIfAny(book) {
            for tab in MeetingTab.allCases {
                book.tab = tab
                try await Task.sleep(for: Self.settle)
            }
            book.tab = .transcript
            try await Task.sleep(for: Self.settle)
            scrollToEnd(window)
            try await Task.sleep(for: Self.settle)
            book.tab = .chat
            book.askAbout("What was decided?")
            try await Task.sleep(for: Self.settle)
            try? await waitFor("the end of the answer") { !book.chatRunning }
            book.query = "the"
            try await Task.sleep(for: Self.settle)
            book.query = ""
        }
        window.close()
        await closed(.meetings)
    }

    private func pickIfAny(_ book: MeetingsBook) async throws -> Bool {
        guard !book.meetings.isEmpty else { return false }
        try await pickAMeeting(book)
        return true
    }

    /// As far through the steps as this run can go. Only a rehearsal answers what's asked.
    private func tourTheWalk() async throws {
        let window = try await openTheWalk()
        for _ in 0..<3 {
            try await Task.sleep(for: Self.settle)
            for permission: Int32 in [0, 2, 1] { try shell.askPermission(index: permission) }
            guard await canContinueSoon() else { break }
            try shell.setupContinue()
        }
        try await Task.sleep(for: Self.settle)
        window.close()
        await closed(.onboarding)
    }

    private func canContinueSoon() async -> Bool {
        for _ in 0..<Self.attempts / 5 {
            if let page = try? await shell.steps(), page.canContinue { return true }
            try? await Task.sleep(for: Self.poll)
        }
        return false
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
