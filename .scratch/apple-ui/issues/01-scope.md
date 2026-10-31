# 01 What the rewrite covers
Type: grilling
Status: resolved
Blocked by:

## Question

Where does this map end, what wins between authentic Apple look and all-Rust, which platforms and macOS versions, which parts of the UI, what "Apple style" means, and whether features or language change.

## Answer

The user delegated this ("risponditi da solo fino ad avere la spec"), so these are the agent's calls:

- The map ends at a spec ready to implement, with a prototype. The rewrite comes after it.
- Authentic Apple look beats keeping everything in Rust: SwiftUI/AppKit over the Rust core.
- macOS 26 or later only.
- All five parts are rewritten: pill, menu bar icon, Settings, Onboarding, Meetings.
- "Apple style" means an Apple system app, such as Notes or System Settings. The pill alone keeps a character of its own.
- Look and interaction change, features don't. New ideas go on a separate list.
- The interface stays in English.
