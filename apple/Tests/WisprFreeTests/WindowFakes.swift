import AppKit
import WfCore

@testable import WisprFree

/// Rust's side of the windows, kept in memory. Each save can be held up, to race another writer.
@MainActor
final class FakeCore: WindowsCore {
    var versioned: Versioned
    var page = SettingsPage.general
    var closed: [AppWindow] = []
    var savedPages: [SettingsPage] = []
    var saves: [Settings] = []
    var refuseWrites = false
    /// Runs inside the next save, before it compares revisions, as a rival writer would.
    var beforeSave: (() -> Void)?
    /// Runs once a save has written, before it replies, as the `SettingsChanged` event may.
    var afterWrite: (() -> Void)?
    /// Holds each save until the test lets it go, while the test looks at what shows.
    var gate: AsyncStream<Void>?

    init(settings: Settings) {
        versioned = Versioned(settings: settings, revision: 1)
    }

    func settings() throws -> Versioned { versioned }

    func save(settings: Settings, revision: UInt64) async throws -> SaveOutcome {
        if let gate {
            for await _ in gate { break }
        }
        beforeSave?()
        beforeSave = nil
        guard revision == versioned.revision else { return .stale(current: versioned) }
        write(settings)
        saves.append(settings)
        afterWrite?()
        if refuseWrites { throw ShellError.Disk(reason: "disk full") }
        return .saved(current: versioned)
    }

    /// What a save does, and what Rust's own changes do.
    func write(_ settings: Settings) {
        versioned = Versioned(settings: settings, revision: versioned.revision + 1)
    }

    func settingsPage() throws -> SettingsPage { page }

    func setSettingsPage(page: SettingsPage) async throws {
        self.page = page
        savedPages.append(page)
    }

    func windowClosed(window: AppWindow) throws {
        closed.append(window)
    }
}

/// A stage that shows nothing and records the order of things.
@MainActor
final class StageRecord {
    var steps: [String] = []

    var stage: WindowStage {
        WindowStage(
            activate: { [self] in steps.append("activate") },
            order: { [self] in steps.append("order \($0.identifier?.rawValue ?? "")") })
    }
}

/// Real settings from a rehearsal's defaults would need a running shell, so tests make their own.
@MainActor
func defaultSettings() async throws -> Settings {
    let folder = Rig.temporaryFolder()
    let shell = try await Shell.rehearse(paths: Rig.paths(in: folder), sink: EventStreams().sink)
    return try shell.settings().settings
}

/// Lets the main actor run until `done`, for AppKit's work that finishes on a later turn.
@MainActor
func eventually(_ done: () -> Bool) async -> Bool {
    for _ in 0..<500 {
        if done() { return true }
        try? await Task.sleep(for: .milliseconds(2))
    }
    return done()
}
