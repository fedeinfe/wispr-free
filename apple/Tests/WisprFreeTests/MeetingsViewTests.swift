import AppKit
import SwiftUI
import Testing
import ViewInspector
import WfCore

@testable import WisprFree

private func row(
    _ title: String, snippet: String = "", hitRow: Int32 = -1, recording: Bool = false,
    busy: Bool = false, failed: Bool = false
) -> MeetingRow {
    MeetingRow(
        id: "id-\(title)", title: title, when: "31 Oct 2026, 10:00 · 2 min", snippet: snippet,
        hitRow: hitRow, recording: recording, busy: busy, failed: failed)
}

private func chip(_ label: String, row: Int32) -> ChatChip {
    ChatChip(
        label: label, row: row, text: row >= 0 ? label : "\(label) ?",
        help: row >= 0 ? "Go to \(label)" : "\(label) matches no line of the transcript")
}

private func stage(_ text: String, _ state: StageMark) -> StageRow {
    StageRow(label: text, state: state, reason: "", text: text)
}

private func line(_ text: String, time: String = "[00:05]", gap: Bool = false) -> TranscriptLine {
    TranscriptLine(time: time, speaker: "You", text: text, gap: gap)
}

@MainActor
private func opened(
    _ changes: [MeetingsChange] = [], selected: String = "id-Launch"
) async throws -> (MeetingsBook, FakeCore) {
    let core = FakeCore(settings: try await defaultSettings())
    let book = MeetingsBook(core: core)
    book.adopt([.selectedId(id: selected)] + changes)
    return (book, core)
}

/// The text of every `Text` in `view`, however deep.
@MainActor
private func texts(_ view: some View) throws -> [String] {
    try view.inspect().findAll(ViewType.Text.self).map { try $0.string() }
}

@MainActor
private func identifiers(_ view: some View) throws -> [String] {
    try view.inspect().findAll(ViewType.Button.self).map { try $0.accessibilityIdentifier() }
}

@MainActor
private func press(_ identifier: String, in view: some View) throws {
    let button = try view.inspect().find(ViewType.Button.self) {
        try $0.accessibilityIdentifier() == identifier
    }
    try button.tap()
}

@MainActor
@Suite struct MeetingsBookChanges {
    @Test func listsTakeEachKindOfChange() async throws {
        let (book, _) = try await opened()

        book.adopt([.transcript(place: .all, rows: [line("one"), line("three")])])
        book.adopt([.transcript(place: .before(index: 1), rows: [line("two")])])
        book.adopt([.transcript(place: .end, rows: [line("four")])])
        book.adopt([.transcript(place: .at(index: 0), rows: [line("uno")])])

        #expect(book.transcript.map(\.text) == ["uno", "two", "three", "four"])
    }

    @Test func aRowPastTheEndIsIgnored() async throws {
        let (book, _) = try await opened()
        book.adopt([.meetings(place: .all, rows: [row("One")])])

        book.adopt([.meetings(place: .at(index: 4), rows: [row("Two")])])

        #expect(book.meetings.map(\.title) == ["One"])
    }

    @Test func rustPicksTheTabAndNoOtherTabIsMadeUp() async throws {
        let (book, _) = try await opened()

        book.adopt([.tab(index: 2)])
        #expect(book.tab == .transcript)

        book.adopt([.tab(index: 9)])
        #expect(book.tab == .transcript)
    }

    @Test func aJumpGoesToTheTranscriptAndTheSameLineCanBeAskedForTwice() async throws {
        let (book, _) = try await opened()

        book.adopt([.jump(row: 3)])
        let first = book.jump
        book.tab = .summary
        book.show(row: 3)

        #expect(book.tab == .transcript)
        #expect(book.jump?.row == 3)
        #expect(book.jump != first)
    }

    @Test func pickingARowSaysWhereTheSearchFoundIt() async throws {
        let (book, core) = try await opened(selected: "")
        book.adopt([.meetings(place: .all, rows: [row("Launch", hitRow: 7)])])

        book.selection = "id-Launch"

        #expect(core.calls == ["select id-Launch 7"])
        #expect(book.hasSelection)
    }

    @Test func pickingTheSameRowOrNothingAsksNothing() async throws {
        let (book, core) = try await opened()

        book.selection = "id-Launch"
        book.selection = nil

        #expect(core.calls.isEmpty)
        #expect(book.selection == "id-Launch")
    }

