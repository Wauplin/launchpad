// Agents: the workspace's agents that need you first, then the running ones, then the others of the last day. Each
// opens in Launchpad: traces, settings and the rest stay there.

import { useEffect, useState } from "react";
import type { Session } from "@/gen/Session";
import { isRunning, needsYou } from "@/lib/sessionStatus";
import { request, subscribe, type ConnectionState } from "../shared/messages";
import { AgentRow } from "./Item";

const DAY_MS = 86_400_000;

export function Agents({ state }: { state: ConnectionState }) {
  const [list, setList] = useState<Session[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!state.workspace || state.phase !== "online") return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = () => void request("sessions", {}).then((l) => { if (alive) { setList(l); setError(null); } }).catch((e) => alive && setError(e instanceof Error ? e.message : String(e)));
    const later = () => { clearTimeout(timer); timer = setTimeout(load, 400); };
    load();
    const off = subscribe(["sessions"], { event: later, resync: later });
    return () => { alive = false; clearTimeout(timer); off(); };
  }, [state.workspace, state.phase]);

  const open = (path: string) => void request("open", { path: `${path}&ws=${encodeURIComponent(state.workspace ?? "")}` }).catch((e) => setError(e instanceof Error ? e.message : String(e)));
  if (error) return <div className="x-body"><p className="x-error">{error}</p></div>;
  if (!list) return <div className="x-body"><div className="x-line"><span className="x-spin" />Loading…</div></div>;
  const need = list.filter(needsYou);
  const running = list.filter((s) => !needsYou(s) && isRunning(s));
  const recent = list.filter((s) => !needsYou(s) && !isRunning(s) && Date.now() - s.status_since < DAY_MS).slice(0, 15);
  const row = (s: Session) => <AgentRow key={s.id} a={{ session_id: s.id, title: s.title, status: s.status, needs_you: needsYou(s), open_question: s.open_question, bot: s.launch_spec.bot?.name ?? null, since: s.status_since, last_message: s.last_message }} open={open} onError={setError} />;
  return <div className="x-body" style={{ gap: 2 }}>
    {need.length > 0 && <><div className="x-sect">Needs you</div>{need.map(row)}</>}
    {running.length > 0 && <><div className="x-sect">Running</div>{running.map(row)}</>}
    {recent.length > 0 && <><div className="x-sect">Earlier today</div>{recent.map(row)}</>}
    {!need.length && !running.length && !recent.length && <p className="x-muted" style={{ margin: "8px 4px" }}>No agent of this workspace ran in the last day.</p>}
  </div>;
}
