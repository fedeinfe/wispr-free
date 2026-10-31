# wispr-free

Free, open-source, **fully local** voice typing for macOS, Windows and Linux, written in Rust.

- **Dictate anywhere:** hold a shortcut, speak, release, and clean text lands in the focused app. Double-tap for hands-free mode.
- **Smart cleanup, on device:** removes fillers and repeated words and resolves self-corrections ("the red one… no, the blue one" → "the blue one"), with formatting and per-app style.
- **Meeting notes:** records you and the other side, gives a live transcript, a summary, action items, and chat about the meeting.
- **Runs on your hardware:** the app recommends models for your machine, and you can turn off heavy stages on older PCs and Macs.
- **Private by design:** no account, no telemetry, no cloud. The only network use is model downloads you start.

> Status: early development. Dictation and meeting notes run on macOS, with no signed release yet. Windows and Linux are not finished. See [`.scratch/v1/spec.md`](.scratch/v1/spec.md), the [ADRs](docs/adr/) and the [benchmarks](docs/benchmarks.md).

Not affiliated with Wispr or Wispr Flow.

## Run it on a Mac

```sh
cargo xtask signing-identity   # once, before granting permissions
cargo xtask bundle --install   # builds Wispr Free.app and copies it to ~/Applications
```

Open the app from `~/Applications`. The first run asks for the microphone, keyboard access and models.

To run it from the repo, build the engine host first: `cargo run` builds only the app, which starts its engines from the `wf-engine-host` beside it.

```sh
cargo build -p wf-engine-host
cargo run -p wispr-free
```

The Swift app is coming in beside it. Neither of these touches the installed app or its data.

```sh
cargo xtask bridge                    # makes what apple/ needs from Rust, then `swift test --package-path apple` works
cargo xtask bundle --dev              # builds target/Wispr Free Dev.app, with its own id and its own data folder
```

## License

MIT for this project's code.

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

The UI is built with [Slint](https://slint.dev), used under the [Slint Royalty-free License 2.0](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md); the badge above is the attribution that license requires.
