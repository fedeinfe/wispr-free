# 15 Meeting capture (mic + system audio)
Status: ready-for-agent
Blocked by: 02

System audio per spec §4/§7 (mac process tap with an SCK fallback, WASAPI loopback, PipeWire monitor). Two-channel incremental Opus/Ogg writer, crash-safe. Mic-in-use detection for meeting-app suggestions (opt-in card).

Done when: a 10 min Zoom/Meet call records both sides on macOS and Windows, and killing the app mid-call keeps all but the last few seconds.

## Comments

**2026-10-30 · implementer (branch `t15-meeting-capture`)**

Done (`crates/meetings`, test-first apart from the OS backends):
- `MeetingWriter`: incremental Ogg Opus, L = You, R = Others, one synced page per second. Two independent mono Opus streams (RFC 7845 family 1), so there is no crosstalk. Never overwrites an existing file.
- `read_meeting` decodes a whole or crash-cut file. `repair_meeting` drops the torn tail and marks end of stream by writing a copy and renaming it.
- `StereoAligner`: pairs the two clocks and pads a stalled channel with silence once the other is 200 ms ahead.
- `MeetingRecorder`: mic + system through `wf_audio::Capture`, writer on its own thread, `MeetingEvent`s for ticket 16 and the pill. Keeps recording if system audio can't be opened.
- `SystemAudioBackend`: macOS process tap (14.4+) with a ScreenCaptureKit fallback, WASAPI loopback, and PulseAudio/PipeWire monitor. The last two go through cpal. The macOS tap doesn't, because cpal opens the *input* of any output device that has one (e.g. USB headsets).
- `mic_clients()` per OS plus the pure `MeetingDetector`: suggests once per mic session, ignores quick mic tests, is off by default, and has no way to start a recording.
- `wf_audio::open_stream` split out of `CpalBackend` for loopback. Behaviour of the mic path is unchanged.
- `examples/meeting.rs`: `record`, `info`, `repair`, `clients`, `tap-test`.

Tests sit at four seams, chosen by the implementer since no human was available to confirm them: the meeting file (write, read, repair), `StereoAligner`, `MeetingDetector` with a fake process list, and `MeetingRecorder` with fake backends.

Verified on macOS (Apple Silicon):
- Crash safety for real: cutting the file at every byte around each page boundary and at a spread of offsets keeps exactly the complete pages, sample-identical to the full decode. A flipped byte keeps everything before the damaged page. SIGKILL on a live writer process keeps all but the last second or so.
- ffmpeg, as an independent decoder, reads finished and repaired files as stereo with no warnings (`--ignored` tests). The `ogg` crate accepts the framing.
- `tap-test` on the real tap, 4 of 4 runs: its own `afplay` tone went through the tap, recorder and Opus file and read back at 1000 Hz at the expected level, using a process-scoped muted tap and no mic. That is the only real audio this ticket touched.
- `mic_clients()` against CoreAudio: real processes resolve to pids and bundle ids, and Slack, Chrome and WebKit classify correctly (with the mic-in-use filter temporarily removed, since nothing was using the mic).
- 90× real time encoding, 18.6 MB/hour.
- `cargo check` and `clippy -D warnings` pass for the Windows and Linux targets, with the C builds stubbed. That proves they type-check, nothing more.

The "Done when" is NOT met yet. Needs human verification:
- **A real Zoom/Meet call on macOS.** Run `meeting record call.opus 600` from a terminal with Microphone and System Audio Recording allowed, confirm both sides are audible and on the right channel with `info` and a player, then kill it mid-call and run `info` and `repair`. The everything-tap (`Scope::Everything`) has never run, because it would capture real audio: only the process-scoped tap has. Its permission behaviour may differ, and it's silent rather than an error if denied.
- **Windows.** Nothing has run there: loopback, `mic_clients()` and the 10 minute call. WASAPI sends nothing during silence, which the aligner should cover.
- **Linux.** Nothing has run there: the monitor source lookup and `pw-dump`. Moving cpal to PulseAudio also changes the mic host there.
- **ScreenCaptureKit fallback.** Compiled only, never run. It needs macOS 13–14.3, or forcing the fallback with Screen Recording allowed.
- **Default-output switch during a recording**, and audio across it. Only the id logic exists; the sample-rate change path has not been exercised.
- **Real meeting apps' mic behaviour.** Bundle ids and exe names in `KNOWN_APPS` come from knowledge of those apps. Only Slack, Chrome and WebKit were seen on this machine. `com.apple.avconferenced` as FaceTime's mic client is unconfirmed.

Not done, left to other tickets:
- **Not wired into the app.** No shortcut, tray item, pill Meeting state or "Take notes for this meeting?" card yet, since `crates/app` is being changed by tickets in progress. Whoever wires it should call `repair_meeting` on leftover files at launch.
- **Ticket 18:** the app must link with `-Wl,-weak_framework,CoreAudio`, or it won't launch below macOS 14.2 (cpal imports the tap API strongly), and needs `NSAudioCaptureUsageDescription`.
- **Ticket 13:** a denied System Audio Recording permission can't be detected from the stream.
- A browser is suggested for any web mic use, since the tab isn't visible from the process list. Resuming a recording (spec §4) and optional audio deletion aren't here. A disk-write failure (`MeetingEvent::Failed`) has no test.
