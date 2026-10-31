import SwiftUI
import WfCore

/// The title, when it was, how far the work after it has got, and what went wrong.
struct MeetingHeader: View {
    let book: MeetingsBook

    private var title: Binding<String> {
        Binding(get: { book.title }, set: { book.editTitle($0) })
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField("Meeting title", text: title, prompt: Text("Untitled meeting"))
                .textFieldStyle(.plain)
                .font(.largeTitle.bold())
                .accessibilityIdentifier("Meeting title")
            Text(book.when)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("When")
            statusRow
            if !book.progressText.isEmpty {
                progress
            }
            if !book.failures.isEmpty {
                Label(book.failures, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("Failures")
            }
            if !book.notice.isEmpty {
                Label(book.notice, systemImage: "info.circle")
                    .accessibilityIdentifier("Notice")
            }
        }
    }

    private var statusRow: some View {
        HStack(spacing: 12) {
            ForEach(book.stages, id: \.label) { stage in
                StageLabel(stage: stage)
            }
            if book.canFinish {
                Button("Finish processing") { book.ask { try $0.finishMeeting() } }
                    .controlSize(.small)
                    .accessibilityIdentifier("Finish processing")
            }
        }
    }

    private var progress: some View {
        HStack(spacing: 12) {
            Text(book.progressText)
                .font(.callout)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("Progress")
            if book.progress >= 0 {
                ProgressView(value: book.progress).frame(width: 120)
            }
            if book.busy {
                Button("Cancel") { book.ask { try $0.cancelMeetingWork() } }
                    .controlSize(.small)
                    .accessibilityIdentifier("Cancel")
            }
        }
    }
}

/// One stage on the status row. Rust words it, and its mark only draws how it stands.
struct StageLabel: View {
    let stage: StageRow

    var body: some View {
        Label(stage.text, systemImage: symbol)
            .font(.caption)
            .foregroundStyle(style)
            .help(stage.reason)
            .accessibilityIdentifier(stage.text)
    }

    private var symbol: String {
        switch stage.state {
        case .waiting: "circle"
        case .done: "checkmark.circle.fill"
        case .skipped: "minus.circle"
        case .failed: "xmark.circle.fill"
        }
    }

    private var style: AnyShapeStyle {
        switch stage.state {
        case .waiting, .skipped: AnyShapeStyle(.secondary)
        case .done: AnyShapeStyle(.primary)
        case .failed: AnyShapeStyle(.red)
        }
    }
}

/// A time in the meeting, which leads to its line if there is one.
struct TimeChip: View {
    let chip: ChatChip
    let book: MeetingsBook

    var body: some View {
        Button(chip.text) { book.show(row: chip.row) }
            .buttonStyle(.link)
            .font(.caption.monospaced())
            .disabled(chip.row < 0)
            .accessibilityLabel(chip.help)
            .accessibilityIdentifier(chip.help)
    }
}
