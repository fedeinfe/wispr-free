import AppKit

/// An accessory app shows no menu bar, but this is still where ⌘, ⌘W and the Edit commands come
/// from, and text fields need them.
enum MainMenu {
    static func make(openSettings: @escaping () -> Void) -> NSMenu {
        let menu = NSMenu()
        menu.addItem(submenu(appMenu(openSettings: openSettings)))
        menu.addItem(submenu(fileMenu()))
        menu.addItem(submenu(editMenu()))
        return menu
    }

    private static func submenu(_ menu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem(title: menu.title, action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }

    private static func appMenu(openSettings: @escaping () -> Void) -> NSMenu {
        let menu = NSMenu(title: "Wispr Free")
        menu.addItem(NSMenuItem(title: "Settings…", key: ",", run: openSettings))
        return menu
    }

    private static func fileMenu() -> NSMenu {
        let menu = NSMenu(title: "File")
        menu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        return menu
    }

    private static func editMenu() -> NSMenu {
        let menu = NSMenu(title: "Edit")
        menu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = menu.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(.separator())
        menu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        menu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        menu.addItem(
            withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        return menu
    }
}
