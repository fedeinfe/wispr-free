# ADR-0001: Clean-room reimplementation of Wispr Flow behaviour

Status: accepted · 2026-10-30

## Context

wispr-free aims for feature and UX parity with Wispr Flow's dictation, notetaker and settings, as a free, local, MIT-licensed app. We studied the shipped Wispr Flow 1.6.1297 app with REA, using static analysis only.

## Decision

- We reproduce **behaviour** (state machines, timings, defaults, UX flows) and write all code, prompts, copy, icons, sounds and illustrations ourselves.
- No Wispr code, prompts, strings, assets, API keys or endpoints enter this repository. The raw study notes stay private, outside the repo.
- Visual design: similar layout and feel (a small dark capsule pill). Windows use system colours and controls (ADR-0006). Our own icon, pill and sounds stay.
- The product name and branding must not suggest an affiliation with Wispr.

## Consequences

The spec (`.scratch/v1/spec.md`) is the source of truth. Contributors should not need, and should not consult, Wispr's binaries.
