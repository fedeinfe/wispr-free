import AppKit
import SwiftUI
import Testing
import ViewInspector
import WfCore

@testable import WisprFree

/// A page on a fake core, with what Rust would have said.
@MainActor
private struct PageRig {
    let core: FakeCore
    let editor: SettingsEditor
    let pages: SettingsPages

    init(_ change: (inout PagesState) -> Void = { _ in }) async throws {
        core = FakeCore(settings: try await defaultSettings())
        editor = try SettingsEditor(core: core)
        pages = SettingsPages(core: core)
        var state = blankPagesState()
        state.isMacos = true
        change(&state)
        pages.adopt(state)
    }
}

private func model(
    index: Int32, name: String, installed: Bool = false, partial: Bool = false,
    downloading: Bool = false
) -> ModelRow {
    ModelRow(
        index: index, name: name, size: "1 GB", usedFor: "", detail: "detail", status: "status",
        installed: installed, partial: partial, busy: downloading, downloading: downloading,
        progress: 0.5, downloadTitle: partial ? "Resume" : "Download")
}

private let shortcuts = [
    ShortcutRow(action: "Push-to-talk", combo: "Fn"),
    ShortcutRow(action: "Hands-free", combo: "Fn+Space"),
]

extension InspectableView {
    fileprivate func has(_ identifier: String) -> Bool {
        (try? find(viewWithAccessibilityIdentifier: identifier)) != nil
    }

    fileprivate func button(_ identifier: String) throws -> InspectableView<ViewType.Button> {
        try find(viewWithAccessibilityIdentifier: identifier).button()
    }

    fileprivate func press(_ identifier: String) throws {
        try button(identifier).tap()
    }
}

@MainActor
@Suite struct GeneralPageTests {
    @Test func everyControlHasTodaysLabelAndThemeAndAppLanguageAreGone() async throws {
        let rig = try await PageRig { $0.shortcuts = shortcuts }
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        for label in [
            "Change Push-to-talk", "Change Hands-free", "Choose automatically", "Test microphone",
            "Auto-detect", "Launch at login", "Show pill", "Show in Dock",
        ] {
            #expect(view.has(label), "\(label)")
        }
        #expect(!view.has("Theme") && !view.has("App language"))
    }

    @Test func aRowChanges() async throws {
        let rig = try await PageRig { $0.shortcuts = shortcuts }
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        try view.press("Change Hands-free")

        #expect(rig.core.calls == ["record 1"])
        #expect((try? view.find(text: "Fn+Space")) != nil)
    }

    @Test func recordingShowsWhatIsHeldAndOnlyStop() async throws {
        let rig = try await PageRig {
            $0.shortcuts = shortcuts
            $0.recordingAction = 1
            $0.recordingKeys = "Press keys…"
        }
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(try view.find(text: "Press keys…").pathToRoot.isEmpty == false)
        #expect(!view.has("Change Hands-free"))
        #expect(try view.button("Change Push-to-talk").isDisabled())

        try view.press("Stop recording")
        #expect(rig.core.calls == ["stop recording"])
    }

    @Test func resetSitsInAContextMenu() async throws {
        let rig = try await PageRig { $0.shortcuts = shortcuts }
        let menu = ResetShortcut(index: 1, action: "Hands-free", pages: rig.pages)

        try menu.inspect().find(button: "Reset").tap()

        #expect(rig.core.calls == ["reset 1"])
        #expect(try menu.inspect().find(button: "Reset").accessibilityIdentifier() == "Reset Hands-free")
    }

    @Test func theRankingShowsOnlyWithAutoOffAndItsEndsDoNotMove() async throws {
        let rows = [
            MicrophoneRow(name: "Desk", title: "1. Desk"),
            MicrophoneRow(name: "USB", title: "2. USB (not connected)"),
        ]
        let auto = try await PageRig { $0.microphones = rows }
        #expect(!(try GeneralPage(editor: auto.editor, pages: auto.pages).inspect().has("Move Desk down")))

        let rig = try await PageRig {
            $0.microphones = rows
            $0.microphoneAuto = false
        }
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(try view.button("Move Desk up").isDisabled())
        #expect(try view.button("Move USB down").isDisabled())
        try view.press("Move Desk down")
        #expect(rig.core.calls == ["move 0 1"])
        #expect(try view.find(text: "2. USB (not connected)").pathToRoot.isEmpty == false)
    }

