import Testing
import WfCore

@testable import WisprFree

@MainActor
private func editorAndCore() async throws -> (SettingsEditor, FakeCore) {
    let core = FakeCore(settings: try await defaultSettings())
    return (try SettingsEditor(core: core), core)
}

@MainActor
@Suite struct SettingsEdits {
    @Test func aChangeShowsAtOnceAndReachesRust() async throws {
        let (editor, core) = try await editorAndCore()
        let shown = !editor.settings.general.showPill

        editor.edit { $0.general.showPill = shown }

        #expect(editor.settings.general.showPill == shown)
        #expect(await eventually { core.saves.count == 1 })
        #expect(core.versioned.settings.general.showPill == shown)
        #expect(core.versioned.revision == 2)
    }

    @Test func aRivalsChangeSurvivesAndTheEditIsAppliedOverIt() async throws {
        let (editor, core) = try await editorAndCore()
        core.beforeSave = {
            var rival = core.versioned.settings
            rival.style.dictionary = ["learned"]
            core.write(rival)
        }

        editor.edit { $0.general.languages = ["it"] }

        #expect(await eventually { core.saves.count == 1 })
        #expect(core.versioned.settings.style.dictionary == ["learned"])
        #expect(core.versioned.settings.general.languages == ["it"])
        #expect(await eventually { editor.settings.style.dictionary == ["learned"] })
    }

    @Test func editsSaveInTheOrderTheyWereMade() async throws {
        let (editor, core) = try await editorAndCore()

        for language in ["en", "it", "fr"] {
            editor.edit { $0.general.languages.append(language) }
        }

        #expect(editor.settings.general.languages == ["en", "it", "fr"])
        #expect(await eventually { core.saves.count == 3 })
        #expect(core.saves.map(\.general.languages) == [["en"], ["en", "it"], ["en", "it", "fr"]])
    }

    @Test func aChangeStillSavingStaysShownAndRustsNewsFollowsOnceItLands() async throws {
        let (editor, core) = try await editorAndCore()
        let (gate, open) = AsyncStream.makeStream(of: Void.self)
        core.gate = gate
        editor.edit { $0.general.languages = ["it"] }

        var rival = core.versioned.settings
        rival.style.dictionary = ["learned"]
        core.write(rival)
        editor.adopt(core.versioned)

        #expect(editor.settings.general.languages == ["it"])
        open.finish()
        #expect(await eventually { core.saves.count == 1 })
        #expect(core.versioned.settings.style.dictionary == ["learned"])
        #expect(await eventually { editor.settings.style.dictionary == ["learned"] })
        #expect(editor.settings.general.languages == ["it"])
    }

    /// Every save also sends `SettingsChanged`, and the event can beat the save's reply.
    @Test func aChangeIsNotShownTwiceWhenRustsEventBeatsTheReply() async throws {
        let (editor, core) = try await editorAndCore()
        var shownWhenTheEventArrived: [String] = []
        core.afterWrite = {
            editor.adopt(core.versioned)
            shownWhenTheEventArrived = editor.settings.style.dictionary
        }

        editor.edit { $0.style.dictionary.append("one") }

        #expect(await eventually { core.saves.count == 1 })
        #expect(shownWhenTheEventArrived == ["one"])
        #expect(await eventually { editor.settings == core.versioned.settings })
    }

    @Test func aSecondChangeDuringASaveShowsOnceToo() async throws {
        let (editor, core) = try await editorAndCore()
        var shownAfterTheSecond: [String] = []
        core.afterWrite = {
            core.afterWrite = nil
            editor.adopt(core.versioned)
            editor.edit { $0.style.dictionary.append("two") }
            shownAfterTheSecond = editor.settings.style.dictionary
        }

        editor.edit { $0.style.dictionary.append("one") }

        #expect(await eventually { core.saves.count == 2 })
        #expect(shownAfterTheSecond == ["one", "two"])
        #expect(await eventually { editor.settings == core.versioned.settings })
        #expect(core.versioned.settings.style.dictionary == ["one", "two"])
    }

    @Test func aChangeStillWaitingStaysShownWhenTheOneBeforeItLands() async throws {
        let (editor, core) = try await editorAndCore()
        let (gate, open) = AsyncStream.makeStream(of: Void.self)
        core.gate = gate
        editor.edit { $0.general.languages.append("en") }
        editor.edit { $0.general.languages.append("it") }

        open.yield()
        #expect(await eventually { core.saves.count == 1 })
        try await Task.sleep(for: .milliseconds(20))

        #expect(core.saves.count == 1)
        #expect(editor.settings.general.languages == ["en", "it"])
        open.finish()
        #expect(await eventually { core.saves.count == 2 })
    }

    @Test func anOlderWordFromRustChangesNothing() async throws {
        let (editor, core) = try await editorAndCore()
        var newer = core.versioned.settings
        newer.general.languages = ["it"]
        core.write(newer)
        editor.adopt(core.versioned)
        var older = core.versioned
        older.revision -= 1
        older.settings.general.languages = ["fr"]

        editor.adopt(older)

        #expect(editor.settings.general.languages == ["it"])
    }

    @Test func aRefusedWriteSaysWhyInRustsWordsAndClearsOnTheNextSave() async throws {
        let (editor, core) = try await editorAndCore()
        core.refuseWrites = true

        editor.edit { $0.general.languages = ["it"] }

        #expect(await eventually { editor.notice != nil })
        #expect(editor.notice == "Couldn't save settings: disk full")
        #expect(editor.settings.general.languages == ["it"])

        core.refuseWrites = false
        editor.edit { $0.general.languages = ["fr"] }

        #expect(await eventually { editor.notice == nil })
        #expect(editor.settings.general.languages == ["fr"])
    }

    /// The real compare and swap, against a change Rust makes itself.
    @Test func editsAndRustsOwnChangesAllLandOnTheRealShell() async throws {
        let folder = Rig.temporaryFolder()
        let shell = try await Shell.rehearse(paths: Rig.paths(in: folder), sink: EventStreams().sink)
        let editor = try SettingsEditor(core: shell)

        editor.edit { $0.style.dictionary.append("one") }
        try await shell.menuAction(action: .language(code: "it"))
        editor.edit { $0.style.dictionary.append("two") }

        #expect(await eventually { (try? shell.settings().settings.style.dictionary.count) == 2 })
        let settings = try shell.settings().settings
        #expect(Set(settings.style.dictionary) == ["one", "two"])
        #expect(settings.general.languages == ["it"])
        #expect(await eventually { editor.settings == settings })
    }
}
