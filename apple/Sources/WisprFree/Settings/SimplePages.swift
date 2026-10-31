import SwiftUI
import WfCore

struct MeetingsPage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    var body: some View {
        SettingsForm(page: .meetings, editor: editor, pages: pages) {
            Section("Recording") {
                SettingToggle(
                    title: "Suggest notes when a meeting starts",
                    hint: "Asks first. It never records by itself.",
                    isOn: editor.binding(\.meetings.autoDetect))
                SettingToggle(
                    title: "Live transcript", isOn: editor.binding(\.meetings.liveTranscript))
                SettingToggle(
                    title: "Keep the audio after the transcript is final",
                    isOn: editor.binding(\.meetings.keepAudio))
                SettingStepper(
                    title: "Maximum length", unit: "min", range: 15...480, step: 15,
                    value: editor.binding(\.meetings.maxLengthMinutes))
            }
            Section("After the meeting") {
                SettingToggle(
                    title: "Refine pass", hint: "Transcribes again with the larger model.",
                    isOn: editor.binding(\.meetings.refinePass))
                SettingToggle(
                    title: "Tell speakers apart", isOn: editor.binding(\.meetings.diarization))
                TextField(
                    "Summary template", text: editor.binding(\.meetings.summaryTemplate),
                    prompt: Text(
                        "Extra instructions for the summary. Leave empty for the standard sections."
                    ),
                    axis: .vertical
                )
                .lineLimit(3...8)
                .accessibilityIdentifier("Summary template")
            }
        }
    }
}

struct SoundsPage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    var body: some View {
        SettingsForm(page: .sounds, editor: editor, pages: pages) {
            Section {
                SettingToggle(
                    title: "Sounds", hint: "Short start, stop, lock and error sounds.",
                    isOn: editor.binding(\.media.sounds))
                SettingPicker(
                    title: "While dictating",
                    hint:
                        "Mute silences every app while the mic is open. Pause only reaches Music and Spotify: macOS doesn't tell apps what else is playing, so browsers and other apps keep playing. macOS asks once before each player can be controlled. Media is left alone during a meeting.",
                    options: MediaWhileDictating.options,
                    selection: editor.binding(\.media.whileDictating))
            }
        }
    }
}

struct PrivacyPage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    @State var confirmingDelete = false

    var body: some View {
        SettingsForm(page: .privacy, editor: editor, pages: pages) {
            Section {
                SettingToggle(
                    title: "Context awareness",
                    hint:
                        "Reads the focused app and nearby text to match style and spelling. Nothing is stored beyond the history entry.",
                    isOn: editor.binding(\.privacy.contextAwareness))
                SettingPicker(
                    title: "History", hint: "Never store also erases the history kept so far.",
                    options: HistoryRetention.options,
                    selection: editor.binding(\.privacy.historyRetention))
            }
            Section {
                FolderLine(path: pages.state.dataFolder) {
                    Button("Open folder") { pages.ask { try $0.openDataFolder() } }
                        .accessibilityIdentifier("Open folder")
                }
                HStack(spacing: 8) {
                    Button("Export all") { pages.ask { try $0.exportAll() } }
                        .accessibilityIdentifier("Export all")
                    Button("Delete all data", role: .destructive) { confirmingDelete = true }
                        .accessibilityIdentifier("Delete all data")
                }
                if !pages.state.dataStatus.isEmpty {
                    Text(pages.state.dataStatus).accessibilityIdentifier("Data status")
                }
            } header: {
                Text("Your data")
            } footer: {
                Text("Everything stays on this computer.")
            }
        }
        .confirmationDialog(
            "Delete all data?", isPresented: $confirmingDelete, titleVisibility: .visible
        ) {
            Button("Yes, delete everything", role: .destructive) {
                pages.ask { try $0.deleteAll() }
            }
            .accessibilityIdentifier("Yes, delete everything")
            Button("Keep it", role: .cancel) {}
                .accessibilityIdentifier("Keep it")
        } message: {
            Text("This removes your settings, models, history and meetings. It can't be undone.")
        }
    }
}

struct AboutPage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    var body: some View {
        SettingsForm(page: .about, editor: editor, pages: pages) {
            Section {
                Text("Free, fully local voice typing. Nothing leaves this computer.")
                LabeledContent {
                    Button("Check for updates") { pages.ask { try $0.checkForUpdates() } }
                        .accessibilityIdentifier("Check for updates")
                } label: {
                    Text("Updates")
                    Text("Opens the releases page in your browser. The app never checks by itself.")
                }
            } header: {
                Text("wispr-free \(pages.state.version)")
            }
            Section("Licenses") {
                Text("wispr-free is MIT licensed.")
                Text("Each model's license is listed on the Models page.")
            }
        }
    }
}