    @Test func noMicrophonesSaysSo() async throws {
        let rig = try await PageRig { $0.microphoneAuto = false }
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect((try? view.find(text: "No microphones found.")) != nil)
    }

    @Test func theLanguagesShowOnlyWithAutoOffAndComeFromRust() async throws {
        let languages = ["English", "Italian"].map { LanguageRow(name: $0, selected: false) }
        let rig = try await PageRig {
            $0.languages = languages
            $0.languagesAuto = false
        }
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        try view.find(viewWithAccessibilityIdentifier: "Italian").toggle().tap()

        #expect(rig.core.calls == ["language 1"])
    }

    @Test func launchAtLoginAndTheDockGoThroughRust() async throws {
        let rig = try await PageRig()
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        try view.find(viewWithAccessibilityIdentifier: "Launch at login").toggle().tap()
        try view.find(viewWithAccessibilityIdentifier: "Show in Dock").toggle().tap()

        #expect(
            rig.core.calls == ["launchAtLogin(on: true)", "showInDock(on: true)"])
        #expect(rig.core.saves.isEmpty)
    }

    @Test func showPillIsAPlainEdit() async throws {
        let rig = try await PageRig()
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        try view.find(viewWithAccessibilityIdentifier: "Show pill").toggle().tap()

        #expect(rig.core.calls.isEmpty)
        #expect(await eventually { rig.core.saves.last?.general.showPill == false })
    }

    @Test func theDockIsOnlyForMacOS() async throws {
        let rig = try await PageRig { $0.isMacos = false }
        let view = try GeneralPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(!view.has("Show in Dock"))
    }
}

@MainActor
@Suite struct ModelsPageTests {
    @Test func aModelOffersWhatItsStateAllows() async throws {
        let rig = try await PageRig {
            $0.models = [
                model(index: 0, name: "Here", installed: true),
                model(index: 1, name: "Half", partial: true),
                model(index: 2, name: "Coming", downloading: true),
                model(index: 3, name: "Absent"),
            ]
        }
        let view = try ModelsPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(view.has("Verify Here") && view.has("Delete Here"))
        #expect(!view.has("Download Here"))
        #expect(try view.button("Download Half").labelView().text().string() == "Resume")
        #expect(view.has("Delete Half"))
        #expect(view.has("Cancel download") && !view.has("Download Coming"))
        #expect(view.has("Download Absent") && !view.has("Delete Absent"))

        try view.press("Download Absent")
        try view.press("Verify Here")
        try view.press("Cancel download")
        #expect(rig.core.calls == ["download 3", "verify 0", "cancel download"])
    }

    @Test func aDeleteAsksFirstWithADestructiveButton() async throws {
        let rig = try await PageRig()
        let row = model(index: 4, name: "Here", installed: true)
        let closed = ModelLine(row: row, pages: rig.pages)
        #expect((try? closed.body.inspect().confirmationDialog()) == nil)

        let line = ModelLine(row: row, pages: rig.pages, confirmingDelete: true)
        let dialog = try line.body.inspect().confirmationDialog()

        #expect(rig.core.calls.isEmpty)
        try dialog.actions().find(button: "Delete").tap()
        #expect(rig.core.calls == ["delete model 4"])
    }

    @Test func aBusyPageHoldsItsButtonsBack() async throws {
        let rig = try await PageRig {
            $0.modelBusy = true
            $0.models = [
                model(index: 0, name: "Here", installed: true), model(index: 1, name: "Absent"),
            ]
        }
        let view = try ModelsPage(editor: rig.editor, pages: rig.pages).inspect()

        for label in ["Download Absent", "Verify Here", "Delete Here", "Change models folder"] {
            #expect(try view.button(label).isDisabled(), "\(label)")
        }
    }

