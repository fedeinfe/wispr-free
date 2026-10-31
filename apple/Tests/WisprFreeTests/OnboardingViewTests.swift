import SwiftUI
import Testing
import ViewInspector
import WfCore

@testable import WisprFree

private let waiting = TrialView(
    phase: .idle, keys: .waiting, microphone: .waiting, models: .waiting,
    message: "Hold Fn, say a sentence, then let go. Nothing gets pasted.", problem: false,
    heard: nil, canStop: false, passed: false)

private func checks(_ keys: Check, _ microphone: Check, _ models: Check) -> [CheckRow] {
    [("Shortcut", keys), ("Microphone", microphone), ("Models", models)].map { name, check in
        CheckRow(name: name, check: check, label: "\(name): \(check)")
    }
}

private func card(
    _ permission: Int32, _ name: String, _ access: Access = .unknown, canAsk: Bool = true,
    canOpenPane: Bool = true, hint: String = ""
) -> PermissionCard {
    PermissionCard(
        permission: permission, name: name, why: "Why \(name)", access: access,
        status: access == .granted ? "Allowed" : "Not allowed yet", canAsk: canAsk,
        canOpenPane: canOpenPane, hint: hint)
}

private func setupPage(
    step: Step = .microphone, current: Int32 = 0, canContinue: Bool = false,
    continueLabel: String = "Continue", permissions: [PermissionCard] = [],
    keysStuck: Bool = false, microphoneHint: String = "", trial: TrialView = waiting
) -> SetupPage {
    SetupPage(
        steps: [], current: current, page: 0, step: step, title: "Title of \(step)",
        sentence: "Sentence of \(step)", stepLabel: "Step \(current + 1) of 4",
        canGoBack: current > 0, canContinue: canContinue, continueLabel: continueLabel,
        permissions: permissions, microphone: .unknown, accessibility: .denied,
        inputMonitoring: .unknown, microphoneLooked: true, microphoneName: "USB mic",
        microphoneMark: .passed, microphoneLine: "USB mic", microphoneHint: microphoneHint,
        keysStuck: keysStuck, modelsReady: false, shortcut: "Fn", trial: trial,
        checks: checks(.waiting, .waiting, .waiting))
}

@MainActor
private struct WalkRig {
    let core: FakeCore
    let walk: SetupWalk
    let pages: SettingsPages
    let editor: SettingsEditor

    init(_ page: SetupPage?, pagesState: PagesState = blankPagesState()) async throws {
        core = FakeCore(settings: try await defaultSettings())
        walk = SetupWalk(core: core)
        pages = SettingsPages(core: core)
        editor = try SettingsEditor(core: core)
        pages.adopt(pagesState)
        if let page { walk.adopt(page) }
    }

    func view() throws -> InspectableView<ViewType.ClassifiedView> {
        try OnboardingView(walk: walk, editor: editor, pages: pages).inspect()
    }

    func press(_ identifier: String) throws {
        try view().find(viewWithAccessibilityIdentifier: identifier).button().tap()
    }

    func shows(_ identifier: String) throws -> Bool {
        (try? view().find(viewWithAccessibilityIdentifier: identifier)) != nil
    }

    func says(_ text: String) throws -> Bool {
        (try? view().find(text: text)) != nil
    }
}

@MainActor
@Suite struct OnboardingWindow {
    @Test func aStepIsASymbolATitleASentenceAndABottomBar() async throws {
        let rig = try await WalkRig(setupPage())

        #expect(try rig.says("Title of microphone"))
        #expect(try rig.says("Sentence of microphone"))
        #expect(try rig.view().find(ViewType.Image.self).actualImage().name() == "mic")
        #expect(try rig.shows("Step 1 of 4"))
    }

    @Test func nothingShowsBeforeRustHasSaidWhat() async throws {
        let rig = try await WalkRig(nil)

        #expect(try !rig.shows("Continue"))
    }

    @Test func theFirstStepHasNoBackAndALaterOneDoes() async throws {
        let rig = try await WalkRig(setupPage())
        #expect(try !rig.shows("Back"))

        rig.walk.adopt(setupPage(current: 1))

        #expect(try rig.shows("Back"))
        try rig.press("Back")
        #expect(rig.core.calls == ["back"])
    }

