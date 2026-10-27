// Composer drawer state. The draft is saved to the daemon, so an unsent prompt survives closing the
// window or restarting the app. Several windows and devices share each workspace draft: saves carry the
// revision they start from, the daemon rejects stale ones (409), and `drafts/changed` events reload idle
// windows. A window that is being edited shows a conflict notice instead of losing either version.

import { isExtension } from "../lib/connection.ts";
import { storageAllowed } from "../lib/clientStorage.ts";
import { useSaveStatus } from "../lib/saveStatus.ts";
import { create } from "zustand";
import type { AgentTraceRef } from "@/gen/AgentTraceRef";
import type { ContextProject } from "@/gen/ContextProject";
import type { FileRef } from "@/gen/FileRef";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { ApiError, api } from "../lib/api.ts";
import { useWorkspace } from "./workspace.ts";

export const emptySpec = (): LaunchSpec => ({
  harness: null,
  profile: null,
  budget_usd: null,
  allow_paid_usage: false,
  target: { kind: "none" },
  // New drafts run in a new task worktree; "Project checkout" sets this to null.
  worktree: { base: null, include_changes: false, merge_when_done: false },
  text: "",
  presets: [],
  context_projects: [],
  github_refs: [],
  files: [],
  attachments: [],
  agent_traces: [],
  model: null,
  effort: null,
  permission_mode: "yolo",
  workspace_id: null,
});

/** What a Launch button starts from: a project, GitHub items, text. */
export type LaunchContext = {
  /** null clears an unrelated draft target when a notification has no mapped project. */
  projectId?: string | null;
  githubRefs?: GitHubRef[];
  contextProjects?: ContextProject[];
  /** Lines from the code viewer. */
  files?: FileRef[];
  agentTraces?: AgentTraceRef[];
};

type Composer = {
  /** A draft is active: shown as the drawer, or as the draft pill when `minimized`. */
  open: boolean;
  minimized: boolean;
  spec: LaunchSpec;
  /** Start conditions; empty means launch now. */
  conditions: LaunchDelay[];
  setConditions: (conditions: LaunchDelay[]) => void;
  openAfterLaunch: boolean;
  loaded: boolean;
  start: (ctx?: LaunchContext) => void;
  followUpFrom: (session: Session) => void;
  rememberLaunch: (spec: LaunchSpec, startedAt: number) => void;
  update: (f: (s: LaunchSpec) => LaunchSpec) => void;
  close: () => void;
  setMinimized: (minimized: boolean) => void;
  discard: () => void;
  setOpenAfterLaunch: (v: boolean) => void;
  restore: () => Promise<void>;
  /** Revision of the saved draft this window started from (0: none saved yet). */
  revision: number;
  /** Another window or device saved a newer draft while this one was being edited. Saves pause until resolved. */
  conflict: boolean;
  /** Replaces this window's draft with the newer saved one. */
  loadNewer: () => Promise<void>;
  /** Overwrites the newer saved draft with this window's draft. */
  keepMine: () => void;
  /** A `drafts/changed` bus event (`workspace` null: the default draft). */
  remoteChanged: (ev: { workspace: string | null; revision: number; client: string | null }) => void;
};

type SavedDraft = { spec?: LaunchSpec; open?: boolean; minimized?: boolean; conditions?: LaunchDelay[]; delay?: LaunchDelay | null; revision?: number } | null;

/** Identifies this window in `drafts/changed` events, so it ignores its own saves. */
const clientId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
const draftScope = (workspace: string | null) => isExtension ? `chrome-extension:${localStorage.getItem("launchpad-extension-device") ?? "unpaired"}:${workspace ?? "default"}` : workspace;
const draftPath = (workspace: string | null) => `/api/launch/draft${draftScope(workspace) ? `?workspace=${encodeURIComponent(draftScope(workspace)!)}` : ""}`;

let saveTimer: ReturnType<typeof setTimeout> | undefined;
/** When this window last changed its draft: a newer remote draft does not replace it during editing. */
let lastLocalEdit = 0;
const EDITING_MS = 5_000;
/** Saves run one after another, so each one starts from the revision the previous one returned. */
let saving: Promise<unknown> = Promise.resolve();

