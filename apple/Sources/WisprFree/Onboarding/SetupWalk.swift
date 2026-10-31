import Observation
import WfCore

/// The page of the setup walk, as Rust last said. Rust keeps the walk, so this lives as long as the
/// app and only ever shows what it's told.
@Observable
final class SetupWalk {
    private(set) var page: SetupPage?
    /// The trial's level, 0 to 1.
    private(set) var level: Float = 0

    @ObservationIgnored let core: any OnboardingCore

    init(core: any OnboardingCore) {
        self.core = core
    }

    func adopt(_ page: SetupPage) {
        self.page = page
        if page.trial.phase != .listening { level = 0 }
    }

    /// Levels arrive for every dictation too, and only a trial shows them.
    func hear(level: Float) {
        if page?.trial.phase == .listening { self.level = level }
    }

    func ask(_ call: (any OnboardingCore) throws -> Void) {
        do {
            try call(core)
        } catch {
            log("the setup walk failed: \(error)")
        }
    }
}
