# 02 Audio capture + level meter
Status: ready-for-agent
Blocked by: 01

cpal input on the default or a selected device, resampled to 16 kHz mono f32, 40 ms frames, 300 ms pre-roll ring buffer. Device hot-plug and a ranked device list. Level meter: RMS dBFS → adaptive floor (min −60 dB, 20 dB range), averaged every 150 ms (spec §1, §3).

Done when: a CLI test records 5 s to WAV and prints levels; unplugging the device falls back cleanly.

## Comments

**2026-10-30 · implementer (branch `t02-audio-capture`)**

Done (`crates/audio`, all test-first):
- `FrameConverter`: interleaved device audio at any rate/channel count → 16 kHz mono 640-sample frames (downmix + rubato `Fft`).
- `Recorder`: remembers the last 300 ms while idle; `start()` seeds the clip with it, `stop()` returns an `AudioClip`.
- `LevelMeter`: RMS dBFS over 150 ms windows → 0..1, floor = quietest window heard (never below −60 dB), 20 dB range, `reset()`. UI smoothing is left to the pill.
- `rank_devices`: user ranking (unplugged skipped), then system default, then the rest.
- `Capture`: thread keeping the best-ranked device open behind an `AudioBackend` trait (`CpalBackend` is the real one). Emits `Frame` and `Device(Some/None)` events, falls back on `device_lost` or when polling (default 2 s) finds the device gone, recovers when a device returns, switches up when a better-ranked device is plugged in, `set_ranking` applies immediately.
- `examples/record.rs`: `cargo run -p wf-audio --example record -- out.wav [seconds] [device-id...]`, plus `--list`. Prints levels and device changes, writes a 16 kHz mono WAV.
- CI: added `libasound2-dev` on Linux.

Tests sit at five seams, chosen by the implementer since no human was available to confirm them: `FrameConverter`, `Recorder`, `LevelMeter`, `rank_devices`, and `Capture` with a fake backend (hot-plug scenarios).

Verified on macOS (Apple Silicon): `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. The record example opened the real USB headset by its persisted id, ran 5 s without restarting across polls, and wrote a valid 16 kHz mono WAV (4.84 s, checked with `afinfo`). `--list` works.

The "Done when" is only partly met. Needs human verification:
- **Levels on real audio.** The agent shell only ever received digital silence (ffmpeg got −91 dB from the same device too, so likely mic permission or a muted headset). Run the example from a terminal with mic permission, speak, check the bars move and the WAV sounds right.
- **Real unplug.** Fallback is proven only against the fake backend, and the dev Mac has one input device. Unplug a mic during a recording with a second one attached, and with none left, and check `device:` lines. Specifically check cpal really reports the loss (or that the 2 s poll catches it).
- **Windows and Linux.** Nothing has run there, and CI has never run. On Linux check the cost of polling ALSA enumeration every 2 s and how noisy the device list is (pseudo-devices).
- **No OS AEC/AGC.** We request none (plain cpal input), but WASAPI drivers may still apply their own processing.

Open for ticket 03/app: pre-roll needs the stream open before key-down, so the OS mic indicator stays on while idle unless `Capture` is opened some other way. Not decided here.

If the consumer stalls for ~10 s the bounded event channel drops events, including a `Device` event. The doc on `Capture` says to read continuously.
