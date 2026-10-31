import SwiftUI
import WfCore

/// Every meeting, newest first, or what the search found.
struct MeetingsSidebar: View {
    @Bindable var book: MeetingsBook

    var body: some View {
        List(selection: $book.selection) {
            ForEach(book.meetings, id: \.id) { meeting in
                MeetingListRow(meeting: meeting)
                    .tag(meeting.id)
                    .accessibilityIdentifier(meeting.title)
            }
        }
        .overlay {
            if book.meetings.isEmpty {
                Text(book.listHint)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(16)
                    .accessibilityIdentifier("List hint")
            }
        }
        .searchable(text: $book.query, placement: .sidebar, prompt: "Search meetings")
    }
}

struct MeetingListRow: View {
    let meeting: MeetingRow

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 4) {
                if meeting.recording {
                    Image(systemName: "record.circle").foregroundStyle(.red)
                        .accessibilityLabel("Recording")
                }
                if meeting.busy {
                    Image(systemName: "gearshape").foregroundStyle(.secondary)
                        .accessibilityLabel("Being processed")
                }
                Text(meeting.title).font(.body.weight(.medium)).lineLimit(1)
                Spacer(minLength: 0)
                if meeting.failed {
                    Image(systemName: "exclamationmark.triangle").foregroundStyle(.red)
                        .accessibilityLabel("Something failed")
                }
            }
            Text(meeting.when).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            if !meeting.snippet.isEmpty {
                Text(meeting.snippet).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        .padding(.vertical, 2)
    }
}
