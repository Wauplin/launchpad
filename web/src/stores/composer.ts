// Composer drawer state. The draft is saved to the daemon, so an unsent prompt survives closing the
// window or restarting the app.

import { create } from "zustand";
import type { ContextProject } from "@/gen/ContextProject";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Session } from "@/gen/Session";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { api } from "@/lib/api";

export const emptySpec = (): LaunchSpec => ({
  harness: null,
  target: { kind: "none" },
  text: "",
  presets: [],
  context_projects: [],
  github_refs: [],
  model: null,
  effort: null,
});

/** What a Launch button starts from: a project, GitHub items, text. */
export type LaunchContext = {
  /** null clears an unrelated draft target when a notification has no mapped project. */
  projectId?: string | null;
  githubRefs?: GitHubRef[];
  contextProjects?: ContextProject[];
};

type Composer = {
  open: boolean;
  spec: LaunchSpec;
  openAfterLaunch: boolean;
  loaded: boolean;
  start: (ctx?: LaunchContext) => void;
  rememberLaunch: (spec: LaunchSpec) => void;
  update: (f: (s: LaunchSpec) => LaunchSpec) => void;
  close: () => void;
  discard: () => void;
  setOpenAfterLaunch: (v: boolean) => void;
  restore: () => Promise<void>;
};

let saveTimer: ReturnType<typeof setTimeout> | undefined;
function save(spec: LaunchSpec, open: boolean) {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void api.put("/api/launch/draft", { spec, open }).catch(() => {}), 400);
}

let lastLaunch: Pick<LaunchSpec, "harness" | "model" | "effort"> = { harness: "claude", model: null, effort: null };

export const useComposer = create<Composer>((set, get) => ({
  open: false,
  spec: emptySpec(),
  openAfterLaunch: false,
  loaded: false,
  rememberLaunch: (spec) => { lastLaunch = { harness: spec.harness, model: spec.model, effort: spec.effort }; },
  start: (ctx) => {
    const cur = get().spec;
    const spec: LaunchSpec = { ...cur, ...lastLaunch };
    if (ctx?.projectId !== undefined) spec.target = ctx.projectId ? { kind: "project", project_id: ctx.projectId } : { kind: "none" };
    for (const r of ctx?.githubRefs ?? []) if (!spec.github_refs.some((g) => g.url === r.url)) spec.github_refs = [...spec.github_refs, r];
    for (const c of ctx?.contextProjects ?? [])
      if (!spec.context_projects.some((p) => p.project_id === c.project_id)) spec.context_projects = [...spec.context_projects, c];
    set({ open: true, spec });
    save(spec, true);
  },
  update: (f) => {
    const spec = f(get().spec);
    set({ spec });
    save(spec, get().open);
  },
  close: () => {
    set({ open: false });
    save(get().spec, false);
  },
  discard: () => {
    const spec = { ...emptySpec(), harness: get().spec.harness };
    set({ spec, open: false });
    save(spec, false);
  },
  setOpenAfterLaunch: (v) => set({ openAfterLaunch: v }),
  restore: async () => {
    try {
      const sessions = await api.get<Session[]>("/api/sessions");
      const latest = sessions.sort((a, b) => b.started_at - a.started_at)[0];
      if (latest) get().rememberLaunch(latest.launch_spec);
      const d = await api.get<{ spec?: LaunchSpec; open?: boolean } | null>("/api/launch/draft");
      if (d?.spec) set({ spec: { ...emptySpec(), ...d.spec }, open: !!d.open });
    } finally {
      set({ loaded: true });
    }
  },
}));
