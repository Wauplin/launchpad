// Answers the runtime messages of shared/messages.ts. Sign-in, connection and settings requests come only from the
// extension's own pages (the side panel); content scripts reach Home only through the dedicated requests below, each
// with its own route and checks (there is no generic Home call), never its credentials or device routes.

import { PAGE_ONLY, RequestError, type Answer, type Args, type Kind, type Message, type Result } from "../shared/messages";
import { botMessage, bots, botThreads, events, launch, ledger, profiles, sessions, stop, takeQuote } from "./agents";
import { clone, doneHandled, localPr, notifications, observed, publish, pushToPr, reviewChoice } from "./lists";
import { anchorHealth, follow, pageContext, pageRows, peekContext, reportAnchors } from "./context";
import { startEvents, stopEvents } from "./events";
import { clientId, discover, signIn } from "./hf";
import { connectSpace, current, disconnect, pollPairing, setWorkspace, signedIn, startPairing, update } from "./home";
import { removeLocal, setLocal } from "./store";

/** A path of Home's web app to open in a tab: absolute, no scheme or host, no dot segments. */
function appPath(path: unknown): string {
  if (typeof path !== "string" || !path.startsWith("/") || path.startsWith("//") || /[\\\s]|(^|\/)\.\.?(\/|$|\?)|%2e/i.test(path) || path.length > 500) throw new RequestError("Not a Launchpad page");
  return path;
}

type Handler<K extends Kind> = (args: Args<K>, sender: chrome.runtime.MessageSender) => Promise<Result<K>>;
const handlers: { [K in Kind]: Handler<K> } = {
  state: async () => current(),
  signin: async () => { await signIn(); await signedIn(); return current(); },
  discover: () => discover(),
  "connect.space": async ({ space, origin }) => { const s = await connectSpace(space, origin); if (s.phase !== "starting") startEvents(); return current(); },
  "connect.address.start": ({ origin }) => startPairing(origin),
  "connect.address.poll": async () => { const r = await pollPairing(); if (r.approved) startEvents(); return { approved: r.approved, state: current() }; },
  disconnect: async () => { await disconnect(stopEvents); return current(); },
  "settings.client-id": async ({ client_id }) => {
    const value = client_id?.trim();
    if (value) await setLocal({ "hf.client_id": value }); else await removeLocal("hf.client_id");
    update({ client_id: await clientId() });
    return current();
  },
  "workspace.set": async ({ id }) => { await setWorkspace(id); return current(); },
  // Hook for the GitHub layer: once Home's page context names the repository's workspace (step 4), the worker calls
  // setWorkspace with it; this request lets a content script ask for the same.
  "workspace.follow": async ({ id }) => { await setWorkspace(id); return current(); },
  "anchors.health": () => anchorHealth(),
  context: async ({ url, fresh }, sender) => {
    const data = await pageContext(url, fresh);
    // Only the tab you look at moves the extension to another workspace.
    if (sender.tab?.active) await follow(data).catch((e) => console.warn("workspace follow", e));
    return data;
  },
  "context.peek": async ({ url }) => peekContext(url),
  rows: ({ url, items }) => pageRows(url, Array.isArray(items) ? items.map(String) : []),
  open: async ({ path }) => {
    const home = current().home;
    if (!home) throw new RequestError("Not connected to a Launchpad Home");
    await chrome.tabs.create({ url: home.origin + appPath(path) });
  },
  "anchors.report": async ({ checks }) => { if (checks && typeof checks === "object") await reportAnchors(checks); },
  bots: () => bots(),
  "bot.message": (args) => botMessage(args),
  "bot.threads": (args) => botThreads(args),
  profiles: () => profiles(),
  launch: (args) => launch(args),
  "branch.push-to-pr": (args) => pushToPr(args),
  sessions: (args) => sessions(args),
  "session.events": (args) => events(args),
  "session.stop": (args) => stop(args),
  "suggestion.ledger": (args) => ledger(args),
  notifications: () => notifications(),
  "inbox.observed": (args) => observed(args),
  "inbox.done-handled": (args) => doneHandled(args),
  "review-choice": (args) => reviewChoice(args),
  "folder.clone": (args) => clone(args),
  "localpr.detail": (args) => localPr(args),
  "localpr.publish": (args) => publish(args),
  "quote.take": async () => takeQuote(),
};

/** The extension's own pages (the side panel, also when opened in a tab); a content script's URL is its page's. */
function fromPage(sender: chrome.runtime.MessageSender) {
  const origin = `chrome-extension://${chrome.runtime.id}`;
  return sender.origin === origin || (!sender.origin && !!sender.url?.startsWith(`${origin}/`));
}

export function route(ready: Promise<unknown>) {
  chrome.runtime.onMessage.addListener((message: Message, sender, reply: (answer: Answer) => void) => {
    if (sender.id !== chrome.runtime.id || !message || typeof message.kind !== "string" || !(message.kind in handlers)) return false;
    const kind = message.kind;
    void (async (): Promise<Answer> => {
      try {
        await ready;
        if (PAGE_ONLY.includes(kind) && !fromPage(sender)) throw new RequestError("Only the Launchpad panel may do this");
        const run = handlers[kind] as Handler<typeof kind>;
        return { ok: true, data: await run(message.args, sender) };
      } catch (e) {
        return { ok: false, error: e instanceof Error ? e.message : String(e), status: e instanceof RequestError ? e.status : undefined };
      }
    })().then(reply);
    return true;
  });
}
