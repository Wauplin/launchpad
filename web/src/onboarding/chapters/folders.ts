// Chapter 8: folder pages, Files, workspaces, Add folder and the knowledge base.

import type { Chapter } from "../tour/types";
import { clickAnchor } from "./helpers";
import { meta } from "./meta";

const findOpen = () => !!document.querySelector('input[placeholder="Find in file"]');

export const folders: Chapter = {
  ...meta("folders"),
  learned: ["A folder page shows its agents, Local PRs and worktrees, and links to its pull requests and issues on GitHub", "Files browses any checkout or worktree, with Find", "Each folder belongs to one workspace; a window shows one workspace", "Add folder clones a GitHub repository or creates a plain folder", "Knowledge is the shared memory of a workspace: notes agents read and write, every change restorable; each bot's own memory is listed there too"],
  steps: [
    {
      id: "folders.page", route: "/projects/demo-storefront", anchor: "folder.page",
      title: "A folder page",
      body: "The line under the title counts running agents, waiting ones and Local PRs. Overview lists running agents, Local PRs (with links to the pull requests and issues on GitHub), worktrees and every agent; Settings holds the worktree setup command and the When done default.",
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
      body: "The workspace name opens its menu: rename it, open it in a new window, open its settings, or create another workspace. There is always one workspace: the last one cannot be deleted.",
    },
    {
      id: "folders.add", route: "/projects", anchor: "folders.add",
      title: "Add folder",
      body: "Folders in the sidebar lists the folders of the workspace. Add folder registers one: Clone from GitHub, or a new plain folder for files without Git. Home stores every folder, so it runs on any runner. Keep as folder turns a temporary folder into one.",
    },
    {
      id: "folders.knowledge", route: "/knowledge", anchor: "knowledge.notes",
      title: "Knowledge",
      body: "Each workspace has a knowledge base: Markdown notes and media, the memory its agents share. Agents read it as files and write with launchpad kb; their harness's own memory is off. Edit notes here, or ask the Librarian (a bot every workspace starts with, its weekly tidy paused until you turn it on) to merge duplicates and keep INDEX.md current. Bot memory, under the notes, lists each bot's own notes.",
    },
    {
      id: "folders.knowledge-changes", route: "/knowledge", anchor: "knowledge.tabs",
      title: "Every change, restorable",
      body: "Changes lists who changed what: the agent with its model and runner, or you, with the diff. Restore previous undoes a change for 30 days. A note edited by two agents at once keeps both versions until you pick one.",
    },
  ],
};
