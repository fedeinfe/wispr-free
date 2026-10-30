# 03 Global key capture + dictation state machine
Status: ready-for-agent
Blocked by: 01

Platform key press/release streams (macOS CGEventTap + Fn via IOHID, Windows LL hook, Linux evdev). A pure, unit-tested state machine implementing every timing rule in spec §2: hold, quick tap discard, double-tap lock, triple-tap cancel, PTT+Space escalation, stray-key discard, paste-last on full release, 20 min cap.

Done when: the state machine has a table-driven test for every rule in §2 and the keys work on macOS.
