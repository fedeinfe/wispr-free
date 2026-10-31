import AppKit
import WfCore

let launch = thisLaunch()
if case .exit(let text, let failed) = launch {
    if failed { log(text) } else { print(text) }
    exit(failed ? EXIT_FAILURE : EXIT_SUCCESS)
}
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = AppDelegate(launch: launch)
app.delegate = delegate
app.run()
