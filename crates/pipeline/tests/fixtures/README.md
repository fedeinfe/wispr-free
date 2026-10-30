# Audio fixtures

16 kHz mono 16-bit WAV, committed so the Silero tests run on real audio.

| File | Source | License |
|---|---|---|
| `speech_en.wav` | LibriVox, *Kriloff's Fables*, [`kriloffsfables_02_kriloff_64kb.mp3`](https://archive.org/details/kriloffsfables_kd_librivox), 6.95–11.2 s | CC0 |
| `speech_mid_phrase_en.wav` | The same recording, 20–32 s. Starts mid-word: Parakeet returned nothing for this clip until `blank_penalty` was set (ticket 07) | CC0 |
| `speech_it.wav` | LibriVox, *Il giornalino di Gian Burrasca*, [`gianburrasca_002_vamba_64kb.mp3`](https://archive.org/details/il_giornalino_di_gian_burrasca_0910_librivox1), 8.6–12.75 s | Public domain |
| `room_tone.wav` | Synthesized (below): pink noise plus 50 Hz hum, peaking at −46 dBFS | Part of this repo |

```sh
ffmpeg -i source.mp3 -ss START -to END -ar 16000 -ac 1 -c:a pcm_s16le -map_metadata -1 -bitexact speech.wav

ffmpeg -f lavfi -i "anoisesrc=color=pink:amplitude=0.004:seed=7:duration=3:sample_rate=16000" \
       -f lavfi -i "sine=frequency=50:duration=3:sample_rate=16000" \
       -filter_complex "[1:a]volume=0.02[h];[0:a][h]amix=inputs=2:normalize=0" \
       -ar 16000 -ac 1 -c:a pcm_s16le -map_metadata -1 -bitexact room_tone.wav
```

macOS `say` is deliberately not used: Apple's license limits system voices to personal, non-commercial use.