    @Test func typingCallsOncePerRealChange() async throws {
        let (book, core) = try await opened([.title(text: "Launch"), .notes(text: "Old")])

        book.editTitle("Launch")
        book.editTitle("Launch plan")
        book.editNotes("Old")
        book.editNotes("New")

        #expect(core.calls == ["title Launch plan", "notes New"])
    }

    @Test func aSearchGoesToRustWhichDecidesWhenItRuns() async throws {
        let (book, core) = try await opened()

        book.query = "bud"
        book.query = "bud"
        book.query = "budget"

        #expect(core.calls == ["search bud", "search budget"])
    }

    @Test func aNameIsSentAndKeptWhileRustSavesIt() async throws {
        let speaker = SpeakerRow(id: 2, label: "Speaker 2", name: "")
        let (book, core) = try await opened([.speakers(rows: [speaker])])

        book.nameSpeaker(speaker, "Giulia")
        book.nameSpeaker(speaker, "Giulia")

        #expect(book.speakers[0].name == "Giulia")
        #expect(core.calls == ["name 2 Giulia"])
    }

    @Test func sendingAsksAndClearsTheDraft() async throws {
        let (book, core) = try await opened()
        book.draft = "Who sends it?"

        book.send()

        #expect(book.draft == "")
        #expect(core.calls == ["ask Who sends it?"])
    }

    @Test func aRefusedCallDoesNoHarm() async throws {
        let (book, core) = try await opened()

        book.ask { _ in throw ShellError.Disk(reason: "disk full") }

        #expect(core.calls.isEmpty)
    }
}

@MainActor
@Suite struct MeetingsWindowViews {
    @Test func theSidebarShowsTitleWhenAndWhySearchFoundIt() async throws {
        let (book, _) = try await opened()
        book.adopt([
            .meetings(place: .all, rows: [row("Launch", snippet: "…the budget…"), row("Other")])
        ])

        let shown = try texts(MeetingsSidebar(book: book))

        #expect(shown.contains("Launch"))
        #expect(shown.contains("…the budget…"))
        #expect(shown.contains("31 Oct 2026, 10:00 · 2 min"))
        let rows = try MeetingsSidebar(book: book).inspect().findAll(MeetingListRow.self)
        #expect(try rows.map { try $0.accessibilityIdentifier() } == ["Launch", "Other"])
    }

