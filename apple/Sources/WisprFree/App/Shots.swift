import AppKit
import ScreenCaptureKit
import WfCore

/// `cargo xtask shots`: every pill state over its own backdrop, from the real panel. Both windows
/// sit behind everything else, and the capture asks for those two alone, so the user sees nothing
/// and nothing activates.
final class Shots {
    /// Long enough for a morph to end.
    private static let settle: Duration = .milliseconds(450)
    /// The panel's size in the sheet, relative to points, so the sheet reads the same on any Mac.
    private static let sheetScale: CGFloat = 2
    private static let labelWidth: CGFloat = 200
    private static let iconSide: CGFloat = 72

    /// The cursor far from the pill, so that it can't change what's shown.
    struct Surroundings: PillSurroundings {
        private let real = RealSurroundings()
        var cursor: WfCore.Point? { WfCore.Point(x: 1, y: 1) }
        var displays: [Display] { real.displays }
        var escapeIsDown: Bool { false }
    }

    private struct Backdrop {
        let name: String
        let appearance: NSAppearance.Name
        let color: NSColor
    }

    private static let backdrops = [
        Backdrop(name: "light", appearance: .aqua, color: .white),
        Backdrop(name: "dark", appearance: .darkAqua, color: NSColor(white: 0.09, alpha: 1)),
        Backdrop(
            name: "colour", appearance: .darkAqua,
            color: NSColor(red: 0.75, green: 0.42, blue: 0.5, alpha: 1)),
    ]

    private static let scenes: [(String, PillScene)] = [
        ("hidden", .state(state: .hidden)),
        ("resting", .state(state: .resting)),
        ("ready", .state(state: .ready)),
        ("listening", .state(state: .listeningPtt)),
        ("locked", .state(state: .listeningHandsFree)),
        ("processing", .state(state: .processing)),
        ("error", .state(state: .error(reason: "No microphone found"))),
        ("meeting", .state(state: .meeting)),
        ("card-copy", .noTextField(text: "words")),
        ("card-meeting", .suggestMeeting(app: "Zoom")),
        // Last, as it outlasts the shots of the next scene.
        ("toast", .toast(text: "Copied — paste with Ctrl+Cmd+V")),
    ]

    private let shell: Shell
    private let pill: PillController
    private let folder: URL

    init(shell: Shell, pill: PillController, folder: String) {
        self.shell = shell
        self.pill = pill
        self.folder = URL(filePath: folder, directoryHint: .isDirectory)
    }

    func take() async {
        do {
            // Asking ScreenCaptureKit without the grant would bring up a prompt.
            guard CGPreflightScreenCaptureAccess() else {
                log("shots need Screen Recording")
                return
            }
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let (windows, frame) = placeWindows()
            var rows: [(String, [CGImage])] = []
            for (name, scene) in Self.scenes {
                rows.append((name, try await shots(of: name, scene, in: windows, frame: frame)))
            }
            try write(sheet(of: rows), to: "sheet.png")
            try write(iconImage(), to: "menu-bar-icon.png")
            log("shots taken")
        } catch {
            log("shots failed: \(error)")
        }
    }

    /// A backdrop behind the panel, the two of them behind every other window.
    private func placeWindows() -> ([NSWindow], NSRect) {
        let panel = pill.panel
        let backdrop = NSWindow(
            contentRect: panel.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        backdrop.isReleasedWhenClosed = false
        panel.level = .normal
        backdrop.orderBack(nil)
        panel.order(.above, relativeTo: backdrop.windowNumber)
        return ([backdrop, panel], panel.frame)
    }

    private func shots(
        of name: String, _ scene: PillScene, in windows: [NSWindow], frame: NSRect
    ) async throws -> [CGImage] {
        try shell.pillAnswer(accepted: false)
        try shell.stage(scene: .state(state: .resting))
        try shell.stage(scene: scene)
        var images: [CGImage] = []
        for backdrop in Self.backdrops {
            NSApp.appearance = NSAppearance(named: backdrop.appearance)
            windows[0].backgroundColor = backdrop.color
            pill.tick()
            try await Task.sleep(for: Self.settle)
            pill.tick()
            let image = try await capture(windows, frame: frame)
            try write(image, to: "\(name)-\(backdrop.name).png")
            images.append(image)
        }
        return images
    }

    private func capture(_ windows: [NSWindow], frame: NSRect) async throws -> CGImage {
        let content = try await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: false)
        let numbers = Set(windows.map { CGWindowID($0.windowNumber) })
        let chosen = content.windows.filter { numbers.contains($0.windowID) }
        guard let display = content.displays.first(where: { $0.frame.contains(frame.origin) })
            ?? content.displays.first
        else { throw ShotError.noDisplay }
        let filter = SCContentFilter(display: display, including: chosen)
        let configuration = SCStreamConfiguration()
        let height = NSScreen.screens.first?.frame.height ?? 0
        let rect = Desktop.rect(of: frame, primaryHeight: height)
        configuration.sourceRect = CGRect(
            x: rect.x - display.frame.minX, y: rect.y - display.frame.minY,
            width: rect.width, height: rect.height)
        let scale = pill.panel.backingScaleFactor
        configuration.width = Int(frame.width * scale)
        configuration.height = Int(frame.height * scale)
        configuration.showsCursor = false
        return try await SCScreenshotManager.captureImage(
            contentFilter: filter, configuration: configuration)
    }

    /// One row a scene, one column a backdrop.
    private func sheet(of rows: [(String, [CGImage])]) -> CGImage? {
        let panel = pill.panel.frame.size
        let (cellWidth, cellHeight) = (panel.width * Self.sheetScale, panel.height * Self.sheetScale)
        let columns = CGFloat(Self.backdrops.count)
        let size = NSSize(
            width: Self.labelWidth + cellWidth * columns, height: cellHeight * CGFloat(rows.count))
        let image = NSImage(size: size, flipped: true) { _ in
            NSColor.black.setFill()
            NSRect(origin: .zero, size: size).fill()
            for (row, (name, cells)) in rows.enumerated() {
                let y = CGFloat(row) * cellHeight
                NSString(string: name).draw(
                    at: NSPoint(x: 8, y: y + cellHeight / 2 - 14),
                    withAttributes: [
                        .font: NSFont.systemFont(ofSize: 24), .foregroundColor: NSColor.white,
                    ])
                for (column, picture) in cells.enumerated() {
                    let x = Self.labelWidth + CGFloat(column) * cellWidth
                    NSImage(cgImage: picture, size: panel).draw(
                        in: NSRect(x: x, y: y, width: cellWidth, height: cellHeight),
                        from: .zero, operation: .copy, fraction: 1, respectFlipped: true,
                        hints: [.interpolation: NSImageInterpolation.none.rawValue])
                }
            }
            return true
        }
        return image.cgImage(forProposedRect: nil, context: nil, hints: nil)
    }

    /// The icon large, in black on white, as a template gives no colour of its own.
    private func iconImage() -> CGImage? {
        let side = Self.iconSide
        let image = NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            NSColor.white.setFill()
            rect.fill()
            StatusItem.icon().draw(in: rect)
            return true
        }
        return image.cgImage(forProposedRect: nil, context: nil, hints: nil)
    }

    private func write(_ image: CGImage?, to name: String) throws {
        guard let image, let data = NSBitmapImageRep(cgImage: image)
            .representation(using: .png, properties: [:])
        else { throw ShotError.noPicture(name) }
        try data.write(to: folder.appending(path: name))
    }

    private enum ShotError: Error {
        case noDisplay
        case noPicture(String)
    }
}
