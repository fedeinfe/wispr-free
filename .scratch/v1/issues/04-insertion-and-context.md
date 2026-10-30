# 04 Text insertion + context
Status: ready-for-agent
Blocked by: 01

Clipboard save → transient write → paste keystroke → restore after 500 ms, per OS, with a terminal Shift+Insert fallback. Context: focused app id, window title, ±500 chars around the cursor, selection (AX / UIA / AT-SPI). Fallback toast when nothing is focused.

Done when: text lands in TextEdit/Notes, Chrome, VS Code and Terminal on macOS, and the previous clipboard is restored.
