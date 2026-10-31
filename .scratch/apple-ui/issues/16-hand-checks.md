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
- [ ] Closing the last window gives the focus back to the app you were in
- [ ] `--by-hand` run: real keys, `permissions::ask`, reopening the app, a real microphone trial, and a real grant through `WindowStage.front`
- [ ] Finish on the dev bundle closes onboarding for good
- [ ] Downloads over the network, and `NSOpenPanel` over onboarding
- [ ] The teal accent shows on a key window
- [ ] Dev app dictates end to end, and its first run works

Real bundle, only after the above:
- [ ] Quit, reinstall, dictate, no permission prompt
- [ ] A model in a chosen protected folder loads from the helper

Then keep the last Slint build until the first full day of use, and merge `apple-ui`.

## Comments

### 2026-10-31, the agent

On the user's order the rewrite was installed and merged to `main` before these checks, so "merge `apple-ui`" above is done and the real-bundle checks run on the installed app. The last Slint app is kept as `/Volumes/Extreme SSD/Applications/Wispr Free Slint.zip`, and `v1` keeps its source.
