# 03 Which toolkit and which bridge to Rust
Type: research
Status: resolved
Blocked by:

## Question

Can every window be built natively on macOS 26+ over the Rust core, including the non-activating pill, and which Swift/Rust bridge fits a CLI-only build with no Xcode project?

## Answer

Yes. SwiftUI views in AppKit-owned windows, a Swift executable, a Rust staticlib through UniFFI 0.32.2, SwiftPM driven by `cargo xtask bundle`, and a separate Rust helper for engines. Liquid Glass on the pill always draws inactive, and memory after the first window is the main cost.

Detail: [research/toolkit.md](../research/toolkit.md).
