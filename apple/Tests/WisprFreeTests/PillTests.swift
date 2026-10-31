import AppKit
import Foundation
import Synchronization
import Testing
import WfCore

@testable import WisprFree

/// A screen of its own, far from any real one, and a cursor that goes where the test says.
@MainActor
final class Surroundings: PillSurroundings {
    static let screen = WfCore.Rect(x: -20000, y: 0, width: 1000, height: 800)

    var cursor: WfCore.Point? = WfCore.Point(x: 1, y: 1)
    var escapeIsDown = false
    var displays: [Display] {
        [Display(bounds: Self.screen, workArea: Self.screen, unitsPerPoint: 1)]
    }
}

/// The real panel and controller on a rehearsal: the real flow with nothing real under it. The
/// panel docks to the made-up screen, so it shows nowhere, and clicks go straight to `sendEvent`.
@MainActor
struct PillRig {
    let folder = Rig.temporaryFolder()
    let events = EventStreams()
    let surroundings = Surroundings()
    let shell: Shell
    let controller: PillController
    let picked = Picked()

    init(catchesMouse: Bool = true) async throws {
        shell = try await Shell.rehearse(paths: Rig.paths(in: folder), sink: events.sink)
        let picked = picked
        controller = PillController(
            shell: shell, surroundings: surroundings, catchesMouse: catchesMouse,
            onMenuAction: { picked.actions.append($0) })
    }

    final class Picked {
        var actions: [MenuAction] = []
    }

    var look: PillLook { controller.model.look }

    /// Puts the cursor on the capsule, which turns Resting into Ready.
    func hoverTheCapsule() {
        controller.tick()
        let capsule = look.capsule
        surroundings.cursor = WfCore.Point(
            x: origin.x + capsule.x + capsule.width / 2,
            y: origin.y + capsule.y + capsule.height / 2)
        controller.tick()
        controller.tick()
    }

    /// Where Rust docked the window, on the screen the test made up.
    private var origin: WfCore.Point {
        let size = pillWindowSize()
        return WfCore.Point(
            x: Surroundings.screen.x + (Surroundings.screen.width - size.width) / 2,
            y: Surroundings.screen.height - size.height)
    }

    /// A button event at the middle of the capsule, in the window's bottom-left coordinates.
    func capsuleEvent(_ type: NSEvent.EventType) throws -> NSEvent {
        let height = pillWindowSize().height
        let capsule = look.capsule
        return try #require(
            NSEvent.mouseEvent(
                with: type,
                location: NSPoint(
                    x: capsule.x + capsule.width / 2,
                    y: height - capsule.y - capsule.height / 2),
                modifierFlags: [], timestamp: 0, windowNumber: controller.panel.windowNumber,
                context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
    }

    func clickTheCapsule() async throws {
        controller.panel.orderFrontRegardless()
        controller.panel.contentView?.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(100))
        controller.panel.sendEvent(try capsuleEvent(.leftMouseDown))
        controller.panel.sendEvent(try capsuleEvent(.leftMouseUp))
    }

    /// Ticks until `done`, as the timer would.
    func tick(until done: () -> Bool) async throws {
        for _ in 0..<500 where !done() {
            controller.tick()
            try await Task.sleep(for: .milliseconds(10))
        }
        controller.tick()
    }

    func next<T>(_ matching: (ShellEvent) -> T?) async -> T? {
        for await event in events.states {
            if let found = matching(event) { return found }
        }
        return nil
    }
}

