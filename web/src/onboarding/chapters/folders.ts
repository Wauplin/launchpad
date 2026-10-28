// Chapter 8: folder pages, Files, workspaces and Add folder.

import type { Chapter } from "../tour/types";
import { clickAnchor } from "./helpers";
import { meta } from "./meta";

const findOpen = () => !!document.querySelector('input[placeholder="Find in file"]');

export const folders: Chapter = {
  ...meta("folders"),
  learned: ["A folder page shows its agents, GitHub items and worktrees", "Files browses any checkout or worktree, with Find", "Workspaces group folders; Everything shows them all", "Add folder registers, browses or clones folders"],
  steps: [
    {
      id: "folders.page", route: "/projects/demo-storefront", anchor: "folder.page",
      title: "A folder page",
      body: "Chips count running agents, waiting ones and Local PRs. Overview lists running agents, GitHub items, worktrees and every agent; Settings holds the worktree setup command and the When done default.",
    },
    {
      id: "folders.files", route: "/projects/demo-storefront/code", anchor: "code.find",
      title: "Files",
      body: "Files browses the checkout or any worktree, read-only, with syntax colors, search across files and Download. Agents' file links open here.",
      tryIt: { text: "open Find.", done: findOpen, doIt: () => clickAnchor("code.find") },
    },
    {
      id: "folders.workspaces", route: "/projects", anchor: "sidebar.workspace-menu",
      title: "Workspaces",
      body: "This menu creates, renames and deletes workspaces and opens one in a new window. Each window shows one workspace; Everything shows every folder.",
    },
    {
      id: "folders.add", route: "/projects", anchor: "sidebar.add-folder",
      title: "Add folder",
      body: "Register a folder you work in: Git repositories of your folders root are suggested, any folder can be browsed, and Clone from GitHub clones one. On SSH machines too.",
    },
  ],
};
