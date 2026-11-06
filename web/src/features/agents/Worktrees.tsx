// Task worktrees of a session: the header chip and the Summary next steps share one card with every
// end action (merge on Home, Local PR, discard). Branches live on Home Git; the folder is a disposable clone.

import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { Session } from "@/gen/Session";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { TerminalMenu } from "@/features/terminals/TerminalMenu";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { isPhone } from "@/lib/phone";
import { ActionButton } from "@/components/ActionButton";

export function useSessionWorktrees(sessionId: string) {
  return useQuery({ queryKey: ["worktrees", "session", sessionId], queryFn: () => api.get<WorktreeInfo[]>(`/api/sessions/${sessionId}/worktrees`), refetchInterval: 10_000 });
}

/** The latest session of a worktree: Home relays worktree actions to its runner. */
export const worktreeSession = (w: WorktreeInfo["worktree"]): string | undefined => w.sessions[w.sessions.length - 1];

/** One line describing the branch, for chips and settings rows. */
export function worktreeState(i: WorktreeInfo): string {
  const w = i.worktree;
  const base = w.base_branch ?? "base";
  if (w.state === "discarded") return "discarded";
  if (w.state === "preparing") return "cloning from Home";
  if (w.merge) return w.merge.phase === "waiting" ? "merge waiting" : w.merge.phase === "conflict" ? "merge conflict" : "merge failed";
  if (w.state === "removed") return "folder removed";
  if (i.dirty_files) return `${i.dirty_files} uncommitted`;
  if (w.push_note) return "not on Home";
  if (w.predraft_id) return "Local PR";
  if (i.ahead) return `${i.ahead} ahead of ${base}`;
  if (w.merged) return `merged into ${w.merged.into}`;
  return `same as ${base}`;
}

const warn = (i: WorktreeInfo) => !!i.worktree.merge && i.worktree.merge.phase !== "waiting";

/** The header chip: branch and state, with the worktree card in a popover. */
export function WorktreeChip({ session }: { session: Session }) {
  const q = useSessionWorktrees(session.id);
  const list = q.data ?? [];
  const primary = list.find((i) => i.worktree.primary) ?? list[0];
  if (!primary) return null;
  return (
    <Popover viewport width={380} trigger={({ toggle }) => (
      <button className={`inline-flex max-w-[260px] items-center gap-1 truncate rounded-[5px] border px-1.5 font-mono text-[11px] ${warn(primary) ? "border-attn-line bg-attn-soft text-attn" : "border-line bg-paper text-ink-2 hover:text-ink"}`} title={`Worktree ${primary.path}`} onClick={toggle}>
        <Icon name="branch" size={11} />
        <span className="truncate">{primary.worktree.branch}</span>
        <span className="font-sans text-ink-3">· {worktreeState(primary)}{list.length > 1 ? ` · +${list.length - 1}` : ""}</span>
      </button>
    )}>
      {() => <div className="grid gap-2 p-1">{list.map((i) => <WorktreeCard key={i.worktree.id} info={i} session={session} />)}</div>}
    </Popover>
  );
}

/** Summary → Next steps: one card per worktree of the session. `mainMerge`: the worktree whose Merge is already the
 * page's main action (header or phone bottom bar), so its card leaves it out. */
export function WorktreeSteps({ session, mainMerge }: { session: Session; mainMerge?: string }) {
  const list = useSessionWorktrees(session.id).data ?? [];
  return <>{list.filter((i) => i.worktree.state !== "discarded").map((i) => (
    <div key={i.worktree.id} className="border-t border-line-2 py-2 first:border-0"><WorktreeCard info={i} session={session} merge={i.worktree.id !== mainMerge} /></div>
  ))}</>;
}

