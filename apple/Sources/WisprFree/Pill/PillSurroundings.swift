import AppKit
import WfCore

/// The outside world, as the pill polls it: it never gets the keyboard, so it can't be told.
protocol PillSurroundings {
    var cursor: WfCore.Point? { get }
    var displays: [Display] { get }
    var escapeIsDown: Bool { get }
}

struct RealSurroundings: PillSurroundings {
    /// `kVK_Escape`.
    private static let escape: CGKeyCode = 53

    var cursor: WfCore.Point? {
        guard let height = NSScreen.screens.first?.frame.height else { return nil }
        let location = NSEvent.mouseLocation
        return WfCore.Point(x: location.x, y: height - location.y)
    }

    var displays: [Display] {
        guard let height = NSScreen.screens.first?.frame.height else { return [] }
        return NSScreen.screens.map { screen in
            Display(
                bounds: Desktop.rect(of: screen.frame, primaryHeight: height),
                workArea: Desktop.rect(of: screen.visibleFrame, primaryHeight: height),
                unitsPerPoint: 1)
        }
    }

    var escapeIsDown: Bool {
        CGEventSource.keyState(.combinedSessionState, key: Self.escape)
    }
}

/// AppKit's global coordinates start at the primary screen's bottom-left corner, and Rust's at its
/// top left.
enum Desktop {
    static func rect(of frame: NSRect, primaryHeight: CGFloat) -> WfCore.Rect {
        WfCore.Rect(
            x: frame.minX, y: primaryHeight - frame.minY - frame.height,
            width: frame.width, height: frame.height)
    }

    /// The bottom-left corner of a window whose top left Rust put at `origin`.
    static func windowOrigin(
        _ origin: WfCore.Point, height: CGFloat, primaryHeight: CGFloat
    ) -> NSPoint {
        NSPoint(x: origin.x, y: primaryHeight - origin.y - height)
    }
}
