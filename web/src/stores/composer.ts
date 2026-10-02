// Composer drawer state. The draft is saved to the daemon, so an unsent prompt survives closing the
// window or restarting the app.

import { create } from "zustand";
import type { ContextProject } from "@/gen/ContextProject";
import type { FileRef } from "@/gen/FileRef";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { Session } from "@/gen/Session";
import type { LaunchDelay } from "@/gen/LaunchDelay";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import { api } from "@/lib/api";

export const emptySpec = (): LaunchSpec => ({
  harness: null,
  target: { kind: "none" },
  text: "",
  presets: [],
  context_projects: [],
  github_refs: [],
  files: [],
  model: null,
  effort: null,
});

/** What a Launch button starts from: a project, GitHub items, text. */
export type LaunchContext = {
  /** null clears an unrelated draft target when a notification has no mapped project. */
  projectId?: string | null;
  githubRefs?: GitHubRef[];
  contextProjects?: ContextProject[];
  /** Lines from the code viewer. */
  files?: FileRef[];
};

type Composer = {
  open: boolean;
  spec: LaunchSpec;
  delay: LaunchDelay | null;
  setDelay: (delay: LaunchDelay | null) => void;
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
function save(spec: LaunchSpec, open: boolean, delay: LaunchDelay | null) {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void api.put("/api/launch/draft", { spec, open, delay }).catch(() => {}), 400);
}

let lastLaunchAt = 0;

export function rememberSuccessfulSession(session: Session) {
  if (session.last_seq > 0 && session.status !== "error" && session.started_at > lastLaunchAt) {
    lastLaunchAt = session.started_at;
    useComposer.getState().rememberLaunch(session.launch_spec);
  }
}

let lastLaunch: Pick<LaunchSpec, "harness" | "model" | "effort"> = { harness: "claude", model: null, effort: null };

export const useComposer = create<Composer>((set, get) => ({
  open: false,
  spec: emptySpec(),
  delay: null,
  setDelay: (delay) => { set({ delay }); save(get().spec, get().open, delay); },
  openAfterLaunch: false,
  loaded: false,
  rememberLaunch: (spec) => { lastLaunch = { harness: spec.harness, model: spec.model, effort: spec.effort }; },
  start: (ctx) => {
    const cur = get().spec;
    const spec: LaunchSpec = { ...cur, ...lastLaunch };
    if (ctx?.projectId !== undefined) spec.target = ctx.projectId ? { kind: "project", project_id: ctx.projectId } : { kind: "none" };
    for (const r of ctx?.githubRefs ?? []) if (!spec.github_refs.some((g) => g.url === r.url)) spec.github_refs = [...spec.github_refs, r];
    const same = (a: FileRef, b: FileRef) => a.root === b.root && a.rev === b.rev && a.path === b.path && a.start_line === b.start_line && a.end_line === b.end_line;
    for (const f of ctx?.files ?? []) if (!(spec.files ?? []).some((g) => same(f, g))) spec.files = [...(spec.files ?? []), f];
    for (const c of ctx?.contextProjects ?? [])
      if (!spec.context_projects.some((p) => p.project_id === c.project_id)) spec.context_projects = [...spec.context_projects, c];
    set({ open: true, spec });
    save(spec, true, get().delay);
  },
  update: (f) => {
    const spec = f(get().spec);
    set({ spec });
    save(spec, get().open, get().delay);
  },
  close: () => {
    set({ open: false });
    save(get().spec, false, get().delay);
  },
  discard: () => {
    const spec = { ...emptySpec(), harness: get().spec.harness };
    set({ spec, open: false, delay: null });
    save(spec, false, null);
  },
  setOpenAfterLaunch: (v) => set({ openAfterLaunch: v }),
  restore: async () => {
    try {
      const sessions = await api.get<Session[]>("/api/sessions");
      const latest = sessions.filter((s) => s.last_seq > 0 && s.status !== "error").sort((a, b) => b.started_at - a.started_at)[0];
      if (latest) rememberSuccessfulSession(latest);
      const d = await api.get<{ spec?: LaunchSpec; open?: boolean; delay?: LaunchDelay | null } | null>("/api/launch/draft");
      if (d?.spec) set({ spec: { ...emptySpec(), ...d.spec }, open: !!d.open, delay: d.delay ?? null });
    } finally {
      set({ loaded: true });
    }
  },
}));
