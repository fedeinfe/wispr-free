# 15 Meeting capture (mic + system audio)
Status: ready-for-agent
Blocked by: 02

System audio per spec §4/§7 (mac process tap with an SCK fallback, WASAPI loopback, PipeWire monitor). Two-channel incremental Opus/Ogg writer, crash-safe. Mic-in-use detection for meeting-app suggestions (opt-in card).

Done when: a 10 min Zoom/Meet call records both sides on macOS and Windows, and killing the app mid-call keeps all but the last few seconds.