type DraftBody = { spec: LaunchSpec; open: boolean; minimized: boolean; conditions: LaunchDelay[] };
type Unsent = { workspace: string | null; body: DraftBody; revision: number; force: boolean; conflict?: boolean };
const pendingKey = `launchpad-composer-pending:${sessionStorage.getItem("launchpad-composer-client") ?? clientId}`;
sessionStorage.setItem("launchpad-composer-client", pendingKey.split(":").at(-1)!);
const unsent = new Map<string | null, Unsent>();
try { for (const entry of JSON.parse(localStorage.getItem(pendingKey) ?? "[]") as Unsent[]) unsent.set(entry.workspace, entry); } catch { /* Unreadable backup. */ }
function backupDrafts() {
  if (!storageAllowed()) return;
  try {
    if (unsent.size) localStorage.setItem(pendingKey, JSON.stringify([...unsent.values()].map((entry) => ({ ...entry, force: false }))));
    else localStorage.removeItem(pendingKey);
    useSaveStatus.getState().set("composer-storage");
  } catch { useSaveStatus.getState().set("composer-storage", "Local draft backup unavailable. Keep this window open until saved."); }
}
function retryDraft(workspace: string | null) {
  const run = async () => {
    while (storageAllowed()) {
      const entry = unsent.get(workspace);
      if (!entry || entry.conflict) return;
      const same = () => (useWorkspace.getState().current ?? null) === workspace;
      try {
        const res = await api.put<{ revision: number }>(draftPath(workspace), { ...entry.body, base_revision: entry.revision, force: entry.force, client: clientId });
        if (same()) useComposer.setState({ revision: res.revision, conflict: false });
        if (unsent.get(workspace) === entry) unsent.delete(workspace);
        else { const next = unsent.get(workspace)!; next.revision = res.revision; next.force = false; }
        backupDrafts();
        useSaveStatus.getState().set(`composer:${workspace}`);
      } catch (e) {
        if (!storageAllowed()) return;
        const latest = unsent.get(workspace)!;
        // A forced save may already have succeeded. Any retry must compare revisions again.
        latest.force = false;
        if (e instanceof ApiError && e.status === 409) {
          latest.conflict = true;
          if (same()) useComposer.setState({ conflict: true });
          useSaveStatus.getState().set(`composer:${workspace}`, "Composer draft changed elsewhere. Open its workspace to Load the newer draft or Keep mine.");
        } else useSaveStatus.getState().set(`composer:${workspace}`, "Composer draft not saved to server. Kept on this device; retrying.");
        backupDrafts();
        return;
      }
    }
  };
  saving = saving.then(run, run);
  return saving;
}
function put(workspace: string | null, body: DraftBody, force = false) {
  const previous = unsent.get(workspace);
  unsent.set(workspace, { workspace, body, revision: previous?.revision ?? useComposer.getState().revision, force, conflict: !force && previous?.conflict });
  backupDrafts();
  return retryDraft(workspace);
}

function save(spec: LaunchSpec, open: boolean, conditions: LaunchDelay[]) {
  clearTimeout(saveTimer);
  lastLocalEdit = Date.now();
  const workspace = useWorkspace.getState().current ?? null;
  const st = useComposer.getState();
  const previous = unsent.get(workspace);
  unsent.set(workspace, { workspace, body: { spec, open, minimized: st.minimized, conditions }, revision: previous?.revision ?? st.revision, force: false, conflict: st.conflict });
  backupDrafts();
  saveTimer = setTimeout(() => { saveTimer = undefined; void retryDraft(workspace); }, 400);
}

function flush(workspace: string | null) {
  clearTimeout(saveTimer);
  saveTimer = undefined;
  return retryDraft(workspace);
}
const retryAll = () => { for (const workspace of unsent.keys()) void retryDraft(workspace); };
window.addEventListener("online", retryAll);
window.addEventListener("launchpad-reconnected", retryAll);
window.addEventListener("pagehide", retryAll);
setInterval(retryAll, 10_000);

async function loadDraft(discardLocal = false) {
  const workspace = useWorkspace.getState().current ?? null;
  let d: SavedDraft;
  const pending = unsent.get(workspace);
  if (pending && !discardLocal) {
    useComposer.setState({ ...pending.body, revision: pending.revision, conflict: !!pending.conflict });
    void retryDraft(workspace);
    return;
  }
  d = await api.get<SavedDraft>(draftPath(workspace));
  if ((useWorkspace.getState().current ?? null) !== workspace) return;
  clearTimeout(saveTimer);
  saveTimer = undefined;
  if (discardLocal) { unsent.delete(workspace); backupDrafts(); useSaveStatus.getState().set(`composer:${workspace}`); }
  const revision = d?.revision ?? 0;
  if (d?.spec) useComposer.setState({ spec: { ...emptySpec(), ...d.spec }, open: !!d.open, minimized: !!d.open && !!d.minimized, conditions: d.conditions ?? (d.delay ? [d.delay] : []), revision, conflict: false });
  else {
    const { spec } = useComposer.getState();
    useComposer.setState({ spec: { ...emptySpec(), harness: spec.harness, profile: spec.profile ?? null, budget_usd: spec.budget_usd ?? null }, open: false, minimized: false, conditions: [], revision, conflict: false });
  }
}

