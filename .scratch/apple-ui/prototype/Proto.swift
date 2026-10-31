// Throwaway look prototype for ticket 05. Fake data, no Rust. It never activates, so it can't take
// the focus. Build: swiftc -parse-as-library -target arm64-apple-macos26 Proto.swift -o proto
import AppKit
import SwiftUI

let accent = Color(red: 0.11, green: 0.55, blue: 0.53)

@main
enum Main {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let args = CommandLine.arguments
        app.appearance = NSAppearance(named: args.contains("light") ? .aqua : .darkAqua)
        let mode = args.count > 1 ? args[1] : "settings"
        let delegate = Delegate(mode: mode)
        app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}

final class Delegate: NSObject, NSApplicationDelegate {
    let mode: String
    var windows: [NSWindow] = []
    init(mode: String) { self.mode = mode }

    func applicationDidFinishLaunching(_ note: Notification) {
        switch mode {
        case "settings": show(SettingsProto(), size: CGSize(width: 860, height: 640), title: "Settings")
        case "meetings": show(MeetingsProto(), size: CGSize(width: 1000, height: 680), title: "Meetings")
        case "onboarding": show(OnboardingProto(), size: CGSize(width: 680, height: 560), title: "Set up Wispr Free")
        default: showPills()
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.4) {
            for window in self.windows.reversed() { print("window \(window.windowNumber) \(Int(window.frame.minX)) \(Int(window.frame.minY)) \(Int(window.frame.width)) \(Int(window.frame.height))") }
            print("screen \(Int(NSScreen.main?.frame.height ?? 0))")
            fflush(stdout)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.2) { exit(0) }
    }

    func show(_ view: some View, size: CGSize, title: String) {
        let window = NSWindow(contentRect: CGRect(origin: CGPoint(x: 160, y: 160), size: size),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                              backing: .buffered, defer: false)
        window.title = title
        window.toolbarStyle = .unified
        window.contentViewController = NSHostingController(rootView: view.tint(accent))
        window.setContentSize(size)
        window.orderFrontRegardless()
        windows.append(window)
    }

    func showPills() {
        let size = CGSize(width: 900, height: 420)
        let back = NSWindow(contentRect: CGRect(origin: CGPoint(x: 160, y: 160), size: size), styleMask: [.borderless], backing: .buffered, defer: false)
        back.contentView = NSHostingView(rootView: Backdrop())
        back.level = .floating
        back.orderFrontRegardless()
        windows.append(back)
        let panel = NSPanel(contentRect: back.frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = .statusBar
        let host = NSHostingView(rootView: PillSheet())
        host.sizingOptions = []
        host.sceneBridgingOptions = []
        panel.contentView = host
        panel.orderFrontRegardless()
        windows.append(panel)
    }
}

// MARK: Pill

struct Backdrop: View {
    var body: some View {
        HStack(spacing: 0) {
            Rectangle().fill(LinearGradient(colors: [.white, Color(white: 0.86)], startPoint: .top, endPoint: .bottom))
            Rectangle().fill(LinearGradient(colors: [Color(red: 0.35, green: 0.25, blue: 0.75), Color(red: 0.9, green: 0.45, blue: 0.35)], startPoint: .topLeading, endPoint: .bottomTrailing))
            Rectangle().fill(Color(white: 0.09))
        }
    }
}

enum Skin: String, CaseIterable { case glass = "A  tinted glass", hud = "B  active HUD blur", solid = "C  solid", dark = "D  dark glass" }

struct Hud: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .hudWindow
        view.blendingMode = .behindWindow
        view.state = .active
        view.appearance = NSAppearance(named: .vibrantDark)
        return view
    }
    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}

struct SkinModifier<S: InsettableShape>: ViewModifier {
    let skin: Skin
    let shape: S
    @ViewBuilder func body(content: Content) -> some View {
        switch skin {
        case .glass: content.glassEffect(.regular.tint(.black.opacity(0.45)), in: shape)
        case .dark: content.glassEffect(.regular.tint(.black.opacity(0.82)), in: shape)
        case .hud: content.background(Hud().clipShape(shape)).overlay(shape.strokeBorder(.white.opacity(0.16), lineWidth: 0.5))
        case .solid: content.background(Color(red: 0.09, green: 0.086, blue: 0.11), in: shape).overlay(shape.strokeBorder(.white.opacity(0.2), lineWidth: 0.5))
        }
    }
}