@Suite @MainActor struct ThePanel {
    @Test func itFollowsTheRecipeAndCanNeverTakeTheKeyboard() async throws {
        let rig = try await PillRig()
        let panel = rig.controller.panel

        #expect(panel.styleMask.contains(.nonactivatingPanel))
        #expect(panel.level.rawValue == 1000)
        #expect(
            panel.collectionBehavior
                == [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle])
        #expect(!panel.hidesOnDeactivate)
        #expect(!panel.canBecomeKey)
        #expect(!panel.canBecomeMain)
        #expect(!panel.hasShadow)
        #expect(!panel.isOpaque)
        #expect(panel.frame.size == NSSize(width: 320, height: 120))
    }

    @Test func clicksPassThroughUnlessTheCursorIsOnTheCapsule() async throws {
        let rig = try await PillRig()
        rig.controller.tick()
        #expect(rig.controller.panel.ignoresMouseEvents)

        rig.hoverTheCapsule()
        #expect(!rig.controller.panel.ignoresMouseEvents)
        #expect(rig.look.shown == .ready)
    }

    @Test func aPillNobodyStartedByHandNeverCatchesTheMouse() async throws {
        let rig = try await PillRig(catchesMouse: false)

        rig.hoverTheCapsule()

        #expect(rig.controller.panel.ignoresMouseEvents)
    }

    @Test func aMoveLandsWhereRustSaidInAppKitsCoordinates() {
        let corner = Desktop.windowOrigin(
            WfCore.Point(x: 340, y: 680), height: 120, primaryHeight: 800)

        #expect(corner == NSPoint(x: 340, y: 0))
    }

    @Test func aScreenIsToldInRustsCoordinates() {
        let rect = Desktop.rect(
            of: NSRect(x: 0, y: 25, width: 1000, height: 750), primaryHeight: 800)

        #expect(rect == WfCore.Rect(x: 0, y: 25, width: 1000, height: 750))
    }

    @Test func nothingRedrawsWhenATickChangesNothing() async throws {
        let rig = try await PillRig()
        rig.controller.tick()
        let redraws = Mutex(0)
        withObservationTracking {
            _ = rig.controller.model.look
        } onChange: {
            redraws.withLock { $0 += 1 }
        }

        rig.controller.tick()
        rig.controller.tick()

        #expect(redraws.withLock { $0 } == 0)

        try rig.shell.stage(scene: .state(state: .processing))
        rig.controller.tick()

        #expect(redraws.withLock { $0 } == 1)
    }
}

@Suite @MainActor struct TheLook {
    @Test func theCapsuleTakesItsShapeFromRust() async throws {
        let rig = try await PillRig()

        try rig.shell.stage(scene: .state(state: .listeningHandsFree))
        rig.controller.tick()

        #expect(rig.look.capsule.width == 100)
        #expect(rig.look.capsule.height == 24)
        #expect(rig.look.bars.count == 10)
    }

    @Test func aChangeOfShapeOrErrorMorphsAndANewBarHeightDoesnt() async throws {
        let rig = try await PillRig()
        try rig.shell.stage(scene: .state(state: .listeningPtt))
        rig.controller.tick()
        let listening = rig.look.shapeKey

        var louder = rig.look
        louder.bars = louder.bars.map { $0 / 2 }
        #expect(louder.shapeKey == listening)

        var failed = rig.look
        failed.shown = .error(reason: "no mic")
        #expect(failed.shapeKey != listening)
    }

    @Test func theHairlineIsHalfAPointAndNeverLessThanAPixel() {
        #expect(PillSkin.hairlineWidth(scale: 2) == 0.5)
        #expect(PillSkin.hairlineWidth(scale: 3) == 0.5)
        #expect(PillSkin.hairlineWidth(scale: 1) == 1)
    }

    @Test func reduceMotionCrossfadesInsteadOfMorphing() {
        #expect(PillMotion.animation(reduceMotion: false) == .snappy)
        #expect(PillMotion.animation(reduceMotion: true) == .smooth)
    }
}

@Suite @MainActor struct TheCard {
    @Test func itSaysWhatRustSaysAndTheAnswerTakesItDown() async throws {
        let rig = try await PillRig()
        try rig.shell.stage(scene: .noTextField(text: "words"))
        rig.controller.tick()
        let card = try #require(rig.look.card)
        #expect(card.text == "No text field here. Copy your text?")
        #expect(card.accept == "Copy")
        #expect(card.decline == "Dismiss")

        try rig.shell.pillAnswer(accepted: false)
        rig.controller.tick()

        #expect(rig.look.card == nil)
    }
}

