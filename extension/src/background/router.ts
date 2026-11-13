// Answers the runtime messages of shared/messages.ts. Sign-in, connection and settings requests come only from the
// extension's own pages (the side panel); content scripts reach Home only through `home.get` and `home.post`, never its
// credentials or device routes.

import { PAGE_ONLY, RequestError, type Answer, type Args, type Kind, type Message, type Result } from "../shared/messages";
import { startEvents, stopEvents } from "./events";
import { clientId, discover, signIn } from "./hf";
import { connectSpace, current, disconnect, homeJson, pollPairing, setWorkspace, signedIn, startPairing, update } from "./home";
import { removeLocal, setLocal } from "./store";

/** Home paths a client may call: under /api/, without dot segments, never this browser's device routes. */
function homePath(path: unknown): string {
  if (typeof path !== "string" || !path.startsWith("/api/") || /(^|\/)\.\.?(\/|$|\?)|%2e/i.test(path) || /^\/api\/(devices|extension)(\/|$|\?)/.test(path)) throw new RequestError("Not a Home path this extension may call");
  return path;
}

type Handler<K extends Kind> = (args: Args<K>) => Promise<Result<K>>;
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
  "home.get": ({ path }) => homeJson(homePath(path)),
  "home.post": ({ path, body, idempotency_key }) => homeJson(homePath(path), { method: "POST", body: body ?? {}, idempotency_key }),
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
        return { ok: true, data: await run(message.args) };
      } catch (e) {
        return { ok: false, error: e instanceof Error ? e.message : String(e), status: e instanceof RequestError ? e.status : undefined };
      }
    })().then(reply);
    return true;
  });
}