struct Bars: View {
    let heights: [CGFloat] = [0.3, 0.55, 0.8, 1, 0.7, 0.9, 0.5, 0.75, 0.4, 0.25]
    var body: some View {
        HStack(spacing: 2) {
            ForEach(heights.indices, id: \.self) { i in
                Capsule().fill(.white.opacity(0.95)).frame(width: 2, height: max(2, heights[i] * 12))
            }
        }
    }
}

struct Pills: View {
    let skin: Skin
    var body: some View {
        VStack(spacing: 16) {
            Text(skin.rawValue).font(.caption2.weight(.medium)).foregroundStyle(.white).padding(.horizontal, 6).padding(.vertical, 2).background(.black.opacity(0.55), in: Capsule())
            Capsule().fill(.clear).frame(width: 42, height: 18).modifier(SkinModifier(skin: skin, shape: Capsule()))
            Bars().frame(width: 60, height: 20).modifier(SkinModifier(skin: skin, shape: Capsule()))
            HStack(spacing: 0) {
                Image(systemName: "xmark").font(.system(size: 8, weight: .bold)).frame(width: 18, height: 18).background(.white.opacity(0.16), in: Circle())
                Spacer(minLength: 4)
                Bars()
                Spacer(minLength: 4)
                Image(systemName: "checkmark").font(.system(size: 8, weight: .bold)).foregroundStyle(.black).frame(width: 18, height: 18).background(.white, in: Circle())
            }
            .foregroundStyle(.white).padding(.horizontal, 3).frame(width: 110, height: 24).modifier(SkinModifier(skin: skin, shape: Capsule()))
            VStack(alignment: .leading, spacing: 8) {
                Text("No text field here. Copy your text?").font(.system(size: 12)).foregroundStyle(.white)
                HStack(spacing: 6) {
                    Spacer()
                    Button("Dismiss") {}.controlSize(.small).buttonStyle(.bordered).environment(\.colorScheme, .dark)
                    Button("Copy") {}.controlSize(.small).buttonStyle(.borderedProminent).environment(\.colorScheme, .dark)
                }
            }
            .padding(10).frame(width: 236).modifier(SkinModifier(skin: skin, shape: RoundedRectangle(cornerRadius: 14, style: .continuous)))
        }
    }
}

