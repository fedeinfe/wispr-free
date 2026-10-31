import Observation
import WfCore

/// The one way any window changes a setting: `edit { $0.x = y }`.
///
/// A change shows at once and saves in the order it was made. Compare and swap does the rest: when
/// something else saved first, the change is applied again to what that left, so neither is lost.
/// Rust's own changes arrive through `adopt`.
@Observable
final class SettingsEditor {
    typealias Change = (inout Settings) -> Void

    /// The settings as the windows should show them, with every change not yet saved in them.
    private(set) var settings: Settings
    /// Why the last change didn't reach the disk.
    private(set) var notice: String?

    @ObservationIgnored private let core: any SettingsCore
    @ObservationIgnored private var latest: Versioned
    @ObservationIgnored private var waiting: [Change] = []
    @ObservationIgnored private var saving = false

    init(core: any SettingsCore) throws {
        let latest = try core.settings()
        self.core = core
        self.latest = latest
        settings = latest.settings
    }

    func edit(_ change: @escaping Change) {
        waiting.append(change)
        show()
        guard !saving else { return }
        saving = true
        Task {
            while let next = waiting.first {
                await save(next)
                waiting.removeFirst()
                show()
            }
            saving = false
        }
    }

    /// Takes in what Rust says is current, unless a newer word got here first.
    func adopt(_ current: Versioned) {
        guard current.revision > latest.revision else { return }
        latest = current
        show()
    }

    private func save(_ change: Change) async {
        do {
            while true {
                var edited = latest.settings
                change(&edited)
                switch try await core.save(settings: edited, revision: latest.revision) {
                case .saved(let current):
                    notice = nil
                    return adopt(current)
                case .stale(let current):
                    adopt(current)
                }
            }
        } catch {
            notice = (error as? ShellError).map(errorText) ?? "\(error)"
            // A refused write still holds for this run.
            if let current = try? core.settings() { adopt(current) }
        }
    }

    private func show() {
        var shown = latest.settings
        for change in waiting { change(&shown) }
        settings = shown
    }
}
