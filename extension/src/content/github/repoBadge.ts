// R1 Repository badge, next to the repository's name on every page of a repository: "In Launchpad · <workspace> /
// <folder>", "Your fork of …" / "Fork of … in Launchpad", or "Not in Launchpad", with a menu. The extension follows the
// folder's workspace (the worker does it when it answers the active tab's context).

import type { ExtContext } from "@/gen/ExtContext";
import { request } from "../../shared/messages";
import { addPopover, askWithoutCloning } from "./addRepo";
import { findAnchor } from "./anchors";
import type { Page } from "./page";
import { chip, mount, unmount, type MenuEntry } from "./widgets";

const ID = "repo-badge";
/** The context the menu shows (the newest, also when the chip itself did not change). */
let latest: ExtContext | null = null;

function open(path: string) {
  void request("open", { path }).catch((e) => console.warn("Launchpad: open", e));
}

function label(ctx: ExtContext): { text: string; tone: "accent" | "quiet" } {
  const f = ctx.folder;
  if (!f) return { text: "Not in Launchpad", tone: "quiet" };
  if (f.relation === "fork") return { text: f.yours ? `Your fork of ${f.repo ?? f.name}` : `Fork of ${f.repo ?? f.name} in Launchpad`, tone: "accent" };
  return { text: `In Launchpad · ${f.workspace} / ${f.name}`, tone: "accent" };
}

function entries(ctx: ExtContext, page: Page, root: ShadowRoot, button: HTMLElement, refresh: () => void): MenuEntry[] {
  const f = ctx.folder;
  if (!f) {
    const repo = page.repo ?? ctx.page.repo ?? "";
    const item = page.number && (page.kind === "issue" || page.kind.startsWith("pr_")) ? { repo, number: page.number, kind: page.kind === "issue" ? "issue" as const : "pr" as const, title: ctx.page.title } : null;
    return [
      { label: "Add to Launchpad", hint: "Clone from GitHub into a workspace", onSelect: () => addPopover(root, button, repo, refresh) },
      { label: "Ask without cloning", hint: item ? `A bot answers about #${item.number}, without a folder` : "A bot answers with this repository as context", onSelect: () => askWithoutCloning(root, button, ctx, item ?? { repo, number: null, kind: "repo" }, refresh) },
      ...(ctx.inbox_unread ? [{ note: `${ctx.inbox_unread} unread notifications of this repository: Launchpad's Inbox shows them in no workspace until it has a folder.` }] : []),
    ];
  }
  const ws = `ws=${encodeURIComponent(f.workspace_id)}`;
  const out: MenuEntry[] = [
    { label: "Open in Launchpad", hint: `The folder ${f.name} in ${f.workspace}`, onSelect: () => open(`/projects/${encodeURIComponent(f.id)}?${ws}`) },
    { label: "Open folder's Files", hint: "Browse its files in Launchpad", onSelect: () => open(`/projects/${encodeURIComponent(f.id)}/code?${ws}`) },
  ];
  if (f.relation === "upstream") out.push({ note: `Your folder ${f.name} is a clone of ${f.repo ?? "a fork"}; this is its upstream.` });
  if (f.relation === "fork") out.push({ note: `Launchpad has ${f.repo ?? "the upstream"} as ${f.name}; this repository is a fork of it.` });
  out.push({ separator: true });
  if (ctx.others.length) out.push({ note: `Also a folder in ${ctx.others.map((o) => o.workspace).join(", ")}. The extension follows ${f.workspace} here; switch workspace in the Launchpad panel.` });
  else out.push({ note: `The extension follows this folder's workspace: ${f.workspace}.` });
  return out;
}

/** Renders the badge (idempotent), or removes it without a context or anchor. */
export function renderRepoBadge(page: Page, ctx: ExtContext | null, refresh: () => void) {
  if (!page.repo || !ctx || ctx.page.repo?.toLowerCase() !== page.repo.toLowerCase()) return unmount(ID);
  const anchor = findAnchor("repo-name", { repo: page.repo, number: page.number });
  if (!anchor) return unmount(ID);
  latest = ctx;
  const { text, tone } = label(ctx);
  const others = ctx.others.map((o) => o.id).join(",");
  const w = mount(ID, `${page.repo.toLowerCase()}|${text}|${ctx.folder?.id ?? ""}|${others}`, anchor.el, anchor.place);
  if (!w.fresh) return;
  const root = w.root;
  const button: HTMLElement = chip(root, text, tone, () => ({ title: text, entries: entries(latest ?? ctx, page, root, button, refresh) }));
}