struct PillSheet: View {
    var skins: [Skin] {
        let turn = CommandLine.arguments.count > 2 ? Int(CommandLine.arguments[2]) ?? 0 : 0
        return (0..<3).map { Skin.allCases[($0 + turn) % 4] }
    }
    var body: some View {
        HStack(spacing: 0) {
            ForEach(skins, id: \.self) { skin in Pills(skin: skin).frame(maxWidth: .infinity) }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: Settings

enum Page: String, CaseIterable, Identifiable {
    case general = "General", models = "Models", pipeline = "Pipeline", style = "Style & Dictionary", meetings = "Meetings", sounds = "Sounds & Media", privacy = "Privacy & Data", about = "About"
    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .models: "square.stack.3d.up"
        case .pipeline: "slider.horizontal.3"
        case .style: "text.book.closed"
        case .meetings: "person.2"
        case .sounds: "speaker.wave.2"
        case .privacy: "hand.raised"
        case .about: "info.circle"
        }
    }
}

struct KeyCombo: View {
    let text: String
    var body: some View {
        Text(text).font(.callout.monospaced()).padding(.horizontal, 7).padding(.vertical, 2)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
    }
}

struct SettingsProto: View {
    @State var page: Page? = .general
    @State var auto = true
    @State var pill = true
    @State var login = false
    @State var theme = "System"
    var body: some View {
        NavigationSplitView {
            List(Page.allCases, selection: $page) { page in
                Label(page.rawValue, systemImage: page.symbol).tag(page)
            }
            .navigationSplitViewColumnWidth(210)
        } detail: {
            Form {
                Section {
                    ForEach([("Push to talk", "⌃ ⌥"), ("Hands-free", "⌃ ⌥ Space"), ("Paste last transcript", "⌃ ⌘ V"), ("Copy last transcript", "⌃ ⌘ C"), ("Meeting notes", "⌃ ⌘ N")], id: \.0) { row in
                        LabeledContent(row.0) {
                            HStack { KeyCombo(text: row.1); Button("Change…") {}.controlSize(.small) }
                        }
                    }
                } header: { Text("Shortcuts") } footer: { Text("Press the new combo and release it to save.").foregroundStyle(.secondary) }
                Section("Microphone") {
                    Toggle("Choose automatically", isOn: $auto)
                    LabeledContent("Input level") { ProgressView(value: 0.45).frame(width: 150) }
                    LabeledContent("Test") { Button("Test Microphone") {}.controlSize(.small) }
                }
                Section("Appearance and Startup") {
                    Picker("Theme", selection: $theme) { ForEach(["System", "Light", "Dark"], id: \.self) { Text($0) } }
                    Toggle("Launch at login", isOn: $login)
                    Toggle("Show pill", isOn: $pill)
                }
            }
            .formStyle(.grouped)
            .navigationTitle(page?.rawValue ?? "")
        }
    }
}

// MARK: Meetings

struct MeetingsProto: View {
    @State var query = ""
    @State var tab = "Summary"
    @State var note = "Roadmap review"
    let rows = [("Roadmap review", "31 Oct, 10:00 · 42 min"), ("Design sync", "30 Oct, 15:30 · 28 min"), ("Hiring plan", "29 Oct, 09:15 · 51 min"), ("Budget Q4", "27 Oct, 17:00 · 33 min")]
    var body: some View {
        NavigationSplitView {
            List(selection: $note) {
                ForEach(rows, id: \.0) { row in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(row.0).font(.body.weight(.medium))
                        Text(row.1).font(.caption).foregroundStyle(.secondary)
                    }.tag(row.0).padding(.vertical, 2)
                }
            }
            .searchable(text: $query, placement: .sidebar, prompt: "Search Meetings")
            .navigationSplitViewColumnWidth(260)
            .toolbar { ToolbarItem(placement: .primaryAction) { Button("Start Notes", systemImage: "record.circle") {} } }
        } detail: {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Roadmap review").font(.largeTitle.weight(.bold))
                        Text("Saturday 31 October, 10:00 · 42 minutes · 3 speakers").foregroundStyle(.secondary)
                    }
                    GroupBox("TL;DR") {
                        Text("The team agreed to ship the new onboarding first and to move the Windows work to next quarter.").frame(maxWidth: .infinity, alignment: .leading).padding(4)
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Key Points").font(.title3.weight(.semibold))
                        ForEach(["Onboarding ships before the redesign", "Windows support moves to next quarter", "Two hires are approved for design"], id: \.self) { point in
                            HStack(alignment: .firstTextBaseline) {
                                Image(systemName: "circle.fill").font(.system(size: 5)).foregroundStyle(.secondary)
                                Text(point)
                                Spacer()
                                Button("12:40") {}.buttonStyle(.link).font(.caption.monospacedDigit())
                            }
                        }
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Action Items").font(.title3.weight(.semibold))
                        ForEach(["Anna: draft the onboarding copy (Friday)", "Marco: move the Windows tickets"], id: \.self) { item in
                            Label(item, systemImage: "circle")
                        }
                    }
                }
                .padding(28).frame(maxWidth: 720, alignment: .leading).frame(maxWidth: .infinity)
            }
            .navigationTitle("")
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Picker("View", selection: $tab) { ForEach(["Summary", "Notes", "Transcript", "Chat"], id: \.self) { Text($0) } }.pickerStyle(.segmented).frame(width: 300)
                }
                ToolbarItem(placement: .primaryAction) { Button("Export", systemImage: "square.and.arrow.up") {} }
            }
        }
    }
}

// MARK: Onboarding

struct OnboardingProto: View {
    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 20) {
                    Image(systemName: "keyboard").font(.system(size: 40, weight: .light)).foregroundStyle(.tint)
                    Text("Keyboard and Paste").font(.largeTitle.weight(.bold))
                    Text("Two permissions let Wispr Free hear your shortcut and paste your words.").foregroundStyle(.secondary).multilineTextAlignment(.center)
                    Form {
                        Section {
                            LabeledContent { Label("Allowed", systemImage: "checkmark.circle.fill").foregroundStyle(.green) } label: { Text("Input Monitoring"); Text("Sees your shortcut while another app is in front.") }
                            LabeledContent { Button("Allow…") {}.buttonStyle(.borderedProminent) } label: { Text("Accessibility"); Text("Pastes your words into the app you're using.") }
                        }
                    }
                    .formStyle(.grouped).scrollDisabled(true).frame(height: 150)
                }
                .padding(32)
            }
            Divider()
            HStack {
                Text("Step 2 of 4").foregroundStyle(.secondary)
                Spacer()
                Button("Back") {}.controlSize(.large)
                Button("Continue") {}.controlSize(.large).buttonStyle(.borderedProminent)
            }
            .padding(16)
        }
    }
}
