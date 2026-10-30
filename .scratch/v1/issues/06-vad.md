# 06 Silero VAD stage
Status: ready-for-agent
Blocked by: 02

Silero VAD behind the `Vad` trait. Trim leading/trailing silence, chunk long dictations at pauses, optional hands-free auto-stop. Toggleable.

Done when: silence-only recordings produce no ASR call and no text.
