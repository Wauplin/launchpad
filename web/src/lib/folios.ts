// Folios: reports, plans and sketchbooks that agents write as Markdown with typed blocks (`/api/folios/{workspace}…`
// on Home, crates/automation/src/folios.rs). The reader (features/folios/) renders them; marks are the owner's
// comments, verdicts, choices and ticks, saved at once and sent with Send feedback.

import { useQuery } from "@tanstack/react-query";
import type { Folio } from "@/gen/Folio";
import type { FolioActivity } from "@/gen/FolioActivity";
import type { FolioDetail } from "@/gen/FolioDetail";
import type { FolioMark } from "@/gen/FolioMark";
import type { FolioRev } from "@/gen/FolioRev";
import type { FolioTarget } from "@/gen/FolioTarget";
import type { FolioRef } from "@/gen/FolioRef";
import { api } from "./api";
import { daemonAsset, daemonUrl } from "./connection";
import type { FolioBlockRequest } from "@/gen/FolioBlockRequest";
import type { FolioVersion } from "@/gen/FolioVersion";

const enc = encodeURIComponent;
export const folioBase = (ws: string) => `/api/folios/${enc(ws)}`;
export const folioPath = (ws: string, slug: string) => `${folioBase(ws)}/${enc(slug)}`;
/** A file of the folio for `<img>`: served sandboxed, never run. */
export const folioAsset = (ws: string, slug: string, path: string, rev?: number | null) => daemonAsset(`${folioPath(ws, slug)}/raw/${path.split("/").map(enc).join("/")}${rev ? `?rev=${rev}` : ""}`);

export const useFolios = (ws: string | null | undefined, archived = false) => useQuery({ queryKey: ["folios", ws, archived], queryFn: () => api.get<Folio[]>(`${folioBase(ws ?? "")}${archived ? "?archived=true" : ""}`), enabled: !!ws, staleTime: 15_000 });
export const useFolio = (ws: string | null | undefined, slug: string | null | undefined, rev?: number | null) => useQuery({ queryKey: ["folio", ws, slug, rev ?? null], queryFn: () => api.get<FolioDetail>(`${folioPath(ws ?? "", slug ?? "")}${rev ? `?rev=${rev}` : ""}`), enabled: !!ws && !!slug, staleTime: 10_000 });
export const useFolioRevs = (ws: string, slug: string, enabled = true) => useQuery({ queryKey: ["folio", ws, slug, "revs"], queryFn: () => api.get<FolioRev[]>(`${folioPath(ws, slug)}/revs`), enabled, staleTime: 10_000 });
export const useFolioDiff = (ws: string, slug: string, rev: number | null) => useQuery({ queryKey: ["folio", ws, slug, "diff", rev], queryFn: () => api.get<{ rev: number; files: { path: string; patch: string; added: number | null; removed: number | null; created: boolean; deleted: boolean }[] }>(`${folioPath(ws, slug)}/revs/${rev}`), enabled: rev != null, staleTime: 60_000 });
export const useFolioTargets = (ws: string, slug: string, enabled = true) => useQuery({ queryKey: ["folio", ws, slug, "targets"], queryFn: () => api.get<FolioTarget[]>(`${folioPath(ws, slug)}/targets`), enabled, staleTime: 10_000 });
/** The folios a session worked on (its Folio tab). */
export const useSessionFolios = (ws: string | null | undefined, session: string | null | undefined) => useQuery({ queryKey: ["folios", ws, "session", session], queryFn: () => api.get<Folio[]>(`${folioBase(ws ?? "")}/_session/${enc(session ?? "")}`), enabled: !!ws && !!session, staleTime: 15_000 });
/** Recent folio writes of the workspace, for receipts in chats and traces. */
export const useFolioActivity = (ws: string | null | undefined) => useQuery({ queryKey: ["folios", ws, "activity"], queryFn: () => api.get<FolioActivity[]>(`${folioBase(ws ?? "")}/_activity`), enabled: !!ws, staleTime: 30_000 });

/** A receipt of a folio write: "created folio Roadmap", "updated folio Roadmap · rev 6 · 2 files". */
export function folioReceipt(a: FolioActivity): string {
  if (a.created) return `created folio ${a.title}`;
  return `updated folio ${a.title} · rev ${a.rev}${a.paths.length > 1 ? ` · ${a.paths.length} files` : ""}`;
}

/** Your marks that Send feedback would send. */
export const unsentMarks = (marks: FolioMark[]) => marks.filter((m) => m.author.kind === "owner" && m.kind !== "reply" && m.sent_at == null);

