import Observation
import WfCore

/// What the pages show, as Rust last said, and the way to ask it for anything else. It lives as long
/// as the app, as a job running on the Models page outlives its window.
@Observable
final class SettingsPages {
    private(set) var state = blankPagesState()
    /// The microphone test's level, 0 to 1.
    private(set) var level: Float = 0
    let tiers = tierRows()

    @ObservationIgnored let core: any SettingsCore

    init(core: any SettingsCore) {
        self.core = core
    }

    func adopt(_ state: PagesState) {
        self.state = state
        if !state.microphoneTesting { level = 0 }
    }

    /// Levels arrive for every dictation too, and only the test shows them.
    func hear(level: Float) {
        if state.microphoneTesting { self.level = level }
    }

    /// A call that only hands Rust an order, whose outcome comes back as a new `state`.
    func ask(_ call: (any SettingsCore) throws -> Void) {
        do {
            try call(core)
        } catch {
            log("the settings pages failed: \(error)")
        }
    }

    func change(_ change: RuledChange) {
        ask { try $0.changeSetting(change: change) }
    }
}
