// Code viewer UI state: open tabs, expanded folders and the last
// position of each file, for each source, plus the last place visited in each project. Kept in
// localStorage; nothing goes to the daemon.

import { create } from "zustand";
import { persist } from "zustand/middleware";

/** `session`: the agent session that opened Files, for a link back to it; `pr`: the pull request (`owner/repo#n`) it was opened from. */
export type CodeSearch = { root?: string; rev?: string; path?: string; line?: number; session?: string; pr?: string; view?: "diff" };

/** Route search validation, kept outside CodeView so the router does not load the viewer eagerly. */
export function validateCodeSearch(s: Record<string, unknown>): CodeSearch {
  const str = (v: unknown) => (typeof v === "string" && v ? v : typeof v === "number" ? String(v) : undefined);
  const line = Number(s.line);
  return { root: str(s.root), rev: str(s.rev), path: str(s.path), line: Number.isInteger(line) && line > 0 ? line : undefined, session: str(s.session), pr: str(s.pr), view: s.view === "diff" ? "diff" : undefined };
}

type SourceState = { tabs: string[]; expanded: string[]; pos: Record<string, number> };

type CodeStore = {
  sources: Record<string, SourceState>;
  last: Record<string, CodeSearch>;
  openTab: (key: string, path: string) => void;
  closeTab: (key: string, path: string) => string | null;
  toggle: (key: string, dir: string) => void;
  expand: (key: string, dirs: string[]) => void;
  setPos: (key: string, path: string, line: number) => void;
  setLast: (projectId: string, s: CodeSearch) => void;
};

export const sourceKey = (root: string, rev: string | null | undefined) => `${root}\n${rev ?? ""}`;

const empty = (): SourceState => ({ tabs: [], expanded: [], pos: {} });

export const useCode = create<CodeStore>()(
  persist(
    (set, get) => {
      const edit = (key: string, f: (s: SourceState) => SourceState) => set({ sources: { ...get().sources, [key]: f(get().sources[key] ?? empty()) } });
      return {
        sources: {},
        last: {},
        openTab: (key, path) => edit(key, (s) => (s.tabs.includes(path) ? s : { ...s, tabs: [...s.tabs, path].slice(-20) })),
        closeTab: (key, path) => {
          const tabs = get().sources[key]?.tabs ?? [];
          const i = tabs.indexOf(path);
          const rest = tabs.filter((t) => t !== path);
          edit(key, (s) => ({ ...s, tabs: rest }));
          return rest[Math.min(i, rest.length - 1)] ?? null;
        },
        toggle: (key, dir) => edit(key, (s) => ({ ...s, expanded: s.expanded.includes(dir) ? s.expanded.filter((d) => d !== dir) : [...s.expanded, dir] })),
        expand: (key, dirs) =>
          edit(key, (s) => {
            const add = dirs.filter((d) => !s.expanded.includes(d));
            return add.length ? { ...s, expanded: [...s.expanded, ...add] } : s;
          }),
        setPos: (key, path, line) =>
          edit(key, (s) => {
            const pos = { ...s.pos, [path]: line };
            const keys = Object.keys(pos);
            if (keys.length > 200) delete pos[keys[0]];
            return { ...s, pos };
          }),
        setLast: (projectId, s) => set({ last: { ...get().last, [projectId]: s } }),
      };
    },
    { name: "launchpad.code" },
  ),
);
