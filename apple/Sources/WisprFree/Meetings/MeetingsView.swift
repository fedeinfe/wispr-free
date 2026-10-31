import SwiftUI

/// The list and the meeting. Both are empty until the library comes across.
struct MeetingsView: View {
    static let shape = WindowShape(
        identifier: "Meetings", title: "Meetings", size: NSSize(width: 1000, height: 680),
        minimum: NSSize(width: 760, height: 480),
        style: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView])

    private static let tabs = ["Summary", "Notes", "Transcript", "Chat"]

    @State private var tab = tabs[0]

    var body: some View {
        NavigationSplitView {
            List {}
                .navigationSplitViewColumnWidth(260)
        } detail: {
            Color.clear
                .toolbar {
                    ToolbarItem(placement: .principal) {
                        Picker("Show", selection: $tab) {
                            ForEach(Self.tabs, id: \.self) { tab in
                                Text(tab).accessibilityIdentifier(tab)
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                    }
                }
        }
        .navigationTitle(Self.shape.title)
    }
}
