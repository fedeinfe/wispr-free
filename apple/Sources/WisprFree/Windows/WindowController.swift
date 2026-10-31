import AppKit
import SwiftUI

/// What sets one window apart from another.
struct WindowShape {
    /// Its `AXIdentifier`.
    let identifier: String
    let title: String
    let size: NSSize
    let minimum: NSSize
    var style: NSWindow.StyleMask = [.titled, .closable, .resizable, .fullSizeContentView]
    var zooms = true
}

/// How a window comes forward. Tests order it in by another way, as `front` takes the focus.
struct WindowStage {
    let activate: () -> Void
    let order: (NSWindow) -> Void
    /// In view, with the focus left where it is.
    let orderQuietly: (NSWindow) -> Void

    func bringForward(_ window: NSWindow) {
        activate()
        order(window)
    }

    /// An accessory app doesn't get the foreground unless it asks.
    static let front = WindowStage(
        activate: { NSApp.activate() }, order: { $0.makeKeyAndOrderFront(nil) },
        orderQuietly: { $0.orderFrontRegardless() })
    /// Behind every other window, with nothing activated, for a run with no person at the Mac.
    static let behind = WindowStage(
        activate: {}, order: { $0.orderBack(nil) }, orderQuietly: { $0.orderBack(nil) })
}

/// One open window. It's made on open and let go on close, and everything its view holds goes
/// with it.
final class WindowController: NSObject, NSWindowDelegate {
    let window: NSWindow
    private let identifier: String
    private let onClose: () -> Void

    init(shape: WindowShape, content: some View, onClose: @escaping () -> Void) {
        window = NSWindow(
            contentRect: NSRect(origin: .zero, size: shape.size), styleMask: shape.style,
            backing: .buffered, defer: false)
        identifier = shape.identifier
        self.onClose = onClose
        super.init()
        let host = NSHostingController(rootView: content)
        host.sizingOptions = []
        host.sceneBridgingOptions = [.title, .toolbars]
        window.contentViewController = host
        window.identifier = NSUserInterfaceItemIdentifier(shape.identifier)
        window.title = shape.title
        window.toolbarStyle = .unified
        window.contentMinSize = shape.minimum
        window.setContentSize(shape.size)
        window.isReleasedWhenClosed = false
        window.isRestorable = false
        window.delegate = self
        if !shape.zooms {
            window.collectionBehavior.insert(.fullScreenNone)
            window.standardWindowButton(.zoomButton)?.isEnabled = false
        }
        window.center()
    }

    deinit {
        log("released \(identifier)")
    }

    func windowWillClose(_ notification: Notification) {
        onClose()
    }
}
