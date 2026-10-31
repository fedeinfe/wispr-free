# Which design skill should guide the native macOS UI

Answers ticket `issues/02-design-skills.md`. Researched 2026-10-31. Downloads were read in `/tmp/wf-skills/`; nothing was installed or run.

## Answer

Use **https://github.com/dickwu/apple-design-skill**, pinned to a commit and kept out of this public repo, as the design **reviewer and HIG reference**. Do not use its "Design improvement mode", and do not use Emil Kowalski's `apple-design`, which is for the web. Neither skill names most of the SwiftUI and AppKit APIs we need, so those come from Apple's developer docs.

## Candidates

The user gave names only, so the first two rows are my inference from popularity and match. Please confirm them.

- **`apple-design` = `emilkowalski/skills`.** It has about 52k stars. Lock files in owid/owid-grapher and shadcn-labs/snapcn record it as the source of the widely vendored copies with this name.
- **`apple-design-skill` = `dickwu/apple-design-skill`.** It is the exact repo name and has about 9 times the stars of the next match.
- **`naplesblue/apple-design-skill`** is the runner-up for that name. Its `motion.md` credits Kowalski's skill. Both main candidates declare `name: apple-design`, so they can't share a skills folder without a rename.

| | dickwu/apple-design-skill | emilkowalski/skills: apple-design | naplesblue/apple-design-skill |
| --- | --- | --- | --- |
| Author | Peilin Wu (`dickwu`), plus commits by Claude | Emil Kowalski | `naplesblue` |
| Licence | **None.** README assigns the guideline text to Apple and grants nothing | MIT | MIT, plus Lucide ISC notice |
| Stars / forks | 2,470 / 206 | 51,990 / 3,039 | 282 / 33 |
| Last push | 2026-10-28 (re-pulls Apple regularly) | Repo 2026-10-28; this skill 2026-09-15 | 2026-07-20 |
| Target | Native first; translates for Flutter, Tauri and others | Web: CSS, Pointer Events, Motion | Web: HTML/CSS tokens, iPhone mockups |
| SwiftUI / AppKit | Named in description; few API names | 0 mentions of either | 0 mentions of either |
| Windows, menu bar, panels | Apple's pages for each; desktop rules in SKILL.md | None | None |
| Sidebars, toolbars | Apple's pages for each | Passing mentions, web only | Passing mentions, web only |
| Materials, Liquid Glass | Apple's page plus a curated guide with SwiftUI and AppKit APIs | `backdrop-filter` imitation; 0 mentions of Liquid Glass | `backdrop-filter` imitation |
| SF Symbols | Apple's page | 0 mentions | Lucide SVG icons instead |
| Typography | Full macOS text style table, 26 pt to 10 pt | CSS tracking and leading only | Web type stack |
| Spacing | Principles only; 0 pt values in `layout.md` | Principles only | Web `clamp()` values |
| Motion | Apple's principles; no spring values | Rich, but web; values unverified (below) | Adapted from Kowalski |
| Accessibility | Sizes, contrast, Reduce Transparency, Increase Contrast | Web media queries | Web checklist |
| Facts vs Apple | All spot-checks matched | Principles matched; spring values not traceable | Not checked: wrong platform |
| Scripts / hooks | One Node script, not needed for use. No hooks, no MCP | None for this skill | None |
| Tokens at session start | ~250 | ~105 | ~180 |
| Tokens when used | ~7,100 body, ~23,000 always-load set, ~55,000 for a full macOS review | ~5,700 | ~2,300, up to ~16,000 with its files |

Token figures are bytes divided by 4, so they are estimates. Claude Code keeps only name and description in context until a skill is invoked. The "full review" is the body plus 12 reference files, the top of the skill's own 8 to 12 file limit.

Other repos, skimmed but not fully reviewed:

