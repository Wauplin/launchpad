// Chapter 5: the GitHub inbox, row actions, PR pages and what to expect from GitHub.

import type { Chapter } from "../tour/types";
import { useComposer } from "@/stores/composer";
import { useWorkspace } from "@/stores/workspace";
import { WS } from "@/demo/world/ids";
import { anchor, waitFor } from "./helpers";
import { meta } from "./meta";

const reviewRequested = () => !!anchor("inbox.reasons")?.querySelector('button[aria-pressed="true"]')?.textContent?.startsWith("Review requested");

export const github: Chapter = {
  ...meta("github"),
  learned: ["The Inbox is your GitHub notifications, cached and refreshed every minute", "Reason chips narrow it; Done and the ⋯ of a row (Launch, Add to the draft) act on it", "PR pages show the conversation, changes and checks, and why Merge is not primary", "Nothing reaches GitHub without your click"],
  setup: () => useWorkspace.getState().switchTo(WS.acme),
  steps: [
    {
      id: "github.inbox", route: "/inbox", anchor: "inbox.row",
      title: "Your GitHub inbox",
      body: "These are your GitHub notifications. Launchpad polls GitHub every 60 seconds and keeps a local cache, so the list opens at once, also on a slow network.",
    },
    {
      id: "github.reasons", route: "/inbox", anchor: "inbox.reasons",
      title: "Why you were notified",
      body: "Chips narrow the list by reason. The sidebar counts unread review requests, mentions and assignments: the threads that wait for you.",
      tryIt: { text: "filter Review requested.", done: reviewRequested, doIt: () => [...(anchor("inbox.reasons")?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.startsWith("Review requested"))?.click() },
    },
    {
      id: "github.launch", route: "/inbox", anchor: "inbox.row",
      title: "Act on a thread",
      body: "Done archives the thread like on GitHub. The ⋯ of a row holds its other actions: Launch starts an agent with the PR or issue as context; Add to the draft adds it to the draft you are writing.",
      tryIt: {
        text: "open the ⋯ of a row and pick Launch to start a draft with that PR.",
        done: () => useComposer.getState().open && useComposer.getState().spec.github_refs.length > 0,
        doIt: async () => { const row = await waitFor(() => anchor("inbox.row")); [...(row?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.trim() === "Launch")?.click(); },
      },
    },
    {
      id: "github.pr", route: "/github/acme-labs/storefront/118", anchor: "item.tabs",
      title: "Pull request pages",
      body: "Conversation, Changes and Review threads, from the local cache. Replies, reviews and labels are posted only when you press their buttons.",
      enter: () => useComposer.getState().setMinimized(true),
    },
    {
      id: "github.merge", route: "/github/acme-labs/storefront/118", anchor: "item.merge",
      title: "Merge, only when GitHub agrees",
      body: "Merge is the primary button only when GitHub reports the PR mergeable and clean and no suggestion waits to be posted. Otherwise it is secondary and says what blocks it. Label filters apply to the 30 most recent cached items of a folder.",
    },
  ],
};
