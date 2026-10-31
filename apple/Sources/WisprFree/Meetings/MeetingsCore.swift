import WfCore

/// What the Meetings window needs of Rust. Each call only leaves an order and returns: the disk
/// work runs in Rust, in the order asked, and what comes of it arrives as changes.
protocol MeetingsCore: AnyObject {
    func toggleMeeting() throws
    func selectMeeting(id: String, row: Int32) throws
    func searchMeetings(query: String) throws
    func editMeetingTitle(title: String) throws
    func editMeetingNotes(notes: String) throws
    func nameSpeaker(speaker: Int32, name: String) throws
    func exportMeeting() throws
    func deleteMeeting() throws
    func resumeMeeting() throws
    func finishMeeting() throws
    func cancelMeetingWork() throws
    func regenerateSummary() throws
    func updateSummary() throws
    func askMeeting(question: String) throws
    func stopMeetingChat() throws
}
