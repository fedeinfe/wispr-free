# ADR-0004: No network by default

Status: accepted · 2026-10-30

## Decision

The app makes no network requests except model downloads the user explicitly starts from the Models page, and an optional "check for updates" link. No telemetry, no crash upload, no accounts. Downloads use HTTPS and are verified against the SHA-256 in the bundled manifest.

## Consequences

Bug reports rely on a user-exported local log bundle.
