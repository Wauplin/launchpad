import { readPage } from "./page";

export type Page = { tab: number; document: string; url: string; title: string };
export type Request = { id: string; session: string; grant: string; page: Page; work: boolean; tool: string; args: Record<string, unknown>; phase: string; prepared: Record<string, unknown> };
const attached = new Set<number>();
const epochs = new Map<number, number>();
const controlling = new Map<number, { session: string; grant: string; until: number }>();
const send = async (tab: number, method: string, params?: Record<string, unknown>): Promise<Record<string, unknown>> => {
  if (!attached.has(tab)) throw new Error("Browser control detached");
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      chrome.debugger.sendCommand({ tabId: tab }, method, params),
      new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error(`Chrome command timed out: ${method}; inspect before continuing`)), 10000); }),
    ]) as Record<string, unknown>;
  } finally { clearTimeout(timer); }
};
export async function release(tab: number) {
  epochs.set(tab, (epochs.get(tab) ?? 0) + 1);
  controlling.delete(tab);
  if (attached.delete(tab)) await chrome.debugger.detach({ tabId: tab }).catch(() => {});
  await chrome.scripting.executeScript({ target: { tabId: tab, frameIds: [0] }, func: () => document.getElementById("__launchpad-control")?.remove() }).catch(() => {});
}
chrome.debugger.onDetach.addListener((source) => { if (source.tabId != null) { attached.delete(source.tabId); controlling.delete(source.tabId); } });
/** The task and grant controlling a tab, if any. */
export const controller = (tab: number) => controlling.get(tab) ?? null;
export async function releaseAll() { await Promise.all([...attached].map(release)); }
export async function reconcile(sessions: { id: string; status: string; quota_pause?: unknown }[]) {
  for (const [tab, owner] of controlling) {
    const session = sessions.find((s) => s.id === owner.session);
    if (!session || session.status !== "working" || session.quota_pause || owner.until < Date.now()) await release(tab);
  }
}

