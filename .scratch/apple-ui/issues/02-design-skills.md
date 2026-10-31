# 02 Which design skill to use
Type: research
Status: resolved
Blocked by:

## Question

Of the two skills the user found, `apple-design` and `apple-design-skill`, which one should guide the design, if any, and is it safe to install?

## Answer

Use `dickwu/apple-design-skill`, pinned to `ff4ebddf`, as reviewer and HIG reference only. It's the one written for native macOS, and its facts matched Apple. `apple-design` is Emil Kowalski's web skill, so it isn't used. The skill has no licence and holds Apple's text, so it's installed locally in `.claude/skills/apple-design`, excluded from git, and never committed. Installed and checked on 2026-10-31.

Detail: [research/design-skills.md](../research/design-skills.md).
