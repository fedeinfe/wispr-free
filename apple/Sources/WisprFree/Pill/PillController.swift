import AppKit
import SwiftUI
import WfCore

/// Runs the pill: a timer at the tier Rust asks for calls `pillTick`, and the frame it gets back
/// drives the panel. Every decision stays in Rust.
final class PillController {
    private let shell: Shell
    private let surroundings: PillSurroundings
    /// Off unless a person started the app, so nothing can click a pill a test put up.
    private let catchesMouse: Bool
    private let onMenuAction: (MenuAction) -> Void
    let model = PillModel()
    private(set) lazy var panel = makePanel()
    private var timer: Timer?
    private var interval: UInt32 = 0
    private var wantsDisplays = true

    init(
        shell: Shell,
        surroundings: PillSurroundings = RealSurroundings(),
        catchesMouse: Bool,
        onMenuAction: @escaping (MenuAction) -> Void
    ) {
        self.shell = shell
        self.surroundings = surroundings
        self.catchesMouse = catchesMouse
        self.onMenuAction = onMenuAction
    }

    isolated deinit {
        timer?.invalidate()
    }

    private func makePanel() -> PillPanel {
        let window = pillWindowSize()
        let size = CGSize(width: window.width, height: window.height)
        let view = PillView(model: model, size: size, actions: actions())
        return PillPanel(size: size, content: view)
    }

    private func actions() -> PillActions {
        PillActions(
            press: { [weak self] in self?.report { try $0.pillPress() } },
            release: { [weak self] in self?.release() },
            menu: { [weak self] event, view in self?.openMenu(event, in: view) },
            cancel: { [weak self] in self?.report { try $0.onPill(input: .cancel) } },
            stop: { [weak self] in self?.report { try $0.onPill(input: .stop) } },
            answer: { [weak self] accepted in
                self?.report { try $0.pillAnswer(accepted: accepted) }
            })
    }

    /// Orders the panel in without activating, and starts polling.
    func start() {
        tick()
        panel.orderFrontRegardless()
    }

    func tick() {
        let displays = wantsDisplays ? surroundings.displays : nil
        do {
            apply(
                try shell.pillTick(
                    cursor: surroundings.cursor, displays: displays,
                    escape: surroundings.escapeIsDown))
        } catch {
            log("the pill's tick failed: \(error)")
        }
    }

    private func apply(_ frame: PillFrame) {
        if let origin = frame.moveTo, let height = NSScreen.screens.first?.frame.height {
            let corner = Desktop.windowOrigin(
                origin, height: panel.frame.height, primaryHeight: height)
            panel.setFrameOrigin(corner)
        }
        let ignores = !(catchesMouse && frame.hitTest)
        if panel.ignoresMouseEvents != ignores {
            panel.ignoresMouseEvents = ignores
        }
        model.look = PillLook(
            shown: frame.shown, capsule: frame.capsule, card: frame.card, tooltip: frame.tooltip,
            bars: frame.bars, barScale: frame.barScale)
        wantsDisplays = frame.wantsDisplays
        poll(every: frame.nextTickMs)
    }

    /// In the common modes, so the pill stays alive while a menu is open.
    private func poll(every milliseconds: UInt32) {
        guard milliseconds != interval else { return }
        interval = milliseconds
        timer?.invalidate()
        let seconds = TimeInterval(milliseconds) / 1000
        let timer = Timer(timeInterval: seconds, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        timer.tolerance = seconds / 10
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func release() {
        let shell = shell
        Task {
            do {
                try await shell.pillRelease()
            } catch {
                log("the pill's release failed: \(error)")
            }
        }
    }

    private func openMenu(_ event: NSEvent, in view: NSView) {
        do {
            let entries = try shell.pillMenu()
            guard !entries.isEmpty else { return }
            let menu = NSMenu()
            MenuBuilder.fill(menu, with: entries, pick: onMenuAction)
            NSMenu.popUpContextMenu(menu, with: event, for: view)
        } catch {
            log("the pill's menu failed: \(error)")
        }
    }

    private func report(_ call: (Shell) throws -> Void) {
        do {
            try call(shell)
        } catch {
            log("the pill's call failed: \(error)")
        }
    }
}
