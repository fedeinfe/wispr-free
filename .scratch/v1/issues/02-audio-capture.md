# 02 Audio capture + level meter
Status: ready-for-agent
Blocked by: 01

cpal input on the default or a selected device, resampled to 16 kHz mono f32, 40 ms frames, 300 ms pre-roll ring buffer. Device hot-plug and a ranked device list. Level meter: RMS dBFS → adaptive floor (min −60 dB, 20 dB range), averaged every 150 ms (spec §1, §3).

Done when: a CLI test records 5 s to WAV and prints levels; unplugging the device falls back cleanly.
