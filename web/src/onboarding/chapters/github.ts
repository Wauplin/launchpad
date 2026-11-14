// Chapter 5: GitHub stays on github.com: folder links, suggestions waiting in Today and what reaches GitHub.

import type { Chapter } from "../tour/types";
import { useWorkspace } from "@/stores/workspace";
import { WS } from "@/demo/world/ids";
import { meta } from "./meta";

export const github: Chapter = {
  ...meta("github"),
  learned: ["Notifications, issues and pull requests open on github.com", "A folder page lists its Local PRs and links to its pull requests and issues on GitHub", "Agent-written comments and reviews wait in Today under GitHub", "Nothing reaches GitHub without your click"],
  setup: () => useWorkspace.getState().switchTo(WS.acme),
  steps: [
    {
      id: "github.folder", route: "/projects/demo-storefront", anchor: "folder.github",
      title: "GitHub stays on GitHub",
      body: "Launchpad does not copy your notifications, issues or pull requests: github.com (with the Chrome extension) shows them. A folder page lists its Local PRs, and these buttons open its pull requests and issues on GitHub in a new tab.",
    },
    {
      id: "github.suggestions", route: "/?filter=github", anchor: "today.filters",
      title: "Suggestions wait in Today",
      body: "Comments and reviews agents write for a GitHub pull request or issue wait here as suggestions. Edit one, then Post it with a click (a review is posted only if the pull request is still at the commit the agent reviewed), or discard it. Open on GitHub, in the ⋯ of a row, shows the item there.",
    },
  ],
};
