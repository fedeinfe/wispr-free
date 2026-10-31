import SwiftUI
import WfCore

/// How a part stands: green when it works, red when it doesn't, an empty ring until it's known.
struct CheckMark: View {
    let check: Check

    var body: some View {
        switch check {
        case .passed:
            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
        case .failed:
            Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
        case .waiting:
            Image(systemName: "circle").foregroundStyle(.secondary)
        }
    }
}

/// One thing to allow: what it's for, whether it is, and the way to change that.
struct PermissionLine: View {
    let card: PermissionCard
    let walk: SetupWalk

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            LabeledContent {
                HStack(spacing: 8) { trailing }
            } label: {
                Text(card.name)
                Text(card.why)
                if card.access != .granted {
                    Text(card.status)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("\(card.name): \(card.status)")
                }
            }
            if !card.hint.isEmpty {
                Text(card.hint).font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private var trailing: some View {
        if card.access == .granted {
            CheckMark(check: .passed)
            Text(card.status).accessibilityIdentifier("\(card.name): \(card.status)")
        }
        if card.canAsk {
            Button("Allow…") { walk.ask { try $0.askPermission(index: card.permission) } }
                .accessibilityIdentifier("Allow \(card.name)")
        }
        if card.canOpenPane {
            Button("Open System Settings") {
                walk.ask { try $0.openPermissionPane(index: card.permission) }
            }
            .accessibilityIdentifier("Open System Settings for \(card.name)")
        }
    }
}

struct MicrophoneStep: View {
    let page: SetupPage
    let walk: SetupWalk

    var body: some View {
        Section {
            ForEach(page.permissions, id: \.permission) { card in
                PermissionLine(card: card, walk: walk)
            }
        }
        Section {
            HStack(spacing: 8) {
                CheckMark(check: page.microphoneMark)
                Text(page.microphoneLine).accessibilityIdentifier(page.microphoneLine)
            }
        } header: {
            Text("Microphone")
        } footer: {
            if !page.microphoneHint.isEmpty {
                Text(page.microphoneHint)
            }
        }
    }
}

struct KeysStep: View {
    let page: SetupPage
    let walk: SetupWalk

    var body: some View {
        Section {
            ForEach(page.permissions, id: \.permission) { card in
                PermissionLine(card: card, walk: walk)
            }
        }
        if page.keysStuck {
            Section {
                Button("Quit and reopen") { walk.ask { try $0.reopenApp() } }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("Quit and reopen")
            } header: {
                Text("One more thing")
            } footer: {
                Text(
                    "Both are allowed, but the shortcut isn't reaching the app. macOS sometimes wants the app reopened after this."
                )
            }
        }
    }
}

struct ModelsStep: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    private var state: PagesState { pages.state }

    private var recommended: String? {
        pages.tiers.first { $0.tier == state.recommendedTier }?.title
    }

    var body: some View {
        Section("This computer") {
            Text(state.hardwareSummary.isEmpty ? "Detecting hardware…" : state.hardwareSummary)
            if let recommended {
                Text("Recommended for it: \(recommended)").foregroundStyle(.secondary)
            }
        }
        Section {
            SettingPicker(
                title: "Quality tier", options: state.offeredTiers.map { ($0.tier, $0.title) },
                selection: Binding(
                    get: { editor.settings.pipeline.tier },
                    set: { pages.change(.tier(tier: $0)) })
            )
            .disabled(state.modelBusy)
        } header: {
            Text("Quality")
        } footer: {
            Text(
                "Lite is quick and small. Balanced also rewrites your words cleanly. High needs the most memory. You can change this later in Settings."
            )
        }
        ModelsFolderSection(pages: pages)
        Section {
            ForEach(state.modelsInUse, id: \.index) { row in
                ModelLine(row: row, pages: pages)
            }
            if !state.modelsInUseSummary.isEmpty {
                Button("Download all") { pages.ask { try $0.downloadModelsInUse() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(state.modelBusy || state.modelsFolderMissing)
                    .accessibilityIdentifier("Download all")
            }
        } header: {
            Text("What it downloads")
        } footer: {
            Text(state.modelsToDownload)
        }
        ErrorLine(text: state.modelError)
    }
}

struct TrialStep: View {
    let page: SetupPage
    let walk: SetupWalk
    let pages: SettingsPages

    private var trial: TrialView { page.trial }

    var body: some View {
        Section("Your turn") {
            if trial.problem {
                ErrorLine(text: trial.message)
            } else {
                Text(trial.message)
            }
            if trial.phase == .listening {
                ProgressView(value: walk.level)
            }
            if trial.canStop {
                Button("Stop") { walk.ask { try $0.stopTrial() } }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("Stop")
            }
        }
        if let heard = trial.heard {
            Section("Heard") {
                Text(heard).accessibilityIdentifier("Heard")
            }
        }
        Section("What this proves") {
            ForEach(page.checks, id: \.name) { row in
                HStack(spacing: 8) {
                    CheckMark(check: row.check)
                    Text(row.name)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(row.label)
                .accessibilityIdentifier(row.label)
            }
        }
        Section {
            if let row = pages.state.shortcuts.first {
                ShortcutLine(index: 0, row: row, pages: pages)
            }
            ErrorLine(text: pages.state.shortcutError)
            Button("Try without the shortcut") { walk.ask { try $0.startTrial() } }
                .disabled(trial.phase != .idle)
                .accessibilityIdentifier("Try without the shortcut")
        } header: {
            Text("If the shortcut does nothing")
        } footer: {
            Text("Change it here, or start with the mouse to check the rest.")
        }
    }
}