/** A worktree's facts and actions, all outline: the session header (or phone bottom bar) holds the one primary. */
export function WorktreeCard({ info: i, session, merge = true }: { info: WorktreeInfo; session?: Session; merge?: boolean }) {
  const w = i.worktree;
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const base = w.base_branch;
  const running = !!session && ["starting", "working", "background"].includes(session.status);
  // On Home, `session` sends the request to the runner that holds the worktree.
  const owner = encodeURIComponent(worktreeSession(w) ?? session?.id ?? "");
  const act = async (action: string, done?: string) => {
    setBusy(true);
    try {
      const r = await api.post<WorktreeInfo>(`/api/worktrees/${w.id}/${action}?session=${owner}`);
      if (done) toast(done);
      return r;
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const predraft = async () => {
    setBusy(true);
    try {
      const p = await api.post<Predraft>(`/api/predrafts?session=${owner}`, { cwd: w.root, title: session?.title });
      if (session) localStorage.setItem(`launchpad.session-predraft.${session.id}`, p.id);
      await navigate({ to: "/predrafts/$id", params: { id: p.id } });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  let text: string;
  if (w.state === "removed") text = `Folder removed after Home saved the branch. Sending a message clones it again.${w.leftovers ? ` ${w.leftovers.files} uncommitted file${w.leftovers.files === 1 ? " is" : "s are"} on ${w.leftovers.branch}.` : ""}`;
  else if (w.state === "preparing") text = "Cloning from Home Git before the agent starts.";
  else if (w.state === "discarded") text = "Discarded: folder and branch deleted. The branch tip stays restorable on Home for 30 days.";
  else if (w.merge) text = w.merge.reason;
  else {
    const parts = [];
    if (base) parts.push(i.ahead ? `${i.ahead} commit${i.ahead === 1 ? "" : "s"} ahead of ${base}` : w.merged ? `merged into ${w.merged.into}` : `no commit beyond ${base}`);
    if (base && i.behind) parts.push(`${i.behind} behind`);
    if (i.dirty_files) parts.push(`${i.dirty_files} uncommitted file${i.dirty_files === 1 ? "" : "s"}`);
    parts.push(i.unpushed ? "new commits go to Home at the end of the turn" : "saved on Home");
    text = parts.join(" · ");
  }
  // Its runner is away: Home shows the branch it holds, and Merge and Create Local PR run on Home.
  const away = i.away;
  const onHome = w.state === "active" || (!!away && w.state === "removed");
  const canMerge = merge && onHome && !w.merge && !!base && i.ahead > 0 && !w.pull_request && !w.predraft_id;
  return (
    <div className="grid gap-1.5 text-xs">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <Icon name="branch" size={13} className="text-ink-3" />
        <b className="font-mono text-[12px]">{w.branch}</b>
        <span className="text-ink-3">{i.project_name}{w.primary ? "" : " · editable context"}</span>
      </div>
      <div className={warn(i) ? "text-attn" : "text-ink-2"}>{text}</div>
      {w.state === "active" && <div className="truncate font-mono text-[11px] text-ink-3" title={i.path}>{i.path}</div>}
      {away && <div className="text-[12px] text-attn">{away}: this is the branch saved on Home. Merge and Create Local PR run on Home.</div>}
      {w.push_note && <div className="text-[12px] text-attn">{w.push_note}</div>}
      {w.merged?.note && <div className="text-[12px] text-ink-2">{w.merged.note}</div>}
      {w.pull_request && <div className="text-[12px] text-ink-2">Continues <a href={w.pull_request.url} target="_blank" rel="noreferrer">#{w.pull_request.number}</a>: Home pushes each new commit to {w.pull_request.head_branch}.</div>}
      {w.cleanup_note && w.state === "active" && !i.ahead && <div className="text-[11px] text-ink-3">Cleanup: {w.cleanup_note}</div>}
      <div className="flex flex-wrap gap-1.5">
        {canMerge && <ActionButton name="worktree.merge" params={{ id: w.id }} vars={{ branch: w.branch, base: base! }} query={{ session: decodeURIComponent(owner) }} disabled={busy || running || i.dirty_files > 0} title={i.dirty_files ? "Commit or discard the uncommitted files first" : running ? "Wait for the agent to finish" : undefined}>Merge into {base}</ActionButton>}
        {w.merge && w.merge.phase !== "waiting" && session && <button className="btn btn-sm" disabled={busy} title="Send the problem to the agent: it rebases, resolves and asks to merge again" onClick={() => void act("fix", "Sent to the agent")}>Fix with agent</button>}
        <TerminalMenu cwd={i.path} small phone={isPhone} align="left" />
        {w.merge && <button className="btn btn-sm" disabled={busy} onClick={() => void act("cancel-merge")}>Cancel merge</button>}
        {onHome && w.primary && !w.merge && !w.pull_request && <button className="btn btn-sm" disabled={busy} title="Turn this branch into a Local PR, published to GitHub only with your click" onClick={() => void predraft()}>Create Local PR</button>}
        {w.state === "removed" && !away && <button className="btn btn-sm" disabled={busy} onClick={() => void act("restore", "Worktree restored")}>Restore folder</button>}
        {(w.state === "active" || w.state === "removed") && !away && <button className="btn btn-sm text-gh-closed" disabled={busy || running} title={`Delete the folder and the branch, including uncommitted work${w.primary ? ", and archive its agents" : ""}. The branch tip stays restorable on Home for 30 days.`} onClick={() => { if (!confirm) return setConfirm(true); setConfirm(false); void act("discard", "Worktree discarded"); }}>{confirm ? `Confirm: delete ${i.ahead ? `${i.ahead} unmerged commits and ` : ""}the folder` : "Discard"}</button>}
        {confirm && <button className="btn btn-sm" onClick={() => setConfirm(false)}>Cancel</button>}
      </div>
    </div>
  );
}