/// The whole way from a click to the words, on the real panel, the generated Swift, and the real
/// flow on fake engines.
@Suite(.timeLimit(.minutes(1))) @MainActor struct EndToEnd {
    @Test func aClickOnTheCapsuleDictatesAndTheWordsComeBack() async throws {
        let rig = try await PillRig()
        rig.hoverTheCapsule()
        #expect(rig.look.shown == .ready)

        try await rig.clickTheCapsule()

        try await rig.tick { rig.look.shown == .listeningHandsFree }
        #expect(rig.look.shown == .listeningHandsFree)
        try await rig.tick { rig.look.bars.contains { $0 > 0.2 } }
        #expect(rig.look.bars.contains { $0 > 0.2 }, "levels from the flow reach the bars")

        try rig.shell.onPill(input: .stop)

        let words = await rig.next { event -> String? in
            if case .recent(let transcripts) = event { transcripts.first } else { nil }
        }
        #expect(words?.hasPrefix("This came through the whole flow") == true)
        try await rig.tick { rig.look.shown == .ready }
        #expect(rig.look.shown == .ready)
    }

    @Test func aPressWithoutThePillShowingReadyStartsNothing() async throws {
        let rig = try await PillRig()
        rig.hoverTheCapsule()
        try rig.shell.stage(scene: .state(state: .processing))
        rig.controller.tick()

        try await rig.clickTheCapsule()
        try await Task.sleep(for: .milliseconds(100))
        rig.controller.tick()

        #expect(rig.look.shown == .processing)
    }

    @Test func aQuitFromTheMenuComesBackForTheAppToAct() async throws {
        let rig = try await PillRig()

        try await rig.shell.menuAction(action: .quit)

        let asked = await rig.next { event -> Bool? in
            if case .quitAsked = event { true } else { nil }
        }
        #expect(asked == true)
    }
}

@Suite @MainActor struct TheMenus {
    @Test func theMenuFollowsWhatRustHoldsAndAnItemHandsBackItsAction() async throws {
        let rig = try await PillRig()
        rig.hoverTheCapsule()
        let menu = NSMenu()
        var picked: [MenuAction] = []

        MenuBuilder.fill(menu, with: try rig.shell.pillMenu()) { picked.append($0) }

        #expect(
            menu.items.map(\.title) == [
                "Paste last transcript", "Recent transcripts", "Microphone", "Language",
                "Pipeline quality", "", "Meetings", "Settings", "Hide for 1 hour", "Quit",
            ])
        #expect(menu.items[5].isSeparatorItem)
        #expect(menu.items[3].submenu?.items.count == 16)
        #expect(menu.items[1].isEnabled == false, "nothing to paste yet")

        menu.performActionForItem(at: 9)
        menu.items[3].submenu?.performActionForItem(at: 2)

        #expect(picked == [.quit, .language(code: "it")])
    }

    @Test func thePillOffersNoMenuUnlessItShowsReady() async throws {
        let rig = try await PillRig()

        #expect(try rig.shell.pillMenu().isEmpty)
    }

    @Test func theStatusMenuKeepsItsItemsAndTheDebugOneReachesEveryState() async throws {
        let rig = try await PillRig()
        let menu = NSMenu()

        MenuBuilder.fill(menu, with: try rig.shell.statusMenu(debug: true)) { _ in }

        #expect(
            menu.items.map(\.title) == [
                "Start meeting notes", "Debug pill", "Meetings…", "Settings…", "Quit wispr-free",
            ])
        #expect(menu.items[1].submenu?.items.count == 15)
    }

    @Test func theMenuBarIconIsATemplate() {
        #expect(StatusItem.icon().isTemplate)
    }

    @Test func theHiddenMainMenuSuppliesTheShortcutsTextFieldsNeed() {
        _ = NSApplication.shared
        var opened = 0
        let menu = MainMenu.make { opened += 1 }
        let items = menu.items.flatMap { $0.submenu?.items ?? [] }
        let keys = Set(
            items.filter { $0.keyEquivalentModifierMask == .command }.map(\.keyEquivalent))
        #expect(keys.isSuperset(of: [",", "w", "z", "x", "c", "v", "a"]))

        let settings = items.first { $0.keyEquivalent == "," }
        settings?.menu?.performActionForItem(at: 0)

        #expect(opened == 1)
        let copy = items.first { $0.keyEquivalent == "c" }
        #expect(copy?.action == #selector(NSText.copy(_:)))
    }
}

@Suite @MainActor struct Options {
    @Test func aPlainStartAsksForNothing() {
        let options = LaunchOptions(arguments: ["wispr-free"])

        #expect(!options.byHand)
        #expect(!options.rehearse)
        #expect(options.pillState == nil)
        #expect(options.pick == nil)
        #expect(options.shots == nil)
    }

    @Test func everyFlagIsRead() {
        let options = LaunchOptions(arguments: [
            "wispr-free", "--by-hand", "--debug-pill", "--rehearse", "--pill-state=locked",
            "--pick=Quit wispr-free", "--shots=/tmp/shots",
        ])

        #expect(options.byHand)
        #expect(options.debugPill)
        #expect(options.rehearse)
        #expect(options.pillState == "locked")
        #expect(options.pick == "Quit wispr-free")
        #expect(options.shots == "/tmp/shots")
    }
}
