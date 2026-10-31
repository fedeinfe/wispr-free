import SwiftUI
import WfCore

struct GeneralPage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    var body: some View {
        SettingsForm(page: .general, editor: editor, pages: pages) {
            shortcuts
            microphone
            languages
            startup
        }
    }

    private var shortcuts: some View {
        Section {
            ForEach(Array(pages.state.shortcuts.enumerated()), id: \.offset) { index, row in
                ShortcutLine(index: Int32(index), row: row, pages: pages)
            }
            ErrorLine(text: pages.state.shortcutError)
        } header: {
            Text("Shortcuts")
        } footer: {
            Text("Press the new combo and release it to save. Esc alone stops recording.")
        }
    }

    private var microphone: some View {
        Section("Microphone") {
            SettingToggle(
                title: "Choose automatically", hint: "Follows the system default.",
                isOn: Binding(
                    get: { pages.state.microphoneAuto },
                    set: { on in pages.ask { try $0.setMicrophoneAuto(on: on) } }))
            if !pages.state.microphoneAuto {
                if pages.state.microphones.isEmpty {
                    Text("No microphones found.").foregroundStyle(.secondary)
                }
                ForEach(Array(pages.state.microphones.enumerated()), id: \.offset) { index, row in
                    MicrophoneLine(
                        index: Int32(index), row: row, count: pages.state.microphones.count,
                        pages: pages)
                }
            }
            LabeledContent {
                HStack(spacing: 12) {
                    ProgressView(value: pages.level)
                    Button(pages.state.microphoneTesting ? "Stop test" : "Test") {
                        pages.ask { try $0.toggleMicrophoneTest() }
                    }
                    .accessibilityIdentifier("Test microphone")
                }
            } label: {
                Text("Microphone level")
            }
            ErrorLine(text: pages.state.microphoneError)
        }
    }

    private var languages: some View {
        Section("My languages") {
            SettingToggle(
                title: "Auto-detect", hint: "Otherwise dictation listens for the languages you pick.",
                isOn: Binding(
                    get: { pages.state.languagesAuto },
                    set: { on in pages.ask { try $0.setLanguagesAuto(on: on) } }))
            if !pages.state.languagesAuto {
                LazyVGrid(
                    columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: 3),
                    alignment: .leading, spacing: 8
                ) {
                    ForEach(Array(pages.state.languages.enumerated()), id: \.offset) { index, row in
                        Toggle(
                            row.name,
                            isOn: Binding(
                                get: { row.selected },
                                set: { _ in pages.ask { try $0.toggleLanguage(index: Int32(index)) } })
                        )
                        .toggleStyle(.checkbox)
                        .accessibilityIdentifier(row.name)
                    }
                }
            }
        }
    }

    private var startup: some View {
        Section("Startup") {
            SettingToggle(
                title: "Launch at login",
                isOn: Binding(
                    get: { editor.settings.general.launchAtLogin },
                    set: { pages.change(.launchAtLogin(on: $0)) }))
            SettingToggle(
                title: "Show pill", hint: "Shortcuts keep working while it's hidden.",
                isOn: editor.binding(\.general.showPill))
            if pages.state.isMacos {
                SettingToggle(
                    title: "Show in Dock",
                    isOn: Binding(
                        get: { editor.settings.general.showInDock },
                        set: { pages.change(.showInDock(on: $0)) }))
            }
        }
    }
}

/// A shortcut: the combo, a small Change…, and Reset in a context menu.
struct ShortcutLine: View {
    let index: Int32
    let row: ShortcutRow
    let pages: SettingsPages

    private var recording: Bool {
        pages.state.recordingAction == UInt32(index)
    }

    private var anyRecording: Bool {
        pages.state.recordingAction != nil
    }

    var body: some View {
        LabeledContent(row.action) {
            HStack(spacing: 8) {
                Text(recording ? pages.state.recordingKeys : row.combo)
                    .font(.system(.body, design: .monospaced))
                if recording {
                    Button("Stop") { pages.ask { try $0.stopRecordingShortcut() } }
                        .controlSize(.small)
                        .accessibilityIdentifier("Stop recording")
                } else {
                    Button("Change…") { pages.ask { try $0.recordShortcut(index: index) } }
                        .controlSize(.small)
                        .disabled(anyRecording)
                        .accessibilityIdentifier("Change \(row.action)")
                }
            }
        }
        .contextMenu {
            ResetShortcut(index: index, action: row.action, pages: pages)
        }
    }
}

struct ResetShortcut: View {
    let index: Int32
    let action: String
    let pages: SettingsPages

    var body: some View {
        Button("Reset") { pages.ask { try $0.resetShortcut(index: index) } }
            .disabled(pages.state.recordingAction != nil)
            .accessibilityIdentifier("Reset \(action)")
    }
}

/// A place in the microphone ranking.
struct MicrophoneLine: View {
    let index: Int32
    let row: MicrophoneRow
    let count: Int
    let pages: SettingsPages

    var body: some View {
        LabeledContent(row.title) {
            HStack(spacing: 4) {
                Button("Move up", systemImage: "chevron.up") {
                    pages.ask { try $0.moveMicrophone(index: index, by: -1) }
                }
                .disabled(index == 0)
                .accessibilityIdentifier("Move \(row.name) up")
                Button("Move down", systemImage: "chevron.down") {
                    pages.ask { try $0.moveMicrophone(index: index, by: 1) }
                }
                .disabled(Int(index) + 1 == count)
                .accessibilityIdentifier("Move \(row.name) down")
            }
            .labelStyle(.iconOnly)
        }
    }
}
