// What the window does with Launchpad references (`lib/refs.ts`): ask Home what a pasted link or ID names, copy an
// object's link, and open one in-app (Markdown links, Go to). Navigation goes through one navigator the shell
// installs (`setRefNavigator`), so components without the router (Markdown in the extension) open links normally.

import type { LpRef } from "@/gen/LpRef";
import type { RefTarget } from "@/gen/RefTarget";
import { toast, toastError } from "@/components/Toast";
import { api } from "./api";
import { copyText } from "./clipboard";
import { appLink, describeRef, looksLikeRef, refId, refLink } from "./refs";
import { humanError } from "@/components/ui";
import { useWorkspace } from "@/stores/workspace";

/** Home resolves a pasted link or ID: in `workspace` (a message box: references of another workspace read as
 * unknown), or anywhere (Go to). */
export const resolveRef = (text: string, workspace?: string | null) =>
  api.get<RefTarget>(`/api/refs/resolve?${new URLSearchParams({ ref: text.trim(), ...(workspace ? { workspace } : {}) })}`);

/** Copies the link of an object (with the window's workspace), and says so with its short ID. */
export async function copyRefLink(r: LpRef) {
  const ws = useWorkspace.getState().current;
  await copyText(refLink(r, location.origin, ws));
  toast(`Link copied · ${describeRef(r)} · ${refId(r)}`);
}

/** Copies the short ID (`lp:…`), for agents and the CLI. */
export async function copyRefId(r: LpRef) {
  await copyText(refId(r));
  toast(`ID copied · ${refId(r)}`);
}

type Navigator = (path: string) => void;
let navigator: Navigator | null = null;

/** The shell installs the router's navigation (`Shell`). */
export function setRefNavigator(n: Navigator | null) {
  navigator = n;
}

/** Opens a resolved reference: switches the window's workspace first when the object is of another one. */
export function openTarget(t: RefTarget) {
  const store = useWorkspace.getState();
  if (t.workspace_id && store.current !== t.workspace_id) store.switchTo(t.workspace_id);
  if (navigator) navigator(t.path);
  else location.assign(t.path);
}

/** Whether this window opens references in-app (the app's shells install a navigator; the extension does not). */
export const hasRefNavigator = () => navigator !== null;

/** A link's target when it is a Launchpad reference to open in this window: a short ID, a full address of this
 * window's origin, or a root-relative path of a Launchpad page (`/folios/<slug>`, as agents write them), made full.
 * Null for anything else (it opens like any link; other relative paths stay file links). */
export function internalHref(href: string): string | null {
  if (/^lp:[a-z_-]+\/\S+$/.test(href)) return href;
  if (href.startsWith("/")) return appLink(href, location.origin);
  if (!/^https?:\/\//i.test(href)) return null;
  try {
    return new URL(href).origin === location.origin ? href : null;
  } catch {
    return null;
  }
}

/** Opens a Launchpad link or ID in this window: Home resolves it (its workspace and page); an address of this
 * window that names no object (a settings page) opens as it is. */
export async function openRef(text: string) {
  try {
    openTarget(await resolveRef(text));
  } catch (e) {
    if (text.startsWith("lp:") || !navigator) { toastError(e); return; }
    const url = new URL(text, location.href);
    url.searchParams.delete("ws");
    navigator(url.pathname + url.search + url.hash);
  }
}

/** A message box without reference tokens (the bot chat): a pasted Launchpad link or ID stays in the text and is
 * also added as a context chip once Home resolved it in `workspace`, so the receiving agent gets its read command
 * (Home also finds references in the text itself). Unknown ones only say why. */
export function addPastedRef(text: string, workspace: string | null, add: (r: LpRef) => void) {
  const pasted = text.trim();
  if (!looksLikeRef(pasted, location.origin)) return;
  void resolveRef(pasted, workspace).then((t) => add(t.ref)).catch((err) => toast(`Not added as context: ${humanError(err)}`));
}
