import AppKit
import SwiftUI

/// The pill's window: never key, never main, over everything and on every Space.
final class PillPanel: NSPanel {
    /// `NSScreenSaverWindowLevel`, which also sits over full-screen apps.
    private static let level = NSWindow.Level(rawValue: 1000)

    init<Content: View>(size: NSSize, content: Content) {
        super.init(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered, defer: false)
        // Before the level, which this resets.
        isFloatingPanel = true
        level = Self.level
        collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        hidesOnDeactivate = false
        becomesKeyOnlyIfNeeded = true
        worksWhenModal = true
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        ignoresMouseEvents = true
        isReleasedWhenClosed = false
        let host = FirstClickHostingView(rootView: content)
        host.sizingOptions = []
        host.sceneBridgingOptions = []
        contentView = host
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// This window is never key, so without this a first click would only be asked to focus it.
private final class FirstClickHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}
