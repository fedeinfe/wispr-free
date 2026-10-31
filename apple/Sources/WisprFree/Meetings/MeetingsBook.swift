import Observation
import WfCore

/// The four tabs, in Rust's order.
enum MeetingTab: Int32, CaseIterable, Identifiable {
    case summary, notes, transcript, chat

    var id: Self { self }

    var title: String {
        switch self {
        case .summary: "Summary"
        case .notes: "Notes"
        case .transcript: "Transcript"
        case .chat: "Chat"
        }
    }
}

/// What the Meetings window shows, as Rust last said. It lives as long as the window, and what it
/// read goes when the window does.
@Observable
final class MeetingsBook {
    private(set) var meetings: [MeetingRow] = []
    private(set) var listHint = ""
    private(set) var detailHint = ""
    private(set) var recording = false
    private(set) var startLabel = ""
    private(set) var selectedId = ""
    private(set) var title = ""
    private(set) var notes = ""
    private(set) var when = ""
    private(set) var thisRecording = false
    private(set) var busy = false
    private(set) var canFinish = false
    private(set) var canResume = false
    private(set) var canSummarize = false
    private(set) var canExport = false
    private(set) var canDelete = false
    private(set) var stages: [StageRow] = []
    private(set) var failures = ""
    private(set) var progressText = ""
    private(set) var progress: Float = -1
    private(set) var notice = ""
    private(set) var summary: [SummaryLine] = []
    private(set) var summaryEmpty = ""
    private(set) var summaryPartial = false
    private(set) var summaryStale = false
    private(set) var transcript: [TranscriptLine] = []
    private(set) var transcriptEmpty = ""
    private(set) var speakers: [SpeakerRow] = []
    private(set) var chat: [ChatMessage] = []
    private(set) var suggestions: [String] = []
    private(set) var chatHint = ""
    private(set) var chatRunning = false
    private(set) var streamingQuestion = ""
    private(set) var streamingText = ""
    private(set) var chatError = ""
    /// The transcript line to show, new each time so that the same one can be asked for twice.
    private(set) var jump: Jump?
    var tab = MeetingTab.summary
    var query = "" {
        didSet {
            if query != oldValue { ask { [query] in try $0.searchMeetings(query: query) } }
        }
    }
    /// What's being typed in the chat. It stays while the tab is away.
    var draft = ""

    struct Jump: Equatable {
        let row: Int32
        let sequence: Int
    }

    @ObservationIgnored let core: any MeetingsCore

    init(core: any MeetingsCore) {
        self.core = core
    }

    var hasSelection: Bool { !selectedId.isEmpty }

    /// For the sidebar's list.
    var selection: String? {
        get { hasSelection ? selectedId : nil }
        set {
            guard let newValue, newValue != selectedId else { return }
            selectedId = newValue
            let row = meetings.first { $0.id == newValue }?.hitRow ?? -1
            ask { try $0.selectMeeting(id: newValue, row: row) }
        }
    }

    func editTitle(_ text: String) {
        guard text != title else { return }
        title = text
        ask { try $0.editMeetingTitle(title: text) }
    }

    func editNotes(_ text: String) {
        guard text != notes else { return }
        notes = text
        ask { try $0.editMeetingNotes(notes: text) }
    }

    func nameSpeaker(_ speaker: SpeakerRow, _ name: String) {
        guard let place = speakers.firstIndex(where: { $0.id == speaker.id }),
            speakers[place].name != name
        else { return }
        speakers[place].name = name
        ask { try $0.nameSpeaker(speaker: speaker.id, name: name) }
    }

    func send() {
        let question = draft
        draft = ""
        askAbout(question)
    }

    func askAbout(_ question: String) {
        ask { try $0.askMeeting(question: question) }
    }

    /// Shows a line of the transcript, on the user's click or Rust's word.
    func show(row: Int32) {
        tab = .transcript
        jump = Jump(row: row, sequence: (jump?.sequence ?? 0) + 1)
    }

    func ask(_ call: (any MeetingsCore) throws -> Void) {
        do {
            try call(core)
        } catch {
            log("the meetings failed: \(error)")
        }
    }

    func adopt(_ changes: [MeetingsChange]) {
        for change in changes {
            take(change)
        }
    }

    private func take(_ change: MeetingsChange) {
        switch change {
        case .meetings(let place, let rows): apply(place, rows, to: &meetings)
        case .listHint(let text): listHint = text
        case .detailHint(let text): detailHint = text
        case .recording(let recording): self.recording = recording
        case .startLabel(let text): startLabel = text
        case .selectedId(let id): selectedId = id
        case .title(let text): title = text
        case .notes(let text): notes = text
        case .when(let text): when = text
        case .thisRecording(let recording): thisRecording = recording
        case .busy(let busy): self.busy = busy
        case .canFinish(let can): canFinish = can
        case .canResume(let can): canResume = can
        case .canSummarize(let can): canSummarize = can
        case .canExport(let can): canExport = can
        case .canDelete(let can): canDelete = can
        case .stages(let rows): stages = rows
        case .failures(let text): failures = text
        case .progressText(let text): progressText = text
        case .progress(let fraction): progress = fraction
        case .notice(let text): notice = text
        case .tab(let index): tab = MeetingTab(rawValue: index) ?? tab
        case .jump(let row): show(row: row)
        case .summary(let lines): summary = lines
        case .summaryEmpty(let text): summaryEmpty = text
        case .summaryPartial(let partial): summaryPartial = partial
        case .summaryStale(let stale): summaryStale = stale
        case .transcript(let place, let rows): apply(place, rows, to: &transcript)
        case .transcriptEmpty(let text): transcriptEmpty = text
        case .speakers(let rows): speakers = rows
        case .chat(let place, let rows): apply(place, rows, to: &chat)
        case .suggestions(let questions): suggestions = questions
        case .chatHint(let text): chatHint = text
        case .chatRunning(let running): chatRunning = running
        case .streamingQuestion(let text): streamingQuestion = text
        case .streamingText(let text): streamingText = text
        case .chatError(let text): chatError = text
        }
    }

    private func apply<Row>(_ place: RowsPlace, _ rows: [Row], to list: inout [Row]) {
        switch place {
        case .all:
            list = rows
        case .at(let index):
            if let row = rows.first, index < list.count { list[Int(index)] = row }
        case .end:
            list += rows
        case .before(let index):
            list.insert(contentsOf: rows, at: min(Int(index), list.count))
        }
    }
}
