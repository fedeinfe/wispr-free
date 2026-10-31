# 16 Hand checks and install
Status: ready-for-human
Blocked by: 15

Needs a person at the Mac: agents can't do these without taking the focus. Spec: Hand checks, Phase 5.

Dev bundle:
- [ ] Real click on the pill fires and doesn't steal focus from the app being typed in
- [ ] Right-click menu opens from the never-key panel, and the first click isn't swallowed
- [ ] Pill shows above full-screen apps and on all Spaces
- [ ] Clicks pass through when `hit_test` is false
- [ ] `NSApp.activate()` brings Settings forward from the menu bar and pill
- [ ] ⌘, ⌘W, ⌘C and ⌘V work in Settings' text fields
- [ ] Dev app dictates end to end, and its first run works

Real bundle, only after the above:
- [ ] Quit, reinstall, dictate, no permission prompt
- [ ] A model in a chosen protected folder loads from the helper

Then keep the last Slint build until the first full day of use, and merge `apple-ui`.

## Comments
