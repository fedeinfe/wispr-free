import SwiftUI
import WfCore

struct SummaryTab: View {
    let book: MeetingsBook

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                warning
                if book.summaryStale {
                    Text("The transcript changed after this summary was written.")
                        .font(.callout).foregroundStyle(.secondary)
                }
                if book.summaryPartial {
                    Text(
                        "This came straight from the notes on each part, because the last step failed. Regenerating may do better."
                    )
                    .font(.callout).foregroundStyle(.secondary)
                }
                if !book.summaryEmpty.isEmpty {
                    Text(book.summaryEmpty).accessibilityIdentifier("No summary")
                }
                ForEach(book.summary.indices, id: \.self) { place in
                    SummaryLineView(line: book.summary[place], book: book)
                }
            }
            .readingColumn()
            .padding(.vertical, 12)
        }
    }

    private var warning: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(
                "Written by the model, so it can miss or misread things. The transcript is one click away."
            )
            .font(.callout).foregroundStyle(.secondary)
            Spacer(minLength: 0)
            if book.summaryStale {
                Button("Update summary") { book.ask { try $0.updateSummary() } }
                    .disabled(!book.canSummarize)
                    .accessibilityIdentifier("Update summary")
            }
            Button("Regenerate") { book.ask { try $0.regenerateSummary() } }
                .disabled(!book.canSummarize)
                .accessibilityIdentifier("Regenerate")
        }
    }
}

struct SummaryLineView: View {
    let line: SummaryLine
    let book: MeetingsBook

    var body: some View {
        switch line.kind {
        case .heading:
            Text(line.text).font(.title3.weight(.semibold)).padding(.top, 8)
        case .item:
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("•").foregroundStyle(.secondary)
                Text(line.text)
                Spacer(minLength: 0)
                if let chip = line.chip {
                    TimeChip(chip: chip, book: book)
                }
            }
        case .plain:
            GroupBox("TL;DR") {
                Text(line.text)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(4)
            }
        }
    }
}

struct NotesTab: View {
    let book: MeetingsBook

    private var notes: Binding<String> {
        Binding(get: { book.notes }, set: { book.editNotes($0) })
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Yours. The summary and the chat can read them.")
                .font(.callout).foregroundStyle(.secondary)
            NotesEditor(text: notes)
        }
        .readingColumn()
        .padding(.vertical, 12)
    }
}

/// `NSTextView` itself, as SwiftUI's `TextEditor` around it took 170 MB of GPU memory the first time
/// it drew, for a second, and kept 6 MB more after its window closed.
struct NotesEditor: NSViewRepresentable {
    @Binding var text: String

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroller = NSTextView.scrollableTextView()
        scroller.drawsBackground = false
        guard let view = scroller.documentView as? NSTextView else { return scroller }
        view.delegate = context.coordinator
        view.isRichText = false
        view.allowsUndo = true
        view.drawsBackground = false
        view.font = .preferredFont(forTextStyle: .body)
        view.textColor = .labelColor
        view.string = text
        view.setAccessibilityIdentifier("Meeting notes")
        return scroller
    }

    func updateNSView(_ scroller: NSScrollView, context: Context) {
        if let view = scroller.documentView as? NSTextView, view.string != text {
            view.string = text
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        @Binding private var text: String

        init(text: Binding<String>) {
            _text = text
        }

        func textDidChange(_ notification: Notification) {
            if let view = notification.object as? NSTextView { text = view.string }
        }
    }
}

struct TranscriptTab: View {
    let book: MeetingsBook

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !book.speakers.isEmpty {
                SpeakerNames(book: book)
                    .readingColumn()
                    .padding(.vertical, 8)
            }
            ScrollViewReader { reader in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        if !book.transcriptEmpty.isEmpty {
                            Text(book.transcriptEmpty).accessibilityIdentifier("Transcript hint")
                        }
                        ForEach(book.transcript.indices, id: \.self) { place in
                            TranscriptRow(
                                line: book.transcript[place],
                                aimed: book.jump?.row == Int32(place)
                            )
                            .id(place)
                        }
                    }
                    .readingColumn()
                    .padding(.vertical, 12)
                }
                // Rust's word to jump may come before the rows, and a click after them.
                .task(id: book.jump) {
                    if let row = book.jump?.row { reader.scrollTo(Int(row), anchor: .top) }
                }
            }
        }
    }
}