const projectCompletionDefaults = new Map<string, boolean>();
export function rememberProjectDefaults(projects: Project[]) {
  projectCompletionDefaults.clear();
  for (const project of projects) projectCompletionDefaults.set(project.id, project.merge_when_done);
}
export function rememberProjectCompletion(projectId: string, merge: boolean) {
  projectCompletionDefaults.set(projectId, merge);
}
function withProjectCompletion(spec: LaunchSpec): LaunchSpec {
  if (spec.target.kind !== "project" || !spec.worktree) return spec;
  return { ...spec, worktree: { ...spec.worktree, merge_when_done: projectCompletionDefaults.get(spec.target.project_id) ?? false } };
}

type RunDefaults = Pick<LaunchSpec, "model" | "effort" | "permission_mode">;
const runs = new Map<string, { startedAt: number; defaults: RunDefaults }>();
const runKey = (harness: LaunchSpec["harness"], profile: LaunchSpec["profile"]) => JSON.stringify([harness, profile ?? null]);

// Remember a deliberate permission selection immediately, including before the next launch.
const permissionChoices: Record<string, { mode: LaunchSpec["permission_mode"]; selectedAt: number }> = (() => {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem("launchpad-permission-modes") ?? "{}");
    if (!saved || typeof saved !== "object" || Array.isArray(saved)) return {};
    return Object.fromEntries(Object.entries(saved).filter(([, value]) => value && (value.mode === "yolo" || value.mode === "approvals" || value.mode === "auto") && Number.isFinite(value.selectedAt)));
  } catch { return {}; }
})();

/** Latest successful launch for this harness/profile, across all projects and workspaces. */
export function rememberedRun(harness: LaunchSpec["harness"], profile: LaunchSpec["profile"]): RunDefaults {
  const key = runKey(harness, profile);
  const run = runs.get(key)?.defaults ?? { model: null, effort: null, permission_mode: "yolo" as const };
  const selected = permissionChoices[key];
  return { ...run, permission_mode: selected && selected.selectedAt > (runs.get(key)?.startedAt ?? 0) ? selected.mode : run.permission_mode };
}

let lastLaunchAt = 0;
let lastLaunch: Pick<LaunchSpec, "harness" | "profile" | "budget_usd" | "model" | "effort" | "permission_mode"> = { harness: null, profile: null, budget_usd: null, model: null, effort: null, permission_mode: "yolo" };

// Ask Launchpad threads keep their own profile/model (`[assistant]` in config), apart from the composer's defaults.
export function rememberSuccessfulSession(session: Session) {
  if (session.launch_spec.harness && session.launch_spec.target.kind !== "assistant" && session.last_seq > 0 && session.status !== "error") {
    useComposer.getState().rememberLaunch(session.launch_spec, session.started_at);
  }
}

