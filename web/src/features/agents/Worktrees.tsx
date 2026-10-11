// Task worktrees of a session: the header chip and the Summary next steps share one card with every
// end action (merge, check out locally, pre-draft PR, discard). Git work runs in the daemon.

import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { Session } from "@/gen/Session";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";

export function useSessionWorktrees(sessionId: string) {
  return useQuery({ queryKey: ["worktrees", "session", sessionId], queryFn: () => api.get<WorktreeInfo[]>(`/api/sessions/${sessionId}/worktrees`), refetchInterval: 10_000 });
}

/** One line describing the branch, for chips and settings rows. */
export function worktreeState(i: WorktreeInfo): string {
  const w = i.worktree;
  const base = w.base_branch ?? "base";
  if (w.state === "discarded") return "discarded";
  if (w.state === "predraft") return "pre-draft PR";
  if (w.state === "handed_off") return "checked out locally";
  if (w.merge) return w.merge.phase === "waiting" ? "merge waiting" : w.merge.phase === "conflict" ? "merge conflict" : "merge failed";
  if (w.state === "removed") return "folder removed";
  if (i.dirty_files) return `${i.dirty_files} uncommitted`;
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
    <Popover width={380} trigger={({ toggle }) => (
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

/** Summary → Next steps: one card per worktree of the session. */
export function WorktreeSteps({ session }: { session: Session }) {
  const list = useSessionWorktrees(session.id).data ?? [];
  return <>{list.filter((i) => i.worktree.state !== "discarded" && i.worktree.state !== "predraft").map((i) => (
    <div key={i.worktree.id} className="border-t border-line-2 py-2 first:border-0"><WorktreeCard info={i} session={session} /></div>
  ))}</>;
}

export function WorktreeCard({ info: i, session }: { info: WorktreeInfo; session?: Session }) {
  const w = i.worktree;
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const base = w.base_branch;
  const running = !!session && ["starting", "working", "background"].includes(session.status);
  const act = async (action: string, done?: string) => {
    setBusy(true);
    try {
      const r = await api.post<WorktreeInfo>(`/api/worktrees/${w.id}/${action}`);
      if (done) toast(done);
      return r;
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  const predraft = async () => {
    setBusy(true);
    try {
      const p = await api.post<Predraft>("/api/predrafts", { cwd: w.root, title: session?.title });
      if (session) localStorage.setItem(`launchpad.session-predraft.${session.id}`, p.id);
      await navigate({ to: "/predrafts/$id", params: { id: p.id } });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  let text: string;
  if (w.state === "removed") text = `Clean folder removed by cleanup; the branch keeps the commits. Sending a message restores it.`;
  else if (w.state === "handed_off") text = `Branch checked out in ${w.repo}, so you can run it there.`;
  else if (w.state === "predraft") text = "Promoted to a pre-draft PR.";
  else if (w.state === "discarded") text = "Discarded: folder and branch deleted.";
  else if (w.merge) text = w.merge.reason;
  else {
    const parts = [];
    if (base) parts.push(i.ahead ? `${i.ahead} commit${i.ahead === 1 ? "" : "s"} ahead of ${base}` : w.merged ? `merged into ${w.merged.into}` : `no commit beyond ${base}`);
    if (base && i.behind) parts.push(`${i.behind} behind`);
    if (i.dirty_files) parts.push(`${i.dirty_files} uncommitted file${i.dirty_files === 1 ? "" : "s"}`);
    text = parts.join(" · ") || "Detached worktree.";
  }
  const canMerge = w.state === "active" && !w.merge && !!base && i.ahead > 0;
  return (
    <div className="grid gap-1.5 text-xs">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <Icon name="branch" size={13} className="text-ink-3" />
        <b className="font-mono text-[11.5px]">{w.branch}</b>
        <span className="text-ink-3">{i.project_name}{w.primary ? "" : " · editable context"}</span>
      </div>
      <div className={warn(i) ? "text-attn" : "text-ink-2"}>{text}</div>
      {w.state === "active" && <div className="truncate font-mono text-[11px] text-ink-3" title={i.path}>{i.path}</div>}
      {canMerge && i.target_checkout === null && <div className="text-[11px] text-ink-3">No checkout has {base} checked out: the merge waits until one does.</div>}
      {w.cleanup_note && w.state === "active" && !i.ahead && <div className="text-[11px] text-ink-3">Cleanup: {w.cleanup_note}</div>}
      <div className="flex flex-wrap gap-1.5">
        {canMerge && <button className="btn btn-sm btn-primary" disabled={busy || running || i.dirty_files > 0} title={i.dirty_files ? "Commit or discard the uncommitted files first" : running ? "Wait for the agent to finish" : `Rebase onto ${base} if needed and fast-forward ${base}${i.target_checkout ? ` in ${i.target_checkout}` : ""}. Waits while that checkout has uncommitted changes.`} onClick={() => void act("merge")}>Merge into {base}</button>}
        {w.merge && w.merge.phase !== "waiting" && session && <button className="btn btn-sm btn-primary" disabled={busy} title="Send the problem to the agent: it rebases, resolves and asks to merge again" onClick={() => void act("fix", "Sent to the agent")}>Fix with agent</button>}
        {w.merge && <button className="btn btn-sm" disabled={busy} onClick={() => void act("cancel-merge")}>Cancel merge</button>}
        {w.state === "active" && w.primary && !w.merge && <button className="btn btn-sm" disabled={busy || running} title={`Check out ${w.branch} in ${w.repo} to run it there. Both folders must be clean.`} onClick={() => void act("handoff", "Branch checked out locally")}>Check out locally</button>}
        {w.state === "handed_off" && <button className="btn btn-sm" disabled={busy || running} title="Restore the previous branch in the project checkout and move the agent back to its worktree" onClick={() => void act("handback", "Back in the worktree")}>Move back to worktree</button>}
        {w.state === "active" && w.primary && !w.merge && <button className="btn btn-sm" disabled={busy} title="Turn this branch into a local pre-draft PR, published to GitHub only with your click" onClick={() => void predraft()}>Create pre-draft PR</button>}
        {w.state === "removed" && <button className="btn btn-sm" disabled={busy} onClick={() => void act("restore", "Worktree restored")}>Restore folder</button>}
        {(w.state === "active" || w.state === "removed") && <button className="btn btn-sm text-gh-closed" disabled={busy || running} title={`Delete the folder and the branch, including uncommitted and unmerged work${w.primary ? ", and archive its agents" : ""}`} onClick={() => { if (!confirm) return setConfirm(true); setConfirm(false); void act("discard", "Worktree discarded"); }}>{confirm ? `Confirm: delete ${i.ahead ? `${i.ahead} unmerged commits and ` : ""}the folder` : "Discard"}</button>}
        {confirm && <button className="btn btn-sm" onClick={() => setConfirm(false)}>Cancel</button>}
      </div>
    </div>
  );
}
