import AppKit

// Reads the real app from outside the way an assistive tool does, and prints JSON for `cargo xtask
// ax-check`, which decides what it means. Nothing here activates anything.
//
//   ax-walk walk <pid>
//   ax-walk close <pid> <window identifier>
//   ax-walk select <pid> <window identifier> <row identifier>
//   ax-walk press <pid> <window identifier> <control identifier>

struct Node: Encodable {
    let role: String
    let identifier: String
    let title: String
    let enabled: Bool
}

struct Window: Encodable {
    let identifier: String
    let title: String
    let nodes: [Node]
}

struct Walk: Encodable {
    let trusted: Bool
    let frontPid: Int32
    let windows: [Window]
}

/// More than any window here has. A tree that goes on past this has gone wrong.
let nodeLimit = 5000

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

func text(_ element: AXUIElement, _ name: String) -> String {
    attribute(element, name) as? String ?? ""
}

func elements(_ element: AXUIElement, _ name: String) -> [AXUIElement] {
    attribute(element, name) as? [AXUIElement] ?? []
}

func descendants(of root: AXUIElement) -> [AXUIElement] {
    var found: [AXUIElement] = []
    var waiting = [root]
    while let element = waiting.popLast(), found.count < nodeLimit {
        found.append(element)
        waiting += elements(element, kAXChildrenAttribute)
    }
    return found
}

func node(_ element: AXUIElement) -> Node {
    Node(
        role: text(element, kAXRoleAttribute), identifier: text(element, kAXIdentifierAttribute),
        title: text(element, kAXTitleAttribute),
        enabled: attribute(element, kAXEnabledAttribute) as? Bool ?? true)
}

/// An app builds its tree for whoever says it's an assistive tool.
func application(_ pid: pid_t) -> AXUIElement {
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    return app
}

func windows(of app: AXUIElement) -> [AXUIElement] {
    elements(app, kAXWindowsAttribute)
}

func walk(_ pid: pid_t) -> Walk {
    let app = application(pid)
    let walked = windows(of: app).map { window in
        Window(
            identifier: text(window, kAXIdentifierAttribute), title: text(window, kAXTitleAttribute),
            nodes: descendants(of: window).map(node))
    }
    return Walk(
        trusted: AXIsProcessTrusted(),
        frontPid: NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0, windows: walked)
}

func window(_ identifier: String, of pid: pid_t) -> AXUIElement {
    guard let window = windows(of: application(pid)).first(where: {
        text($0, kAXIdentifierAttribute) == identifier
    }) else { fail("no window \(identifier)") }
    return window
}

func close(_ identifier: String, of pid: pid_t) {
    guard let button = attribute(window(identifier, of: pid), kAXCloseButtonAttribute) else {
        fail("\(identifier) has no close button")
    }
    press(button as! AXUIElement)
}

func press(_ element: AXUIElement) {
    let result = AXUIElementPerformAction(element, kAXPressAction as CFString)
    guard result == .success else { fail("pressing failed: \(result.rawValue)") }
}

func select(row identifier: String, in windowIdentifier: String, of pid: pid_t) {
    let nodes = descendants(of: window(windowIdentifier, of: pid))
    guard let row = nodes.first(where: { text($0, kAXIdentifierAttribute) == identifier }) else {
        fail("no row \(identifier)")
    }
    let selectable = ([row] + ancestors(of: row)).first {
        text($0, kAXRoleAttribute) == kAXRowRole
    }
    let result = AXUIElementSetAttributeValue(
        selectable ?? row, kAXSelectedAttribute as CFString, kCFBooleanTrue)
    guard result == .success else { fail("selecting failed: \(result.rawValue)") }
}

func press(control identifier: String, in windowIdentifier: String, of pid: pid_t) {
    let nodes = descendants(of: window(windowIdentifier, of: pid))
    guard let control = nodes.first(where: { text($0, kAXIdentifierAttribute) == identifier }) else {
        fail("no control \(identifier)")
    }
    press(control)
}

func ancestors(of element: AXUIElement) -> [AXUIElement] {
    var found: [AXUIElement] = []
    var next = attribute(element, kAXParentAttribute)
    while let parent = next, found.count < nodeLimit {
        let parent = parent as! AXUIElement
        found.append(parent)
        next = attribute(parent, kAXParentAttribute)
    }
    return found
}

func fail(_ reason: String) -> Never {
    FileHandle.standardError.write(Data("\(reason)\n".utf8))
    exit(1)
}

let arguments = Array(CommandLine.arguments.dropFirst())
guard arguments.count >= 2, let pid = pid_t(arguments[1]) else {
    fail("usage: ax-walk walk|close|select|press <pid> [window] [row or control]")
}
switch (arguments[0], arguments.count) {
case ("walk", 2):
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    FileHandle.standardOutput.write(try! encoder.encode(walk(pid)))
case ("close", 3):
    close(arguments[2], of: pid)
case ("select", 4):
    select(row: arguments[3], in: arguments[2], of: pid)
case ("press", 4):
    press(control: arguments[3], in: arguments[2], of: pid)
default:
    fail("usage: ax-walk walk|close|select|press <pid> [window] [row or control]")
}
