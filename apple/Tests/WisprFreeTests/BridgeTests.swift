import Foundation
import Testing
import WfCore

@testable import WisprFree

/// `Shell` through the generated Swift, on the real flow in a temp folder. Nothing here starts a
/// dictation, touches the clipboard or shows a window.
struct Rig {
    let folder: URL
    let events = EventStreams()
    let shell: Shell

    init(folder: URL = Rig.temporaryFolder()) async throws {
        self.folder = folder
        shell = try await Shell.start(paths: Rig.paths(in: folder), sink: events.sink)
    }

    static func temporaryFolder() -> URL {
        FileManager.default.temporaryDirectory.appending(path: "wf-bridge-\(UUID().uuidString)")
    }

    static func paths(in folder: URL) -> Paths {
        Paths(
            settings: folder.appending(path: "settings.toml").path,
            pill: folder.appending(path: "pill.toml").path,
            models: folder.appending(path: "models").path,
            history: folder.appending(path: "history.sqlite3").path,
            meetings: folder.appending(path: "meetings").path,
            engineHost: "/nowhere/wf-engine-host"
        )
    }

    /// The next state event `matching` asks for, skipping the others.
    func nextState<T>(_ matching: (ShellEvent) -> T?) async -> T? {
        for await event in events.states {
            if let found = matching(event) { return found }
        }
        return nil
    }
}

private func toast(_ event: ShellEvent) -> String? {
    if case .toast(let text) = event { text } else { nil }
}

private func changed(_ event: ShellEvent) -> Versioned? {
    if case .settingsChanged(let current) = event { current } else { nil }
}

@Suite struct SettingsRoundTrips {
    @Test func everyKindOfFieldComesBackAsItWasSent() async throws {
        let rig = try await Rig()
        let start = try rig.shell.settings()
        var edited = start.settings
        edited.general.showPill.toggle()
        edited.general.languages = ["it", "en"]
        edited.general.theme = .dark
        edited.pipeline.tier = .high
        edited.pipeline.cleanupStrength = .high
        edited.pipeline.modelsFolder = "/Volumes/Extreme SSD/caffè è qui"
        edited.pipeline.vad.toggle()
        edited.style.email = .formal
        edited.style.dictionary = ["Wispr", "perché", "日本語"]
        edited.style.replacements = [Replacement(wrong: "teh", right: "the")]
        edited.style.snippets = [Snippet(trigger: "sig", text: "line one\nline two")]
        edited.shortcuts.pushToTalk = ["Fn", "Space"]
        edited.shortcuts.pasteLast = ["Ctrl", "Meta", "V"]
        edited.meetings.maxLengthMinutes = 90
        edited.media.whileDictating = .mute
        edited.privacy.historyRetention = .delete24h

        let outcome = try await rig.shell.save(settings: edited, revision: start.revision)

        #expect(outcome == .saved(current: Versioned(settings: edited, revision: start.revision + 1)))
        #expect(try rig.shell.settings().settings == edited)
    }

    @Test func noFolderIsNotAnEmptyFolder() async throws {
        let rig = try await Rig()
        let start = try rig.shell.settings()
        #expect(start.settings.pipeline.modelsFolder == nil)

        _ = try await rig.shell.save(settings: start.settings, revision: start.revision)

        #expect(try rig.shell.settings().settings.pipeline.modelsFolder == nil)
    }

    @Test func aSaveIsThereAfterARestart() async throws {
        let rig = try await Rig()
        let start = try rig.shell.settings()
        var edited = start.settings
        edited.style.dictionary = ["Wispr"]
        _ = try await rig.shell.save(settings: edited, revision: start.revision)

        let restarted = try await Rig(folder: rig.folder)

        #expect(try restarted.shell.settings().settings == edited)
    }
}

@Suite struct CompareAndSwap {
    @Test func aSaveFromAnOldRevisionChangesNothingAndHandsBackTheLatest() async throws {
        let rig = try await Rig()
        let old = try rig.shell.settings()
        var first = old.settings
        first.general.showPill = false
        _ = try await rig.shell.save(settings: first, revision: old.revision)
        let latest = try rig.shell.settings()

        var late = old.settings
        late.media.sounds = false
        let outcome = try await rig.shell.save(settings: late, revision: old.revision)

        #expect(outcome == .stale(current: latest))
        #expect(try rig.shell.settings() == latest)
    }