    @Test func aRowMarksWhatIsGoingOnWithIt() throws {
        let plain = try MeetingListRow(meeting: row("Plain")).inspect().findAll(ViewType.Image.self)
        #expect(plain.isEmpty)

        let marks = try MeetingListRow(
            meeting: row("All", recording: true, busy: true, failed: true)
        ).inspect().findAll(ViewType.Image.self)
        #expect(try marks.map { try $0.actualImage().name() } == [
            "record.circle", "gearshape", "exclamationmark.triangle",
        ])
    }

    @Test func anEmptyListSaysHowToBegin() async throws {
        let (book, _) = try await opened(selected: "")
        book.adopt([.listHint(text: "No meetings yet. Start one here.")])

        #expect(try texts(MeetingsSidebar(book: book)) == ["No meetings yet. Start one here."])
    }

    @Test func withNothingPickedTheWindowSaysSoAndWithOneItShowsTheTab() async throws {
        let (book, _) = try await opened(selected: "")
        book.adopt([.detailHint(text: "Pick a meeting.")])
        let view = MeetingsView(book: book)

        #expect(try texts(view).contains("Pick a meeting."))
        #expect(throws: (any Error).self) { try view.inspect().find(MeetingHeader.self) }

        book.adopt([.selectedId(id: "id-Launch")])
        _ = try MeetingsView(book: book).inspect().find(MeetingHeader.self)
    }

    @Test(arguments: [
        (MeetingTab.summary, "SummaryTab"), (.notes, "NotesTab"), (.transcript, "TranscriptTab"),
        (.chat, "ChatTab"),
    ])
    func eachTabHasItsPanel(tab: MeetingTab, name: String) async throws {
        let (book, _) = try await opened()
        book.tab = tab
        let view = try MeetingsView(book: book).inspect()

        let shown = [
            (try? view.find(SummaryTab.self)) != nil ? "SummaryTab" : nil,
            (try? view.find(NotesTab.self)) != nil ? "NotesTab" : nil,
            (try? view.find(TranscriptTab.self)) != nil ? "TranscriptTab" : nil,
            (try? view.find(ChatTab.self)) != nil ? "ChatTab" : nil,
        ].compactMap { $0 }

        #expect(shown == [name])
    }

    @Test func theHeaderShowsTheStatusRowAndWhatWentWrong() async throws {
        let (book, _) = try await opened([
            .when(text: "31 Oct 2026, 10:00 · 2 min"),
            .stages(rows: [stage("Speakers", .failed), stage("Second pass off", .skipped)]),
            .failures(text: "Speakers: the model is damaged"),
            .notice(text: "Saved to /tmp/a.md"),
        ])

        let shown = try texts(MeetingHeader(book: book))

        #expect(shown.contains("31 Oct 2026, 10:00 · 2 min"))
        #expect(shown.contains("Speakers"))
        #expect(shown.contains("Second pass off"))
        #expect(shown.contains("Speakers: the model is damaged"))
        #expect(shown.contains("Saved to /tmp/a.md"))
    }

    @Test func eachStageDrawsHowItStands() throws {
        let symbols = try [StageMark.waiting, .done, .skipped, .failed].map { state in
            try StageLabel(stage: stage("One", state)).inspect().find(ViewType.Image.self)
                .actualImage().name()
        }

        #expect(
            symbols == ["circle", "checkmark.circle.fill", "minus.circle", "xmark.circle.fill"])
    }

    @Test func finishAndCancelShowOnlyWhenTheyMeanSomething() async throws {
        let (book, core) = try await opened()
        #expect(try identifiers(MeetingHeader(book: book)).isEmpty)

        book.adopt([
            .canFinish(can: true), .busy(busy: true), .progressText(text: "Writing the summary"),
            .progress(fraction: 0.5),
        ])

        let view = MeetingHeader(book: book)
        #expect(try identifiers(view) == ["Finish processing", "Cancel"])
        _ = try view.inspect().find(ViewType.ProgressView.self)
        try press("Finish processing", in: view)
        try press("Cancel", in: view)
        #expect(core.calls == ["finish", "cancel work"])
    }

    @Test func progressThatCannotBeMeasuredDrawsNoBar() async throws {
        let (book, _) = try await opened([
            .progressText(text: "Telling the speakers apart"), .progress(fraction: -1),
        ])

        #expect(throws: (any Error).self) {
            try MeetingHeader(book: book).inspect().find(ViewType.ProgressView.self)
        }
    }

    @Test func aTitleEditedGoesToRust() async throws {
        let (book, core) = try await opened([.title(text: "Launch")])
        let field = try MeetingHeader(book: book).inspect().find(ViewType.TextField.self)

        try field.setInput("Launch plan")

        #expect(core.calls == ["title Launch plan"])
    }

    @Test func aTimeWithALineJumpsAndOneWithoutStaysPut() async throws {
        let (book, _) = try await opened()
        let real = TimeChip(chip: chip("[00:12]", row: 3), book: book)
        let madeUp = TimeChip(chip: chip("[09:09]", row: -1), book: book)

        #expect(try texts(madeUp) == ["[09:09] ?"])
        #expect(try madeUp.inspect().find(ViewType.Button.self).isDisabled())
        #expect(try !real.inspect().find(ViewType.Button.self).isDisabled())
        #expect(
            try real.inspect().find(ViewType.Button.self).accessibilityIdentifier()
                == "Go to [00:12]")

        try real.inspect().find(ViewType.Button.self).tap()

        #expect(book.tab == .transcript)
        #expect(book.jump?.row == 3)
    }

    @Test func theSummaryShowsItsLinesAndAsksForNewOnes() async throws {
        let (book, core) = try await opened([
            .summary(lines: [
                SummaryLine(kind: .plain, text: "Done by Thursday.", time: "", row: -1, chip: nil),
                SummaryLine(kind: .heading, text: "Key points", time: "", row: -1, chip: nil),
                SummaryLine(
                    kind: .item, text: "The budget", time: "[01:15]", row: 2,
                    chip: chip("[01:15]", row: 2)),
            ]),
            .canSummarize(can: true),
        ])
        let view = SummaryTab(book: book)

        let shown = try texts(view)
        #expect(shown.contains("Done by Thursday."))
        #expect(shown.contains("Key points"))
        #expect(shown.contains("The budget"))
        #expect(shown.contains("[01:15]"))
        #expect(!(try identifiers(view).contains("Update summary")))

        try press("Regenerate", in: view)
        #expect(core.calls == ["regenerate"])
    }

    @Test func aStaleSummarySaysSoAndOffersToCatchUp() async throws {
        let (book, core) = try await opened([
            .summaryStale(stale: true), .canSummarize(can: true),
        ])
        let view = SummaryTab(book: book)

        #expect(try texts(view).contains("The transcript changed after this summary was written."))
        try press("Update summary", in: view)
        #expect(core.calls == ["update summary"])
    }

    @Test func aMeetingThatCannotBeSummarizedHoldsTheButtonsBack() async throws {
        let (book, _) = try await opened([.summaryStale(stale: true), .canSummarize(can: false)])

        let buttons = try SummaryTab(book: book).inspect().findAll(ViewType.Button.self)

        #expect(try buttons.allSatisfy { try $0.isDisabled() })
    }

    @Test func aPartialSummarySaysWhy() async throws {
        let (book, _) = try await opened([.summaryPartial(partial: true)])

        #expect(try texts(SummaryTab(book: book)).contains {
            $0.hasPrefix("This came straight from the notes")
        })
    }

    @Test func noSummaryYetSaysSo() async throws {
        let (book, _) = try await opened([.summaryEmpty(text: "No summary yet.")])

        #expect(try texts(SummaryTab(book: book)).contains("No summary yet."))
    }

    @Test func notesTypedGoToRust() async throws {
        let (book, core) = try await opened([.notes(text: "Old")])
        let notes = Binding(get: { book.notes }, set: { book.editNotes($0) })
        let editor = NotesEditor.Coordinator(text: notes)
        let view = NSTextView()
        view.string = "New"

        editor.textDidChange(Notification(name: NSText.didChangeNotification, object: view))

        #expect(core.calls == ["notes New"])
    }

    @Test func notesShowWhatRustHolds() async throws {
        let (book, _) = try await opened([.notes(text: "Call the supplier")])

        let editor = try NotesTab(book: book).inspect().find(NotesEditor.self).actualView()

        #expect(editor.text == "Call the supplier")
    }

    @Test func aTranscriptLineShowsItsTimeSpeakerAndWords() throws {
        #expect(try texts(TranscriptRow(line: line("Hello"), aimed: false)) == [
            "[00:05]", "You", "Hello",
        ])
    }

    @Test func aGapShowsAsAGapWithNoSpeaker() throws {
        #expect(try texts(TranscriptRow(line: line("Nothing heard", gap: true), aimed: false)) == [
            "[00:05]", "Nothing heard",
        ])
    }

    @Test func anEmptyTranscriptSaysWhyAndSpeakersCanBeNamed() async throws {
        let speaker = SpeakerRow(id: 1, label: "Speaker 1", name: "")
        let (book, core) = try await opened([
            .transcriptEmpty(text: "Nothing was transcribed."), .speakers(rows: [speaker]),
        ])
        let view = TranscriptTab(book: book)

        #expect(try texts(view).contains("Nothing was transcribed."))
        try view.inspect().find(ViewType.TextField.self).setInput("Giulia")
        #expect(core.calls == ["name 1 Giulia"])
    }

    @Test func noSpeakersShowsNoNames() async throws {
        let (book, _) = try await opened()

        #expect(throws: (any Error).self) {
            try TranscriptTab(book: book).inspect().find(SpeakerNames.self)
        }
    }

    @Test func theChatExplainsItselfThenShowsAnswersWithTheirTimes() async throws {
        let message = ChatMessage(
            question: "Who?",
            blocks: [ChatBlock(text: "Giulia.", chips: [chip("[00:12]", row: 1)])], note: "Stopped."
        )
        let (book, _) = try await opened([.chatHint(text: "Answers come only from this meeting.")])
        #expect(try texts(ChatTab(book: book)).contains("Answers come only from this meeting."))

        book.adopt([.chatHint(text: ""), .chat(place: .all, rows: [message])])

        let shown = try texts(ChatMessageView(message: message, book: book))
        #expect(shown == ["Who?", "Giulia.", "[00:12]", "Stopped."])
    }

    @Test func anAnswerOnItsWayShowsAndAFailureSaysWhy() async throws {
        let (book, _) = try await opened([
            .streamingQuestion(text: "Who?"), .streamingText(text: "Thinking…"),
            .chatError(text: "The chat needs a model."),
        ])

        let shown = try texts(ChatTab(book: book))

        #expect(shown.contains("Who?"))
        #expect(shown.contains("Thinking…"))
        #expect(shown.contains("The chat needs a model."))
    }

    @Test func sendWaitsForWordsAndUsesGlass() async throws {
        let (book, core) = try await opened()
        let input = ChatInput(book: book)
        let send = {
            try input.inspect().find(ViewType.Button.self) {
                try $0.accessibilityIdentifier() == "Send"
            }
        }

        #expect(try send().isDisabled())
        #expect(try send().buttonStyle() is GlassProminentButtonStyle)

        book.draft = "Who?"
        #expect(!(try send().isDisabled()))
        try send().tap()

        #expect(core.calls == ["ask Who?"])
    }

    @Test func returnSendsAndDoesNothingWhileAnAnswerRuns() async throws {
        let (book, core) = try await opened()
        book.draft = "Who?"
        let field = try ChatInput(book: book).inspect().find(ViewType.TextField.self)

        try field.callOnSubmit()
        #expect(core.calls == ["ask Who?"])

        book.adopt([.chatRunning(running: true)])
        book.draft = "And?"
        try ChatInput(book: book).inspect().find(ViewType.TextField.self).callOnSubmit()
        #expect(core.calls == ["ask Who?"])
        #expect(book.draft == "And?")
    }

    @Test func whileAnAnswerRunsStopTakesSendsPlaceAndSuggestionsGo() async throws {
        let (book, core) = try await opened([
            .suggestions(questions: ["Draft a follow-up email"])
        ])
        let view = ChatInput(book: book)
        #expect(try identifiers(view) == ["Draft a follow-up email", "Send"])

        try press("Draft a follow-up email", in: view)
        #expect(core.calls == ["ask Draft a follow-up email"])

        book.adopt([.chatRunning(running: true)])
        #expect(try identifiers(view) == ["Stop"])
        try press("Stop", in: view)
        #expect(core.calls.last == "stop chat")
    }

    /// The dialog's actions, as a dialog itself would take the focus in a real run.
    @Test func deleteAsksFirstAndOnlyTheDestructiveAnswerDeletes() async throws {
        let (book, core) = try await opened()
        let closed = DeleteMeetingButton(book: book)
        #expect((try? closed.body.inspect().confirmationDialog()) == nil)
        try closed.body.inspect().find(ViewType.Button.self).tap()
        #expect(core.calls.isEmpty)

        let asking = DeleteMeetingButton(book: book, confirming: true)
        let dialog = try asking.body.inspect().confirmationDialog()

        try dialog.actions().find(button: "Keep it").tap()
        #expect(core.calls.isEmpty)
        try dialog.actions().find(button: "Delete for good").tap()
        #expect(core.calls == ["delete meeting"])
    }
}

