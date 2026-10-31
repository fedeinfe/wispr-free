# 04 What the UI needs from the core
Type: research
Status: resolved
Blocked by:

## Question

What does each window show and do today, and what is the exact surface between the UI and the Rust core?

## Answer

Five surfaces: pill and card, menu bar icon, Settings, Onboarding, Meetings. The logic is nearly toolkit-free already, the pill's model is pure, and 142 window tests need rewriting. Eight mismatches need a decision, and the spec settles them.

Detail: [research/surface.md](../research/surface.md).
