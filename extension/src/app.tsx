// Side panel: start a task on the tabs you choose, watch it, approve what matters. Everything else
// (trace, costs, model, follow-ups, schedules, presets) stays in Launchpad, one click away.

import { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, useQueryClient } from "@tanstack/react-query";
import { HarnessProvider } from "@/components/HarnessMark";
import { Toasts, toastError } from "@/components/Toast";
import type { FileAttachment } from "@/gen/FileAttachment";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Session } from "@/gen/Session";
import { api } from "@/lib/api";
import { daemonUrl } from "@/lib/connection";
import { useProjects, useSessions } from "@/lib/queries";
import { isRunning, isQuotaPaused, needsYou } from "@/lib/sessionStatus";
import { connectEvents, onConnectionChange, onEvent } from "@/lib/ws";
import { emptySpec, rememberSuccessfulSession, rememberedRun } from "@/stores/composer";
import { Box, chipOf, suggestProject, useAgents, useWindowTabs, type Quote } from "./compose";
import { Favicon, Icon } from "./icons";
import { grantTabs, hostOf, isBrowserTask, openInLaunchpad, prefs, rememberSites, requestSites, savePrefs, siteOf, tabChip, useReviews, useSites, type Grant, type TabChip } from "./lp";
import { StatusPill, TaskView, taskStatus } from "./task";

type View = { kind: "new" } | { kind: "task"; id: string } | { kind: "tasks" } | { kind: "settings" };
const VIEW = "launchpad-extension-view";
const DRAFT = "launchpad-extension-draft";
type Draft = { text: string; chips: TabChip[] | null; quote: Quote | null; images: FileAttachment[]; profile: string | null; act: boolean | null; project: string | null };
const emptyDraft = (): Draft => ({ text: "", chips: null, quote: null, images: [], profile: null, act: null, project: null });
function loadDraft(): Draft {
  try { return { ...emptyDraft(), ...JSON.parse(localStorage.getItem(DRAFT) ?? "{}") }; } catch { return emptyDraft(); }
}

