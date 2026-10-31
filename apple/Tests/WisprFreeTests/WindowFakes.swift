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
    /// What the pages asked of Rust, in order.
    var calls: [String] = []
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

    func changeSetting(change: RuledChange) throws { calls.append("\(change)") }
    func recordShortcut(index: Int32) throws { calls.append("record \(index)") }
    func stopRecordingShortcut() throws { calls.append("stop recording") }
    func resetShortcut(index: Int32) throws { calls.append("reset \(index)") }
    func setMicrophoneAuto(on: Bool) throws { calls.append("microphone auto \(on)") }
    func moveMicrophone(index: Int32, by: Int32) throws { calls.append("move \(index) \(by)") }
    func toggleMicrophoneTest() throws { calls.append("test microphone") }
    func setLanguagesAuto(on: Bool) throws { calls.append("languages auto \(on)") }
    func toggleLanguage(index: Int32) throws { calls.append("language \(index)") }
    func addWord(word: String) throws { calls.append("add \(word)") }
    func removeWord(index: Int32) throws { calls.append("remove word \(index)") }
    func addReplacement() throws { calls.append("add replacement") }
    func editReplacement(index: UInt32, row: Replacement) throws {
        calls.append("edit replacement \(index) \(row.wrong) \(row.right)")
    }
    func removeReplacement(index: UInt32) throws { calls.append("remove replacement \(index)") }
    func addSnippet() throws { calls.append("add snippet") }
    func editSnippet(index: UInt32, row: Snippet) throws {
        calls.append("edit snippet \(index) \(row.trigger) \(row.text)")
    }
    func removeSnippet(index: UInt32) throws { calls.append("remove snippet \(index)") }
    func download(index: Int32) throws { calls.append("download \(index)") }
    func cancelDownload() throws { calls.append("cancel download") }
    func verify(index: Int32) throws { calls.append("verify \(index)") }
    func deleteModel(index: Int32) throws { calls.append("delete model \(index)") }
    func chooseModelsFolder() throws { calls.append("choose folder") }
    func folderPicked(folder: String) throws { calls.append("picked \(folder)") }
    func useDefaultModelsFolder() throws { calls.append("default folder") }
    func openModelsFolder() throws { calls.append("open models folder") }
    func recheckModelsFolder() throws { calls.append("check again") }
    func moveFolder() throws { calls.append("move folder") }
    func switchFolder() throws { calls.append("switch folder") }
    func cancelPendingFolder() throws { calls.append("cancel pending") }
    func cancelMove() throws { calls.append("cancel move") }
    func openDataFolder() throws { calls.append("open data folder") }
    func exportAll() throws { calls.append("export all") }
    func deleteAll() throws { calls.append("delete all") }
    func checkForUpdates() throws { calls.append("check for updates") }
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
