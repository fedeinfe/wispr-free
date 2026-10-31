import SwiftUI
import WfCore

struct ModelsPage: View {
    let editor: SettingsEditor
    let pages: SettingsPages

    private var state: PagesState { pages.state }

    var body: some View {
        SettingsForm(page: .models, editor: editor, pages: pages) {
            computer
            ModelsFolderSection(pages: pages)
            ErrorLine(text: state.modelError)
            downloaded
            available
        }
    }

    private var recommended: String? {
        pages.tiers.first { $0.tier == state.recommendedTier }?.title
    }

    private var computer: some View {
        Section("This computer") {
            Text(state.hardwareSummary.isEmpty ? "Detecting hardware…" : state.hardwareSummary)
            if let recommended {
                Text("Recommended quality tier: \(recommended)").foregroundStyle(.secondary)
            }
        }
    }

    private var downloadedRows: [ModelRow] { state.models.filter(\.installed) }
    private var availableRows: [ModelRow] { state.models.filter { !$0.installed } }

    private var downloaded: some View {
        Section {
            if downloadedRows.isEmpty {
                Text(state.nothingDownloaded).foregroundStyle(.secondary)
            }
            ForEach(downloadedRows, id: \.index) { row in
                ModelLine(row: row, pages: pages)
            }
        } header: {
            Text("Downloaded")
        } footer: {
            Text(state.downloadedSummary)
        }
    }

    @ViewBuilder private var available: some View {
        if !availableRows.isEmpty {
            Section {
                ForEach(availableRows, id: \.index) { row in
                    ModelLine(row: row, pages: pages)
                }
            } header: {
                Text("Available")
            } footer: {
                Text(
                    "Downloads start only when you ask, and every file is checked against its SHA-256."
                )
            }
        }
    }
}

/// Where the models live, and what's asked and running about that. Onboarding has it too.
struct ModelsFolderSection: View {
    let pages: SettingsPages

    private var state: PagesState { pages.state }

    var body: some View {
        Section {
            FolderLine(path: state.modelsFolder) {
                Button("Open folder") { pages.ask { try $0.openModelsFolder() } }
                    .disabled(state.modelsFolderMissing)
                    .accessibilityIdentifier("Open folder")
                Button("Change…") { pages.ask { try $0.chooseModelsFolder() } }
                    .disabled(state.modelBusy || !state.pendingFolder.isEmpty)
                    .accessibilityIdentifier("Change models folder")
                if state.modelsFolderCustom {
                    Button("Use default") { pages.ask { try $0.useDefaultModelsFolder() } }
                        .disabled(state.modelBusy || !state.pendingFolder.isEmpty)
                        .accessibilityIdentifier("Use default")
                }
            }
            if state.modelsReadOnly {
                Text("This run only reads this folder, so nothing here can change.")
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("Read only")
            }
            if state.modelsFolderMissing {
                ErrorLine(
                    text: "This folder isn't available. Connect its drive, or choose another folder.")
                Button("Check again") { pages.ask { try $0.recheckModelsFolder() } }
                    .disabled(state.modelBusy)
                    .accessibilityIdentifier("Check again")
            }
            if !state.pendingFolder.isEmpty {
                pending
            }
            if state.movingModels {
                moving
            }
        } header: {
            Text("Models folder")
        } footer: {
            Text("Models take gigabytes. Keep them anywhere, an external drive included.")
        }
    }

    @ViewBuilder private var pending: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Move to \(state.pendingFolder)?")
            Text(state.pendingSummary).foregroundStyle(.secondary)
            HStack(spacing: 8) {
                Button("Move them") { pages.ask { try $0.moveFolder() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(state.modelBusy || state.modelsReadOnly)
                    .accessibilityIdentifier("Move them")
                Button("Switch without moving") { pages.ask { try $0.switchFolder() } }
                    .disabled(state.modelBusy)
                    .accessibilityIdentifier("Switch without moving")
                Button("Cancel") { pages.ask { try $0.cancelPendingFolder() } }
                    .accessibilityIdentifier("Keep the current folder")
            }
        }
    }

    private var moving: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Moving models… The originals stay until every file is copied.")
                    .foregroundStyle(.secondary)
                ProgressView(value: state.moveProgress)
            }
            Button("Cancel") { pages.ask { try $0.cancelMove() } }
                .accessibilityIdentifier("Cancel moving models")
        }
    }
}

/// One model: what it is, how it stands, and what can be done to it.
struct ModelLine: View {
    let row: ModelRow
    let pages: SettingsPages

    @State var confirmingDelete = false

    private var state: PagesState { pages.state }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            LabeledContent {
                HStack(spacing: 8) { buttons }
            } label: {
                Text(row.name)
                if !row.usedFor.isEmpty {
                    Text(row.usedFor)
                }
                Text(row.detail)
                Text(row.status).foregroundStyle(.secondary)
            }
            if row.downloading {
                ProgressView(value: row.progress)
            }
        }
        .confirmationDialog(
            "Delete \(row.name)?", isPresented: $confirmingDelete, titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                pages.ask { try $0.deleteModel(index: row.index) }
            }
            .accessibilityIdentifier("Yes, delete \(row.name)")
            Button("Keep it", role: .cancel) {}
        } message: {
            Text("You can download it again.")
        }
    }

    @ViewBuilder private var buttons: some View {
        if row.downloading {
            Button("Cancel") { pages.ask { try $0.cancelDownload() } }
                .accessibilityIdentifier("Cancel download")
        }
        if !row.installed && !row.downloading {
            Button(row.downloadTitle) { pages.ask { try $0.download(index: row.index) } }
                .disabled(state.modelBusy || state.modelsFolderMissing || state.modelsReadOnly)
                .accessibilityIdentifier("Download \(row.name)")
        }
        if row.installed {
            Button("Verify") { pages.ask { try $0.verify(index: row.index) } }
                .disabled(state.modelBusy)
                .accessibilityIdentifier("Verify \(row.name)")
        }
        if row.installed || row.partial {
            Button("Delete", role: .destructive) { confirmingDelete = true }
                .disabled(state.modelBusy || state.modelsReadOnly)
                .accessibilityIdentifier("Delete \(row.name)")
        }
    }
}