    @Test func continueWaitsForRustAndCarriesItsLabel() async throws {
        let rig = try await WalkRig(setupPage(canContinue: false))
        #expect(try rig.view().find(button: "Continue").isDisabled())

        rig.walk.adopt(setupPage(canContinue: true, continueLabel: "Finish"))

        #expect(try !rig.view().find(button: "Finish").isDisabled())
        try rig.press("Finish")
        #expect(rig.core.calls == ["continue"])
    }

    @Test func eachStepTheWalkReachesShowsItsOwnPage() async throws {
        let rig = try await WalkRig(setupPage(step: .keys, permissions: [card(2, "Input Monitoring")]))
        #expect(try rig.shows("Allow Input Monitoring"))

        rig.walk.adopt(setupPage(step: .models))
        #expect(try rig.shows("Quality tier"))
        #expect(try !rig.shows("Allow Input Monitoring"))

        rig.walk.adopt(setupPage(step: .trial))
        #expect(try rig.shows("Try without the shortcut"))
    }

    @Test func aNoticeFromRustShowsAboveTheStep() async throws {
        var state = blankPagesState()
        state.notice = "Couldn't save settings: disk full"
        let rig = try await WalkRig(setupPage(), pagesState: state)

        #expect(try rig.shows("Notice"))
    }
}

@MainActor
@Suite struct PermissionLines {
    @Test func whatRustAllowsIsWhatShows() async throws {
        let rig = try await WalkRig(
            setupPage(permissions: [card(0, "Microphone access", canAsk: false)]))

        #expect(try !rig.shows("Allow Microphone access"))
        #expect(try rig.shows("Open System Settings for Microphone access"))
        #expect(try rig.shows("Microphone access: Not allowed yet"))
    }

    @Test func allowAndOpenAskRustForTheirOwnPermission() async throws {
        let rig = try await WalkRig(setupPage(permissions: [card(2, "Input Monitoring")]))

        try rig.press("Allow Input Monitoring")
        try rig.press("Open System Settings for Input Monitoring")

        #expect(rig.core.calls == ["ask 2", "open pane 2"])
    }

    @Test func anAllowedPermissionSaysSoAndOffersNothing() async throws {
        let rig = try await WalkRig(
            setupPage(permissions: [
                card(1, "Accessibility", .granted, canAsk: false, canOpenPane: false)
            ]))

        #expect(try rig.shows("Accessibility: Allowed"))
        #expect(try !rig.shows("Allow Accessibility"))
        #expect(try !rig.shows("Open System Settings for Accessibility"))
    }

    @Test func aHintFromRustShows() async throws {
        let rig = try await WalkRig(
            setupPage(permissions: [card(1, "Accessibility", hint: "Turn on Wispr Free.")]))

        #expect(try rig.says("Turn on Wispr Free."))
    }

    @Test func keysThatNeverArriveOfferAReopen() async throws {
        let rig = try await WalkRig(setupPage(step: .keys))
        #expect(try !rig.shows("Quit and reopen"))

        rig.walk.adopt(setupPage(step: .keys, keysStuck: true))

        try rig.press("Quit and reopen")
        #expect(rig.core.calls == ["reopen"])
    }

    @Test func noMicrophoneSaysWhatToDo() async throws {
        let rig = try await WalkRig(setupPage(microphoneHint: "Plug in a headset."))

        #expect(try rig.says("Plug in a headset."))
    }
}

@MainActor
@Suite struct ModelsStepPage {
    private func models(_ change: (inout PagesState) -> Void = { _ in }) -> PagesState {
        var state = blankPagesState()
        state.modelsInUseSummary = "1 model, 500 MB"
        state.modelsToDownload = "1 model, 500 MB to download"
        state.offeredTiers = tierRows().filter { $0.tier != .custom }
        change(&state)
        return state
    }

