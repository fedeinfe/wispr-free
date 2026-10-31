import AppKit
import WfCore

/// What a menu item runs. The item holds on to it, as `target` alone wouldn't.
private final class ItemTarget: NSObject {
    private let run: () -> Void

    init(run: @escaping () -> Void) {
        self.run = run
    }

    @objc func fire() {
        run()
    }
}

extension NSMenuItem {
    /// An item that runs a closure.
    convenience init(title: String, key: String = "", run: @escaping () -> Void) {
        let target = ItemTarget(run: run)
        self.init(title: title, action: #selector(ItemTarget.fire), keyEquivalent: key)
        self.target = target
        representedObject = target
    }
}

/// Fills an `NSMenu` with what Rust said it holds. Every title and rule comes from there.
enum MenuBuilder {
    static func fill(_ menu: NSMenu, with entries: [MenuEntry], pick: @escaping (MenuAction) -> Void) {
        menu.removeAllItems()
        menu.autoenablesItems = false
        for entry in entries {
            menu.addItem(item(for: entry, pick: pick))
        }
    }

    private static func item(for entry: MenuEntry, pick: @escaping (MenuAction) -> Void) -> NSMenuItem {
        switch entry {
        case .item(let title, let action):
            return NSMenuItem(title: title) { pick(action) }
        case .submenu(let title, let entries):
            let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            let submenu = NSMenu(title: title)
            fill(submenu, with: entries, pick: pick)
            item.submenu = submenu
            item.isEnabled = !entries.isEmpty
            return item
        case .separator:
            return .separator()
        }
    }
}