    @Test func aReadOnlyFolderSaysSoAndOffersNoChange() async throws {
        let rig = try await PageRig {
            $0.modelsReadOnly = true
            $0.models = [
                model(index: 0, name: "Here", installed: true), model(index: 1, name: "Absent"),
            ]
        }
        let view = try ModelsPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(view.has("Read only"))
        #expect(try view.button("Download Absent").isDisabled())
        #expect(try view.button("Delete Here").isDisabled())
        #expect(!(try view.button("Verify Here").isDisabled()))
    }

    @Test func aMissingFolderOffersTheWayBack() async throws {
        let rig = try await PageRig {
            $0.modelsFolderMissing = true
            $0.models = [model(index: 1, name: "Absent")]
        }
        let view = try ModelsPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(try view.button("Download Absent").isDisabled())
        #expect(try view.button("Open folder").isDisabled())
        try view.press("Check again")
        #expect(rig.core.calls == ["check again"])
    }

    @Test func aFolderInWaitingAsksWhatHappensToTheModels() async throws {
        let rig = try await PageRig {
            $0.pendingFolder = "/Volumes/Drive"
            $0.pendingSummary = "1 model (2 GB) is in the current folder."
        }
        let view = try ModelsPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(try view.button("Change models folder").isDisabled())
        for label in ["Move them", "Switch without moving", "Keep the current folder"] {
            try view.press(label)
        }
        #expect(rig.core.calls == ["move folder", "switch folder", "cancel pending"])
    }

    @Test func aMoveShowsItsProgressAndCanStop() async throws {
        let rig = try await PageRig { $0.movingModels = true }
        let view = try ModelsPage(editor: rig.editor, pages: rig.pages).inspect()

        try view.press("Cancel moving models")

        #expect(rig.core.calls == ["cancel move"])
    }

    @Test func useDefaultShowsOnlyForAChosenFolder() async throws {
        let plain = try await PageRig()
        #expect(!(try ModelsPage(editor: plain.editor, pages: plain.pages).inspect().has("Use default")))

        let rig = try await PageRig { $0.modelsFolderCustom = true }
        try ModelsPage(editor: rig.editor, pages: rig.pages).inspect().press("Use default")
        #expect(rig.core.calls == ["default folder"])
    }

    @Test func emptyDownloadedShowsWhatRustSays() async throws {
        let rig = try await PageRig { $0.nothingDownloaded = "Connect the folder." }
        let view = try ModelsPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect((try? view.find(text: "Connect the folder.")) != nil)
    }
}

@MainActor
@Suite struct PipelinePageTests {
    @Test func theTiersComeFromRust() async throws {
        let rig = try await PageRig()

        #expect(rig.pages.tiers.map(\.title) == ["Lite", "Balanced", "High", "Custom"])
    }

    @Test func stagesGoThroughRustAndPlainValuesDoNot() async throws {
        let rig = try await PageRig()
        let view = try PipelinePage(editor: rig.editor, pages: rig.pages).inspect()

        try view.find(viewWithAccessibilityIdentifier: "Noise suppression").toggle().tap()
        try view.find(viewWithAccessibilityIdentifier: "LLM cleanup").toggle().tap()
        #expect(rig.core.calls == ["denoise(on: true)", "llmCleanup(on: true)"])

        try view.find(viewWithAccessibilityIdentifier: "Stop hands-free after silence").toggle().tap()
        #expect(rig.core.calls.count == 2)
    }

    @Test func aForcedVadCannotBeTurnedOff() async throws {
        let rig = try await PageRig { $0.vadForced = true }
        let view = try PipelinePage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(try view.find(viewWithAccessibilityIdentifier: "Silence trimming (VAD)").isDisabled())
    }

    @Test func aMissingModelOffersItsDownloadAndAPresentOneDoesNot() async throws {
        let row = model(index: 7, name: "Whisper")
        let missing = StageModel(
            row: row, needed: true, ready: false, missing: true, downloading: false,
            text: "Not downloaded · 1 GB")
        let rig = try await PageRig { $0.asrMenu = StageMenu(options: ["Whisper"], selected: 0, model: missing) }
        let view = try PipelinePage(editor: rig.editor, pages: rig.pages).inspect()

        try view.press("Download Whisper")

        #expect(rig.core.calls == ["download 7"])
        #expect((try? view.find(text: "Not downloaded · 1 GB")) != nil)
    }

