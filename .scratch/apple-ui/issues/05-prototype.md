# 05 Prove the stack with a real pill
Type: prototype
Status: resolved
Blocked by: 03

## Question

Does a Swift app calling a Rust library, with a non-activating glass pill and a settings window, build and run from the command line on this Mac?

## Answer

Yes, it builds and runs from the command line. The agent reviewed this prototype itself, as the user delegated.

What ran, in `/tmp/wf-stack`, on a copy of `ef09446`, with the sources saved in [prototype/stack](../prototype/stack):

- The real `Dictation` flow as a staticlib with no engines. The split took one Cargo feature and two `cfg` lines ([repo-changes.patch](../prototype/stack/repo-changes.patch)).
- UniFFI, an xcframework and SwiftPM produced an 8.4 MB binary, with a status item and a non-activating glass panel.
- A click sent through the panel fired the SwiftUI button, reached Rust and came back as an event. The app stayed inactive and the front app didn't change. A click from the window server is still a hand check, because the event posted to our own pid arrived without a window.
- The whole `Settings` value crossed the bridge as records and came back equal, with a derive and two custom types. No mirror structs are needed.

Footprint: 18.1 MB idle, 43.9 MB with Settings open, 41.0 MB after closing, 42.1 MB after 5 more cycles, so no leak.

Look, in fake-data windows ([Proto.swift](../prototype/Proto.swift)): `settings-dark.png`, `settings-light.png`, `meetings-dark.png` and `onboarding-dark.png` already read as system apps. They're captured inactive, so accent colours don't show.

The pill drops glass. Over a white page, white on the card or capsule background measured 2.6:1 to 2.7:1 for glass at 45% tint, 2.3:1 to 2.4:1 at 82% tint and 2.3:1 to 2.4:1 for an active HUD blur, against 16.7:1 for solid on every backdrop (`pills-*.png`, named by the skin that sits over white).
