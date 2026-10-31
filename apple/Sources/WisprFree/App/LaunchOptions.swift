import Foundation

/// What the command line asks of a run. With none of it the app takes no keys and no clicks, so a
/// test that starts it can't race the real app or open the microphone.
struct LaunchOptions: Equatable {
    /// A person started this: listen to the keys and let the pill catch the mouse.
    var byHand = false
    /// Adds the status menu's Debug pill. Always on in debug builds.
    var debugPill = false
    /// Puts the pill in this state after starting.
    var pillState: String?
    /// Runs the flow on nothing real.
    var rehearse = false
    /// Presses the status menu items with these titles after starting, in order.
    var picks: [String] = []
    /// Windows come up behind everything else and nothing activates, so a run takes no focus.
    var behind = false
    /// Watches the setup as a person's run does, which opens onboarding at launch if it's needed.
    /// Only with `behind`, or the window would take the focus from whoever is at the Mac.
    var setup = false
    /// Takes the pill's pictures into this folder, then quits.
    var shots: String?
    /// Takes the windows' pictures into this folder, then quits.
    var windowShots: String?
    /// Stands in for the folder picker's answer, for a run with nobody to pick.
    var pickFolder: String?
    /// Uses every window and closes it this many times, then says so.
    var tour: Int?
    /// Moves through the first meeting, timing the main thread, then quits.
    var walkMeeting = false

    init(arguments: [String]) {
        #if DEBUG
            debugPill = true
        #endif
        for argument in arguments.dropFirst() {
            switch argument {
            case "--behind": behind = true
            case "--by-hand": byHand = true
            case "--debug-pill": debugPill = true
            case "--rehearse": rehearse = true
            case "--setup": setup = true
            case "--walk-meeting": walkMeeting = true
            default:
                pillState = Self.value(of: "--pill-state=", in: argument) ?? pillState
                picks += Self.value(of: "--pick=", in: argument).map { [$0] } ?? []
                shots = Self.value(of: "--shots=", in: argument) ?? shots
                windowShots = Self.value(of: "--window-shots=", in: argument) ?? windowShots
                pickFolder = Self.value(of: "--pick-folder=", in: argument) ?? pickFolder
                tour = Self.value(of: "--tour=", in: argument).flatMap(Int.init) ?? tour
            }
        }
    }

    /// Whether a run that no person started watches the setup.
    var watchesSetup: Bool { setup && behind }

    private static func value(of flag: String, in argument: String) -> String? {
        argument.hasPrefix(flag) ? String(argument.dropFirst(flag.count)) : nil
    }
}