    @Test func aRunningDownloadCanBeCancelledFromTheStage() async throws {
        let running = StageModel(
            row: model(index: 7, name: "Whisper", downloading: true), needed: true, ready: false,
            missing: true, downloading: true, text: "Downloading…")
        let rig = try await PageRig { $0.vadMenu = StageMenu(options: ["Whisper"], selected: 0, model: running) }
        let view = try PipelinePage(editor: rig.editor, pages: rig.pages).inspect()

        try view.press("Cancel download")

        #expect(rig.core.calls == ["cancel download"])
    }
}

@MainActor
@Suite struct StylePageTests {
    @Test func everyRowCanBeRemoved() async throws {
        let rig = try await PageRig {
            $0.dictionary = ["Whisper"]
            $0.replacements = [Replacement(wrong: "wisper", right: "Whisper")]
            $0.snippets = [Snippet(trigger: "addr", text: "Via Roma")]
        }
        let view = try StylePage(editor: rig.editor, pages: rig.pages).inspect()

        try view.press("Remove Whisper")
        try view.press("Remove replacement wisper")
        try view.press("Remove snippet addr")

        #expect(rig.core.calls == ["remove word 0", "remove replacement 0", "remove snippet 0"])
    }

    @Test func rowsAreAddedInsideTheirSection() async throws {
        let rig = try await PageRig()
        let view = try StylePage(editor: rig.editor, pages: rig.pages).inspect()

        try view.press("Add replacement")
        try view.press("Add snippet")

        #expect(rig.core.calls == ["add replacement", "add snippet"])
    }

    @Test func addIsOffUntilThereIsAWord() async throws {
        let rig = try await PageRig()
        let view = try StylePage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(try view.button("Add word").isDisabled())
    }

    @Test func aStyleIsAPlainEdit() async throws {
        let rig = try await PageRig()
        let binding = rig.editor.binding(\.style.email)

        binding.wrappedValue = .excited

        #expect(rig.editor.settings.style.email == .excited)
        #expect(await eventually { rig.core.saves.last?.style.email == .excited })
        #expect(rig.core.calls.isEmpty)
    }
}

@MainActor
@Suite struct OtherPagesTests {
    @Test func meetingsHasTodaysLabels() async throws {
        let rig = try await PageRig()
        let view = try MeetingsPage(editor: rig.editor, pages: rig.pages).inspect()

        for label in [
            "Suggest notes when a meeting starts", "Live transcript",
            "Keep the audio after the transcript is final", "Maximum length", "Refine pass",
            "Tell speakers apart", "Summary template",
        ] {
            #expect(view.has(label), "\(label)")
        }
    }

    @Test func soundsHasTodaysLabels() async throws {
        let rig = try await PageRig()
        let view = try SoundsPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect(view.has("Sounds") && view.has("While dictating"))
    }

    @Test func deleteAllAsksFirstWithADestructiveButtonAndCanBeKept() async throws {
        let rig = try await PageRig()
        let closed = PrivacyPage(editor: rig.editor, pages: rig.pages)
        #expect((try? closed.body.inspect().confirmationDialog()) == nil)

        let page = PrivacyPage(editor: rig.editor, pages: rig.pages, confirmingDelete: true)
        let dialog = try page.body.inspect().confirmationDialog()

        #expect(rig.core.calls.isEmpty)
        try dialog.actions().find(button: "Keep it").tap()
        #expect(rig.core.calls.isEmpty)
        try dialog.actions().find(button: "Yes, delete everything").tap()
        #expect(rig.core.calls == ["delete all"])
    }

    @Test func theDataRowsReachRust() async throws {
        let rig = try await PageRig { $0.dataStatus = "Exported to /tmp/x" }
        let view = try PrivacyPage(editor: rig.editor, pages: rig.pages).inspect()

        try view.press("Export all")
        try view.press("Open folder")

        #expect(rig.core.calls == ["export all", "open data folder"])
        #expect(view.has("Data status"))
    }

    @Test func aboutSaysTheVersionAndNoLongerSlint() async throws {
        let rig = try await PageRig { $0.version = "1.2.3" }
        let view = try AboutPage(editor: rig.editor, pages: rig.pages).inspect()

        #expect((try? view.find(text: "wispr-free 1.2.3")) != nil)
        #expect((try? view.find(textWhere: { text, _ in text.contains("Slint") })) == nil)
        try view.press("Check for updates")
        #expect(rig.core.calls == ["check for updates"])
    }
}

