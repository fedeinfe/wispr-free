import Observation
import WfCore

/// Which page Settings is on. It starts on the last one, and Rust remembers each move.
@Observable
final class SettingsNavigation {
    let rows = settingsPages()
    private(set) var page: SettingsPage

    @ObservationIgnored private let core: any SettingsCore

    init(core: any SettingsCore) {
        self.core = core
        page = (try? core.settingsPage()) ?? .general
    }

    var title: String {
        rows.first { $0.page == page }?.title ?? ""
    }

    /// For the sidebar's list, where nothing selected isn't a place to be.
    var selection: SettingsPage? {
        get { page }
        set {
            guard let newValue, newValue != page else { return }
            page = newValue
            Task { [core] in
                do {
                    try await core.setSettingsPage(page: newValue)
                } catch {
                    log("keeping the page failed: \(error)")
                }
            }
        }
    }
}
