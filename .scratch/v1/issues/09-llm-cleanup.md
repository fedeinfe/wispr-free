# 09 LLM cleanup
Status: ready-for-agent
Blocked by: 08

llama.cpp behind the `Llm` trait. Our own system prompt implementing spec §3 rules + strength + style. Guards: no preamble, 0.5–1.3× length check, 2–3 s timeout → rule-cleaned fallback. Lazy load and unload after idle. Prompt-injection test (dictated 'ignore previous instructions…' stays as text).

Done when: an eval set (≥ 50 cases: self-corrections, lists, numbers, styles) scores ≥ 90% on Balanced, run via `cargo xtask eval`.
