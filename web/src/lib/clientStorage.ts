// Stop all background writers before erasing authenticated browser data.
let signedOut = false;
export const storageAllowed = () => !signedOut;
export async function signOut() {
  if (signedOut) return;
  signedOut = true;
  // Other windows must stop their writers and erase their in-memory data too.
  try { localStorage.setItem("launchpad-sign-out", `${Date.now()}-${Math.random()}`); } catch { /* Storage may be disabled. */ }
  for (const storage of [localStorage, sessionStorage]) {
    try {
      for (let i = storage.length - 1; i >= 0; i--) {
        const key = storage.key(i)!;
        if (key.startsWith("launchpad") || /^(diff-viewed|github-reply:|predraft-draft:|suggestion-edit:)/.test(key)) storage.removeItem(key);
      }
    } catch { /* Continue clearing the other stores. */ }
  }
  try {
    // Clear the store rather than waiting for deletion, which another tab's open DB can block.
    await new Promise<void>((resolve) => {
      const req = indexedDB.open("launchpad-cache", 1);
      req.onupgradeneeded = () => req.result.createObjectStore("queries");
      req.onerror = () => resolve();
      req.onsuccess = () => {
        const db = req.result;
        const tx = db.transaction("queries", "readwrite");
        tx.objectStore("queries").clear();
        tx.oncomplete = tx.onerror = tx.onabort = () => { db.close(); resolve(); };
      };
    });
  } finally { location.replace("/auth"); }
}
if (typeof window !== "undefined") window.addEventListener("storage", (ev) => { if (ev.key === "launchpad-sign-out" && ev.newValue && !signedOut) void signOut(); });
