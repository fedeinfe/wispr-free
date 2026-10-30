# Glossary

**Dictation**: one capture → transcribe → clean → insert cycle, started by a shortcut or a pill click.

**Pill**: the floating always-on-top indicator showing dictation state and the waveform. Docks to the bottom, left or right screen edge.

**Push-to-talk (PTT)**: dictation that lasts while the shortcut is held; release inserts the text.

**Hands-free (locked)**: dictation that continues after the keys are released, entered by double-tapping PTT or with the hands-free shortcut. It ends on the next press, Stop, or Cancel. _Avoid:_ "popo", "toggle mode".

**Cancel**: abort the current dictation and discard its audio. _Avoid:_ dismiss.

**Insertion**: putting the final text into the focused app (clipboard swap + paste keystroke + restore).

**Context**: focused app identity, window title and the text around the cursor, read through the accessibility APIs.

**Rule cleanup**: the deterministic text pass (fillers, stutters, explicit edit commands). Always on.

**LLM cleanup**: the optional local-model rewrite that applies self-corrections, punctuation, formatting and style.

**Self-correction**: a spoken revision ("red, no, blue") that the output resolves to its final intent.

**Cleanup strength**: None / Light / Medium / High. How far LLM cleanup may depart from the spoken words.

**Style**: tone and punctuation preset per app category (Formal, Casual, Very casual, Excited).

**Snippet**: a spoken trigger phrase that expands to stored text.

**Dictionary**: user words that bias ASR and fix spelling, plus replacements (wrong → right).

**Pipeline tier**: Lite / Balanced / High / Custom. A bundle of stage toggles and model picks matched to the hardware.

**Engine host**: the child process, the app's own binary, that holds one loaded engine. Killing it is how a model unloads (ADR-0005).

**Model catalog**: the versioned manifest of downloadable models with size, checksum, license and RAM needs.

**Models folder**: where downloaded models live: the app data folder, or a folder the user chose, possibly on an external drive.

**Meeting**: one notetaker recording with two channels: "You" (mic) and "Others" (system audio).

**Refine pass**: the post-meeting re-transcription with a larger model plus diarization.

**Live transcript**: what's transcribed during the recording, one line per VAD chunk per channel. It never holds up the recording, and the refine pass replaces it.

**Gap**: a stretch of a meeting with no live text, marked with why (ASR behind, ASR failed, app stopped). The refine pass fills gaps.

**Speaker**: a voice in the Others channel, numbered by diarization ("Speaker 2"). The number is stable within a meeting. Names are stored apart, so a rename survives refine and regeneration.

**Meeting record**: a meeting's folder: its audio, live log, refined transcript, summary, notes and how each stage went.

**Summary**: the AI-generated meeting notes (TL;DR, key points, decisions, action items). Distinct from the user's own **Notes**.

**Meeting chat**: Q&A grounded in a meeting's transcript, notes and summary, citing timestamps.

**Citation**: a `[mm:ss]` in a chat answer. It's checked against the transcript whenever it's shown: one that matches a line links to it, any other is shown as unverified and leads nowhere.