export const useComposer = create<Composer>((set, get) => ({
  open: false,
  minimized: false,
  spec: emptySpec(),
  conditions: [],
  setConditions: (conditions) => { set({ conditions }); save(get().spec, get().open, conditions); },
  openAfterLaunch: false,
  loaded: false,
  rememberLaunch: (spec, startedAt) => {
    if (!spec.harness || spec.target.kind === "assistant") return;
    const key = runKey(spec.harness, spec.profile);
    if (startedAt > (runs.get(key)?.startedAt ?? 0)) runs.set(key, { startedAt, defaults: { model: spec.model, effort: spec.effort, permission_mode: spec.permission_mode ?? "yolo" } });
    if (startedAt > lastLaunchAt) {
      lastLaunchAt = startedAt;
      lastLaunch = { harness: spec.harness, profile: spec.profile ?? null, budget_usd: spec.budget_usd ?? null, model: spec.model, effort: spec.effort, permission_mode: spec.permission_mode ?? "yolo" };
    }
  },
  start: (ctx) => {
    const cur = get().spec;
    let spec: LaunchSpec = get().open ? { ...cur } : { ...cur, ...lastLaunch, permission_mode: rememberedRun(lastLaunch.harness, lastLaunch.profile).permission_mode };
    if (ctx?.projectId !== undefined) spec.target = ctx.projectId ? { kind: "project", project_id: ctx.projectId } : { kind: "none" };
    if (!get().open || JSON.stringify(spec.target) !== JSON.stringify(cur.target)) spec = withProjectCompletion(spec);
    for (const r of ctx?.githubRefs ?? []) if (!spec.github_refs.some((g) => g.url === r.url)) spec.github_refs = [...spec.github_refs, r];
    const same = (a: FileRef, b: FileRef) => a.root === b.root && a.rev === b.rev && a.path === b.path && a.start_line === b.start_line && a.end_line === b.end_line;
    for (const f of ctx?.files ?? []) if (!(spec.files ?? []).some((g) => same(f, g))) spec.files = [...(spec.files ?? []), f];
    for (const c of ctx?.contextProjects ?? [])
      if (!spec.context_projects.some((p) => p.project_id === c.project_id)) spec.context_projects = [...spec.context_projects, c];
    for (const trace of ctx?.agentTraces ?? []) if (!(spec.agent_traces ?? []).some((t) => t.session_id === trace.session_id)) spec.agent_traces = [...(spec.agent_traces ?? []), trace];
    // Adding context items to a minimized draft keeps it minimized (the pill flashes); Launch buttons open it.
    const collect = !!ctx && ctx.projectId === undefined && !!(ctx.githubRefs?.length || ctx.files?.length || ctx.contextProjects?.length || ctx.agentTraces?.length);
    set({ open: true, minimized: get().open && get().minimized && collect, spec });
    save(spec, true, get().conditions);
  },
  followUpFrom: (session) => {
    const spec: LaunchSpec = {
      ...emptySpec(), ...lastLaunch, permission_mode: rememberedRun(lastLaunch.harness, lastLaunch.profile).permission_mode,
      target: session.launch_spec.target.kind === "browser" ? { ...session.launch_spec.target, source_session_id: session.id, grants: [] } : { kind: "session_folder", session_id: session.id },
      worktree: null,
      agent_traces: session.last_seq > 0 ? [{ session_id: session.id, through_seq: session.last_seq }] : [],
    };
    set({ spec, open: true, minimized: false, conditions: [] });
    save(spec, true, []);
  },
  update: (f) => {
    const previous = get().spec;
    let spec = f(previous);
    if (spec.permission_mode !== previous.permission_mode && spec.harness === previous.harness && spec.profile === previous.profile) {
      permissionChoices[runKey(spec.harness, spec.profile)] = { mode: spec.permission_mode, selectedAt: Date.now() };
      try { localStorage.setItem("launchpad-permission-modes", JSON.stringify(permissionChoices)); } catch { /* Session history still retains launched choices. */ }
    }
    if (JSON.stringify(spec.target) !== JSON.stringify(previous.target) || (!previous.worktree && spec.worktree)) spec = withProjectCompletion(spec);
    set({ spec });
    save(spec, get().open, get().conditions);
  },
  close: () => {
    set({ open: false, minimized: false });
    save(get().spec, false, get().conditions);
  },
  setMinimized: (minimized) => {
    set({ minimized });
    save(get().spec, get().open, get().conditions);
  },
  discard: () => {
    const { harness, profile, budget_usd } = get().spec;
    const spec = { ...emptySpec(), harness, profile, budget_usd, ...rememberedRun(harness, profile) };
    set({ spec, open: false, minimized: false, conditions: [] });
    save(spec, false, []);
  },
  setOpenAfterLaunch: (v) => set({ openAfterLaunch: v }),
  restore: async () => {
    try {
      const [sessions, projects] = await Promise.all([api.get<Session[]>("/api/sessions"), api.get<Project[]>("/api/projects")]);
      rememberProjectDefaults(projects);
      for (const session of sessions) rememberSuccessfulSession(session);
    } catch { /* Defaults can refresh later; recover local drafts even while offline. */ }
    try { await loadDraft(); }
    finally { set({ loaded: true }); }
  },
  revision: 0,
  conflict: false,
  loadNewer: async () => {
    await saving;
    await loadDraft(true).catch(() => {});
  },
  keepMine: () => {
    const { spec, open, minimized, conditions } = get();
    set({ conflict: false });
    clearTimeout(saveTimer);
    saveTimer = undefined;
    void put(useWorkspace.getState().current ?? null, { spec, open, minimized, conditions }, true);
  },
  remoteChanged: (ev) => {
    if (ev.client === clientId || !get().loaded) return;
    if ((ev.workspace ?? null) !== draftScope(useWorkspace.getState().current ?? null) || ev.revision <= get().revision) return;
    // Being edited here (unsaved change, recent typing or an open conflict): ask; otherwise follow the other window.
    const editing = unsent.has(useWorkspace.getState().current ?? null) || saveTimer !== undefined || Date.now() - lastLocalEdit < EDITING_MS || !!get().conflict;
    if (editing && get().open) set({ conflict: true });
    else void get().loadNewer();
  },
}));

useWorkspace.subscribe((s, prev) => {
  if (s.current === prev.current) return;
  const { loaded } = useComposer.getState();
  void (loaded ? flush(prev.current) : Promise.resolve()).then(() => useComposer.getState().restore());
});
