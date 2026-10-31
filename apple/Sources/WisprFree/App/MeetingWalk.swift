import AppKit
import WfCore

/// How long the main thread went without getting back to its run loop, which is what a hitch is
/// when the screen waits on it. A timer that can't fire on time shows a gap that long.
final class StallWatch {
    private static let beat: TimeInterval = 0.004
    private var timer: Timer?
    private var last = ContinuousClock.now
    private(set) var longest: Duration = .zero
    private(set) var overOneFrame = 0
    private(set) var overTenth = 0

    func start() {
        last = .now
        let timer = Timer(timeInterval: Self.beat, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        timer.tolerance = 0
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func stop() {
        tick()
        timer?.invalidate()
        timer = nil
    }

    private func tick() {
        let now = ContinuousClock.now
        let gap = now - last
        last = now
        longest = max(longest, gap)
        if gap > .milliseconds(33) { overOneFrame += 1 }
        if gap > .milliseconds(100) { overTenth += 1 }
    }
}

/// `cargo xtask meeting-walk`: everything a meeting shows, opened and moved through as a person
/// would, timing the main thread throughout. The window sits behind the others, like any tool
/// run's, so it says nothing of the frames the GPU would draw for a window on show.
final class MeetingWalk {
    private static let settle: Duration = .milliseconds(400)
    private static let poll: Duration = .milliseconds(10)
    private static let attempts = 1500
    /// One wheel notch's worth of scrolling, as fast as a 120 Hz screen could ask for it.
    private static let scrollStep: CGFloat = 40
    private static let scrollEvery: Duration = .milliseconds(8)

    private let shell: Shell
    private let windows: Windows

    init(shell: Shell, windows: Windows) {
        self.shell = shell
        self.windows = windows
    }

    func take() async {
        do {
            try await walk()
            log("walked")
        } catch {
            log("the walk failed: \(error)")
        }
    }

    private enum WalkError: Error {
        case noWindow
        case nothing(String)
    }

    private func walk() async throws {
        var window: NSWindow?
        var book: MeetingsBook?
        await timed("open the window") {
            try? await self.shell.menuAction(action: .openMeetings)
            window = await self.until { self.windows.window(.meetings) }
            book = self.windows.book
            _ = await self.until { book?.meetings.first }
        }
        guard let window, let book, let meeting = book.meetings.first else {
            throw WalkError.noWindow
        }
        await timed("open the meeting") {
            book.selection = meeting.id
            _ = await self.until { book.transcript.isEmpty ? nil : true }
        }
        log("meeting: \(book.transcript.count) lines, \(book.summary.count) summary lines")
        for tab in [MeetingTab.notes, .chat, .summary, .transcript] {
            await timed("show \(tab.title)") { book.tab = tab }
        }
        await timed("scroll the transcript") { await self.scrollAllTheWay(window) }
        await timed("jump to the first line") { book.show(row: 0) }
        await timed("jump to the last line") {
            book.show(row: Int32(book.transcript.count - 1))
        }
        await timed("search") {
            for end in 1...6 { book.query = String("budget".prefix(end)) }
            try? await Task.sleep(for: Self.settle)
            book.query = ""
        }
        book.tab = .chat
        await timed("stream an answer") {
            book.askAbout("What was decided?")
            _ = await self.until { book.chatRunning ? true : nil }
            _ = await self.until { book.chatRunning ? nil : true }
        }
        log("chat: \(book.chat.count) messages, error \"\(book.chatError)\"")
        await timed("close") {
            window.close()
            _ = await self.until { self.windows.window(.meetings) == nil ? true : nil }
        }
    }

    private func scrollAllTheWay(_ window: NSWindow) async {
        guard let content = window.contentView,
            let page = WindowShots.scrollers(in: content).compactMap(\.documentView)
                .max(by: { $0.frame.height < $1.frame.height })
        else { return }
        var place: CGFloat = 0
        var steps = 0
        while place < page.frame.height - page.visibleRect.height, steps < 20_000 {
            place += Self.scrollStep
            steps += 1
            let target = page.isFlipped ? place : page.bounds.maxY - place
            page.scroll(NSPoint(x: 0, y: target))
            try? await Task.sleep(for: Self.scrollEvery)
        }
        log("scrolled \(Int(place)) points in \(steps) steps")
    }

    /// Times `step` and what it leaves to settle.
    private func timed(_ name: String, _ step: () async -> Void) async {
        let watch = StallWatch()
        let started = ContinuousClock.now
        watch.start()
        await step()
        let done = ContinuousClock.now - started
        try? await Task.sleep(for: Self.settle)
        watch.stop()
        log(
            "stalls \(name): longest \(Self.ms(watch.longest)) ms, \(watch.overOneFrame) over 33 ms, "
                + "\(watch.overTenth) over 100 ms, done in \(Self.ms(done)) ms")
    }

    private func until<T>(_ value: () -> T?) async -> T? {
        for _ in 0..<Self.attempts {
            if let value = value() { return value }
            try? await Task.sleep(for: Self.poll)
        }
        return nil
    }

    private static func ms(_ duration: Duration) -> Int {
        Int(duration / .milliseconds(1))
    }
}
