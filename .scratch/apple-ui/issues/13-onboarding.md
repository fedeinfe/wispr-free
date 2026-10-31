# 13 Onboarding
Status: ready-for-agent
Blocked by: 12

Spec: Onboarding, Phase 3. Logic and copy don't change: `research/surface.md`, `crates/app/src/onboarding` as moved to `wf-shell`.

Window 680×560, one step per page, bottom bar with Step n of 4, Back and Continue. `steps`, `start_trial`, `arm_trial`. Grants detected live at the same 1 s and 5 s rates. The first run shows all steps, later runs only broken ones. The Models step keeps the folder choice above the downloads, and closes Settings. Opening Settings during onboarding raises onboarding. Trial messages name the failed part. System Audio stays out. Run the Onboarding design review.

Done when: the ported tests and `ax-check` pass, a run with a reset dev data root shows the first-run path through to the trial step without any permission prompt being triggered by the check itself, and shots in both appearances are read by you. Don't request a grant from an automated run.

## Comments
