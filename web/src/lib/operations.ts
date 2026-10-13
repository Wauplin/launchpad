// Keep the same operation ID after a lost response, including across page reloads.
// Store only a fingerprint and ID; prompts, images and credentials stay out of this ledger.
export async function operationKey(path: string, body: unknown, storage: Pick<Storage, "getItem" | "setItem" | "removeItem"> = localStorage) {
  const request = JSON.stringify(body);
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`${path}\n${request}`));
  const hash = Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
  const key = `launchpad.operation.${hash}`;
  const existing = storage.getItem(key);
  const id = existing ?? crypto.randomUUID();
  // If durable storage fails, stop before sending rather than risk a duplicate after reload.
  if (!existing) storage.setItem(key, id);
  return { id, delivered: () => { if (storage.getItem(key) === id) storage.removeItem(key); } };
}

export function retryProtected(path: string): boolean {
  return path === "/api/sessions" || /^\/api\/sessions\/[^/]+\/input$/.test(path);
}

/** Owner explicitly starts another attempt after inspecting an uncertain result. */
export async function abandonOperation(path: string, body: unknown) {
  (await operationKey(path, body)).delivered();
}
