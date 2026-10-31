# 14 Meetings
Status: ready-for-agent
Blocked by: 11

Spec: Meetings, Bridge, Phase 4, Memory. Behaviour: `research/surface.md`. Look: `prototype/Proto.swift`, `prototype/meetings-dark.png`.

`NavigationSplitView` 1000×680, list and `.searchable(placement: .sidebar)` with the 150 ms debounce, start button in the toolbar. Segmented Summary, Notes, Transcript, Chat in `.principal`, Export and the rest in `.primaryAction` with `ToolbarSpacer`. Title saves after 700 ms. Stage chips become a status row. Chat streaming, `TimeChips`, suggestions, Stop, and `.glassProminent` Send on the floating input. Library calls are `async`, in order, and never on the main actor. Run the Meetings design review.

Done when: the ported tests and `ax-check` pass, a real 30-minute fixture displays with no hitch and you say how you measured that, footprint after using every window is recorded against the 55 MB goal, and shots in both appearances are read by you. If it ends above 55 MB, report it. Don't build the separate-process fallback.

## Comments
