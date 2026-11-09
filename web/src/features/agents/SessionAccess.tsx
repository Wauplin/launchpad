// Summary → Credentials: the session's lease from Home's vault (with Revoke) and its access log, the requests
// that received a token. Information, not a guard: token scopes decide what a request may do.

import type { ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { Access } from "@/gen/Access";
import type { LeaseInfo } from "@/gen/LeaseInfo";
import type { Session } from "@/gen/Session";
import { LeaseRow } from "@/features/settings/CredentialsView";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";

export function SessionAccess({ session, wrap }: { session: Session; wrap: (body: ReactNode) => ReactNode }) {
  const qc = useQueryClient();
  const live = ["starting", "working", "background", "waiting"].includes(session.status);
  const leases = useQuery({ queryKey: ["vault-leases", session.id], queryFn: () => api.get<LeaseInfo[]>(`/api/vault/leases?session_id=${session.id}`), refetchInterval: live ? 30_000 : false });
  const access = useQuery({ queryKey: ["session-access", session.id], queryFn: () => api.get<Access[]>(`/api/sessions/${session.id}/access`), refetchInterval: live ? 15_000 : false });
  const active = (leases.data ?? []).filter((l) => !l.revoked_at);
  const log = access.data ?? [];
  if (!active.length && !log.length) return null;
  // Model API calls repeat often: one line per host, method and path.
  const grouped = new Map<string, { a: Access; n: number }>();
  for (const a of log) {
    const key = a.action ?? `${a.host} ${a.method} ${a.path}`;
    const g = grouped.get(key);
    if (g) g.n += 1;
    else grouped.set(key, { a, n: 1 });
  }
  return wrap(
    <div className="grid gap-2 text-[12px]">
      {active.map((l) => <LeaseRow key={l.id} lease={l} onDone={() => qc.invalidateQueries({ queryKey: ["vault-leases", session.id] })} />)}
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
