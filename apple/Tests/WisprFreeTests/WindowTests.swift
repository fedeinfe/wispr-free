import AppKit
import SwiftUI
import Testing
import WfCore

@testable import WisprFree

@MainActor
private struct WindowsRig {
    let core: FakeCore
    let record = StageRecord()
    let windows: Windows

    init() async throws {
        core = FakeCore(settings: try await defaultSettings())
        windows = Windows(
            core: core, editor: try SettingsEditor(core: core), pages: SettingsPages(core: core),
            stage: record.stage)
    }
}

/// Holds its token for as long as the view exists, to show that the view went too.
private final class Held {}

private struct HoldingView: View {
    let held: Held
    var body: some View { Text("held") }
}

@MainActor
@Suite struct WindowLives {
    @Test func aWindowActivatesBeforeItComesForward() async throws {
        let rig = try await WindowsRig()

        rig.windows.open(.settings)

        #expect(rig.record.steps == ["activate", "order Settings"])
    }

    @Test func raisingBringsTheOpenWindowForwardAndMakesNoOther() async throws {
        let rig = try await WindowsRig()
        rig.windows.open(.meetings)
        let window = rig.windows.window(.meetings)

        rig.windows.raise(.meetings)

        #expect(rig.record.steps.suffix(2) == ["activate", "order Meetings"])
        #expect(rig.windows.window(.meetings) === window)
        rig.windows.window(.meetings)?.close()
    }

    @Test func raisingAClosedWindowDoesNothing() async throws {
        let rig = try await WindowsRig()

        rig.windows.raise(.meetings)

        #expect(rig.record.steps.isEmpty)
        #expect(rig.windows.window(.meetings) == nil)
    }

    @Test func closingTellsRustAndForgetsTheWindow() async throws {
        let rig = try await WindowsRig()
        rig.windows.open(.onboarding)

        rig.windows.window(.onboarding)?.close()

        #expect(rig.core.closed == [.onboarding])
        #expect(rig.windows.window(.onboarding) == nil)
    }

    @Test func closingReleasesTheControllerTheWindowAndTheView() async throws {
        weak var controller: WindowController?
        weak var window: NSWindow?
        weak var held: Held?
        do {
            let token = Held()
            let made = WindowController(
                shape: MeetingsView.shape, content: HoldingView(held: token), onClose: {})
            controller = made
            window = made.window
            held = token
            made.window.layoutIfNeeded()
            made.window.close()
        }

        #expect(await eventually { controller == nil })
        #expect(await eventually { window == nil })
        #expect(await eventually { held == nil })
    }

    @Test(arguments: [AppWindow.settings, .meetings, .onboarding])
    func everyRealWindowIsLetGoWhenItCloses(_ kind: AppWindow) async throws {
        let rig = try await WindowsRig()
        weak var window: NSWindow?
        rig.windows.open(kind)
        window = rig.windows.window(kind)
        window?.layoutIfNeeded()

        window?.close()

        #expect(await eventually { window == nil })
    }

    @Test func aWindowOpensAgainAfterClosing() async throws {
        let rig = try await WindowsRig()
        rig.windows.open(.meetings)
        rig.windows.window(.meetings)?.close()

        rig.windows.open(.meetings)

        #expect(rig.windows.window(.meetings) != nil)
        rig.windows.window(.meetings)?.close()
    }

    @Test func theWindowsKeepTheSpecsSizes() async throws {
        let rig = try await WindowsRig()
        let sizes: [(AppWindow, NSSize, NSSize)] = [
            (.settings, NSSize(width: 860, height: 640), NSSize(width: 760, height: 480)),
            (.meetings, NSSize(width: 1000, height: 680), NSSize(width: 760, height: 480)),
            (.onboarding, NSSize(width: 680, height: 560), NSSize(width: 680, height: 460)),
        ]
        for (kind, size, minimum) in sizes {
            rig.windows.open(kind)
            let window = try #require(rig.windows.window(kind))
            #expect(window.contentView?.frame.size == size)
            #expect(window.contentMinSize == minimum)
            window.close()
        }
    }

    @Test func settingsHasNoMinimiseAndNoZoom() async throws {
        let rig = try await WindowsRig()
        rig.windows.open(.settings)
        let window = try #require(rig.windows.window(.settings))

        #expect(!window.styleMask.contains(.miniaturizable))
        #expect(window.standardWindowButton(.zoomButton)?.isEnabled == false)
        #expect(window.collectionBehavior.contains(.fullScreenNone))
        window.close()
    }

    /// On a made-up place far from any screen, so it shows nowhere.
    @Test func aRunBehindOrdersTheWindowInAndActivatesNothing() {
        _ = NSApplication.shared
        let window = NSWindow(
            contentRect: NSRect(x: -20000, y: 0, width: 100, height: 100),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false

        WindowStage.behind.bringForward(window)

        #expect(window.isVisible)
        #expect(!window.isKeyWindow)
        #expect(!NSApp.isActive)
        window.close()
    }

    @Test func theLaunchOptionsCarryEveryPickInOrder() {
        let options = LaunchOptions(arguments: [
            "app", "--behind", "--pick=Finish setup…", "--pick=Settings…", "--window-shots=/tmp/x",
        ])

        #expect(options.behind)
        #expect(options.picks == ["Finish setup…", "Settings…"])
        #expect(options.windowShots == "/tmp/x")
    }
}
