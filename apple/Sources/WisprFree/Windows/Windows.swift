import AppKit
import SwiftUI
import WfCore

/// What the windows need of Rust, so that they can run on a fake. Each window adds its own.
protocol WindowsCore: SettingsCore {
    func windowClosed(window: AppWindow) throws
}

extension Shell: WindowsCore {}

/// The open windows. Rust says what opens and what comes forward, and this does it.
final class Windows {
    private let core: any WindowsCore
    private let editor: SettingsEditor
    private let stage: WindowStage
    private var controllers: [AppWindow: WindowController] = [:]

    init(core: any WindowsCore, editor: SettingsEditor, stage: WindowStage) {
        self.core = core
        self.editor = editor
        self.stage = stage
    }

    func window(_ window: AppWindow) -> NSWindow? {
        controllers[window]?.window
    }

    func open(_ window: AppWindow) {
        let controller = controller(for: window)
        controllers[window] = controller
        stage.bringForward(controller.window)
    }

    func raise(_ window: AppWindow) {
        if let controller = controllers[window] {
            stage.bringForward(controller.window)
        }
    }

    private func controller(for window: AppWindow) -> WindowController {
        let onClose: () -> Void = { [weak self] in self?.closed(window) }
        switch window {
        case .settings:
            return WindowController(
                shape: SettingsView.shape,
                content: SettingsView(navigation: SettingsNavigation(core: core))
                    .environment(editor),
                onClose: onClose)
        case .meetings:
            return WindowController(
                shape: MeetingsView.shape, content: MeetingsView(), onClose: onClose)
        case .onboarding:
            return WindowController(
                shape: OnboardingView.shape, content: OnboardingView(), onClose: onClose)
        }
    }

    /// AppKit is still in the middle of closing it, so the controller lasts until it's done.
    private func closed(_ window: AppWindow) {
        let closing = controllers.removeValue(forKey: window)
        DispatchQueue.main.async { withExtendedLifetime(closing) {} }
        do {
            try core.windowClosed(window: window)
        } catch {
            log("closing \(window) failed: \(error)")
        }
    }
}
