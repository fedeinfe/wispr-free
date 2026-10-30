# 08 Rule cleanup
Status: ready-for-agent
Blocked by: 01

Deterministic pass: per-language filler lists, stutter/duplicate collapse, explicit edit commands (scratch that / delete that / never mind), simple slot self-corrections ("six, no, seven"). Pure function, property and golden tests. Target < 5 ms for 500 words.

Done when: the spec §8 'um so I I think…' case passes and the golden suite covers English and Italian.

## Comments

**2026-10-30 · implementer (branch `t08-rule-cleanup`)**

Done:
- `wf_pipeline::rules::rule_cleanup(text, language) -> String`: a pure function in `crates/pipeline/src/rules.rs`. Per-language lexicons (en, it; `None`/unknown uses all) cover fillers and set-off filler phrases ("you know", "diciamo"), 1–4-word repeat collapse with an idiom exception list ("had had", "piano piano"), and edit commands that delete the preceding clause. "scratch that" counts anywhere; "delete that", "never mind" and "lascia perdere" only before a pause, so "delete that file" survives. Simple slot self-corrections cover "at six, no, seven" → "at seven" and "Tuesday at five, no wait, Wednesday at five". Longer revisions are left to LLM cleanup.
- Ends with deterministic sentence casing plus a final period, so spec §8 "um so I I think we should uh ship it" → "So I think we should ship it." passes with the LLM off.
- Tests (test-first, one seam: the public function): a golden table of 54 en/it cases including negative cases, and properties over 5k generated messy transcripts (idempotent, only deletes words, no filler left, capitalized and finished sentence). A budget test checks 500 words < 5 ms; measured ~0.9 ms debug and ~0.08 ms release. No new dependencies; only `crates/pipeline` changed.
- Since this is a sub-agent run, the seam was chosen without asking: the single public function, with no tests against internals.

Needs human verification / follow-ups:
- The lexicons are a first cut. Check them against real Whisper/Parakeet output, especially Italian, and whether "tipo"/"diciamo" are removed too eagerly.
- Downstream styles (10) must strip the final period or lowercase for Casual/Very casual, since rule cleanup always emits sentence case.