    @Test func reapplyingToTheFreshValueKeepsBothChanges() async throws {
        let rig = try await Rig()
        let old = try rig.shell.settings()
        var first = old.settings
        first.general.showPill = false
        _ = try await rig.shell.save(settings: first, revision: old.revision)

        var late = old.settings
        late.media.sounds = false
        guard case .stale(var fresh) = try await rig.shell.save(settings: late, revision: old.revision)
        else {
            Issue.record("the save from the old revision went through")
            return
        }
        fresh.settings.media.sounds = false
        let retried = try await rig.shell.save(settings: fresh.settings, revision: fresh.revision)

        guard case .saved(let current) = retried else {
            Issue.record("the retry didn't go through")
            return
        }
        #expect(!current.settings.general.showPill)
        #expect(!current.settings.media.sounds)
    }
}

@Suite struct Events {
    @Test func aSaveComesBackOnTheStateStream() async throws {
        let rig = try await Rig()
        let start = try rig.shell.settings()
        var edited = start.settings
        edited.media.sounds = false

        _ = try await rig.shell.save(settings: edited, revision: start.revision)

        let announced = await rig.nextState(changed)
        #expect(announced == Versioned(settings: edited, revision: start.revision + 1))
    }

    /// Crosses from a thread of the flow's, not from the caller's.
    @Test func theFlowsOwnThreadsReachTheStream() async throws {
        let rig = try await Rig()

        try rig.shell.pasteLast()

        let text = await rig.nextState(toast)
        #expect(text?.isEmpty == false)
    }

    @Test func aFloodOfLevelsKeepsTheNewestAndPushesOutNoState() async throws {
        let streams = EventStreams()

        for level in 1...10_000 {
            streams.sink.onEvent(event: .level(value: Float(level)))
            if level == 5_000 {
                streams.sink.onEvent(event: .toast(text: "still here"))
            }
        }

        var states = streams.states.makeAsyncIterator()
        var levels = streams.levels.makeAsyncIterator()
        #expect(await states.next() == .toast(text: "still here"))
        #expect(await levels.next() == 10_000)
    }

    @Test func levelsStayOffTheStateStream() async throws {
        let streams = EventStreams()

        streams.sink.onEvent(event: .level(value: 0.5))
        streams.sink.onEvent(event: .pill(state: .meeting))

        var states = streams.states.makeAsyncIterator()
        #expect(await states.next() == .pill(state: .meeting))
    }
}

@Suite struct Errors {
    @Test func aPathThatIsntAbsoluteComesBackAsAnError() async throws {
        var paths = Rig.paths(in: Rig.temporaryFolder())
        paths.history = "history.sqlite3"
        let events = EventStreams()

        await #expect(throws: ShellError.UnclearPath(path: "history.sqlite3")) {
            _ = try await Shell.start(paths: paths, sink: events.sink)
        }
    }

    @Test func aDiskThatRefusesComesBackAsAnError() async throws {
        let folder = Rig.temporaryFolder()
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let file = folder.appending(path: "not-a-folder")
        try Data().write(to: file)
        var paths = Rig.paths(in: folder)
        paths.settings = file.appending(path: "settings.toml").path
        let events = EventStreams()
        let shell = try await Shell.start(paths: paths, sink: events.sink)
        let start = try shell.settings()

        await #expect {
            _ = try await shell.save(settings: start.settings, revision: start.revision)
        } throws: { error in
            guard case ShellError.Disk(let reason) = error else { return false }
            return !reason.isEmpty
        }
    }

    /// A shortcut Rust can't read is a thrown error. In a call that can't throw it would be fatal.
    @Test func aKeyRustDoesntKnowIsThrownNotFatal() async throws {
        let rig = try await Rig()
        let start = try rig.shell.settings()
        var edited = start.settings
        edited.shortcuts.pushToTalk = ["NotAKey"]

        await #expect(throws: (any Error).self) {
            _ = try await rig.shell.save(settings: edited, revision: start.revision)
        }
        #expect(try rig.shell.settings() == start)
    }
}

@Suite struct Life {
    @Test func quittingReturnsWithNothingRunning() async throws {
        let rig = try await Rig()

        try await rig.shell.quit()
    }

    @Test func devPathsAreBesideTheRealRootNotInIt() throws {
        let paths = try devPaths()

        for path in [paths.settings, paths.pill, paths.models, paths.history, paths.meetings] {
            #expect(path.contains("/wispr-free-dev/"), "\(path)")
        }
    }
}
