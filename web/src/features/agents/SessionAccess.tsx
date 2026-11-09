// Summary → Run → Credentials: the tools and MCP servers granted to the session's workspace, green when the session's
// lease holds them, and its access log, the requests that received a token. Information, not a guard: token scopes
// decide what a request may do. To change an agent's tools, stop it and launch a new one.

import { useQuery } from "@tanstack/react-query";
import type { Access } from "@/gen/Access";
import type { LeaseInfo } from "@/gen/LeaseInfo";
import type { Session } from "@/gen/Session";
import { Token } from "@/components/ui";
import { useWorkspaceTools } from "@/features/settings/ToolDefaults";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { useProjects } from "@/lib/queries";

export function SessionAccess({ session }: { session: Session }) {
  const live = ["starting", "working", "background", "waiting"].includes(session.status);
  const leases = useQuery({ queryKey: ["vault-leases", session.id], queryFn: () => api.get<LeaseInfo[]>(`/api/vault/leases?session_id=${session.id}`), refetchInterval: live ? 30_000 : false });
  const access = useQuery({ queryKey: ["session-access", session.id], queryFn: () => api.get<Access[]>(`/api/sessions/${session.id}/access`), refetchInterval: live ? 15_000 : false });
  const workspaceId = useProjects().data?.find((p) => p.id === session.project_id)?.workspace_id ?? null;
  const { items } = useWorkspaceTools(workspaceId);
  // The latest lease: the current one while the session runs, the last one after it stopped.
  const lease = [...(leases.data ?? [])].sort((a, b) => b.created_at - a.created_at)[0];
  const log = access.data ?? [];
  if (!lease && !log.length) return null;
  const on = new Set(lease?.tools ?? []);
  // Tools of the lease that the workspace no longer grants still show, as on.
  const tools = [...items, ...[...on].filter((k) => !items.some((t) => t.key === k)).map((k) => ({ key: k, name: k, mcp: k.startsWith("mcp:") }))];
  // Model API calls repeat often: one line per host, method and path.
  const grouped = new Map<string, { a: Access; n: number }>();
  for (const a of log) {
    const key = a.action ?? `${a.host} ${a.method} ${a.path}`;
    const g = grouped.get(key);
    if (g) g.n += 1;
    else grouped.set(key, { a, n: 1 });
  }
  return (
    <div className="mt-3 grid gap-2 border-t border-line-2 pt-3 text-[12px]">
      <b className="text-[11px] font-semibold tracking-wider text-ink-3 uppercase">Credentials</b>
      {tools.length > 0 && (
        <span className="flex flex-wrap items-center gap-1">
          {tools.map((t) => on.has(t.key)
            ? <Token key={t.key} tone="teal" icon="check" title={`${t.name}${t.mcp ? " (MCP server)" : ""}: this agent receives its token`}>{t.name}</Token>
            : <Token key={t.key} title={`${t.name}${t.mcp ? " (MCP server)" : ""}: off for this agent`} className="opacity-60">{t.name}</Token>)}
        </span>
      )}
      <span className="text-ink-3">{lease ? `Green: tools this agent can use with Home's tokens. ${lease.revoked_at ? `Lease ended ${ago(lease.revoked_at)}.` : `Lease renewed ${ago(lease.renewed_at)}.`} To change them, stop this agent and launch a new one.` : "This agent holds no lease."}</span>
      {log.length > 0 && (
        <details>
          <summary className="cursor-pointer text-ink-2">Requests that received a token · {log.length}</summary>
          <ul className="mt-1 grid gap-0.5">
            {[...grouped.values()].slice(0, 60).map(({ a, n }) => (
              <li key={`${a.at}-${a.path}`} className="flex gap-2">
                <span className="w-[70px] shrink-0 text-ink-3">{ago(a.at)}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11px]" title={`${a.method} https://${a.host}${a.path} → ${a.status}`}>{a.action ? <b className="font-sans text-[12px]">{a.action}</b> : `${a.method} ${a.host}${a.path}`}</span>
                <span className={a.status >= 400 ? "text-danger-ink" : "text-ink-3"}>{a.status}{n > 1 ? ` · ×${n}` : ""}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}
