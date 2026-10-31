import AppKit
import SwiftUI

/// The capsule's left and right buttons, straight from AppKit: a right click needs the event to
/// open a menu with, and Rust tracks the drag by polling the cursor.
struct PressArea: NSViewRepresentable {
    let press: () -> Void
    let release: () -> Void
    let menu: (NSEvent, NSView) -> Void

    func makeNSView(context: Context) -> PressView {
        let view = PressView()
        update(view)
        return view
    }

    func updateNSView(_ view: PressView, context: Context) {
        update(view)
    }

    private func update(_ view: PressView) {
        view.press = press
        view.release = release
        view.openMenu = menu
    }
}

final class PressView: NSView {
    var press: () -> Void = {}
    var release: () -> Void = {}
    var openMenu: (NSEvent, NSView) -> Void = { _, _ in }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        press()
    }

    override func mouseUp(with event: NSEvent) {
        release()
    }

    override func rightMouseDown(with event: NSEvent) {
        openMenu(event, self)
    }
}