function Root() {
  const [view, setViewState] = useState<View>(() => { try { return JSON.parse(localStorage.getItem(VIEW) ?? "") as View; } catch { return { kind: "new" }; } });
  const setView = (v: View) => { setViewState(v); localStorage.setItem(VIEW, JSON.stringify(v)); };
  const [online, setOnline] = useState(true);
  const [menu, setMenu] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const client = useQueryClient();
  const sessions = useSessions().data ?? [];
  const reviews = useReviews().data ?? [];
  const task = view.kind === "task" ? sessions.find((s) => s.id === view.id) ?? null : null;
  useEffect(() => onConnectionChange(setOnline), []);
  useEffect(() => onEvent((ev) => {
    if (ev.topic !== "browser") return;
    void client.invalidateQueries({ queryKey: ["browser-grants"] });
    if (ev.type === "review") void client.invalidateQueries({ queryKey: ["browser-reviews"] });
    if (ev.type === "sites") void client.invalidateQueries({ queryKey: ["browser-sites"] });
  }), [client]);
  useEffect(() => {
    const changed = (message: { type: string }) => { if (message.type === "browser-changed") void client.invalidateQueries({ queryKey: ["browser-grants"] }); };
    chrome.runtime.onMessage.addListener(changed);
    return () => chrome.runtime.onMessage.removeListener(changed);
  }, [client]);
  useEffect(() => { for (const s of sessions) rememberSuccessfulSession(s); }, [sessions]);
  // A review of another task is never hidden behind the current view.
  const elsewhere = reviews.filter((r) => view.kind !== "task" || r.session !== view.id);
  const elsewhereTask = sessions.find((s) => s.id === elsewhere[0]?.session);

  const title = view.kind === "task" ? task?.title ?? "Task" : view.kind === "tasks" ? "Tasks" : view.kind === "settings" ? "Settings" : "New chat";
  const archive = async (s: Session) => {
    try { await api.post(`/api/sessions/${s.id}/close`); setView({ kind: "tasks" }); } catch (e) { toastError(e); }
  };
  const stopSharing = async (s: Session) => {
    const grants = await api.get<Grant[]>("/api/browser/grants");
    await Promise.all(grants.filter((g) => g.session === s.id).map((g) => api.delete(`/api/browser/grants/${g.id}`))).catch(toastError);
    await client.invalidateQueries({ queryKey: ["browser-grants"] });
  };
  const rename = async (s: Session, text: string) => {
    setRenaming(null);
    if (text.trim() && text.trim() !== s.title) await api.post(`/api/sessions/${s.id}/rename`, { title: text.trim() }).catch(toastError);
  };

  return <div className="x-app">
    <header className="x-top">
      {view.kind === "new" ? <img className="x-logo" src={chrome.runtime.getURL("icons/48.png")} alt="" /> : <button className="x-ib" aria-label="Back" title={view.kind === "task" ? "All tasks" : "Back"} onClick={() => setView(view.kind === "task" ? { kind: "tasks" } : { kind: "new" })}><Icon name="back" /></button>}
      <span className="x-title">{task && renaming !== null
        ? <input autoFocus aria-label="Task title" value={renaming} onChange={(e) => setRenaming(e.target.value)} onBlur={() => void rename(task, renaming)} onKeyDown={(e) => { if (e.key === "Enter") void rename(task, renaming); if (e.key === "Escape") setRenaming(null); }} />
        : title}</span>
      {task && <StatusPill s={task} reviews={reviews} />}
      {view.kind !== "new" && <button className="x-ib" aria-label="New chat" title="New chat" onClick={() => setView({ kind: "new" })}><Icon name="edit" /></button>}
      {view.kind !== "tasks" && view.kind !== "task" && <button className="x-ib" aria-label="Tasks" title="Tasks" onClick={() => setView({ kind: "tasks" })}><Icon name="list" /></button>}
      <button className="x-ib" aria-label="More" title="More" onClick={() => setMenu(true)}><Icon name="more" /></button>
    </header>
    {menu && <><div className="x-scrim" onClick={() => setMenu(false)} />
      <div className="x-menu x-top-menu" role="menu" onClick={() => setMenu(false)}>
        {task && <>
          <button className="x-opt" onClick={() => openInLaunchpad(`/agents?session=${task.id}`)}><Icon name="ext" size={16} />Open in Launchpad</button>
          <button className="x-opt" onClick={() => setRenaming(task.title)}><Icon name="edit" size={16} />Rename</button>
          <button className="x-opt" onClick={() => void stopSharing(task)}><Icon name="close" size={16} />Stop sharing all tabs</button>
          {task.status !== "closed" && <button className="x-opt x-danger" onClick={() => void archive(task)}><Icon name="archive" size={16} />Archive task</button>}
          <div className="x-sep" />
        </>}
        {!task && <button className="x-opt" onClick={() => openInLaunchpad("/agents")}><Icon name="ext" size={16} />Open Launchpad</button>}
        <button className="x-opt" onClick={() => setView({ kind: "settings" })}><Icon name="gear" size={16} /><span className="x-d"><b style={{ fontWeight: 400 }}>Settings</b><small>Connection, sites allowed to Act</small></span></button>
      </div></>}
    {!online && <div className="x-banner x-off" style={{ margin: "0 12px 6px" }}><Icon name="plug" size={15} /><span>Can't reach Launchpad. Reconnecting… Your tasks keep running on your computer.</span></div>}
    {elsewhere.length > 0 && elsewhereTask && <button className="x-banner x-warn" style={{ margin: "0 12px 6px", border: 0, cursor: "pointer", font: "inherit", textAlign: "left" }} onClick={() => setView({ kind: "task", id: elsewhereTask.id })}>
      <Icon name="warn" size={15} /><span>“{elsewhereTask.title}” needs your OK</span><Icon name="back" size={14} className="x-flip" /></button>}
    {view.kind === "task" ? <TaskView key={view.id} id={view.id} />
      : view.kind === "tasks" ? <Tasks open={(id) => setView({ kind: "task", id })} />
      : view.kind === "settings" ? <Settings />
      : <NewChat open={(id) => setView({ kind: "task", id })} />}
    <Toasts />
  </div>;
}

function NewChat({ open }: { open: (id: string) => void }) {
  const [draft, setDraftState] = useState<Draft>(loadDraft);
  const setDraft = (f: (d: Draft) => Draft) => setDraftState(f);
  useEffect(() => localStorage.setItem(DRAFT, JSON.stringify(draft)), [draft]);
  const [busy, setBusy] = useState(false);
  const [conflict, setConflict] = useState<{ grants: Grant[]; title: string } | null>(null);
  const client = useQueryClient();
  const tabs = useWindowTabs();
  const agents = useAgents();
  const projects = (useProjects().data ?? []).filter((p) => !p.is_temp);
  const sites = useSites().data ?? [];
  const sessions = useSessions().data ?? [];
  const reviews = useReviews().data ?? [];
  const current = tabs.find((t) => t.active);
  // Until the owner edits the tabs, the box proposes the current tab.
  const chips = draft.chips ?? (current?.id != null && /^https?:/.test(current.url ?? "") ? [chipOf(current)] : []);
  const profile = agents.find((a) => a.id === (draft.profile ?? prefs().profile)) ?? agents[0] ?? null;
  const allowed = chips.length > 0 && chips.every((c) => sites.includes(siteOf(c.url)));
  const act = draft.act ?? (allowed || prefs().act);
  const project = draft.project === null ? null : projects.find((p) => p.id === draft.project) ?? null;
  const suggestion = useMemo(() => suggestProject(projects, chips), [projects, chips]);

  useEffect(() => {
    // "Ask Launchpad about selection" in the context menu: the tab and its selected text.
    const take = async () => {
      const { askSelection } = await chrome.storage.session.get("askSelection") as { askSelection?: { tab: number; text?: string } };
      if (!askSelection) return;
      await chrome.storage.session.remove("askSelection");
      const chip = await tabChip(askSelection.tab);
      if (!chip) return;
      setDraft((d) => ({ ...d, chips: [{ ...chip, activeTab: true }, ...(d.chips ?? []).filter((c) => c.tab !== chip.tab)], quote: askSelection.text ? { text: askSelection.text.slice(0, 4000), tab: chip.tab } : d.quote, text: d.text || "Explain this." }));
    };
    void take();
    const listener = (message: { type: string }) => { if (message.type === "selection-ready") void take(); };
    chrome.runtime.onMessage.addListener(listener);
    return () => chrome.runtime.onMessage.removeListener(listener);
  }, []);

  const launch = async (grants: Grant[], actNow: boolean) => {
    if (!profile) throw new Error("Add an agent profile in Launchpad → Settings → Profiles");
    const run = rememberedRun(profile.harness, profile.id);
    const quote = draft.quote ? `${draft.quote.text.split("\n").map((l) => `> ${l}`).join("\n")}\n\n` : "";
    const spec: LaunchSpec = {
      ...emptySpec(), harness: profile.harness, profile: profile.id, budget_usd: profile.budget_usd,
      model: run.model, effort: run.effort, permission_mode: run.permission_mode,
      target: { kind: "browser", source_session_id: null, grants: grants.map((g) => g.id), project_id: project?.id ?? null, act: actNow },
      worktree: project ? { base: null, include_changes: false, merge_when_done: project.merge_when_done } : null,
      text: `${quote}${draft.text.trim()}`, attachments: draft.images,
    };
    const session = await api.post<Session>("/api/sessions", spec);
    savePrefs({ profile: profile.id, act: actNow });
    // Saved here: the view changes right away, before an effect of this component could run.
    const cleared = { ...emptyDraft(), profile: profile.id };
    localStorage.setItem(DRAFT, JSON.stringify(cleared));
    setDraftState(cleared);
    await client.invalidateQueries({ queryKey: ["browser-grants"] });
    open(session.id);
  };
  const send = async () => {
    if (busy || !draft.text.trim()) return;
    setBusy(true);
    let grants: Grant[] = [];
    try {
      if (!chips.length) throw new Error("Share at least one tab with + Tabs.");
      if (!await requestSites(chips)) throw new Error("Chrome access to the site was declined");
      grants = await grantTabs(chips);
      await launch(grants, act);
    } catch (e) {
      const busyTab = e instanceof Error && /Another task is using this tab/.test(e.message);
      if (busyTab && grants.length) setConflict({ grants, title: chips.length === 1 ? chips[0].title : "one of these tabs" });
      else {
        await Promise.all(grants.map((g) => api.delete(`/api/browser/grants/${g.id}`).catch(() => {})));
        toastError(e);
      }
    } finally { setBusy(false); }
  };
  const resolve = async (move: boolean) => {
    if (!conflict) return;
    setBusy(true);
    try {
      if (move) for (const g of conflict.grants) await api.post(`/api/browser/grants/${g.id}/control`, { action: "take-tab" });
      await launch(conflict.grants, move);
      setConflict(null);
    } catch (e) { toastError(e); }
    finally { setBusy(false); }
  };
  const waiting = sessions.filter((s) => isBrowserTask(s) && s.status !== "closed" && taskStatus(s, reviews).tone === "need");
  const suggest = (text: string, all = false) => setDraft((d) => ({ ...d, text, chips: all ? tabs.filter((t) => t.id != null && /^https?:/.test(t.url ?? "")).map(chipOf) : d.chips }));

  return <>
    <div className="x-body">
      {waiting.length > 0 && <div style={{ display: "grid", gap: 2 }}><div className="x-sect" style={{ margin: "0 4px" }}>Needs you</div>{waiting.slice(0, 3).map((s) => <TaskRow key={s.id} s={s} open={open} />)}</div>}
      <div className="x-hello">
        <h1>{chips.length > 1 ? "What should I do with these tabs?" : "What should I do with this page?"}</h1>
        <div className="x-muted x-small">I can read the tabs you add below. Nothing is shared until you send.</div>
        <div className="x-sugg">
          <button onClick={() => suggest("Summarize this page.")}><Icon name="sum" size={16} />Summarize this page</button>
          <button onClick={() => suggest("Explain the part I selected on this page.")}><Icon name="q" size={16} />Explain the part I select</button>
          <button onClick={() => suggest("Compare these pages and tell me the main differences.", true)}><Icon name="cmp" size={16} />Compare my open tabs</button>
        </div>
      </div>
      {conflict && <div className="x-card x-ask" role="alert">
        <h3><Icon name="hand" size={16} />Another task is using {conflict.title}</h3>
        <p className="x-small">Only one task can act in a tab at a time.</p>
        <div className="x-btns"><button className="x-btn x-pri x-wide" disabled={busy} onClick={() => void resolve(true)}>Move it here</button><button className="x-btn x-wide" disabled={busy} onClick={() => void resolve(false)}>Use Read instead</button></div>
      </div>}
      {suggestion && !project && draft.project === null && <button className="x-banner x-off" style={{ border: 0, cursor: "pointer", font: "inherit", textAlign: "left" }} onClick={() => setDraft((d) => ({ ...d, project: suggestion.id }))}>
        <Icon name="folder" size={15} /><span>This page belongs to <b>{suggestion.name}</b>. Work in that project?</span></button>}
    </div>
    <div className="x-foot">
      <Box chips={chips} setChips={(c) => setDraft((d) => ({ ...d, chips: c }))} text={draft.text} setText={(text) => setDraft((d) => ({ ...d, text }))}
        images={draft.images} setImages={(images) => setDraft((d) => ({ ...d, images }))}
        quote={draft.quote} clearQuote={() => setDraft((d) => ({ ...d, quote: null }))}
        profile={profile} setProfile={(id) => setDraft((d) => ({ ...d, profile: id }))}
        act={act} setAct={(a, always) => { setDraft((d) => ({ ...d, act: a })); void rememberSites(always, sites).then(() => client.invalidateQueries({ queryKey: ["browser-sites"] })).catch(toastError); }} sites={sites}
        project={project?.id ?? null} setProject={(id) => setDraft((d) => ({ ...d, project: id }))}
        busy={busy} canSend={!!draft.text.trim() && chips.length > 0 && !!profile} placeholder={chips.length ? "Ask about these tabs, or tell me what to do…" : "Add a tab with + Tabs, then ask…"}
        onSend={() => void send()} />
    </div>
  </>;
}

function TaskRow({ s, open }: { s: Session; open: (id: string) => void }) {
  const reviews = useReviews().data ?? [];
  const projects = useProjects().data ?? [];
  const project = projects.find((p) => p.id === s.project_id)?.name;
  const detail = s.quota_pause ? s.quota_pause.message : reviews.some((r) => r.session === s.id) ? "Waiting for your OK" : s.last_message ?? (isRunning(s) ? "Working…" : "");
  const target = s.launch_spec.target;
  return <button className="x-task" onClick={() => open(s.id)}>
    <span style={{ marginTop: 2 }}>{target.kind === "browser" && target.project_id ? <Icon name="folder" size={16} /> : <Icon name="globe" size={16} />}</span>
    <span className="x-d"><b>{s.title}</b><small>{[project, detail].filter(Boolean).join(" · ")}</small></span>
    {isRunning(s) && !taskStatus(s, reviews).tone.startsWith("need") ? <span className="x-spin" style={{ marginTop: 3 }} /> : <StatusPill s={s} reviews={reviews} />}
  </button>;
}

function Tasks({ open }: { open: (id: string) => void }) {
  const sessions = (useSessions().data ?? []).filter(isBrowserTask);
  const reviews = useReviews().data ?? [];
  const needs = sessions.filter((s) => s.status !== "closed" && (taskStatus(s, reviews).tone === "need" || needsYou(s)));
  const running = sessions.filter((s) => !needs.includes(s) && (isRunning(s) || isQuotaPaused(s)));
  const earlier = sessions.filter((s) => !needs.includes(s) && !running.includes(s)).sort((a, b) => b.status_since - a.status_since).slice(0, 20);
  return <div className="x-body" style={{ gap: 0, paddingTop: 0 }}>
    {!sessions.length && <p className="x-faint" style={{ margin: "24px 8px" }}>No browser tasks yet. Start one from New chat.</p>}
    {needs.length > 0 && <><div className="x-sect">Needs you</div>{needs.map((s) => <TaskRow key={s.id} s={s} open={open} />)}</>}
    {running.length > 0 && <><div className="x-sect">Running</div>{running.map((s) => <TaskRow key={s.id} s={s} open={open} />)}</>}
    {earlier.length > 0 && <><div className="x-sect">Earlier</div>{earlier.map((s) => <TaskRow key={s.id} s={s} open={open} />)}</>}
    {sessions.length > 0 && <button className="x-btn x-link x-small" style={{ marginTop: 8 }} onClick={() => openInLaunchpad("/agents")}>All tasks in Launchpad ↗</button>}
  </div>;
}

function Settings() {
  const client = useQueryClient();
  const sites = useSites().data ?? [];
  const agents = useAgents();
  const [profile, setProfile] = useState(prefs().profile ?? agents[0]?.id ?? "");
  const forget = (origin: string) => void api.post("/api/browser/sites/forget", { origin }).then(() => client.invalidateQueries({ queryKey: ["browser-sites"] })).catch(toastError);
  const disconnect = async () => {
    await chrome.storage.local.remove("daemon");
    localStorage.removeItem("launchpad-daemon-origin");
    location.reload();
  };
  return <div className="x-body" style={{ gap: 0 }}>
    <div className="x-sect" style={{ marginLeft: 4 }}>Connection</div>
    <div className="x-setrow"><Icon name="plug" /><span className="x-d"><b style={{ fontWeight: 500 }}>Connected to Launchpad</b><small>{daemonUrl("") || "this computer"}</small></span><button className="x-btn x-sm" onClick={() => void disconnect()}>Disconnect</button></div>
    <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>Default agent</div>
    <div className="x-setrow"><Icon name="agent" /><span className="x-d"><select className="x-field" value={profile} onChange={(e) => { setProfile(e.target.value); savePrefs({ profile: e.target.value }); }}>{agents.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}</select></span></div>
    <div className="x-sect" style={{ marginLeft: 4, marginTop: 16 }}>Sites always allowed to Act</div>
    {!sites.length && <p className="x-small x-faint" style={{ margin: "6px 4px" }}>None. Check “Always allow Act” in the Read/Act menu, or “Always allow” when the agent asks to go to a new site.</p>}
    {sites.map((origin) => <div key={origin} className="x-setrow"><Favicon url={origin} title={hostOf(origin)} /><span className="x-d"><b style={{ fontWeight: 500 }}>{hostOf(origin)}</b><small>{origin}</small></span><button className="x-btn x-sm" onClick={() => forget(origin)}>Remove</button></div>)}
    <p className="x-small x-faint" style={{ margin: "8px 4px" }}>Tasks that share only these sites start in Act, and the agent can go to them without asking. Submitting, paying and other consequential actions always ask.</p>
    <button className="x-btn x-link x-small" style={{ marginTop: 8 }} onClick={() => openInLaunchpad("/settings")}>More settings in Launchpad ↗</button>
  </div>;
}

export async function mount() {
  document.documentElement.classList.add("x-panel");
  const client = new QueryClient({ defaultOptions: { queries: { refetchOnWindowFocus: false, retry: 1, staleTime: 30_000 } } });
  connectEvents(client);
  createRoot(document.getElementById("root")!).render(<QueryClientProvider client={client}><HarnessProvider><Root /></HarnessProvider></QueryClientProvider>);
}
