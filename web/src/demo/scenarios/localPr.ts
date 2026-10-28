// Chapters 4 and 5: draft a Local PR for issue #42 (docs, OpenCode). `launchpad pr create` calls the `onLocalPr`
// hooks, which the GitHub area uses to add the Local PR (and an agent-written suggestion) to the world.

import { at, commit, edit, final, localPr, onLocalPr, plan, scenario, think, tool } from "../player";
import { finishIssue42 } from "../world/github";

// The GitHub area adds the Local PR and a review suggestion on docs PR #44.
onLocalPr((r) => { if (r.issue === 42) finishIssue42(r.sessionId); });

const webhooksDiff = `@@ -40,3 +40,24 @@ Every delivery carries an \`Acme-Signature\` header; verify it before trusting the body.

 See [Verifying signatures](./signatures.md) for examples in four languages.
+
+## Retries
+
+A delivery fails when your endpoint does not answer with a \`2xx\` status within 10 seconds.
+Failed deliveries are retried with exponential backoff:
+
+| Attempt | Delay after the previous attempt |
+| ------- | -------------------------------- |
+| 2       | 1 minute                         |
+| 3       | 5 minutes                        |
+| 4       | 30 minutes                       |
+| 5       | 2 hours                          |
+| 6       | 12 hours                         |
+
+After the sixth attempt the event is marked **failed** and appears in the dashboard,
+where you can send it again. Every retry has the same \`Acme-Delivery\` ID, so use it
+to ignore duplicates.
+
+Endpoints that fail every delivery for 3 days are disabled; we email the account owner first.
`;

const sidebarDiff = `@@ -12,6 +12,7 @@
       "items": [
         "webhooks/overview",
         "webhooks/events",
+        "webhooks/overview#retries",
         "webhooks/signatures"
       ]
`;

export const localPrBody = `Closes #42.

Adds a **Retries** section to the webhooks guide: the timeout, the backoff schedule, what happens after the last attempt, and how to ignore duplicate deliveries with \`Acme-Delivery\`.

Checked with \`pnpm docs:check\` (links and anchors).`;

export default scenario({
  id: "local-pr",
  folder: "docs",
  harness: "opencode",
  title: "Draft a Local PR for issue #42",
  timeline: [
    at(0.4, think("Issue #42 asks for the webhook retry policy in the docs. I will read the issue, find the right page, write the section and open a Local PR for review.")),
    at(0.8, tool("bash", "gh issue view 42 --repo acme-labs/docs", { seconds: 1, output: "title:\tDocument the webhook retry policy\nstate:\tOPEN\nlabels:\tdocumentation, good first issue\n--\nCustomers ask how often we retry failed webhook deliveries and when we give up.\nThe policy is in billing-api/src/webhooks/retry.rs but not in the docs." })),
    at(0.8, plan([["Read the retry policy", "in_progress"], ["Write the Retries section", "pending"], ["Check links", "pending"], ["Open a Local PR", "pending"]])),
    at(0.8, tool("read", "docs/webhooks/overview.md", { seconds: 0.6, output: "# Webhooks\n…\n## Signatures\nEvery delivery carries an `Acme-Signature` header…" })),
    at(0.8, edit("docs/webhooks/overview.md", webhooksDiff)),
    at(0.6, edit("docs/sidebar.json", sidebarDiff)),
    at(0.8, tool("bash", "pnpm docs:check", { seconds: 2.2, output: "Checked 214 pages, 1,032 links, 388 anchors.\nNo broken links." })),
    at(0.5, commit("Document the webhook retry policy")),
    at(0.5, plan([["Read the retry policy", "completed"], ["Write the Retries section", "completed"], ["Check links", "completed"], ["Open a Local PR", "in_progress"]])),
    at(0.8, tool("bash", "launchpad pr create --title \"Document the webhook retry policy\" --body-file /tmp/pr-body.md", { seconds: 0.8, output: "Local PR created: Document the webhook retry policy\nIt stays on this computer until you press Publish." })),
    at(0, localPr({ title: "Document the webhook retry policy", body: localPrBody, issue: 42 })),
    at(0.5, plan([["Read the retry policy", "completed"], ["Write the Retries section", "completed"], ["Check links", "completed"], ["Open a Local PR", "completed"]])),
    final(`**A Local PR is ready for issue #42.** It adds a Retries section to the webhooks guide: the 10-second timeout, the backoff schedule (1 min to 12 h, six attempts), what happens after the last attempt and how to ignore duplicates.

- Files: \`docs/webhooks/overview.md\`, \`docs/sidebar.json\`
- Check: \`pnpm docs:check\`, no broken links
- Nothing is on GitHub yet: open the Local PR in Review and press **Publish** when it looks right.`),
  ],
});
