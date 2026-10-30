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

**Onboarding**: the first-run setup window: microphone, keyboard and paste, models, trial dictation. It shows once and comes back only for a step that broke.

**Trial dictation**: a real dictation that pastes nothing and shows its text in the onboarding window, to prove the shortcut, microphone and models. It names the part that failed.

**Engine host**: the child process, the app's own binary, that holds one loaded engine. Killing it is how a model unloads (ADR-0005).

**Model catalog**: the versioned manifest of downloadable models with size, checksum, license and RAM needs.

**Models folder**: where downloaded models live: the app data folder, or a folder the user chose, possibly on an external drive.

**Meeting**: one notetaker recording with two channels: "You" (mic) and "Others" (system audio).

**Refine pass**: the post-meeting re-transcription with a larger model plus diarization.

**Summary**: the AI-generated meeting notes (TL;DR, key points, decisions, action items). Distinct from the user's own **Notes**.

**Meeting chat**: Q&A grounded in a meeting's transcript, notes and summary, citing timestamps.
