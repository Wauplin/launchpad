import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { create } from "zustand";
import type { Thread } from "@/gen/Thread";
import type { ThreadPreview } from "@/gen/ThreadPreview";
import type { InboxSnapshot } from "@/gen/InboxSnapshot";
import type { Project } from "@/gen/Project";
import { Markdown } from "@/components/Markdown";
import { toast, toastError } from "@/components/Toast";
import { LaunchButton } from "@/features/launch/Radial";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useProjects } from "@/lib/queries";

export const useInbox = () => useQuery({ queryKey: ["inbox"], queryFn: () => api.get<InboxSnapshot>("/api/inbox"), staleTime: Infinity });
const useView = create<{ selected: string | null; repo: string | null; unread: boolean; scroll: number; drafts: Record<string, string> }>(() => ({ selected: null, repo: null, unread: false, scroll: 0, drafts: {} }));
function launchContext(t: Thread, projects: Project[]) {
  const project = projects.find((p) => p.gh_repo?.toLowerCase() === t.repo.toLowerCase()) ?? projects.find((p) => p.gh_upstream?.toLowerCase() === t.repo.toLowerCase());
  return { projectId: project?.id ?? null, githubRefs: [{ repo: t.repo, number: t.number, title: t.title, url: t.url, kind: t.kind === "PullRequest" ? "pr" : t.kind === "Issue" ? "issue" : t.kind.toLowerCase() }] };
}
const labels: Record<string, string> = { review_requested: "Review requested", mention: "Mentioned", team_mention: "Team mention", assign: "Assigned", ci_activity: "CI activity", state_change: "State changed" };
const reason = (s: string) => labels[s] ?? s.replaceAll("_", " ");
type Action = { id: string; action: string };
function useAction() {
  const qc = useQueryClient();
  return useMutation({
    mutationKey: ["inbox-action"],
    mutationFn: ({ id, action }: Action) => api.post(`/api/inbox/${id}/action`, { action }),
    onMutate: async ({ id, action }) => {
      await qc.cancelQueries({ queryKey: ["inbox"] });
      const prior = qc.getQueryData<InboxSnapshot>(["inbox"])?.threads.find((t) => t.id === id);
      qc.setQueryData<InboxSnapshot>(["inbox"], (s) => s && ({ ...s, threads: action === "done" || action === "unsubscribe" ? s.threads.filter((t) => t.id !== id) : s.threads.map((t) => t.id === id ? { ...t, unread: action === "unread", local_unread: action === "unread" } : t) }));
      return prior;
    },
    onError: (e, { id }, prior) => {
      if (prior) qc.setQueryData<InboxSnapshot>(["inbox"], (s) => s && ({ ...s, threads: [...s.threads.filter((t) => t.id !== id), prior].sort((a, b) => b.updated_at.localeCompare(a.updated_at)) }));
      toastError(e);
    },
    onSettled: () => void qc.invalidateQueries({ queryKey: ["inbox"] }),
  });
}
const ThreadRow = memo(function ThreadRow({ t, selected, projects, select, act, busy }: { t: Thread; selected: boolean; projects: Project[]; select: (t: Thread) => void; act: (a: Action) => void; busy: boolean }) {
  return <div className={`group relative flex h-[70px] items-center gap-2 border-b border-line-2 px-3 ${selected ? "bg-select" : "hover:bg-chrome"}`}>
    <button className="flex min-w-0 flex-1 items-start gap-2 text-left" onClick={() => select(t)} title={t.title}>
      <span className={`mt-1.5 size-2 flex-none rounded-full ${t.unread ? "bg-work" : "bg-transparent"}`} />
      <span className="min-w-0 flex-1">
        <span className={`block truncate ${t.unread ? "font-semibold" : "text-ink-2"}`}>{t.title}</span>
        <span className="mt-1 flex items-center gap-2 text-[11px] text-ink-3">
          <span className="truncate">{t.repo}{t.number !== null && ` #${t.number}`}</span>
          {t.state && <span className={`flex-none ${t.state === "merged" ? "text-gh-merged" : t.state === "open" ? "text-gh-open" : ""}`}>{t.state}</span>}
          <span className="flex-none text-ink-2">{reason(t.reason)}</span>
          {t.participant_count > 0 && <span className="truncate" title={t.participants.join(", ")}>{t.participants.slice(0, 2).join(", ")}{t.participant_count > 2 && ` +${t.participant_count - 2}`}</span>}
          <span className="ml-auto flex-none">{formatAge(Date.parse(t.updated_at))}</span>
          {t.local_unread && <span title="Unread only in Launchpad">local</span>}
        </span>
      </span>
    </button>
    <div className="absolute right-11 z-10 flex items-center gap-1 rounded-lg bg-chrome p-1 opacity-0 pointer-events-none group-hover:pointer-events-auto group-focus-within:pointer-events-auto group-hover:opacity-100 group-focus-within:opacity-100" onClick={(e) => e.stopPropagation()}>
      <button className="btn btn-sm" title="Mark as done" disabled={busy} onClick={() => act({ id: t.id, action: "done" })}>Done</button>
      <button className="btn btn-sm" title="Ignore future notifications and mark as done" disabled={busy} onClick={() => act({ id: t.id, action: "unsubscribe" })}>Unsubscribe</button>
      <button className="btn btn-sm" title={t.unread ? "Mark as read" : "Mark as unread locally (github.com stays read)"} disabled={busy} onClick={() => act({ id: t.id, action: t.unread ? "read" : "unread" })}>{t.unread ? "Read" : "Unread"}</button>
    </div>
    <LaunchButton small primary={false} label={null} ctx={launchContext(t, projects)} />
  </div>;
});
function RepoChips({ threads, selected }: { threads: Thread[]; selected: string | null }) {
  const ref = useRef<HTMLDivElement>(null);
  const repos = useMemo(() => {
    const counts = new Map<string, number>();
    threads.forEach((t) => counts.set(t.repo, (counts.get(t.repo) ?? 0) + 1));
    return [...counts].sort(([a], [b]) => a.localeCompare(b));
  }, [threads]);
  const v = useVirtualizer({ count: repos.length, horizontal: true, getScrollElement: () => ref.current, estimateSize: () => 184, overscan: 2, useFlushSync: false });
  return <div className="flex h-10 flex-none items-center gap-2 border-b border-line px-3">
    <button className={`btn btn-sm ${selected === null ? "bg-select!" : ""}`} onClick={() => useView.setState({ repo: null, scroll: 0 })}>All {threads.length}</button>
    <div ref={ref} className="min-w-0 flex-1 overflow-x-auto">
      <div className="relative h-7" style={{ width: v.getTotalSize() }}>
        {v.getVirtualItems().map((item) => { const [repo, count] = repos[item.index]; return <button key={repo} title={repo} className={`btn btn-sm absolute top-0 w-[178px] ${selected === repo ? "bg-select!" : ""}`} style={{ left: item.start }} onClick={() => useView.setState({ repo, scroll: 0 })}><span className="truncate">{repo}</span><span className="ml-auto">{count}</span></button>; })}
      </div>
    </div>
  </div>;
}
function PreviewPane({ t, projects }: { t: Thread; projects: Project[] }) {
  const qc = useQueryClient();
  const preview = useQuery({ queryKey: ["inbox-preview", t.id], queryFn: () => api.get<ThreadPreview | null>(`/api/inbox/${t.id}/preview`), staleTime: Infinity });
  const draft = useView((s) => s.drafts[t.id] ?? "");
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const refresh = useMutation({ mutationFn: (force: boolean) => api.post(`/api/inbox/${t.id}/preview`, { force }), onSuccess: () => { setRefreshError(null); void qc.invalidateQueries({ queryKey: ["inbox-preview", t.id] }); }, onError: (e) => setRefreshError(String(e)) });
  const reply = useMutation({ mutationFn: () => api.post(`/api/inbox/${t.id}/reply`, { text: draft }), onSuccess: () => { useView.setState((s) => ({ drafts: { ...s.drafts, [t.id]: "" } })); toast("Reply posted"); void qc.invalidateQueries({ queryKey: ["inbox-preview", t.id] }); }, onError: toastError });
  const supported = t.kind === "Issue" || t.kind === "PullRequest";
  const hasSummary = supported || t.kind === "Release" || t.kind === "Commit";
  useEffect(() => { if (hasSummary) refresh.mutate(false); }, [t.id, t.updated_at]); // Refresh in the background while the cached summary renders.
  const p = preview.data;
  return <section className="flex min-h-0 w-[40%] min-w-[320px] flex-col border-l border-line bg-paper">
    <div className="flex-none border-b border-line p-4">
      <div className="text-xs text-ink-3">{t.repo}{t.number !== null && ` #${t.number}`} · {t.kind} · {t.state ?? ""}</div>
      <h2 className="mt-2 mb-3 text-[16px] font-semibold">{t.title}</h2>
      <div className="flex flex-wrap items-center gap-2">
        <LaunchButton small ctx={launchContext(t, projects)} />
        <button className="btn btn-sm" onClick={() => void api.post("/api/open", { url: t.url }).catch(toastError)}>Open on GitHub</button>
        {hasSummary && <button className="btn btn-sm" disabled={refresh.isPending} onClick={() => refresh.mutate(true)}>{refresh.isPending ? "Refreshing…" : "Refresh"}</button>}
        <button className="btn btn-sm ml-auto" onClick={() => useView.setState({ selected: null })}>Close</button>
      </div>
      {(t.review_decision || t.checks) && <div className="mt-2 text-xs text-ink-2">{[t.review_decision && `Review: ${reason(t.review_decision.toLowerCase())}`, t.checks && `Checks: ${t.checks.toLowerCase()}`].filter(Boolean).join(" · ")}</div>}
    </div>
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      {refreshError && <p className="text-gh-closed">{refreshError}</p>}
      {preview.isError && <p className="text-gh-closed">{String(preview.error)}</p>}
      {p ? <>
        <div className="mb-2 text-xs text-ink-3">{p.author} · summary cached {formatAge(p.fetched_at)} ago</div>
        <Markdown text={p.body || "No description."} />
        <h3 className="mt-6 mb-3 text-xs font-semibold text-ink-2">Latest comments (up to 8)</h3>
        {p.comments.map((c) => <article key={c.url} className="mb-4 border-t border-line pt-3"><div className="mb-2 text-xs text-ink-3">{c.author} · {formatAge(Date.parse(c.created_at))} ago <button className="ml-2 text-teal-ink hover:underline" onClick={() => void api.post("/api/open", { url: c.url }).catch(toastError)}>Open comment</button></div><Markdown text={c.body} /></article>)}
      </> : <p className="text-ink-3">{hasSummary ? refresh.isPending ? "Fetching the thread summary…" : "No cached summary. Click Refresh to try again." : "This notification has no issue or PR preview. Open it on GitHub for details."}</p>}
    </div>
    {supported && <div className="flex-none border-t border-line bg-chrome p-3">
      <textarea aria-label="Quick reply" className="field-input h-20 resize-y py-2" placeholder="Write a reply…" value={draft} disabled={reply.isPending} onChange={(e) => useView.setState((s) => ({ drafts: { ...s.drafts, [t.id]: e.target.value } }))} />
      <div className="mt-2 flex items-center justify-between gap-2 text-xs text-ink-3"><span>Posts to GitHub when you click.</span><button className="btn btn-primary btn-sm" disabled={!draft.trim() || reply.isPending} onClick={() => reply.mutate()}>{reply.isPending ? "Posting…" : "Post reply"}</button></div>
    </div>}
  </section>;
}
export function InboxView() {
  const { data, error } = useInbox();
  const projects = useProjects().data ?? [];
  const view = useView();
  const ref = useRef<HTMLDivElement>(null);
  const qc = useQueryClient();
  const action = useAction();
  const sync = useMutation({ mutationFn: () => api.post("/api/inbox/sync"), onError: toastError, onSettled: () => void qc.invalidateQueries({ queryKey: ["inbox"] }) });
  const threads = data?.threads ?? [];
  const counted = useMemo(() => threads.filter((t) => !view.unread || t.unread), [threads, view.unread]);
  const visible = useMemo(() => counted.filter((t) => !view.repo || t.repo === view.repo), [counted, view.repo]);
  const selected = threads.find((t) => t.id === view.selected);
  const v = useVirtualizer({ count: visible.length, getScrollElement: () => ref.current, estimateSize: () => 70, getItemKey: (i) => visible[i].id, overscan: 4, useFlushSync: false, initialOffset: view.scroll });
  const previousFilter = useRef({ repo: view.repo, unread: view.unread });
  useEffect(() => {
    if (previousFilter.current.repo !== view.repo || previousFilter.current.unread !== view.unread) v.scrollToOffset(0);
    previousFilter.current = { repo: view.repo, unread: view.unread };
  }, [view.repo, view.unread]);
  const mutate = action.mutate;
  const select = useCallback((t: Thread) => {
    useView.setState({ selected: t.id });
    if (t.unread) mutate({ id: t.id, action: "read" });
  }, [mutate]);
  // Warm the next few cached previews without making GitHub requests.
  useEffect(() => {
    const at = visible.findIndex((t) => t.id === view.selected);
    for (const t of visible.slice(Math.max(0, at + 1), Math.max(0, at + 1) + 3)) void qc.prefetchQuery({ queryKey: ["inbox-preview", t.id], queryFn: () => api.get<ThreadPreview | null>(`/api/inbox/${t.id}/preview`), staleTime: 60_000 });
  }, [view.selected, visible, qc]);
  return <div className="flex h-full flex-col">
    <div className="flex h-14 flex-none items-center gap-3 border-b border-line px-4">
      <h1 className="text-lg font-semibold">Inbox</h1><span className="text-xs text-ink-3">{threads.filter((t) => t.unread).length} unread</span>
      <label className="ml-auto flex items-center gap-2 text-xs"><input type="checkbox" checked={view.unread} onChange={(e) => useView.setState({ unread: e.target.checked, scroll: 0 })} />Unread only</label>
      <span className="text-xs text-ink-3">{data?.status.syncing ? "Syncing…" : data?.status.last_sync ? `Synced ${formatAge(data.status.last_sync)} ago` : "Waiting for GitHub"}</span>
      <button className="btn btn-sm" disabled={sync.isPending || data?.status.syncing} onClick={() => sync.mutate()}>Refresh</button>
    </div>
    {(error || data?.status.error) && <div className="border-b border-line bg-attn-soft px-4 py-2 text-xs text-gh-closed">{String(error ?? data?.status.error)} · Cached notifications remain available. Sign in with gh auth login if needed.</div>}
    <RepoChips threads={counted} selected={view.repo} />
    <div className="flex min-h-0 flex-1">
      <div data-inbox-list ref={ref} className="min-w-0 flex-1 overflow-y-auto" onScroll={() => useView.setState({ scroll: ref.current?.scrollTop ?? 0 })}>
        {visible.length === 0 && <p className="p-8 text-center text-ink-3">{data?.status.syncing ? "Fetching notifications…" : view.unread || view.repo ? "No notifications match these filters." : "Your inbox is clear."}</p>}
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((item) => <div key={item.key} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${item.start}px)` }}><ThreadRow t={visible[item.index]} selected={visible[item.index].id === view.selected} projects={projects} select={select} act={mutate} busy={action.isPending} /></div>)}
        </div>
      </div>
      {selected && <PreviewPane key={selected.id} t={selected} projects={projects} />}
    </div>
  </div>;
}