struct SpeakerNames: View {
    let book: MeetingsBook

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 200), spacing: 12)], spacing: 8) {
            ForEach(book.speakers, id: \.id) { speaker in
                HStack(spacing: 8) {
                    Text(speaker.label).font(.callout).foregroundStyle(.secondary)
                    TextField(
                        "Name of \(speaker.label)",
                        text: Binding(
                            get: { speaker.name }, set: { book.nameSpeaker(speaker, $0) }),
                        prompt: Text("Name")
                    )
                    .textFieldStyle(.roundedBorder)
                    .labelsHidden()
                    .accessibilityIdentifier("Name of \(speaker.label)")
                }
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

struct TranscriptRow: View, Equatable {
    let line: TranscriptLine
    /// Picked out, having been jumped to.
    let aimed: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(line.time)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
            if line.gap {
                Text(line.text).font(.callout).foregroundStyle(.secondary)
            } else {
                VStack(alignment: .leading, spacing: 2) {
                    Text(line.speaker).font(.caption).foregroundStyle(.secondary)
                    Text(line.text)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(4)
        .background(aimed ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear))
    }
}

struct ChatTab: View {
    let book: MeetingsBook
    private static let end = "end"

    var body: some View {
        ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if !book.chatHint.isEmpty {
                        Text(book.chatHint)
                            .font(.callout).foregroundStyle(.secondary)
                            .accessibilityIdentifier("Chat hint")
                    }
                    ForEach(book.chat.indices, id: \.self) { place in
                        ChatMessageView(message: book.chat[place], book: book)
                    }
                    if !book.streamingQuestion.isEmpty {
                        ChatQuestion(text: book.streamingQuestion)
                        Text(book.streamingText).accessibilityIdentifier("Answer on its way")
                    }
                    if !book.chatError.isEmpty {
                        Label(book.chatError, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.red)
                            .accessibilityIdentifier("Chat error")
                    }
                    Color.clear.frame(height: 1).id(Self.end)
                }
                .readingColumn()
                .padding(.vertical, 12)
            }
            .safeAreaBar(edge: .bottom) {
                ChatInput(book: book).readingColumn().padding(.bottom, 12)
            }
            .onChange(of: book.streamingText) { reader.scrollTo(Self.end, anchor: .bottom) }
            .onChange(of: book.chat.count) { reader.scrollTo(Self.end, anchor: .bottom) }
            .onAppear { reader.scrollTo(Self.end, anchor: .bottom) }
        }
    }
}

struct ChatQuestion: View {
    let text: String

    var body: some View {
        GroupBox {
            Text(text).frame(maxWidth: .infinity, alignment: .leading).padding(4)
        }
    }
}

struct ChatMessageView: View {
    let message: ChatMessage
    let book: MeetingsBook

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ChatQuestion(text: message.question)
            ForEach(message.blocks.indices, id: \.self) { place in
                let block = message.blocks[place]
                VStack(alignment: .leading, spacing: 4) {
                    Text(block.text)
                    if !block.chips.isEmpty {
                        HStack(spacing: 8) {
                            ForEach(block.chips.indices, id: \.self) { chip in
                                TimeChip(chip: block.chips[chip], book: book)
                            }
                        }
                    }
                }
            }
            if !message.note.isEmpty {
                Text(message.note).font(.callout).foregroundStyle(.secondary)
            }
        }
    }
}

/// The one floating control over content, so the one place with glass outside the system's bars.
struct ChatInput: View {
    @Bindable var book: MeetingsBook

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !book.chatRunning {
                HStack(spacing: 8) {
                    ForEach(book.suggestions, id: \.self) { question in
                        Button(question) { book.askAbout(question) }
                            .accessibilityIdentifier(question)
                    }
                }
            }
            HStack(spacing: 8) {
                TextField("Ask about this meeting", text: $book.draft)
                    .textFieldStyle(.roundedBorder)
                    .labelsHidden()
                    .onSubmit {
                        if !book.chatRunning && !book.draft.isEmpty { book.send() }
                    }
                    .accessibilityIdentifier("Ask about this meeting")
                if book.chatRunning {
                    Button("Stop") { book.ask { try $0.stopMeetingChat() } }
                        .accessibilityIdentifier("Stop")
                } else {
                    Button("Send") { book.send() }
                        .buttonStyle(.glassProminent)
                        .disabled(book.draft.isEmpty)
                        .accessibilityIdentifier("Send")
                }
            }
        }
    }
}
