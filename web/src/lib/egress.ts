// Exit routes (crates/core/src/egress.rs): URLs of a GitHub organization, or URL prefixes, that leave through one
// runner, the exit runner. Only Home has them; elsewhere the query stays off. The list refreshes on the `egress`
// bus event (lib/ws.ts).

import { useQuery, type useQueryClient } from "@tanstack/react-query";
import type { ExitRoute } from "@/gen/ExitRoute";
import type { ExitRouteState } from "@/gen/ExitRouteState";
import { api } from "./api";
import { useStatus } from "./queries";
import { runAction } from "./actions";
import { useRunners } from "./runners";

/** `GET /api/egress`: the routes as configured and their exits' states. */
export type EgressList = { routes: ExitRoute[]; states: ExitRouteState[] };

export function useEgress() {
  const home = useStatus().data?.role === "home";
  return useQuery({ queryKey: ["egress"], queryFn: () => api.get<EgressList>("/api/egress"), enabled: home, staleTime: Infinity });
}

/** The route a folder's remote takes: a GitHub organization route of its owner, or a URL prefix of its remote. */
export function routeOfFolder(states: ExitRouteState[], repo: string | null | undefined, remote: string | null | undefined): ExitRouteState | undefined {
  const owner = repo?.split("/")[0]?.toLowerCase();
  const url = remote?.toLowerCase() ?? "";
  return states.find((s) => (s.github_org && owner === s.github_org.toLowerCase()) || (!s.github_org && s.urls.some((u) => {
    const p = u.toLowerCase().replace(/\/+$/, "");
    return url === p || url.startsWith(`${p}/`);
  })));
}

/** A route name from an organization or a URL: lowercase letters, digits, `.`, `_` and `-`, at most 40. */
export function routeName(from: string): string {
  let s = from.trim().toLowerCase();
  try { if (/^https?:\/\//.test(s)) s = new URL(s).hostname; } catch { /* Not a URL: the text itself. */ }
  return s.replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40);
}

/** A name not taken by `routes` (adds -2, -3…). */
export function freeName(base: string, routes: ExitRoute[], except?: string): string {
  const taken = new Set(routes.filter((r) => r.name !== except).map((r) => r.name));
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) if (!taken.has(`${base.slice(0, 37)}-${n}`)) return `${base.slice(0, 37)}-${n}`;
}

/** What a route covers, in words: "GitHub organization · 8 prefixes", "2 URL prefixes", "Whole host · 443". */
export function routeKind(s: ExitRouteState): string {
  if (s.kind === "github_org") return `GitHub organization · ${s.prefixes.length} prefixes`;
  if (s.urls.length === 1) {
    try {
      const u = new URL(s.urls[0]);
      if (u.pathname === "/" || u.pathname === "") return `Whole host · ${u.port || (u.protocol === "http:" ? "80" : "443")}`;
    } catch { /* Shown as a prefix below. */ }
    return "1 URL prefix";
  }
  return `${s.urls.length} URL prefixes`;
}

/** The thing a route reaches, as a title: the organization, or the host of its first URL. */
export function routeTarget(r: { github_org: string | null; urls: string[] }): string {
  if (r.github_org) return r.github_org;
  try { return new URL(r.urls[0]).host; } catch { return r.urls[0] ?? ""; }
}

/** Saves the whole list of routes. */
export function saveRoutes(qc: ReturnType<typeof useQueryClient>, routes: ExitRoute[]): Promise<EgressList> {
  return runAction<EgressList>("egress.set", {}, { routes }).then((r) => {
    qc.setQueryData(["egress"], r);
    return r;
  });
}

/** The machine runners an exit can be (never Burst), connected first, plus `current` when it names no runner. */
export function useExitRunners(current?: string): { name: string; online: boolean }[] {
  const runners = (useRunners().data ?? []).filter((r) => r.runner.state === "approved" && !r.runner.burst);
  const out = runners.map((r) => ({ name: r.runner.name, online: r.connected })).sort((a, b) => Number(b.online) - Number(a.online));
  if (current && !out.some((r) => r.name.toLowerCase() === current.toLowerCase() || runners.some((x) => x.runner.id === current))) out.push({ name: current, online: false });
  return out;
}

