import SwiftUI
import WfCore

struct PipelinePage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    private var pipeline: Pipeline { editor.settings.pipeline }

    var body: some View {
        SettingsForm(page: .pipeline, editor: editor, pages: pages) {
            quality
            stages
            models
        }
    }

    private var quality: some View {
        Section {
            SettingPicker(
                title: "Quality tier", options: pages.tiers.map { ($0.tier, $0.title) },
                selection: Binding(
                    get: { pipeline.tier }, set: { pages.change(.tier(tier: $0)) }))
        } header: {
            Text("Quality")
        } footer: {
            Text("A tier sets the stages and models below. Changing one of them makes it Custom.")
        }
    }

    private var stages: some View {
        Section("Stages") {
            SettingToggle(
                title: "Noise suppression", hint: "For noisy rooms and cheap mics.",
                isOn: Binding(
                    get: { pipeline.denoise }, set: { pages.change(.denoise(on: $0)) }))
            if pages.state.vadForced {
                SettingToggle(
                    title: "Silence trimming (VAD)",
                    hint: "Always on with Whisper, which invents text in silence.",
                    isOn: .constant(true)
                )
                .disabled(true)
            } else {
                SettingToggle(
                    title: "Silence trimming (VAD)",
                    isOn: Binding(get: { pipeline.vad }, set: { pages.change(.vad(on: $0)) }))
            }
            SettingToggle(
                title: "LLM cleanup", hint: "Rewrites the transcript with a local model.",
                isOn: Binding(
                    get: { pipeline.llmCleanup }, set: { pages.change(.llmCleanup(on: $0)) }))
            SettingPicker(
                title: "Cleanup strength", options: CleanupStrength.options,
                selection: editor.binding(\.pipeline.cleanupStrength))
            SettingToggle(
                title: "Stop hands-free after silence",
                isOn: editor.binding(\.pipeline.vadAutoStop))
            SettingStepper(
                title: "Silence before stopping", unit: "s", range: 1...60,
                value: editor.binding(\.pipeline.vadAutoStopSeconds)
            )
            .disabled(!pipeline.vadAutoStop)
        }
    }

    private var models: some View {
        Section("Models") {
            StageLine(title: "Speech recognition", menu: pages.state.asrMenu, pages: pages) {
                .asrModel(option: $0)
            }
            StageLine(title: "Voice detection", menu: pages.state.vadMenu, pages: pages) {
                .vadModel(option: $0)
            }
            StageLine(title: "Cleanup model", menu: pages.state.llmMenu, pages: pages) {
                .llmModel(option: $0)
            }
            SettingPicker(
                title: "Keep models loaded", options: KeepModels.options,
                selection: editor.binding(\.pipeline.keepModelsLoaded))
            ErrorLine(text: pages.state.modelError)
        }
    }
}

/// A stage's model: the menu, and under it how that model stands and the way to get it.
struct StageLine: View {
    let title: String
    let menu: StageMenu
    let pages: SettingsPages
    let change: (Int32) -> RuledChange

    var body: some View {
        Picker(
            title,
            selection: Binding(get: { menu.selected }, set: { pages.change(change($0)) })
        ) {
            ForEach(Array(menu.options.enumerated()), id: \.offset) { option, name in
                Text(name).tag(Int32(option))
            }
        }
        .accessibilityIdentifier(title)
        if let model = menu.model {
            HStack(spacing: 8) {
                if model.needed {
                    Image(
                        systemName: model.ready
                            ? "checkmark.circle.fill" : "arrow.down.circle"
                    )
                    .foregroundStyle(model.ready ? Color.green : Color.secondary)
                }
                Text(model.text).foregroundStyle(.secondary)
                Spacer()
                if model.downloading {
                    Button("Cancel") { pages.ask { try $0.cancelDownload() } }
                        .accessibilityIdentifier("Cancel download")
                } else if model.missing {
                    Button(model.row.downloadTitle) {
                        pages.ask { try $0.download(index: model.row.index) }
                    }
                    .disabled(pages.state.modelBusy || pages.state.modelsReadOnly)
                    .accessibilityIdentifier("Download \(model.row.name)")
                }
            }
            if model.downloading {
                ProgressView(value: model.row.progress)
            }
        }
    }
}
