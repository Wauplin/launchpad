// Composer drawer state. The draft is saved to the daemon, so an unsent prompt survives closing the
// window or restarting the app.

import { create } from "zustand";
import type { AgentTraceRef } from "@/gen/AgentTraceRef";
import type { ContextProject } from "@/gen/ContextProject";
import type { FileRef } from "@/gen/FileRef";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Session } from "@/gen/Session";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { api } from "@/lib/api";
import { draftQuery, useWorkspace } from "./workspace";

export const emptySpec = (): LaunchSpec => ({
  harness: null,
  variant: null,
  budget_usd: null,
  allow_paid_usage: false,
  target: { kind: "none" },
  text: "",
  presets: [],
  context_projects: [],
  github_refs: [],
  files: [],
  attachments: [],
  agent_traces: [],
  model: null,
  effort: null,
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
  rememberLaunch: (spec: LaunchSpec) => void;
  update: (f: (s: LaunchSpec) => LaunchSpec) => void;
  close: () => void;
  setMinimized: (minimized: boolean) => void;
  discard: () => void;
  setOpenAfterLaunch: (v: boolean) => void;
  restore: () => Promise<void>;
};

let saveTimer: ReturnType<typeof setTimeout> | undefined;
function save(spec: LaunchSpec, open: boolean, conditions: LaunchDelay[]) {
  clearTimeout(saveTimer);
  // Each workspace has its own draft; the address is taken now, before a workspace switch.
  const path = `/api/launch/draft${draftQuery()}`;
  const minimized = useComposer.getState().minimized;
  saveTimer = setTimeout(() => void api.put(path, { spec, open, minimized, conditions }).catch(() => {}), 400);
}

/** A switch saves the pending draft of the previous workspace at once, then loads the next one. */
function flush(spec: LaunchSpec, open: boolean, minimized: boolean, conditions: LaunchDelay[], workspace: string | null) {
  clearTimeout(saveTimer);
  return api.put(`/api/launch/draft${workspace ? `?workspace=${encodeURIComponent(workspace)}` : ""}`, { spec, open, minimized, conditions }).catch(() => {});
}

let lastLaunchAt = 0;

export function rememberSuccessfulSession(session: Session) {
  if (session.launch_spec.harness && session.last_seq > 0 && session.status !== "error" && session.started_at > lastLaunchAt) {
    lastLaunchAt = session.started_at;
    useComposer.getState().rememberLaunch(session.launch_spec);
  }
}

let lastLaunch: Pick<LaunchSpec, "harness" | "variant" | "budget_usd" | "model" | "effort"> = { harness: null, variant: null, budget_usd: null, model: null, effort: null };

export const useComposer = create<Composer>((set, get) => ({
  open: false,
  minimized: false,
  spec: emptySpec(),
  conditions: [],
  setConditions: (conditions) => { set({ conditions }); save(get().spec, get().open, conditions); },
  openAfterLaunch: false,
  loaded: false,
  rememberLaunch: (spec) => { lastLaunch = { harness: spec.harness, variant: spec.variant ?? null, budget_usd: spec.budget_usd ?? null, model: spec.model, effort: spec.effort }; },
  start: (ctx) => {
    const cur = get().spec;
    const spec: LaunchSpec = { ...cur, ...lastLaunch };
    if (ctx?.projectId !== undefined) spec.target = ctx.projectId ? { kind: "project", project_id: ctx.projectId } : { kind: "none" };
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
  update: (f) => {
    const spec = f(get().spec);
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
    const { harness, variant, budget_usd } = get().spec;
    const spec = { ...emptySpec(), harness, variant, budget_usd };
    set({ spec, open: false, minimized: false, conditions: [] });
    save(spec, false, []);
  },
  setOpenAfterLaunch: (v) => set({ openAfterLaunch: v }),
  restore: async () => {
    try {
      const sessions = await api.get<Session[]>("/api/sessions");
      const latest = sessions.filter((s) => s.launch_spec.harness && s.last_seq > 0 && s.status !== "error").sort((a, b) => b.started_at - a.started_at)[0];
      if (latest) rememberSuccessfulSession(latest);
      const d = await api.get<{ spec?: LaunchSpec; open?: boolean; minimized?: boolean; conditions?: LaunchDelay[]; delay?: LaunchDelay | null } | null>(`/api/launch/draft${draftQuery()}`);
      if (d?.spec) set({ spec: { ...emptySpec(), ...d.spec }, open: !!d.open, minimized: !!d.open && !!d.minimized, conditions: d.conditions ?? (d.delay ? [d.delay] : []) });
      else set({ spec: { ...emptySpec(), harness: get().spec.harness, variant: get().spec.variant ?? null, budget_usd: get().spec.budget_usd ?? null }, open: false, minimized: false, conditions: [] });
    } finally {
      set({ loaded: true });
    }
  },
}));

useWorkspace.subscribe((s, prev) => {
  if (s.current === prev.current) return;
  const { spec, open, minimized, conditions, loaded } = useComposer.getState();
  void (loaded ? flush(spec, open, minimized, conditions, prev.current) : Promise.resolve()).then(() => useComposer.getState().restore());
});