    @Test func theFolderChoiceComesBeforeTheDownloads() async throws {
        let rig = try await WalkRig(setupPage(step: .models), pagesState: models())
        let view = try rig.view()

        let order = view.findAll(ViewType.Button.self).compactMap {
            try? $0.accessibilityIdentifier()
        }

        let folder = try #require(order.firstIndex(of: "Change models folder"))
        let download = try #require(order.firstIndex(of: "Download all"))
        #expect(folder < download)
    }

    @Test func downloadAllAsksForEverythingInUse() async throws {
        let rig = try await WalkRig(setupPage(step: .models), pagesState: models())

        try rig.press("Download all")

        #expect(rig.core.calls == ["download all"])
    }

    @Test func downloadAllGoesOnceEverythingIsThere() async throws {
        let state = models {
            $0.modelsInUseSummary = ""
            $0.modelsToDownload = "Everything is downloaded."
        }
        let rig = try await WalkRig(setupPage(step: .models), pagesState: state)

        #expect(try !rig.shows("Download all"))
        #expect(try rig.says("Everything is downloaded."))
    }

    @Test func aMissingFolderHoldsTheDownloadsBack() async throws {
        let state = models { $0.modelsFolderMissing = true }
        let rig = try await WalkRig(setupPage(step: .models), pagesState: state)

        let button = try rig.view().find(viewWithAccessibilityIdentifier: "Download all").button()
        #expect(button.isDisabled())
    }

    @Test func pickingATierGoesThroughItsRule() async throws {
        let rig = try await WalkRig(setupPage(step: .models), pagesState: models())
        let picker = try rig.view().find(viewWithAccessibilityIdentifier: "Quality tier").picker()

        try picker.select(value: PipelineTier.lite)

        #expect(rig.core.calls == ["\(RuledChange.tier(tier: .lite))"])
    }
}

@MainActor
@Suite struct TrialStepPage {
    private func trial(
        phase: Phase = .idle, message: String = "Hold Fn.", problem: Bool = false,
        heard: String? = nil, canStop: Bool = false
    ) -> SetupPage {
        setupPage(
            step: .trial,
            trial: TrialView(
                phase: phase, keys: .waiting, microphone: .waiting, models: .waiting,
                message: message, problem: problem, heard: heard, canStop: canStop, passed: false))
    }

    @Test func theMessageComesFromRustAndNamesThePart() async throws {
        let rig = try await WalkRig(trial(message: "No microphone found", problem: true))

        let text = try rig.view().find(text: "No microphone found")

        #expect(try text.attributes().foregroundColor() == .red)
    }

    @Test func tryingWithoutTheShortcutGoesToRust() async throws {
        let rig = try await WalkRig(trial())

        try rig.press("Try without the shortcut")

        #expect(rig.core.calls == ["start trial"])
    }

    @Test func aRunningTrialHoldsTheMouseButtonBackAndOffersStop() async throws {
        let rig = try await WalkRig(trial(phase: .listening, canStop: true))

        let button = try rig.view().find(viewWithAccessibilityIdentifier: "Try without the shortcut")
        #expect(try button.button().isDisabled())
        try rig.press("Stop")
        #expect(rig.core.calls == ["stop trial"])
    }

    @Test func whatItHeardShowsOnceThereIsSomething() async throws {
        let rig = try await WalkRig(trial())
        #expect(try !rig.shows("Heard"))

        rig.walk.adopt(trial(heard: "Testing one two."))

        #expect(try rig.says("Testing one two."))
    }

    @Test func everyPartSaysHowItStands() async throws {
        let rig = try await WalkRig(trial())

        for name in ["Shortcut", "Microphone", "Models"] {
            #expect(try rig.shows("\(name): waiting"))
        }
    }

    @Test func theLevelShowsOnlyWhileATrialListens() async throws {
        let rig = try await WalkRig(trial(phase: .listening))

        rig.walk.hear(level: 0.5)
        #expect(rig.walk.level == 0.5)

        rig.walk.adopt(trial())
        #expect(rig.walk.level == 0)

        rig.walk.hear(level: 0.5)
        #expect(rig.walk.level == 0, "an ordinary dictation")
    }
}