@MainActor
@Suite struct MeetingsToolbarControls {
    @Test func theToolbarHoldsTheFourTabs() async throws {
        let (book, _) = try await opened()
        let view = try MeetingsView(book: book).inspect()

        for tab in MeetingTab.allCases {
            #expect(
                try view.find(viewWithAccessibilityIdentifier: tab.title).text().string()
                    == tab.title)
        }
    }

    @Test func theTabsWaitForAMeeting() async throws {
        let (book, _) = try await opened(selected: "")
        let picker = try MeetingsView(book: book).inspect().find(ViewType.Picker.self)

        #expect(picker.isDisabled())
    }

    @Test func theStartButtonSaysWhatRustSaysAndToggles() async throws {
        let (book, core) = try await opened([
            .recording(recording: true), .startLabel(text: "Stop meeting notes"),
        ])
        let button = try MeetingsView(book: book).inspect().find(
            viewWithAccessibilityIdentifier: "Stop meeting notes")

        try button.button().tap()

        #expect(core.calls == ["toggle"])
    }

    /// When each shows is Rust's, and the toolbar's `if`s are held by `ax-check`, as no inspection
    /// reaches inside them.
    @Test func theActionsReachRust() async throws {
        let (book, core) = try await opened()

        try KeepRecordingButton(book: book).inspect().find(ViewType.Button.self).tap()
        try ExportMeetingButton(book: book).inspect().find(ViewType.Button.self).tap()

        #expect(core.calls == ["resume", "export meeting"])
    }
}

@MainActor
@Suite struct MeetingsWindowLife {
    @Test func theBookComesAndGoesWithItsWindow() async throws {
        let core = FakeCore(settings: try await defaultSettings())
        let record = StageRecord()
        let windows = Windows(
            core: core, editor: try SettingsEditor(core: core), pages: SettingsPages(core: core),
            walk: SetupWalk(core: core), stage: record.stage)
        #expect(windows.book == nil)

        windows.open(.meetings)
        weak let book = windows.book
        #expect(book != nil)

        windows.close(.meetings)

        #expect(windows.book == nil)
        #expect(await eventually { book == nil })
        #expect(core.closed == [.meetings])
    }
}
