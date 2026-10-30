# 11 Model catalog, downloader, hardware tiers
Status: ready-for-agent
Blocked by: 01

Bundled versioned manifest (url, sha256, size, license, langs, ram). Resumable verified downloads with progress, delete, verify. Hardware detection (RAM, cores, SIMD, Metal/CUDA/Vulkan + VRAM) → recommended tier. Tier = preset of stage toggles and models (spec §6).

Done when: a fresh install on an 8 GB machine recommends Lite and downloads only what Lite needs.

## Comments

**2026-10-30 · implementer (branch `t11-models-hw-tiers`)**

Done (all in `wf-models`; API, manifest schema and tier rule are in the notes file `11-models-and-hw-tiers.md`):
- `catalog.toml`: versioned manifest bundled with `include_str!`, 14 files across 11 candidate models (Silero VAD, whisper.cpp, Parakeet int8, GGUF LLMs), URLs pinned to Hugging Face commits, every entry `candidate = true`. `Catalog::from_toml` validates schema, tier references and slot kinds, checksums, HTTPS (http only for loopback) and file names that can't leave the model folder.
- Tier presets: stage toggles plus models. `models_for_tier` is exactly what a tier downloads.
- `ModelStore`: resumable downloads (HTTP Range into `.part`), SHA-256 and size checks before rename, progress, cancel, status, verify, delete, `install_tier`. Handles servers that ignore Range and redirects; `https_only` applies on every redirect hop for https URLs.
- `hardware::detect()` and `HardwareInfo::recommended_tier()`. Hardware is a plain value, so the done-when is a unit test: `tests/fresh_install.rs` runs an 8 GB machine → Lite → only Lite's files requested from a local server.

Verified on macOS (Apple Silicon): `cargo build`, `cargo test`, `cargo clippy --all-targets -D warnings`, `cargo fmt --check`. All 14 catalog URLs answer at their pinned revisions; sizes match, Hugging Face confirmed 13 SHA-256 values and `tokens.txt` was hashed by hand. Only Silero was actually downloaded: the ignored test `live_download` passes real TLS, the HF CDN redirect, SHA-256 and a Range resume.

Needs human verification:
- `detect()` has only run on Apple Silicon. The Linux and Windows code (nvidia-smi, AMD sysfs, `reg query`) was typechecked for those targets, and its parsers are unit-tested against sample output written by hand, not captured from real machines. A failed probe drops the GPU, which at worst recommends a lower tier.
- Vulkan is assumed, not probed: an AMD GPU on Linux, or any adapter with a VRAM registry value on Windows, is reported as Vulkan-capable. Intel Macs report no GPU. No test for the https-to-http redirect refusal (needs a TLS test server); checked by reading ureq's source.
- No model other than Silero was loaded by an engine. Whether sherpa-onnx 2.0.1 loads Silero v5 and `llama-cpp-2` 0.1.160 loads Qwen3.5 is open (tickets 06, 09).
- `ram_mb` values are estimates until ticket 14. The tier picks are placeholders until ticket 14.
- Balanced and High set `diarization = true` but the catalog has no diarization models yet (ticket 16).
- Tier thresholds involve judgement calls on spec §6, listed in the notes: please review them.
