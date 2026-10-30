# 11 Model catalog, downloader, hardware tiers
Status: ready-for-agent
Blocked by: 01

Bundled versioned manifest (url, sha256, size, license, langs, ram). Resumable verified downloads with progress, delete, verify. Hardware detection (RAM, cores, SIMD, Metal/CUDA/Vulkan + VRAM) → recommended tier. Tier = preset of stage toggles and models (spec §6).

Done when: a fresh install on an 8 GB machine recommends Lite and downloads only what Lite needs.
