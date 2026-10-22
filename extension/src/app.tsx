import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, useQuery, useQueryClient } from "@tanstack/react-query";
import { createMemoryHistory, Outlet, useSearch } from "@tanstack/react-router";
import { RouterProvider } from "@tanstack/react-router";
import { makeRouter } from "@/app/router";
import { HarnessProvider } from "@/components/HarnessMark";
import { Composer } from "@/features/launch/Composer";
import { SessionPane } from "@/features/agents/SessionPane";
import { SaveStatus } from "@/components/SaveStatus";
import { Toasts, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { connectEvents } from "@/lib/ws";
import { daemonUrl } from "@/lib/connection";
import { useSessions } from "@/lib/queries";
import { useLayout } from "@/stores/layout";
import { useComposer } from "@/stores/composer";
import type { Target } from "@/gen/Target";

type Page = { tab: number; document: string; url: string; title: string };
type Grant = { id: string; session: string | null; page: Page; device: string; touched: number };

function newDraft(text = "") {
  useComposer.getState().start();
  useComposer.getState().update((s) => ({ ...s, target: { kind: "browser", source_session_id: null, grants: [] }, worktree: null, text, context_projects: [], agent_traces: [], attachments: [], files: [], github_refs: [] }));
  useComposer.getState().setConditions([]);
}

function PanelRoot() {
  const { open, spec, update } = useComposer();
  const selected = useLayout((s) => s.selectedSession);
  const sessions = useSessions().data ?? [];
  const current = !open ? sessions.find((s) => s.id === selected) : null;
  const query = useQuery({ queryKey: ["browser-grants"], queryFn: () => api.get<Grant[]>("/api/browser/grants"), refetchInterval: 5000 });
  const [busy, setBusy] = useState(false);
  const [page, setPage] = useState<{ title: string; url: string; id: number } | null>(null);
  const [connection, setConnection] = useState(true);
  const client = useQueryClient();
  const target: Target | undefined = open ? spec.target : current?.launch_spec.target;
  const grantIds = target?.kind === "browser" ? target.grants : [];
  const grants = (query.data ?? []).filter((g) => grantIds.includes(g.id) || !!current && g.session === current.id);

  useEffect(() => {
    const refresh = async () => {
      const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
      setPage(tab?.id != null ? { title: tab.title ?? "Current page", url: tab.url ?? "", id: tab.id } : null);
    };
    const changed = () => void refresh();
    changed();
    chrome.tabs.onActivated.addListener(changed);
    chrome.tabs.onUpdated.addListener(changed);
    return () => { chrome.tabs.onActivated.removeListener(changed); chrome.tabs.onUpdated.removeListener(changed); };
  }, []);
  useEffect(() => {
    // Trace context from the shared session actions prepares a browser draft with fresh grants.
    if (open && spec.target.kind !== "browser") update((s) => ({ ...s, target: { kind: "browser", source_session_id: null, grants: [] }, worktree: null, context_projects: [] }));
  }, [open, spec.target.kind, update]);
  useEffect(() => {
    const check = async () => { try { setConnection(!!(await api.get("/api/health"))); } catch { setConnection(false); } };
    const timer = setInterval(() => void check(), 15_000);
    return () => clearInterval(timer);
  }, []);

  async function addPage(tabId?: number, selection = false) {
    if (busy) return;
    setBusy(true);
    try {
      const tab = tabId == null ? page : await chrome.tabs.get(tabId).then((t) => ({ id: t.id!, url: t.url ?? "", title: t.title ?? "Page" }));
      if (!tab || !/^https?:/.test(tab.url)) throw new Error("Choose an HTTP or HTTPS page. Chrome settings and the Chrome Web Store cannot be read.");
      // The panel button is an explicit gesture granting host access; the selection menu already grants activeTab.
      if (!selection && !await chrome.permissions.request({ origins: [`${new URL(tab.url).origin}/*`] })) throw new Error("Page access was declined");
      const captured = await chrome.runtime.sendMessage({ type: "capture", tab: tab.id });
      if (!captured) throw new Error("The extension worker is unavailable. Reload the extension and try again.");
      if (captured.error) throw new Error(captured.error);
      const grant = await api.post<Grant>("/api/browser/grants", captured);
      if (!useComposer.getState().open && current) {
        await api.post(`/api/browser/sessions/${current.id}/grants/${grant.id}`);
        await client.invalidateQueries({ queryKey: ["browser-grants"] });
        return;
      }
      if (!useComposer.getState().open) newDraft();
      update((s) => ({ ...s, text: selection ? "Explain the selected passage. Use browser_read with mode selection." : s.text,
        target: { kind: "browser", source_session_id: s.target.kind === "browser" ? s.target.source_session_id : null, grants: [...(s.target.kind === "browser" ? s.target.grants : []), grant.id] }, worktree: null, context_projects: [] }));
      await client.invalidateQueries({ queryKey: ["browser-grants"] });
    } catch (error) { toastError(error); }
    finally { setBusy(false); }
  }
  useEffect(() => {
    const selection = async () => {
      const { askSelection } = await chrome.storage.session.get("askSelection") as { askSelection?: { tab: number } };
      if (!askSelection) return;
      await chrome.storage.session.remove("askSelection");
      await addPage(askSelection.tab, true);
    };
    void selection();
    const listener = (message: { type: string }) => { if (message.type === "selection-ready") void selection(); };
    chrome.runtime.onMessage.addListener(listener);
    return () => chrome.runtime.onMessage.removeListener(listener);
  }, []);

  return <div className="flex h-dvh min-w-0 flex-col bg-paper text-ink">
    <header className="flex items-center gap-2 border-b border-line p-2"><b className="flex-1">Launchpad</b><button className="btn btn-sm" onClick={() => newDraft()}>New agent</button><button className="btn btn-sm" onClick={() => void chrome.tabs.create({ url: daemonUrl("/agents") })}>Open Launchpad ↗</button></header>
    {!connection && <p className="m-2 text-xs text-danger" role="status">Launchpad is offline. Reconnecting; your conversation stays on the daemon.</p>}
    <section className="grid gap-2 border-b border-line p-2 text-xs">
      <div className="min-w-0 truncate" title={page?.url}><b>Current page:</b> {page?.title ?? "Choose a tab"}</div>
      <div className="flex gap-2"><button className="btn btn-sm" disabled={busy} onClick={() => void addPage()}>{busy ? "Adding…" : "+ Add this page"}</button><button className="btn btn-sm" disabled={busy} onClick={() => { newDraft("Summarize this page. Read the article with the browser tools and cite its URL."); void addPage(); }}>Summarize page</button></div>
      {grants.map((g) => <div key={g.id} className="flex min-w-0 items-center gap-2 rounded-md bg-teal-soft p-2 text-teal-ink"><span className="min-w-0 flex-1 truncate" title={g.page.url}>Read this page · {g.page.title} {g.touched < Date.now() - 1_800_000 ? "(expired)" : ""}</span><button aria-label={`Remove page access to ${g.page.title}`} onClick={() => { void api.delete(`/api/browser/grants/${g.id}`).then(() => client.invalidateQueries({ queryKey: ["browser-grants"] })).catch(toastError); if (open) update((s) => ({ ...s, target: s.target.kind === "browser" ? { ...s.target, grants: s.target.grants.filter((id) => id !== g.id) } : s.target })); }}>×</button></div>)}
      {!grants.length && <p className="text-ink-3">No page shared. Adding a page grants this draft read access; switching tabs never changes it. Follow-ups need their own grant.</p>}
    </section>
    <main className="relative flex min-h-0 flex-1 flex-col"><Outlet /><Composer embedded /></main>
    <SaveStatus /><Toasts />
  </div>;
}

function BrowserSessions() {
  const search = useSearch({ strict: false });
  const selected = useLayout((s) => s.selectedSession);
  const sessions = (useSessions().data ?? []).filter((s) => s.launch_spec.target.kind === "browser");
  const sessionId = search.session ?? selected;
  const show = useLayout((s) => s.show);
  if (sessionId && sessions.some((s) => s.id === sessionId)) return <SessionPane key={sessionId} sessionId={sessionId} onClear={() => { useLayout.getState().clearSession(sessionId); }} />;
  return <div className="grid gap-2 overflow-y-auto p-3"><p className="text-sm text-ink-2">Share a page, write your task and choose a profile in the launch options.</p><select className="field-input w-full" aria-label="Recent browser sessions" value="" onChange={(e) => show(e.target.value)}><option value="" disabled>Recent browser sessions</option>{sessions.slice(0, 50).map((s) => <option key={s.id} value={s.id}>{s.title} · {s.status}</option>)}</select></div>;
}

export async function mount() {
  document.documentElement.classList.add("lp-phone");
  document.documentElement.style.zoom = "1";
  const client = new QueryClient({ defaultOptions: { queries: { refetchOnWindowFocus: false, retry: 1, staleTime: 30_000 } } });
  await useComposer.getState().restore();
  const s = useComposer.getState();
  if (!s.open) newDraft();
  connectEvents(client);
  const router = makeRouter({ Root: PanelRoot, AgentsView: BrowserSessions, InboxView: BrowserSessions }, createMemoryHistory({ initialEntries: ["/agents"] }));
  createRoot(document.getElementById("root")!).render(<QueryClientProvider client={client}><HarnessProvider><RouterProvider router={router} /></HarnessProvider></QueryClientProvider>);
}
