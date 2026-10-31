// Throwaway: a Swift app over the real flow. Runs a fixed script, prints what it measures, exits.
import AppKit
import SwiftUI
import WfCore

final class StreamSink: EventSink {
    let continuation: AsyncStream<CoreEvent>.Continuation
    init(_ continuation: AsyncStream<CoreEvent>.Continuation) { self.continuation = continuation }
    func onEvent(event: CoreEvent) { continuation.yield(event) }
}

func footprint() -> Double {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
    let status = withUnsafeMutablePointer(to: &info) { pointer in
        pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count) }
    }
    return status == KERN_SUCCESS ? Double(info.phys_footprint) / 1_048_576 : -1
}

func report(_ what: String) {
    print(String(format: "%-34@ %6.1f MB", what as NSString, footprint()))
    fflush(stdout)
}

@Observable final class Model {
    var clicks = 0
    var answers = 0
    var core: Core?
    func click() {
        clicks += 1
        core?.pillClicked()
    }
}

struct PillView: View {
    let model: Model
    let heights: [CGFloat] = [0.3, 0.55, 0.8, 1, 0.7, 0.9, 0.5, 0.75, 0.4, 0.25]
    var body: some View {
        VStack {
            Spacer()
            Button(action: model.click) {
                HStack(spacing: 2) {
                    ForEach(heights.indices, id: \.self) { i in
                        Capsule().fill(.white.opacity(0.95)).frame(width: 2, height: max(2, heights[i] * 12))
                    }
                }
                .frame(width: 60, height: 20)
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .glassEffect(.regular.tint(.black.opacity(0.45)), in: Capsule())
            .padding(.bottom, 24)
        }
    }
}

final class Delegate: NSObject, NSApplicationDelegate {
    let model = Model()
    var panel: NSPanel?
    var status: NSStatusItem?
    var settings: NSWindow?

    func applicationDidFinishLaunching(_ note: Notification) {
        report("AppKit, before anything")
        let status = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        status.button?.image = NSImage(systemSymbolName: "waveform", accessibilityDescription: "Wispr Free")
        let menu = NSMenu()
        menu.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "")
        status.menu = menu
        self.status = status
        report("status item")
        makePanel()
        report("glass pill shown")
        startCore()
        report("Rust flow started")
        if let core = model.core {
            var settings = core.settings()
            settings.general.showPill.toggle()
            settings.style.dictionary.append("Wispr")
            settings.pipeline.modelsFolder = "/Volumes/Extreme SSD/test"
            settings.shortcuts.pushToTalk = ["Fn", "Space"]
            let back = core.echo(settings: settings)
            print("settings round trip equal \(back == settings), shortcut \(back.shortcuts.pushToTalk), tier \(back.pipeline.tier), folder \(back.pipeline.modelsFolder ?? "-")")
        }
        exit(0)
    }

    func makePanel() {
        let screen = NSScreen.main!.frame
        let frame = CGRect(x: screen.midX - 160, y: screen.minY, width: 320, height: 120)
        let panel = NSPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = NSWindow.Level(rawValue: 1000)
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        panel.hidesOnDeactivate = false
        let host = NSHostingView(rootView: PillView(model: model))
        host.sizingOptions = []
        host.sceneBridgingOptions = []
        panel.contentView = host
        panel.orderFrontRegardless()
        self.panel = panel
    }

    func startCore() {
        let (stream, continuation) = AsyncStream.makeStream(of: CoreEvent.self, bufferingPolicy: .bufferingNewest(64))
        let data = NSTemporaryDirectory() + "wf-stack-data"
        try? FileManager.default.createDirectory(atPath: data, withIntermediateDirectories: true)
        model.core = Core.start(data: data, helper: "/usr/bin/false", sink: StreamSink(continuation))
        Task {
            for await event in stream {
                print("event: \(event)")
                if case .toast = event { self.model.answers += 1 }
            }
        }
    }

    func clickPill() {
        guard let panel, let screen = NSScreen.main?.frame else { return }
        NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp]) { event in
            print("event seen in app: \(event.type.rawValue) window \(event.window?.windowNumber ?? -1)")
            return event
        }
        let point = CGPoint(x: panel.frame.midX, y: screen.height - (panel.frame.minY + 34))
        let pid = ProcessInfo.processInfo.processIdentifier
        for type in [CGEventType.leftMouseDown, .leftMouseUp] {
            CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left)?.postToPid(pid)
        }
    }

    func openSettings() {
        let size = CGSize(width: 860, height: 640)
        let window = NSWindow(contentRect: CGRect(origin: CGPoint(x: 200, y: 200), size: size),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.title = "Settings"
        window.contentViewController = NSHostingController(rootView: SettingsProto().tint(accent))
        window.setContentSize(size)
        window.orderFrontRegardless()
        settings = window
    }

    func closeSettings() {
        settings?.orderOut(nil)
        settings?.contentViewController = nil
        settings = nil
    }

    func script() async {
        let front = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?"
        try? await Task.sleep(for: .seconds(3))
        report("idle 3 s")
        clickPill()
        try? await Task.sleep(for: .seconds(1))
        if model.clicks == 0, let panel {
            print("posted click did not arrive, sending through the window")
            let spot = CGPoint(x: 160, y: 34)
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                if let event = NSEvent.mouseEvent(with: type, location: spot, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: panel.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1) {
                    panel.sendEvent(event)
                }
                try? await Task.sleep(for: .milliseconds(120))
            }
            try? await Task.sleep(for: .seconds(1))
        }
        print("front was \(front), now \(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?")")
        print("clicks \(model.clicks), answers from Rust \(model.answers), app active \(NSApp.isActive), panel key \(panel?.isKeyWindow ?? false), front same \(front == NSWorkspace.shared.frontmostApplication?.bundleIdentifier)")
        exit(0)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = Delegate()
app.delegate = delegate
app.run()
