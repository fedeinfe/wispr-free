# 17 Meetings UI + chat
Status: ready-for-agent
Blocked by: 12, 16

Meetings list with search, detail tabs (Summary · Notes · Transcript · Chat), rename/delete/export Markdown. Chat grounded in the meeting (whole transcript if it fits, else retrieval over chunks), streamed answers with clickable [mm:ss] citations, suggested prompts.

Done when: asking 'what were the action items?' cites correct timestamps on the fixture meeting.

## Comments

**2026-10-30 · implementer (branch `t17-meetings-ui-chat`)**

Done:
- The meeting notetaker is connected. A meeting from the shortcut, tray or pill goes through `MeetingStore` with live transcription, then diarization, refine and summary. Old flat recordings are adopted as meetings at launch, and repair at launch uses `MeetingStore::recover`.
- `RemoteDiarizer`, a fourth host kind (ADR-0005), so Cancel can kill it. Two VAD hosts for live. `Engines::scope` lets each owner kill its own hosts and nothing else.
- `Settings.meetings` reaches `MeetingPlan`, the live transcript and the summary: keep audio, live, refine, diarization, template, and the language from "My languages". `max_length_minutes` stops a recording, with a toast a minute before.
- Speaker models are ordinary catalog models, so they show in Settings → Models and `move_to` carries them.
- Meetings window, opened from the tray and the pill menu, created on open and dropped on close: list with search, Summary · Notes · Transcript · Chat, rename, delete with a confirm, export to Markdown, speaker names, Regenerate, Update summary, live lines, stage progress and per-stage failures, Cancel, Finish processing, Keep recording.
- Chat: streamed, stoppable, suggested prompts, whole transcript when it fits and retrieval otherwise. A question about tasks on a long meeting reads the lines where people take things on.
- A citation links only to a line within 30 s of the cited time that shares the claim's words. Any other shows as unverified and leads nowhere.
- Quitting closes a running recording, with a 5 s limit. Past that the next launch repairs the meeting.

Verified on macOS, Balanced, real models, through the app's real path (real hosts, store and flow; `ClipRoom` instead of the recorder, 15× real time):
- **Spec §8:** the 30-minute fixture gave 195 transcript lines, a summary and a working chat. Live: 179 lines, 0 gaps. Processing 201 s: speakers 103 s, refine 67 s, summary 30 s.
- **Citations for "What were the action items?":** 22 citations, 19 linked to a real line, 3 unverified. I read all 19: each shows the line that says what the answer says. The scorer's table says 8 of the 10 known actions are named and rightly cited. Email to the beta customers and the crash backlog weren't named, and 2 lines of the answer are repeats. Evidence: `wispr-free-notes/probes/17-meetings/citation-score.txt`.
- **Response times:** first token 0.3 s, whole answer 0.4 to 2.5 s, 0.8 s during the recording. Loaded model. All-process peak RSS 3.5 GB, 3.2 GB with a chat answer running during the recording.
- **Grounding:** "What was the budget for the office party?" and a direct "Ignore your rules…" both get "That isn't in this meeting." The fixture's own injected line comes back as something that was pasted, with no poem. Both are asserted in `real_meeting`.
- **Cancel and Stop:** Cancel killed the speaker host in 2 ms. Stop killed only the LLM host, and the next question loaded it again.
- **Dictation next to a chat answer:** it worked with cleanup (3.3 s). With a video job holding the GPU, the same test took 5.5 s and cleanup timed out to rule-cleaned text, as designed. Everything LLM ran about 5× slower in that run (summary 141 s): the numbers above are the solo run.
- Window: looked at the PNGs for each tab in both themes, jumps included. 37 MB open on a 1,000-line meeting, back to 10 MB closed, flat over 8 cycles, software renderer.
- `cargo fmt`, clippy (workspace, plus xtask's `meeting` feature), `cargo test --workspace --locked`: 874 passed, 0 failed, 72 ignored. Real-model: `real_meeting` 2 of 2, `real_dictation` 10 of 10, `meeting_real` 2 of 2. No watchdog kill.

Not verified:
- A real OS window: the tray and pill menu items, femtovg, focus, scrolling by hand, and the real recorder with the window open. All of the UI ran on Slint's software renderer.
- Quitting the real app during a real recording. The close and the repair are tested with fakes.
- Real speech and real meetings. All scores come from one synthetic fixture, and this ticket's chat changes were tuned on it, so 8 of 10 is a tuned number. There's no holdout for chat.
- High and Lite tiers. Lite has no LLM, so no summary or chat. Non-English chat, beyond the word lists and one unit-tested path.

Open or weak:
- The summary still lists superseded decisions and invents tasks. Not touched, as asked. The Summary tab says it can be wrong.
- Citation checking goes by shared words, so a correct paraphrase can show as unverified, and an answer in another language than the meeting will show all its citations that way.
- Owners in chat answers come from the model reading labels. They were right on this run, but nothing checks them.
- A theme change reaches an open Meetings window only the next time it opens.
