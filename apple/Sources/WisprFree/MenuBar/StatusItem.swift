import AppKit
import WfCore

/// The menu bar icon. The menu is filled as it opens, so it's never out of date.
final class StatusItem: NSObject, NSMenuDelegate {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    private let menu = NSMenu()
    private let entries: () -> [MenuEntry]
    private let pick: (MenuAction) -> Void

    init(entries: @escaping () -> [MenuEntry], pick: @escaping (MenuAction) -> Void) {
        self.entries = entries
        self.pick = pick
        super.init()
        item.button?.image = Self.icon()
        item.button?.toolTip = "wispr-free"
        menu.delegate = self
        item.menu = menu
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        MenuBuilder.fill(menu, with: entries(), pick: pick)
    }

    /// Presses the item with this title, the way a click would.
    func press(_ title: String) -> Bool {
        menuNeedsUpdate(menu)
        let index = menu.indexOfItem(withTitle: title)
        guard index >= 0 else { return false }
        menu.performActionForItem(at: index)
        return true
    }

    /// The capsule with three bars that the app has always had, as a template so the menu bar
    /// colours it.
    static func icon() -> NSImage {
        let side: CGFloat = 18
        let unit = side / 32
        let image = NSImage(size: NSSize(width: side, height: side), flipped: true) { _ in
            NSColor.black.set()
            let outline = NSBezierPath(
                roundedRect: NSRect(x: 4 * unit, y: 10 * unit, width: 24 * unit, height: 12 * unit),
                xRadius: 6 * unit, yRadius: 6 * unit)
            outline.lineWidth = 2.5 * unit
            outline.stroke()
            for (x, y, height) in [(10.0, 14.0, 4.0), (15.0, 13.0, 6.0), (20.0, 14.0, 4.0)] {
                NSBezierPath(
                    roundedRect: NSRect(
                        x: x * unit, y: y * unit, width: 2 * unit, height: height * unit),
                    xRadius: unit, yRadius: unit
                ).fill()
            }
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "wispr-free"
        return image
    }
}
