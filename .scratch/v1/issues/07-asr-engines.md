# 07 ASR engines
Status: ready-for-agent
Blocked by: 01, 06

`Asr` trait with a whisper.cpp implementation (Metal/CUDA/Vulkan features) and a sherpa-onnx implementation (Parakeet-class transducers). Dictionary and context biasing (initial prompt / hotwords), language auto or fixed. Streaming chunk mode for meetings and long dictations.

Done when: the same WAV fixtures transcribe through both engines from a CLI, with real-time factor reported.