| Repo | Licence, stars | Note |
| --- | --- | --- |
| ehmo/platform-design-skills (`macos`) | MIT, 634 | Most SwiftUI and AppKit sample code, but last pushed in March 2026 and never mentions Liquid Glass. Overstates Apple: it wants a shortcut on every menu item and allows custom menus before View, while Apple places them between View and Window |
| RichardZhengQuan/swiftui-native-design-skill | Apache-2.0, 17 | Best native-structure rules, and four API names checked out at Apple. Written for Codex, little AppKit, one push ever (2026-06-11). Depends on companion skills we lack, and treats a hard-coded path in the author's home folder as required evidence if it exists |
| haider-nawaz/liquid-glass-skill | Manifest says MIT, no LICENSE file; 72 | SwiftUI glass APIs only; last pushed February 2026 |
| NutshellEngineering, XOVIET-GAME, tzzs, mr8lu, CarlosZiegler | Various, under 20 stars | HIG mirrors or digests, not compared in depth. Far less adoption than dickwu. mr8lu also runs a local MCP server against Apple's live data |
| chaos-xxl, bowen31337, dayviwong | Various, under 25 stars | Web-focused by their own descriptions, so not relevant here. bowen31337's skill is also `name: apple-design` |
| s1gmamale1, SudewaJay | Various, under 35 stars | Broad Apple design families. Not reviewed: they would need to displace dickwu on stars, freshness and sourcing first |

## Fact checks against Apple

Checked against the JSON behind Apple's HIG pages and API docs, not secondary sources.

**dickwu matches Apple on:**

- Sizes: macOS type 13 pt default and 10 pt minimum, plus the text style table; controls 28x28 pt default and 20x20 pt minimum.
- Contrast: 4.5:1 up to 17 pt; 3:1 at 18 pt or for bold text. Also the 35% dimming layer behind clear glass.
- Structure: at most two sidebar levels in general, nothing critical at the bottom of a sidebar or window, and every toolbar item also in the menu bar.
- Settings and appearance: a noncustomizable settings toolbar, and no app-specific appearance setting.
- API names: `NSGlassEffectView`, `NSGlassEffectContainerView`, `GlassEffectContainer` and `glassEffect(_:in:)` all exist from macOS 26. `UIDesignRequiresCompatibility` is ignored from macOS 27.

**Whole-mirror diff:** 7,169 of 7,515 text fragments across all 123 generated pages matched Apple directly. I read the remaining fragments that a second normalising pass could not place. They are link markup, image alt text, and older Apple wording. None addresses an agent.

**The mirror is already slightly stale.** Apple changed 4 pages on 2026-10-28, after dickwu's same-day pull: `designing-for-macos`, `gestures`, `pointing-devices` (touch on Mac) and `app-icons` (icon formats). Read those 4 live. This is upstream drift, not tampering: the skill's copies predate Apple's edit and lack the Mac touch guidance.

**Kowalski's skill:**

- Its eight principles match Apple's June 2026 page.
- Its table of "values Apple ships" (damping and response for picture-in-picture, rotation and drawers) is not in the *Designing Fluid Interfaces* transcript. That transcript gives only 100% and 80% damping, for Now Playing, and no response values. The deceleration constant is named there without a number. They may come from slides or code I couldn't see, so treat them as unverified.
- Its glass recipe is a CSS imitation, the opposite of adopting the system material.

## Recommendation

**Pick dickwu.** It is the only one of the two written for this platform, its facts hold up, it is fresh to within days, and it can cite Apple's page and heading for each finding. Kowalski's skill is the more trusted and better licensed, but it has no native content, so most of its advice would need translating and its materials section points the wrong way. If agents need motion ideas, read its principles once and don't install it.

Conditions for using dickwu:

1. **Reviewer, not art director.** Its improvement mode asks for a custom palette, display type and a signature element. Our target is a system app, so skip that mode and use system colours and text styles. Its craft lens does accept a quiet utility as a valid outcome.
2. **Do not commit it.** With no licence and Apple's text inside, it must not enter this public MIT repo. Neither `.claude/` nor its contents are ignored today, so the steps below add a local exclude. Keep quotes in specs short.
3. **Load by surface to control cost.** A full review is about 55k tokens. Name the files per surface in the spec. `panels.md` is listed only in the skill's full index and not in its routing table, so name it for the pill.
4. **Install per project.** The trigger text is broad, including any codebase or pull request audit, so it shouldn't load in unrelated repos.
5. **Fill the known gaps elsewhere.** APIs: 0 hits for `NSPanel`, `NSToolbar`, `NSWindow`, `NSStatusItem`, `ToolbarSpacer` or `.formStyle`, so use Apple's developer docs. Spacing: neither skill has a macOS scale, so use system layout defaults and the macOS kit in Apple's design resources. Motion: neither has verified native spring values, so use SwiftUI's system animations.

