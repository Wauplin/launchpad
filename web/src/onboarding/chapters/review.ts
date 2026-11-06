// Chapter 4: Changes, line notes, Local PRs, Today's Review and Send filters and agent-written suggestions.

import type { Chapter } from "../tour/types";
import { api } from "@/lib/api";
import { world } from "@/demo/world";
import { clickButton } from "./helpers";
import { meta } from "./meta";

const ID = "demo-s-checkout";
const LOCAL_PR = "demo-pr-refunds";
let messages = 0;
const userMessages = () => (world.events[ID] ?? []).filter((e) => e.body.kind === "user_message").length;
const NOTES = "Review notes:\n\n- `tests/checkout.spec.ts` line 18: also wait for the payment request, not only the order request.";

export const review: Chapter = {
  ...meta("review"),
  learned: ["Changes shows the agent's diff, with viewed markers", "Line notes go back to the agent as one message", "Local PRs stay on this machine until Publish or Merge locally", "Today's Review, Send and GitHub filters collect everything that waits for your click"],
  setup: () => { messages = userMessages(); },
  steps: [
    {
      id: "review.changes", route: `/agents?session=${ID}&view=changes`, anchor: "session.changes",
      title: "Changes",
      body: "Changes shows what the agent changed in its worktree, file by file. Mark files as viewed as you go; the markers follow you to other devices.",
    },
    {
      id: "review.notes", anchor: "session.changes",
      title: "Line notes",
      body: "Click a line to add a note. Send notes collects them in one message that you read before it goes to the agent, which then addresses them.",
      tryIt: {
        text: "add a line note and send it, or let the tour send one.",
        done: () => userMessages() > messages,
        doIt: () => api.post(`/api/sessions/${ID}/input`, { text: NOTES }),
      },
    },
    {
      id: "review.localpr", route: `/predrafts/${LOCAL_PR}`, anchor: "predraft.actions",
      title: "Local PRs",
      body: "Agents can open a Local PR: a pull request that stays on this machine. Merge locally fast-forwards your branch without GitHub; Publish pushes the branch and creates the PR on GitHub.",
      tryIt: {
        text: "press Publish, then confirm. The demo shows what would happen; nothing is sent.",
        done: () => !!world.predrafts.find((p) => p.id === LOCAL_PR)?.published_url,
        doIt: async () => { await clickButton("Publish", "predraft.actions"); await clickButton("Push and create PR", "predraft.actions"); },
      },
    },
    {
      id: "review.today", route: "/?filter=review", anchor: "today.filters",
      title: "Review in Today",
      body: "Review lists finished agents you have not seen and Local PRs to publish. A row offers Merge only when it is safe: the agent is Done, its branch is on Home, nothing is uncommitted and the base did not move. Otherwise it says Review changes. A folder's Branches tab lists all its agent branches.",
    },
    {
      id: "review.suggestion", route: "/github/acme-labs/storefront/118", anchor: "suggestion.card",
      title: "Suggestions",
      body: "Agents can write a comment or a review on a GitHub PR. It stays here as a suggestion: edit it, then post it with a click, or discard it. Nothing reaches GitHub on its own.",
    },
    {
      id: "review.send", route: "/?filter=send", anchor: "today.filters",
      title: "Drafts to send",
      body: "Messages agents drafted for your Slack, Discord or webhook channels wait under Send; GitHub suggestions wait under GitHub. Edit a draft, then Send it, or Discard it. A send that did not complete is marked uncertain and never sent again. Done keeps what you sent for 30 days.",
    },
  ],
};
