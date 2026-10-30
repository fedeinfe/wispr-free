# 20 Make the last inert settings work, and give the app an icon
Status: ready-for-agent
Blocked by: 19

Added during implementation: tickets 12 and 19 left two settings saved and shown in Settings with no effect. Spec §3 and §5 ask for both.

- `media.while_dictating`: mute system output or pause media while dictating, and put it back afterwards, also when a dictation is cancelled or fails and when the app quits mid-dictation.
- `style.auto_add_corrections`: when the user corrects a word just after a dictation, add it to the dictionary.
- The bundle from ticket 13 has no app icon. Make an original one.

macOS only for now.

Done when: both settings change what the app does on macOS, each off by its setting, and `Wispr Free.app` shows an icon.
