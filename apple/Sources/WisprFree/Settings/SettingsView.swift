import SwiftUI
import WfCore

/// The sidebar and the page. The window keeps to it: minimise and zoom are off, as in the HIG.
struct SettingsView: View {
    static let shape = WindowShape(
        identifier: "Settings", title: "Settings", size: NSSize(width: 860, height: 640),
        minimum: NSSize(width: 760, height: 480), zooms: false)

    @Bindable var navigation: SettingsNavigation
    let editor: SettingsEditor
    let pages: SettingsPages

    var body: some View {
        NavigationSplitView {
            List(navigation.rows, id: \.page, selection: $navigation.selection) { row in
                Label(row.title, systemImage: row.page.symbol)
                    .accessibilityIdentifier(row.title)
            }
            .toolbar(removing: .sidebarToggle)
            .navigationSplitViewColumnWidth(210)
        } detail: {
            SettingsPageView(page: navigation.page, editor: editor, pages: pages)
                .navigationTitle(navigation.title)
        }
    }
}

/// The page on the right.
struct SettingsPageView: View {
    let page: SettingsPage
    let editor: SettingsEditor
    let pages: SettingsPages

    var body: some View {
        switch page {
        case .general: GeneralPage(editor: editor, pages: pages)
        case .models: ModelsPage(editor: editor, pages: pages)
        case .pipeline: PipelinePage(editor: editor, pages: pages)
        case .style: StylePage(editor: editor, pages: pages)
        case .meetings: MeetingsPage(editor: editor, pages: pages)
        case .sounds: SoundsPage(editor: editor, pages: pages)
        case .privacy: PrivacyPage(editor: editor, pages: pages)
        case .about: AboutPage(editor: editor, pages: pages)
        }
    }
}

extension SettingsPage {
    fileprivate var symbol: String {
        switch self {
        case .general: "gearshape"
        case .models: "cpu"
        case .pipeline: "slider.horizontal.3"
        case .style: "character.book.closed"
        case .meetings: "person.2"
        case .sounds: "speaker.wave.2"
        case .privacy: "hand.raised"
        case .about: "info.circle"
        }
    }
}