For the spec (ticket 06): ADR-0001 asks for our own palette and a warm settings window, which needs reconciling with a system-app look built on semantic colours.

## Safety

Safe to install as files, on the steps below. Neither main candidate has hooks, an MCP server or telemetry, and no repo contained injection text.

- **Coverage.** All 15 downloaded repos got a pattern scan for run, install, send and override phrasing, plus hidden Unicode. For dickwu I read `SKILL.md`, `AGENTS.md`, `GEMINI.md`, `liquid-glass.md`, `code-audit.md` and `cross-platform.md`, and diffed the generated pages. For Kowalski I read the whole skill.
- **Hidden Unicode.** dickwu's only hit is U+200B in `icons.md`, which is also in Apple's own JSON. Kowalski's `apple-design` has none; the repo's other hits are a code-fence escape and an emoji example.
- **dickwu's script.** `scripts/pull-hig.mjs` fetches only `developer.apple.com` and writes or prunes inside `references/`. The install leaves it out.
- **dickwu's `AGENTS.md`.** It tells agents to run that script and `npx skills add . --list` before finishing changes to that repo. This targets contributors, and `SKILL.md` itself runs nothing. The install leaves it out too.
- **Kowalski's skill.** An "Initial Response" block makes the agent reply with one canned line when invoked without a question. It is odd but harmless.
- **Other repos.** XOVIET-GAME's README pipes `bun.sh/install` into bash. s1gmamale1's `install.sh` symlinks skills into `~/.claude/skills` and `~/.agents/skills`. Neither is used here.
- **Not verified.** GitHub returned no stargazer data, so I can't say whether any star count is organic.

## Install

Run from the repo root. The tar command takes only `SKILL.md` and `references/`, so no script or agent config comes with it. I tested the download and extraction in `/tmp`: 128 files, 1.4 MB, identical to the reviewed copy. The exclude and `git status` lines were not tested.

```sh
SHA=ff4ebddf0488598723a82bfdce1afb04de0b1c44
mkdir -p .claude/skills/apple-design
curl -fsSL "https://github.com/dickwu/apple-design-skill/archive/$SHA.tar.gz" \
  | tar -xzf - --strip-components=1 -C .claude/skills/apple-design \
      "apple-design-skill-$SHA/SKILL.md" "apple-design-skill-$SHA/references"
echo '.claude/skills/apple-design/' >> .git/info/exclude
git status --short .claude   # must print nothing
```

Start a new session and use `/apple-design`. Worktrees don't get the folder, so repeat the `mkdir`, `curl` and `tar` lines there. To remove it, delete `.claude/skills/apple-design`. To update, review the new commit, then rerun with its SHA.

## Sources

- Skills: [dickwu](https://github.com/dickwu/apple-design-skill), [emilkowalski](https://github.com/emilkowalski/skills), [naplesblue](https://github.com/naplesblue/apple-design-skill), [ehmo](https://github.com/ehmo/platform-design-skills), [RichardZhengQuan](https://github.com/RichardZhengQuan/swiftui-native-design-skill), [haider-nawaz](https://github.com/haider-nawaz/liquid-glass-skill)
- Apple: [HIG](https://developer.apple.com/design/human-interface-guidelines), page data at `developer.apple.com/tutorials/data/design/human-interface-guidelines/<page>.json`, [NSGlassEffectView](https://developer.apple.com/documentation/appkit/nsglasseffectview), [UIDesignRequiresCompatibility](https://developer.apple.com/documentation/bundleresources/information-property-list/uidesignrequirescompatibility), [Design resources](https://developer.apple.com/design/resources/), [WWDC18 session 803](https://developer.apple.com/videos/play/wwdc2018/803/). Claude Code: [Skills docs](https://code.claude.com/docs/en/skills)
