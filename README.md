# wispr-free

Free, open-source, **fully local** voice typing for macOS, in Rust and SwiftUI.

- **Dictate anywhere:** hold a shortcut, speak, release, and clean text lands in the focused app. Double-tap for hands-free mode.
- **Smart cleanup, on device:** removes fillers and repeated words and resolves self-corrections ("the red one… no, the blue one" → "the blue one"), with formatting and per-app style.
- **Meeting notes:** records you and the other side, gives a live transcript, a summary, action items, and chat about the meeting.
- **Runs on your hardware:** the app recommends models for your machine, and you can turn off heavy stages on older PCs and Macs.
- **Private by design:** no account, no telemetry, no cloud. The only network use is model downloads you start.

> Status: early development. Dictation and meeting notes run on macOS 26 or later, with no signed release yet. Windows and Linux are parked. See [`.scratch/v1/spec.md`](.scratch/v1/spec.md), the [ADRs](docs/adr/) and the [benchmarks](docs/benchmarks.md).

Not affiliated with Wispr or Wispr Flow.

## Run it on a Mac

```sh
cargo xtask signing-identity   # once, before granting permissions
cargo xtask bundle --install   # builds Wispr Free.app and copies it to ~/Applications
```

Open the app from `~/Applications`. The first run asks for the microphone, keyboard access and models.

To work on it without touching the installed app or its data, use the dev bundle. It needs Xcode.

```sh
cargo xtask bridge                    # makes what apple/ needs from Rust, then `swift test --package-path apple` works
cargo xtask bundle --dev              # builds target/Wispr Free Dev.app, with its own id and its own data folder
```

Only the bundle `cargo xtask bundle` builds, started with no tool's flags, is the real app. A dev run takes no keys or microphone unless you start it with `--by-hand`.

## License

MIT.
