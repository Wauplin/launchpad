---
name: launchpad
description: Prepare local pre-draft pull requests and agent-written comment or review suggestions in Launchpad. Use in a Launchpad pre-draft worktree or when asked to prepare a local PR or suggested GitHub response.
---

# Local pull requests

Launchpad keeps exploratory PRs on this machine until the owner clicks Publish. Never push, open a GitHub PR, or publish agent-written text as part of this workflow.

In a Launchpad pre-draft worktree, commit changes on its existing `lp/` branch and keep the worktree. Run `launchpad pr show` to inspect its local record. Finish by running `launchpad pr update --title "Concrete change" --body-file /path/to/description.md`. Use the CLI path and `LAUNCHPAD_PROFILE` specified in the session prompt; `dev` connects to port 7318 and the default `prod` connects to 7317.

The description starts with the problem and resulting behavior. Add validation and material limitations. Keep paragraphs and list items on single lines. Do not claim checks that were not run.

`launchpad pr list` lists local records. Outside a pre-draft worktree, `launchpad pr create --title "Concrete change" --body-file /path/to/description.md` copies the current committed HEAD into a new dedicated worktree. Commit changes first. The JSON result gives the new `root`; switch to that folder for further changes and CLI updates. It requires a registered project with an origin remote. It never moves or removes the original checkout.

Review feedback arrives in the same agent session. Address each note, commit changes, and update the local description again. Leave publication to the owner.

## Suggested comments and reviews

Use the exact CLI path, profile and session id supplied in the Launchpad session prompt. Write the suggested text to a file, then save it with `launchpad suggest comment --repo owner/repo --number 123 --body-file /path/to/comment.md --session-id <session-id>`. This only saves a local draft; it does not call GitHub.

For a PR review, inspect a specific committed head and use `launchpad suggest review --repo owner/repo --number 123 --head <40-character-commit> --body-file /path/to/review.md --session-id <session-id>`. The default is a comment review; use `--event approve` or `--event request-changes` when appropriate. Use the commit you actually reviewed, never substitute a newer PR head without reviewing it. Inline suggestions are not supported yet; describe paths and lines in the review body.

For a local pre-draft, replace `--repo` and `--number` with `--predraft <id>`. Its card lets the owner add the text to local review notes. Publishing the pre-draft does not publish these suggestions. The owner can edit, post or discard GitHub drafts in the issue or PR conversation. Stop after saving the suggestion and report its id; leave posting to the owner.

## Platform shell syntax

Use the exact profile and CLI invocation in the session prompt. Unix prompts use `LAUNCHPAD_PROFILE=dev '/path/to/launchpad'`; Windows prompts use PowerShell syntax, `$env:LAUNCHPAD_PROFILE = 'dev'; & 'C:\path\launchpad.exe'`. Append the requested `pr` or `suggest` arguments to that invocation. The publication rules are identical on every platform: suggestions stay local until the owner clicks Post.