@MainActor
@Suite struct NoticeTests {
    @Test func aNoticeSitsOnTopOfThePageThatRaisedIt() async throws {
        let rig = try await PageRig {
            $0.notice = "Couldn't save"
            $0.noticePage = .pipeline
        }

        let raised = try PipelinePage(editor: rig.editor, pages: rig.pages).inspect()
        let other = try SoundsPage(editor: rig.editor, pages: rig.pages).inspect()

        let section = try raised.find(ViewType.Section.self)
        #expect((try? section.find(viewWithAccessibilityIdentifier: "Notice")) != nil)
        #expect(!other.has("Notice"))
    }

    @Test func aRefusedSaveShowsWhereverTheUserIs() async throws {
        let rig = try await PageRig()
        rig.core.refuseWrites = true

        rig.editor.edit { $0.media.sounds = false }
        #expect(await eventually { rig.editor.notice != nil })

        let view = try SoundsPage(editor: rig.editor, pages: rig.pages).inspect()
        #expect(view.has("Notice"))
    }
}

@MainActor
@Suite struct SettingsPagesModelTests {
    @Test func levelsCountOnlyDuringTheTestAndStopWithIt() async throws {
        let rig = try await PageRig()

        rig.pages.hear(level: 0.5)
        #expect(rig.pages.level == 0)

        var testing = rig.pages.state
        testing.microphoneTesting = true
        rig.pages.adopt(testing)
        rig.pages.hear(level: 0.5)
        #expect(rig.pages.level == 0.5)

        testing.microphoneTesting = false
        rig.pages.adopt(testing)
        #expect(rig.pages.level == 0)
    }

    @Test func theFolderPanelAsksForOneFolder() {
        let panel = FolderPanel.make(from: "/tmp")

        #expect(panel.canChooseDirectories && !panel.canChooseFiles)
        #expect(!panel.allowsMultipleSelection)
        #expect(panel.directoryURL?.path(percentEncoded: false).hasPrefix("/tmp") == true)
    }
}

/// The real `Shell`, on a rehearsal: what crosses, and in what order.
@MainActor
@Suite struct PagesAcrossTheBridge {
    private func rehearsal() async throws -> (Shell, EventStreams) {
        let streams = EventStreams()
        let shell = try await Shell.rehearse(
            paths: Rig.paths(in: Rig.temporaryFolder()), sink: streams.sink)
        return (shell, streams)
    }

    @Test func settingsOpensWithItsPagesAlreadyFilledIn() async throws {
        let (shell, streams) = try await rehearsal()

        try await shell.menuAction(action: .openSettings)

        var filled = false
        for await event in streams.states {
            switch event {
            case .pages(let state): filled = !state.shortcuts.isEmpty
            case .open(.settings):
                #expect(filled)
                return
            default: break
            }
        }
    }

    @Test func aRuleChangesTheSettingsAndTheEditorFollows() async throws {
        let (shell, streams) = try await rehearsal()
        let editor = try SettingsEditor(core: shell)

        try shell.changeSetting(change: .tier(tier: .lite))

        for await event in streams.states {
            if case .settingsChanged(let current) = event, current.settings.pipeline.tier == .lite {
                editor.adopt(current)
                break
            }
        }
        #expect(editor.settings.pipeline.tier == .lite)
    }

    @Test func choosingAFolderAsksSwiftForIt() async throws {
        let (shell, streams) = try await rehearsal()

        try shell.chooseModelsFolder()

        for await event in streams.states {
            if case .pickFolder(let start) = event {
                #expect(start.hasSuffix("models"))
                return
            }
        }
    }

    @Test func aFenceCrossesAsReadOnly() async throws {
        let (shell, _) = try await rehearsal()
        let outside = Rig.temporaryFolder()

        try shell.folderPicked(folder: outside.path(percentEncoded: false))
        try shell.switchFolder()

        let state = try await shell.pagesState()
        #expect(state?.modelsReadOnly == true)
    }
}