// This function is serialized by scripting.executeScript. Keep all DOM logic self-contained.
export function targetPage(expected: Page, tool: string, args: Record<string, unknown>, fingerprint?: string) {
  try {
  const state = globalThis as typeof globalThis & { __launchpadDocument?: string; __launchpadRefs?: Map<string, Element> };
  if (state.__launchpadDocument !== expected.document || location.href !== expected.url) throw new Error("Page changed; inspect a fresh outline");
  const fields = tool === "browser_propose_form" ? args.fields as { ref: string; value: string }[] : [{ ref: String(args.ref ?? ""), value: String(args.text ?? args.value ?? "") }];
  if (!Array.isArray(fields) || fields.length < 1 || fields.length > 30) throw new Error("Provide 1–30 fields");
  const targets = fields.map((field) => {
    const el = state.__launchpadRefs?.get(field.ref);
    if (!el?.isConnected || !el.getClientRects().length || el.closest("[hidden],[aria-hidden=true],[data-launchpad-private]")) throw new Error("Stale or unavailable ref; read a new outline");
    const input = el instanceof HTMLInputElement ? el : null;
    const label = (el.getAttribute("aria-label") || (input?.labels?.[0]?.textContent) || el.textContent || input?.placeholder || "").trim().slice(0, 180);
    const identity = `${input?.type} ${input?.name} ${input?.autocomplete} ${el.id} ${label}`;
    if (/password|one.?time|otp|verification|security.?code/i.test(identity) || el.closest('[autocomplete="current-password"],[autocomplete="new-password"],[autocomplete="one-time-code"]')) throw new Error("Secret field: use browser_handover; passwords and one-time codes cannot be read or typed");
    if (input?.type === "file") throw new Error("File uploads require browser_handover; the agent cannot select local files");
    if (el.matches(":disabled,[aria-disabled=true]") || input?.readOnly || el instanceof HTMLTextAreaElement && el.readOnly) throw new Error("Target is disabled or read only");
    const editing = ["browser_type", "browser_propose_form"].includes(tool);
    if (editing && !(el instanceof HTMLInputElement && ["text", "search", "email", "url", "tel", "number"].includes(el.type) || el instanceof HTMLTextAreaElement || el instanceof HTMLElement && el.isContentEditable)) throw new Error("Target is not an ordinary editable text field");
    if (tool === "browser_select" && (!(el instanceof HTMLSelectElement) || el.multiple)) throw new Error("Choose a single select control");
    if (field.value.length > 16000) throw new Error("Field value too long");
    const select = el instanceof HTMLSelectElement ? el : null;
    const index = select ? Array.from(select.options).filter((o) => !o.matches(":disabled")).findIndex((o) => o.value === String(args.value)) : -1;
    if (tool === "browser_select" && (index < 0 || index > 200)) throw new Error("Option not available");
    const consequential = tool === "browser_propose_form" || /send|submit|pay|buy|purchase|delete|remove|publish|invite|checkout|confirm|order|transfer/i.test(label) || el.matches('button:not([type=button]),input[type=submit],input[type=image]') || /cc-|credit|card.?number|iban|payment|billing/i.test(identity) || tool === "browser_press" && args.key === "Enter";
    // Fingerprint both identity and current value so a review cannot apply to a changed field.
    return { ref: field.ref, label, tag: el.tagName, type: input?.type, value: field.value, current: input?.value ?? (el instanceof HTMLTextAreaElement || select ? (el as HTMLTextAreaElement).value : ""), consequential, index, html: el.outerHTML.slice(0, 4000) };
  });
  const signature = JSON.stringify(targets);
  if (fingerprint && fingerprint !== signature) throw new Error("Target changed since review; inspect and propose again");
  const first = state.__launchpadRefs!.get(targets[0].ref)!;
  first.scrollIntoView({ block: "center", inline: "center" });
  const r = first.getBoundingClientRect();
  const x = r.x + r.width / 2, y = r.y + r.height / 2;
  const hit = document.elementFromPoint(x, y);
  if (hit !== first && !first.contains(hit)) throw new Error("Target is covered by another element");
  const highlight = document.createElement("div");
  highlight.dataset.launchpadPrivate = "true";
  highlight.style.cssText = `position:fixed;pointer-events:none;z-index:2147483646;border:3px solid #0d7266;border-radius:4px;left:${r.x}px;top:${r.y}px;width:${r.width}px;height:${r.height}px`;
  document.documentElement.append(highlight);
  setTimeout(() => highlight.remove(), 1200);
  return { targets: targets.map(({ html: _html, ...target }) => target), fingerprint: signature, confirmation: targets.some((t) => t.consequential), x, y, error: null };
  } catch (error) { return { targets: [], fingerprint: "", confirmation: false, x: 0, y: 0, error: String(error) }; }
}
async function inject<T>(tab: number, func: (...args: any[]) => T, args: any[] = []): Promise<T> {
  const [result] = await chrome.scripting.executeScript({ target: { tabId: tab, frameIds: [0] }, func, args: args.map((arg) => arg === undefined ? null : arg) });
  if (!result || result.result == null) throw new Error("Document unavailable");
  return result.result as T;
}
async function target(tab: number, args: any[]) {
  const result = await inject(tab, targetPage, args);
  if (result.error) throw new Error(result.error);
  return result;
}
async function outline(page: Page) { return inject(page.tab, readPage, [page, "browser_outline", {}]); }
const origin = (url: string) => { const u = new URL(url); if (!["http:", "https:"].includes(u.protocol) || u.username || u.password) throw new Error("Only ordinary HTTP(S) URLs are supported"); return u.origin; };
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
async function key(tab: number, name: string, modifiers = 0) {
  const keys: Record<string, [string, number]> = { Enter: ["Enter", 13], Tab: ["Tab", 9], Escape: ["Escape", 27], Backspace: ["Backspace", 8], Delete: ["Delete", 46], ArrowDown: ["ArrowDown", 40], ArrowUp: ["ArrowUp", 38], ArrowLeft: ["ArrowLeft", 37], ArrowRight: ["ArrowRight", 39], Home: ["Home", 36], End: ["End", 35], Space: [" ", 32], a: ["a", 65] };
  if (!keys[name]) throw new Error("Unsupported key");
  const [value, code] = keys[name];
  await send(tab, "Input.dispatchKeyEvent", { type: "keyDown", key: value, windowsVirtualKeyCode: code, modifiers });
  await send(tab, "Input.dispatchKeyEvent", { type: "keyUp", key: value, windowsVirtualKeyCode: code, modifiers });
}
async function click(tab: number, x: number, y: number) {
  await send(tab, "Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  await send(tab, "Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
}
async function attach(req: Request, tab: number) {
  await chrome.tabs.update(tab, { active: true });
  if (!attached.has(tab)) { await chrome.debugger.attach({ tabId: tab }, "1.3"); attached.add(tab); }
  controlling.set(tab, { session: req.session, grant: req.grant, until: Date.now() + 1_800_000 });
  await chrome.scripting.executeScript({ target: { tabId: tab }, func: () => {
    if (document.getElementById("__launchpad-control")) return;
    // A thin outline marks the tab, and a small corner pill offers Stop and Take control. Only trusted
    // clicks count; the outline ignores pointer events so it never covers the page.
    const root = document.createElement("div"); root.id = "__launchpad-control"; root.dataset.launchpadPrivate = "true";
    const frame = document.createElement("div");
    frame.style.cssText = "position:fixed;inset:0;z-index:2147483646;pointer-events:none;box-shadow:inset 0 0 0 3px #0d7266";
    const pill = document.createElement("div");
    pill.style.cssText = "position:fixed;right:14px;bottom:14px;z-index:2147483647;display:flex;align-items:center;gap:8px;padding:6px 6px 6px 12px;border-radius:999px;background:#17202b;color:#fff;font:13px/1.2 system-ui,sans-serif;box-shadow:0 8px 24px rgba(0,0,0,.25)";
    const label = document.createElement("span"); label.textContent = "Launchpad is working in this tab";
    pill.append(label);
    for (const [action, text] of [["take-over", "Take control"], ["stop", "Stop"]]) {
      const button = document.createElement("button");
      button.textContent = text;
      button.style.cssText = `all:unset;cursor:pointer;border-radius:999px;padding:5px 11px;font-weight:600;${action === "stop" ? "background:#fff;color:#17202b" : "color:#fff;border:1px solid rgba(255,255,255,.35)"}`;
      button.addEventListener("click", (event) => { if (event.isTrusted) void chrome.runtime.sendMessage({ type: "page-control", action }); });
      pill.append(button);
    }
    root.append(frame, pill);
    document.documentElement.append(root);
  } });
}
async function group(req: Request, tab: number) {
  const stored = await chrome.storage.session.get(`group:${req.session}`);
  let groupId = stored[`group:${req.session}`] as number | undefined;
  if (groupId != null) try { await chrome.tabGroups.get(groupId); } catch { groupId = undefined; }
  groupId = await chrome.tabs.group({ tabIds: tab, ...(groupId == null ? {} : { groupId }) });
  await chrome.tabGroups.update(groupId, { title: "Launchpad", color: "cyan" });
  await chrome.storage.session.set({ [`group:${req.session}`]: groupId });
}
/** Consumes a request ID before input: event replay or worker suspension can never dispatch it twice. */
async function consume(id: string) {
  const consumed = await chrome.storage.session.get(`action:${id}`);
  if (consumed[`action:${id}`]) throw new Error("Action already dispatched; inspect outcome, never retry");
  await chrome.storage.session.set({ [`action:${id}`]: true });
}

/** A task's "New tab" handle has no page yet: it can only open a site, in a new tab of the task's group. */
async function openFromHandle(req: Request) {
  if (!["browser_navigate", "browser_open_tab"].includes(req.tool)) throw new Error("No page yet: open a site with browser_navigate first");
  const url = String(req.args.url);
  const site = origin(url);
  if (req.phase === "prepare") return { confirmation: true, url };
  await consume(req.id);
  if (!await chrome.permissions.contains({ origins: [`${site}/*`] })) throw new Error("Owner must allow the destination site in the review card");
  const tab = await chrome.tabs.create({ url, active: false });
  if (tab.id == null) throw new Error("Cannot open tab");
  await group(req, tab.id);
  for (let i = 0; i < 60; i++) { if ((await chrome.tabs.get(tab.id)).status === "complete") break; await sleep(100); }
  await sleep(150);
  const captured = await inject(tab.id, readPage);
  if (origin(captured.url) !== site) throw new Error("Navigation reached a new site; add the destination explicitly");
  const page = { tab: tab.id, document: captured.document, url: captured.url, title: captured.title };
  return { page, summary: `${req.tool} opened a new tab once; inspect outline for outcome`, outline: await outline(page) };
}

export async function work(req: Request) {
  if (req.page.tab < 0) return openFromHandle(req);
  let page = req.page;
  const epoch = epochs.get(page.tab) ?? 0;
  const checkControl = () => { if ((epochs.get(req.page.tab) ?? 0) !== epoch) throw new Error("Owner released control during action"); };
  if (!req.work && req.tool !== "browser_handover") throw new Error("Work permission required");
  if (req.phase === "read") throw new Error("Invalid action phase");
  const navigation = ["browser_navigate", "browser_open_tab"].includes(req.tool);
  const noTarget = navigation || ["browser_wait", "browser_handover"].includes(req.tool);
  const scroll = req.tool === "browser_scroll";
  const snapshot = await inject(page.tab, readPage, [page, "browser_read", { limit: 1 }]);
  if (!("error" in snapshot)) await chrome.storage.session.set({ [`grant:${req.grant}`]: true });
  if ("error" in snapshot && snapshot.error && req.tool !== "browser_handover") throw new Error(snapshot.error);
  const prepared = noTarget || scroll && !req.args.ref ? { confirmation: navigation && origin(String(req.args.url)) !== origin(page.url), ...(navigation ? { url: String(req.args.url) } : {}) } : await target(page.tab, [page, req.tool, req.args, req.phase === "execute" ? req.prepared.fingerprint : undefined]);
  checkControl();
  if (req.phase === "prepare") {
    if (req.tool === "browser_handover") await release(page.tab);
    return prepared;
  }
  // A request ID is consumed before input. Event replay or worker suspension cannot execute it twice.
  await consume(req.id);
  checkControl();
  if (req.tool === "browser_handover") {
    const captured = await inject(page.tab, readPage);
    page = { tab: page.tab, document: captured.document, url: captured.url, title: captured.title };
    if (origin(page.url) !== origin(req.page.url)) throw new Error("Handover changed site; add the page explicitly");
    return { page, outline: await outline(page), summary: "Owner continued after handover" };
  }
  if (navigation) {
    const url = String(req.args.url); origin(url);
    if (!await chrome.permissions.contains({ origins: [`${origin(url)}/*`] })) throw new Error("Owner must allow the destination site in the review card");
    await release(page.tab);
    const tab = req.tool === "browser_open_tab" ? await chrome.tabs.create({ url, active: false }) : await chrome.tabs.update(page.tab, { url });
    if (tab?.id == null) throw new Error("Cannot open tab");
    await group(req, tab.id);
    page = { ...page, tab: tab.id };
    for (let i = 0; i < 60; i++) { if ((await chrome.tabs.get(tab.id)).status === "complete") break; await sleep(100); }
  } else if (req.tool === "browser_wait") {
    const timeout = Math.min(10000, Math.max(0, Number(req.args.timeout_ms) || 5000));
    const start = Date.now();
    while (true) {
      const fresh = await chrome.tabs.get(page.tab);
      if (!fresh.url || origin(fresh.url) !== origin(page.url)) throw new Error("New site requires an explicit grant");
      const captured = await inject(page.tab, readPage);
      page = { tab: page.tab, document: captured.document, url: captured.url, title: captured.title };
      const content = await inject(page.tab, readPage, [page, "browser_read", { mode: "full", limit: 16000 }]);
      const refReady = req.args.ref ? await inject(page.tab, (ref: string) => {
        const state = globalThis as typeof globalThis & { __launchpadRefs?: Map<string, Element> };
        const el = state.__launchpadRefs?.get(ref); return !!el?.isConnected && el.getClientRects().length > 0;
      }, [String(req.args.ref)]) : false;
      if (req.args.text ? ("text" in content ? content.text : "").includes(String(req.args.text)) : req.args.ref ? refReady : fresh.status === "complete") break;
      if (Date.now() - start > timeout) throw new Error("Wait timed out; inspect the page before continuing");
      await sleep(200);
    }
  } else {
    await attach(req, page.tab);
    await group(req, page.tab);
    checkControl();
    if (scroll) {
      const p = prepared as { x?: number; y?: number };
      await send(page.tab, "Input.dispatchMouseEvent", { type: "mouseWheel", x: p.x ?? 100, y: p.y ?? 200, deltaX: 0, deltaY: Math.max(-2000, Math.min(2000, Number(req.args.delta_y) || 500)) });
    } else if (req.tool === "browser_propose_form") {
      // Revalidate each field immediately before trusted input; a partial failure is uncertain.
      for (const field of req.args.fields as { ref: string; value: string }[]) {
        const p = await target(page.tab, [page, "browser_type", { ref: field.ref, text: field.value }]);
        checkControl();
        await click(page.tab, p.x, p.y); await key(page.tab, "a", navigator.platform.includes("Mac") ? 4 : 2); await key(page.tab, "Backspace");
        await send(page.tab, "Input.insertText", { text: field.value });
      }
    } else {
      const p = prepared as { x: number; y: number; targets: { index: number }[] };
      if (req.tool === "browser_press" || req.tool === "browser_select") await inject(page.tab, (ref: string) => {
        const state = globalThis as typeof globalThis & { __launchpadRefs?: Map<string, Element> };
        const el = state.__launchpadRefs?.get(ref); if (!(el instanceof HTMLElement)) throw new Error("Cannot focus target"); el.focus(); return true;
      }, [String(req.args.ref)]);
      else await click(page.tab, p.x, p.y);
      if (req.tool === "browser_type") { await key(page.tab, "a", navigator.platform.includes("Mac") ? 4 : 2); await key(page.tab, "Backspace"); await send(page.tab, "Input.insertText", { text: String(req.args.text ?? "") }); }
      else if (req.tool === "browser_press") await key(page.tab, String(req.args.key));
      else if (req.tool === "browser_select") {
        await key(page.tab, "Home");
        for (let i = 0; i < p.targets[0].index; i++) await key(page.tab, "ArrowDown");
        const value = await inject(page.tab, (ref: string) => {
          const state = globalThis as typeof globalThis & { __launchpadRefs?: Map<string, Element> };
          const el = state.__launchpadRefs?.get(ref); return el instanceof HTMLSelectElement ? el.value : null;
        }, [String(req.args.ref)]);
        if (value !== req.args.value) throw new Error("Selection outcome differs from requested value; inspect before continuing");
      }
    }
  }
  await sleep(150);
  const captured = await inject(page.tab, readPage);
  if (origin(captured.url) !== origin(navigation ? String(req.args.url) : page.url)) { await release(page.tab); throw new Error("Navigation reached a new site; add the destination explicitly"); }
  page = { tab: page.tab, document: captured.document, url: captured.url, title: captured.title };
  return { page, summary: `${req.tool} dispatched once; inspect outline for outcome`, outline: await outline(page) };
}