/** "1 comment, 4 verdicts, 1 decision". */
export function marksSummary(marks: FolioMark[]): string {
  const n = (k: string) => marks.filter((m) => m.kind === k).length;
  const parts: string[] = [];
  for (const [k, one, many] of [["comment", "comment", "comments"], ["verdict", "verdict", "verdicts"], ["choice", "decision", "decisions"], ["check", "tick", "ticks"]] as const) {
    const c = n(k);
    if (c) parts.push(`${c} ${c === 1 ? one : many}`);
  }
  return parts.join(", ") || "nothing yet";
}

export const VERDICTS = [
  { value: "do", label: "Do" },
  { value: "later", label: "Later" },
  { value: "wont", label: "No" },
  { value: "discuss", label: "?" },
] as const;

export const VERDICT_LABEL: Record<string, string> = { do: "Do", later: "Later", wont: "Won't do", discuss: "Discuss" };

/** Text of a YAML value for display (YAML reads numbers and dates as other types). */
export const txt = (v: unknown): string => (v == null ? "" : typeof v === "string" ? v : typeof v === "number" || typeof v === "boolean" ? String(v) : JSON.stringify(v));

/** Adds a folio reference to a context list (the same folio and part only once). */
export function addFolio(list: FolioRef[] | undefined, ref: FolioRef): FolioRef[] {
  const key = (f: FolioRef) => `${f.id}:${f.page ?? ""}:${f.section ?? ""}:${(f.items ?? []).join(",")}`;
  const all = list ?? [];
  return all.some((f) => key(f) === key(ref)) ? all : [...all, ref];
}

/** "Roadmap › plan.md › waves", "Roadmap · W1, F-07": the part of a folio a reference names. */
export function folioRefLabel(f: FolioRef): string {
  const part = f.section ? ` › ${f.section}` : f.items?.length ? ` · ${f.items.join(", ")}` : f.page && f.page !== "index.md" ? ` › ${f.page}` : "";
  return `${f.title || f.slug}${part}`;
}

/** A reference to a whole folio (or one part of it) for a context. */
export const folioRef = (f: Pick<Folio, "id" | "slug" | "title" | "rev">, part: Partial<Pick<FolioRef, "page" | "section" | "items" | "quote" | "rev">> = {}): FolioRef => ({ id: f.id, slug: f.slug, title: f.title, rev: f.rev, ...part });

/** What the renderer reads besides pages: files of the folio (CSV, sketches, images) and its data API. The reader
 * reads them from Home; a shared snapshot from its own folio.json (no data API there). */
export type FolioFiles = {
  text: (path: string) => Promise<string>;
  dataUrl: (path: string) => Promise<string>;
  /** None: a shared snapshot (sketches show their default state). */
  data: { get: (key: string) => Promise<unknown>; set: (key: string, value: unknown) => Promise<void>; delete: (key: string) => Promise<void>; list: (prefix: string) => Promise<string[]> } | null;
};

const keyPath = (key: string) => key.split("/").map(enc).join("/");

/** The reader's files: Home's routes at the rev shown. */
export function homeFiles(ws: string, slug: string, rev: number | null): FolioFiles {
  const base = folioPath(ws, slug);
  const get = async (path: string) => {
    const r = await fetch(daemonUrl(`${base}/file?path=${enc(path)}${rev ? `&rev=${rev}` : ""}`), { credentials: "include" });
    if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
    return r;
  };
  return {
    text: async (path) => (await get(path)).text(),
    dataUrl: async (path) => {
      const r = await fetch(folioAsset(ws, slug, path, rev), { credentials: "include" });
      if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
      const blob = await r.blob();
      return await new Promise<string>((ok, ko) => { const f = new FileReader(); f.onload = () => ok(String(f.result)); f.onerror = () => ko(f.error); f.readAsDataURL(blob); });
    },
    data: {
      get: async (key) => {
        try { return (await api.get<{ value: unknown }>(`${base}/data/${keyPath(key)}`)).value; } catch (e) { if ((e as { status?: number }).status === 404) return undefined; throw e; }
      },
      set: async (key, value) => { await api.put(`${base}/data/${keyPath(key)}`, { value }); },
      delete: async (key) => { await api.delete(`${base}/data/${keyPath(key)}`); },
      list: async (prefix) => (await api.get<{ key: string }[]>(`${base}/data?prefix=${enc(prefix)}`)).map((e) => e.key),
    },
  };
}

/** The block requests of a workspace: blocks and containers outside the catalog that agents use. */
export const useBlockRequests = (ws: string | null | undefined, enabled = true) => useQuery({ queryKey: ["folios", ws, "blocks"], queryFn: () => api.get<FolioBlockRequest[]>(`${folioBase(ws ?? "")}/_blocks`), enabled: !!ws && enabled, staleTime: 60_000 });
export const useFolioVersions = (ws: string, slug: string, enabled = true) => useQuery({ queryKey: ["folio", ws, slug, "versions"], queryFn: () => api.get<FolioVersion[]>(`${folioPath(ws, slug)}/versions`), enabled, staleTime: 10_000 });
