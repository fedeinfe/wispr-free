import SwiftUI
import WfCore

/// The list and the meeting. Rust keeps both, and the book says what it last heard.
struct MeetingsView: View {
    static let shape = WindowShape(
        identifier: "Meetings", title: "Meetings", size: NSSize(width: 1000, height: 680),
        minimum: NSSize(width: 760, height: 480),
        style: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])

    @Bindable var book: MeetingsBook

    var body: some View {
        NavigationSplitView {
            MeetingsSidebar(book: book)
                .navigationSplitViewColumnWidth(260)
                .toolbar {
                    ToolbarItem(placement: .primaryAction) {
                        Button(
                            book.startLabel,
                            systemImage: book.recording ? "stop.fill" : "mic"
                        ) {
                            book.ask { try $0.toggleMeeting() }
                        }
                        .accessibilityIdentifier(book.startLabel)
                    }
                }
        } detail: {
            detail
                .toolbar { toolbar }
        }
        .navigationTitle(Self.shape.title)
        .toolbar(removing: .title)
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            Picker("Show", selection: $book.tab) {
                ForEach(MeetingTab.allCases) { tab in
                    Text(tab.title)
                        .tag(tab)
                        .accessibilityIdentifier(tab.title)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .disabled(!book.hasSelection)
        }
        if book.canResume {
            ToolbarItem(placement: .primaryAction) { KeepRecordingButton(book: book) }
            ToolbarSpacer(.fixed, placement: .primaryAction)
        }
        if book.canExport {
            ToolbarItem(placement: .primaryAction) { ExportMeetingButton(book: book) }
        }
        if book.canDelete {
            ToolbarItem(placement: .primaryAction) { DeleteMeetingButton(book: book) }
        }
    }

    @ViewBuilder private var detail: some View {
        if book.hasSelection {
            VStack(spacing: 0) {
                MeetingHeader(book: book)
                    .readingColumn()
                    .padding(.top, 12)
                    .padding(.bottom, 8)
                tabContent
            }
        } else {
            Text(book.detailHint)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 320)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .accessibilityIdentifier("Detail hint")
        }
    }

    @ViewBuilder private var tabContent: some View {
        switch book.tab {
        case .summary: SummaryTab(book: book)
        case .notes: NotesTab(book: book)
        case .transcript: TranscriptTab(book: book)
        case .chat: ChatTab(book: book)
        }
    }
}

extension View {
    /// The 720 pt column every meeting reads in.
    func readingColumn() -> some View {
        padding(.horizontal, 28)
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity)
    }
}

struct KeepRecordingButton: View {
    let book: MeetingsBook

    var body: some View {
        Button("Keep recording", systemImage: "play.fill") {
            book.ask { try $0.resumeMeeting() }
        }
        .accessibilityIdentifier("Keep recording")
    }
}

struct ExportMeetingButton: View {
    let book: MeetingsBook

    var body: some View {
        Button("Export", systemImage: "square.and.arrow.up") {
            book.ask { try $0.exportMeeting() }
        }
        .accessibilityIdentifier("Export")
    }
}

/// Destructive, so it asks first.
struct DeleteMeetingButton: View {
    let book: MeetingsBook
    @State var confirming = false

    var body: some View {
        Button("Delete", systemImage: "trash") { confirming = true }
            .accessibilityIdentifier("Delete")
            .confirmationDialog(
                "Delete this meeting?", isPresented: $confirming, titleVisibility: .visible
            ) {
                Button("Delete for good", role: .destructive) {
                    book.ask { try $0.deleteMeeting() }
                }
                .accessibilityIdentifier("Delete for good")
                Button("Keep it", role: .cancel) {}
                    .accessibilityIdentifier("Keep it")
            }
    }
}
