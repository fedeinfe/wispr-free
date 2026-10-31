import SwiftUI
import WfCore

/// The setup walk's window: one step per page, with the bottom bar Rust fills in.
struct OnboardingView: View {
    static let shape = WindowShape(
        identifier: "Set up Wispr Free", title: "Set up Wispr Free",
        size: NSSize(width: 680, height: 560), minimum: NSSize(width: 680, height: 460))

    let walk: SetupWalk
    let editor: SettingsEditor
    let pages: SettingsPages

    var body: some View {
        VStack(spacing: 0) {
            if let page = walk.page {
                heading(page)
                form(page)
                Divider()
                bottomBar(page)
            }
        }
    }

    private func heading(_ page: SetupPage) -> some View {
        VStack(spacing: 8) {
            Image(systemName: page.step.symbol)
                .font(.largeTitle)
                .foregroundStyle(.tint)
                .accessibilityHidden(true)
            Text(page.title).font(.largeTitle.bold())
            Text(page.sentence)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 28)
        .padding(.top, 20)
    }

    /// A new step starts at its top.
    private func form(_ page: SetupPage) -> some View {
        Form {
            if let notice {
                Section {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier("Notice")
                }
            }
            switch page.step {
            case .microphone: MicrophoneStep(page: page, walk: walk)
            case .keys: KeysStep(page: page, walk: walk)
            case .models: ModelsStep(editor: editor, pages: pages)
            case .trial: TrialStep(page: page, walk: walk, pages: pages)
            }
        }
        .formStyle(.grouped)
        .id(page.step)
    }

    private var notice: String? {
        if let notice = editor.notice { return notice }
        return pages.state.notice.isEmpty ? nil : pages.state.notice
    }

    private func bottomBar(_ page: SetupPage) -> some View {
        HStack(spacing: 12) {
            Text(page.stepLabel)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(page.stepLabel)
            Spacer()
            if page.canGoBack {
                Button("Back") { walk.ask { try $0.setupBack() } }
                    .accessibilityIdentifier("Back")
            }
            Button(page.continueLabel) { walk.ask { try $0.setupContinue() } }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(!page.canContinue)
                .accessibilityIdentifier(page.continueLabel)
        }
        .padding(16)
    }
}

extension Step {
    fileprivate var symbol: String {
        switch self {
        case .microphone: "mic"
        case .keys: "keyboard"
        case .models: "arrow.down.circle"
        case .trial: "waveform"
        }
    }
}
