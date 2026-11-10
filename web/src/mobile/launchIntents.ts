// What the installed app's shortcuts and Share target ask for (web/public/manifest.webmanifest): `?open=ask` opens
// the Ask bot's chat, `?open=launch` the composer, and `share_title`/`share_text`/`share_url` open the composer
// with the shared text as the prompt. The parameters leave the address at once (other parameters stay).

import { useNavigate, useRouter } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { askTarget, useBots } from "@/lib/bots";
import { useScope } from "@/lib/workspace";
import { useComposer } from "@/stores/composer";

type Intent = { kind: "ask" } | { kind: "launch"; text: string };

const KEYS = ["open", "share_title", "share_text", "share_url"];

/** Reads and removes the intent from the address; only the first call of the page load gets it. */
let taken = false;
function takeIntent(): Intent | null {
  if (taken) return null;
  taken = true;
  const params = new URLSearchParams(location.search);
  const open = params.get("open");
  const shared = ["share_title", "share_text", "share_url"].map((k) => params.get(k)?.trim() ?? "").filter(Boolean);
  if (!KEYS.some((k) => params.has(k))) return null;
  if (shared.length) return { kind: "launch", text: shared.join("\n") };
  if (open === "ask") return { kind: "ask" };
  if (open === "launch") return { kind: "launch", text: "" };
  return null;
}

function stripIntent(): string {
  const params = new URLSearchParams(location.search);
  for (const k of KEYS) params.delete(k);
  const rest = params.toString();
  return location.pathname + (rest ? `?${rest}` : "") + location.hash;
}

/** Mounted once in each shell. */
export function useLaunchIntents() {
  const router = useRouter();
  const navigate = useNavigate();
  const [intent, setIntent] = useState<Intent | null>(null);
  useEffect(() => {
    const found = takeIntent();
    if (!found) return;
    router.history.replace(stripIntent());
    setIntent(found);
  }, [router]);

  const bots = useBots();
  const workspaceId = useScope().workspace.id;
  const composerLoaded = useComposer((s) => s.loaded);
  useEffect(() => {
    if (!intent) return;
    if (intent.kind === "ask") {
      // Wait for the bots and the window's workspace (an empty id while the workspaces load).
      if (bots.isPending || !workspaceId) return;
      setIntent(null);
      const bot = askTarget(bots.data ?? [], workspaceId);
      if (bot) void navigate({ to: "/bots/$id", params: { id: bot.id }, search: {} });
      else void navigate({ to: "/bots", search: { new: true } });
      return;
    }
    // The saved draft loads first, so it does not replace the shared text.
    if (!composerLoaded) return;
    setIntent(null);
    const composer = useComposer.getState();
    composer.start();
    if (intent.text) composer.update((s) => ({ ...s, text: s.text.trim() ? `${s.text}\n\n${intent.text}` : intent.text }));
  }, [intent, bots.isPending, bots.data, workspaceId, composerLoaded, navigate]);
}
