---
name: launchpad
description: Prepare and update local pre-draft pull requests in Launchpad. Use when working in a Launchpad pre-draft worktree or asked to prepare a local PR.
---

# Local pull requests

Launchpad keeps exploratory PRs on this machine until the owner clicks Publish. Never push, open a GitHub PR, or publish agent-written text as part of this workflow.

In a Launchpad pre-draft worktree, commit changes on its existing `lp/` branch and keep the worktree. Run `launchpad pr show` to inspect its local record. Finish by running `launchpad pr update --title "Concrete change" --body-file /path/to/description.md`. Use the CLI path and `LAUNCHPAD_PROFILE` specified in the session prompt; `dev` connects to port 7318 and the default `prod` connects to 7317.

The description starts with the problem and resulting behavior. Add validation and material limitations. Keep paragraphs and list items on single lines. Do not claim checks that were not run.

`launchpad pr list` lists local records. Outside a pre-draft worktree, `launchpad pr create --title "Concrete change" --body-file /path/to/description.md` copies the current committed HEAD into a new dedicated worktree. Commit changes first. The JSON result gives the new `root`; switch to that folder for further changes and CLI updates. It requires a registered project with an origin remote. It never moves or removes the original checkout.

Review feedback arrives in the same agent session. Address each note, commit changes, and update the local description again. Leave publication to the owner.
