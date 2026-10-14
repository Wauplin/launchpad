import { openExternal } from "@/lib/platform";
import { Link } from "@tanstack/react-router";
import { AddContext, GitHubHTML, reference, itemKey, itemPath } from "@/features/github/lib";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Thread } from "@/gen/Thread";
import type { ThreadPreview } from "@/gen/ThreadPreview";
import type { InboxSnapshot } from "@/gen/InboxSnapshot";
import type { Project } from "@/gen/Project";
import { Markdown } from "@/components/Markdown";
import { Popover } from "@/components/Popover";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useProjects } from "@/lib/queries";
import { usePhone } from "@/lib/phone";
import { useInbox, useInboxThreads, useView, whyOf, whys, type Why } from "./data";
export { useInbox, useInboxNeeds, useInboxThreads } from "./data";

function launchContext(t: Thread, projects: Project[]) {
  const project = projects.find((p) => p.gh_repo?.toLowerCase() === t.repo.toLowerCase()) ?? projects.find((p) => p.gh_upstream?.toLowerCase() === t.repo.toLowerCase());
  return { projectId: project?.id ?? null, githubRefs: [{ repo: t.repo, number: t.number, title: t.title, url: t.url, kind: t.kind === "PullRequest" ? "pr" : t.kind === "Issue" ? "issue" : t.kind.toLowerCase() }] };
}
const labels: Record<string, string> = { review_requested: "Review requested", mention: "Mentioned", team_mention: "Team mention", assign: "Assigned", ci_activity: "CI activity", state_change: "State changed" };
const reason = (s: string) => labels[s] ?? s.replaceAll("_", " ");
type Action = { id: string; action: string };
export function useInboxAction() {
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
const ThreadRow = memo(function ThreadRow({ t, selected, projects, select, act, busy }: { t: Thread; selected: boolean; projects: Project[]; select: (t: Thread) => void; act: (t: Thread, action: string) => void; busy: boolean }) {
  const phone = usePhone();
  return <div className={`flex ${phone ? "h-[76px]" : "h-[70px]"} items-center gap-2 border-b border-line-2 px-3 ${selected ? "bg-select" : "hover:bg-chrome"}`}>
    <button className="flex min-w-0 flex-1 items-start gap-2 text-left" onClick={() => select(t)} title={t.title}>
      <span className={`mt-1.5 size-2 flex-none rounded-full ${t.unread ? "bg-work" : "bg-transparent"}`} />
      <span className="min-w-0 flex-1">
        <span className={`block truncate ${t.unread ? "font-semibold" : "text-ink-2"}`}>{t.title}</span>
        <span className="mt-1 flex items-center gap-2 overflow-hidden text-[11px] text-ink-3">
          <span className="truncate">{t.repo}{t.number !== null && ` #${t.number}`}</span>
          {t.state && <span className={`flex-none ${t.state === "merged" ? "text-gh-merged" : t.state === "open" ? "text-gh-open" : ""}`}>{t.state}</span>}
          <span className={`flex-none ${whyOf(t.reason) === "other" ? "text-ink-2" : "font-semibold text-attn"}`}>{reason(t.reason)}</span>
          {t.participant_count > 0 && <span className="truncate" title={t.participants.join(", ")}>{t.participants.slice(0, 2).join(", ")}{t.participant_count > 2 && ` +${t.participant_count - 2}`}</span>}
          <span className="ml-auto flex-none">{formatAge(Date.parse(t.updated_at))}</span>
          {t.local_unread && <span title="Unread only in Launchpad">local</span>}
        </span>
      </span>
    </button>
    <button className="btn btn-sm flex-none" title="Mark as done" aria-label={`Mark ${t.title} as done`} disabled={busy} onClick={() => act(t, "done")}><Icon name="check" size={12} />{!phone && "Done"}</button>
    {/* The phone offers Launch and + Add in the preview. */}
    {!phone && <LaunchButton small primary={false} ctx={launchContext(t, projects)} />}
    {!phone && t.number !== null && (t.kind === "Issue" || t.kind === "PullRequest") && <AddContext refItem={reference(t.repo,{number:t.number,title:t.title,url:t.url},t.kind === "PullRequest" ? "pr" : "issue")}/> }
  </div>;
});
function RepoPicker({ threads, selected }: { threads: Thread[]; selected: string | null }) {
  const phone = usePhone();
  const [query, setQuery] = useState("");
  const repos = useMemo(() => {
    const counts = new Map<string, number>();
    threads.forEach((t) => counts.set(t.repo, (counts.get(t.repo) ?? 0) + 1));
    const q = query.trim().toLowerCase();
    return [...counts].filter(([r]) => !q || r.toLowerCase().includes(q)).sort(([a], [b]) => a.localeCompare(b));
  }, [threads, query]);
  return <Popover align={phone ? "left" : "right"} width={phone ? "min(300px, calc(100vw - 24px))" : 300} className={phone ? "w-full" : ""} trigger={({ toggle }) => <button className={`btn btn-sm max-w-[min(260px,100%)] ${selected ? "bg-select!" : ""}`} title={selected ?? "Filter by repository"} onClick={toggle}><span className="truncate">Repository: {selected ?? "all"}</span><Icon name="chev" size={12} /></button>}>
    {(close) => <RepoList repos={repos} total={threads.length} selected={selected} query={query} setQuery={setQuery} pick={(repo) => { useView.setState({ repo, scroll: 0 }); setQuery(""); close(); }} />}
  </Popover>;
}
function RepoList({ repos, total, selected, query, setQuery, pick }: { repos: [string, number][]; total: number; selected: string | null; query: string; setQuery: (q: string) => void; pick: (repo: string | null) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: repos.length, getScrollElement: () => ref.current, estimateSize: () => 30, overscan: 6, useFlushSync: false });
  const row = "flex h-[30px] w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px] hover:bg-select";
  return <div className="grid gap-1">
    <input autoFocus aria-label="Search repositories" className="field-input h-8" placeholder="Search repositories…" value={query} onChange={(e) => setQuery(e.target.value)} />
    <button className={`${row} ${selected === null ? "bg-select font-semibold" : ""}`} onClick={() => pick(null)}><span className="flex-1">All repositories</span><span className="text-ink-3">{total}</span></button>
    <div ref={ref} className="max-h-60 overflow-y-auto">
      {repos.length === 0 && <p className="p-2 text-xs text-ink-3">No repository matches.</p>}
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((item) => { const [repo, count] = repos[item.index]; return <button key={repo} title={repo} className={`${row} absolute top-0 left-0 ${selected === repo ? "bg-select font-semibold" : ""}`} style={{ transform: `translateY(${item.start}px)` }} onClick={() => pick(repo)}><span className="min-w-0 flex-1 truncate">{repo}</span><span className="text-ink-3">{count}</span></button>; })}
      </div>
    </div>
  </div>;
}
function PreviewPane({ t, projects, act, busy }: { t: Thread; projects: Project[]; act: (t: Thread, action: string) => void; busy: boolean }) {
  const qc = useQueryClient();
  const preview = useQuery({ queryKey: ["inbox-preview", t.id], queryFn: () => api.get<ThreadPreview | null>(`/api/inbox/${t.id}/preview`), staleTime: Infinity });
  const draft = useView((s) => s.drafts[t.id] ?? "");
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const refresh = useMutation({ mutationFn: (force: boolean) => api.post(`/api/inbox/${t.id}/preview`, { force }), onSuccess: () => { setRefreshError(null); void qc.invalidateQueries({ queryKey: ["inbox-preview", t.id] }); }, onError: (e) => setRefreshError(String(e)) });
  const reply = useMutation({ mutationFn: () => api.post(`/api/inbox/${t.id}/reply`, { text: draft }), onSuccess: () => { useView.setState((s) => ({ drafts: { ...s.drafts, [t.id]: "" } })); toast("Reply posted"); void qc.invalidateQueries({ queryKey: ["inbox-preview", t.id] }); }, onError: toastError });
  const supported = t.kind === "Issue" || t.kind === "PullRequest";
  const hasSummary = supported || t.kind === "Release" || t.kind === "Commit";
  useEffect(() => { if (hasSummary) refresh.mutate(false); }, [t.id, t.updated_at]); // Refresh in the background while the cached summary renders.
  useEffect(() => { if(supported && t.number !== null) void qc.prefetchQuery({queryKey:itemKey(t.repo,t.number),queryFn:()=>api.get(itemPath(t.repo,t.number!)),staleTime:60_000}); }, [t.id, t.updated_at]);
  const p = preview.data;
  const phone = usePhone();
  return <section className={`flex min-h-0 flex-col bg-paper ${phone ? "w-full" : "w-[40%] min-w-[320px] border-l border-line"}`}>
    <div className="flex-none border-b border-line p-4">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="text-xs text-ink-3">{t.repo}{t.number !== null && ` #${t.number}`} · {t.kind}{t.state && ` · ${t.state}`} · {reason(t.reason)}</div>
          <h2 className="mt-2 mb-3 text-[16px] font-semibold">{t.title}</h2>
        </div>
        <button className="btn btn-sm flex-none" onClick={() => useView.setState({ selected: null })}>{phone ? "Back" : "Close"}</button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <button className="btn btn-sm" title="Mark as done" disabled={busy} onClick={() => act(t, "done")}><Icon name="check" size={12} />Done</button>
        <button className="btn btn-sm" title={t.unread ? "Mark as read" : "Mark as unread locally (github.com stays read)"} disabled={busy} onClick={() => act(t, t.unread ? "read" : "unread")}>{t.unread ? "Mark read" : "Mark unread"}</button>
        <button className="btn btn-sm" title="Ignore future notifications and mark as done" disabled={busy} onClick={() => act(t, "unsubscribe")}>Unsubscribe</button>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <LaunchButton small ctx={launchContext(t, projects)} />
        {supported && t.number !== null && <><Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={{owner:t.repo.split("/")[0],repo:t.repo.split("/")[1],number:String(t.number)}}>Full conversation</Link><AddContext refItem={reference(t.repo,{number:t.number,title:t.title,url:t.url},t.kind === "PullRequest" ? "pr" : "issue")}/></>}
        <button className="btn btn-sm" onClick={() => openExternal(t.url)}>Open on GitHub</button>
        {hasSummary && <button className="btn btn-sm" disabled={refresh.isPending} onClick={() => refresh.mutate(true)}>{refresh.isPending ? "Refreshing…" : "Refresh"}</button>}
      </div>
      {(t.review_decision || t.checks) && <div className="mt-2 text-xs text-ink-2">{[t.review_decision && `Review: ${reason(t.review_decision.toLowerCase())}`, t.checks && `Checks: ${t.checks.toLowerCase()}`].filter(Boolean).join(" · ")}</div>}
    </div>
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      {refreshError && <p className="text-gh-closed">{refreshError}</p>}
      {preview.isError && <p className="text-gh-closed">{String(preview.error)}</p>}
      {p ? <>
        <div className="mb-2 text-xs text-ink-3">{p.author} · summary cached {formatAge(p.fetched_at)} ago</div>
        {p.body_html ? <GitHubHTML html={p.body_html} /> : <Markdown text={p.body || "No description."} />}
        <h3 className="mt-6 mb-3 text-xs font-semibold text-ink-2">Latest comments (up to 8)</h3>
        {p.comments.map((c) => <article key={c.url} className="mb-4 border-t border-line pt-3"><div className="mb-2 text-xs text-ink-3">{c.author} · {formatAge(Date.parse(c.created_at))} ago <button className="ml-2 text-teal-ink hover:underline" onClick={() => openExternal(c.url)}>Open comment</button></div><AddContext refItem={{repo:t.repo,number:t.number,title:t.title,url:c.url,kind:"comment"}}/>{c.body_html ? <GitHubHTML html={c.body_html} /> : <Markdown text={c.body} />}</article>)}
      </> : <p className="text-ink-3">{hasSummary ? refresh.isPending ? "Fetching the thread summary…" : "No cached summary. Click Refresh to try again." : "This notification has no issue or PR preview. Open it on GitHub for details."}</p>}
    </div>
    {supported && <div className="flex-none border-t border-line bg-chrome p-3">
      <textarea aria-label="Quick reply" className="field-input h-20 resize-y py-2" placeholder="Write a reply…" value={draft} disabled={reply.isPending} onChange={(e) => useView.setState((s) => ({ drafts: { ...s.drafts, [t.id]: e.target.value } }))} />
      <div className="mt-2 flex items-center justify-between gap-2 text-xs text-ink-3"><span>Posts to GitHub when you click.</span><button className="btn btn-primary btn-sm" disabled={!draft.trim() || reply.isPending} onClick={() => reply.mutate()}>{reply.isPending ? "Posting…" : "Post reply"}</button></div>
    </div>}
  </section>;
}
export function InboxView() {
  const phone = usePhone();
  const { data, error } = useInbox();
  const projects = useProjects().data ?? [];
  const view = useView();
  const ref = useRef<HTMLDivElement>(null);
  const qc = useQueryClient();
  const action = useInboxAction();
  const sync = useMutation({ mutationFn: () => api.post("/api/inbox/sync"), onError: toastError, onSettled: () => void qc.invalidateQueries({ queryKey: ["inbox"] }) });
  const { threads, total, scope, allRepos } = useInboxThreads();
  const hidden = total - threads.length;
  const counted = useMemo(() => threads.filter((t) => !view.unread || t.unread), [threads, view.unread]);
  const inRepo = useMemo(() => counted.filter((t) => !view.repo || t.repo === view.repo), [counted, view.repo]);
  const counts = useMemo(() => {
    const c: Record<Why, number> = { review: 0, mention: 0, assign: 0, other: 0, all: inRepo.length };
    inRepo.forEach((t) => c[whyOf(t.reason)]++);
    return c;
  }, [inRepo]);
  const visible = useMemo(() => view.why === "all" ? inRepo : inRepo.filter((t) => whyOf(t.reason) === view.why), [inRepo, view.why]);
  const selected = threads.find((t) => t.id === view.selected);
  // A repository picked among all repos may not belong to the workspace.
  useEffect(() => { if (view.repo && !threads.some((t) => t.repo === view.repo)) useView.setState({ repo: null }); }, [allRepos, scope.workspace?.id]);
  const v = useVirtualizer({ count: visible.length, getScrollElement: () => ref.current, estimateSize: () => (phone ? 76 : 70), getItemKey: (i) => visible[i].id, overscan: 4, useFlushSync: false, initialOffset: view.scroll });
  const previousFilter = useRef({ repo: view.repo, unread: view.unread, why: view.why });
  useEffect(() => {
    const prev = previousFilter.current;
    if (prev.repo !== view.repo || prev.unread !== view.unread || prev.why !== view.why) v.scrollToOffset(0);
    previousFilter.current = { repo: view.repo, unread: view.unread, why: view.why };
  }, [view.repo, view.unread, view.why]);
  // Opening the Inbox with nothing selected shows the first unread review request, else the first unread item. It stays unread until you act.
  const autoSelected = useRef(false);
  useEffect(() => {
    if (autoSelected.current || !data || !visible.length) return;
    autoSelected.current = true;
    // The phone shows the list first; a row opens its preview.
    if (phone) return;
    if (selected) return;
    const first = visible.find((t) => t.unread && t.reason === "review_requested") ?? visible.find((t) => t.unread);
    if (first) { useView.setState({ selected: first.id }); const at = visible.indexOf(first); if (at > 0) v.scrollToIndex(at, { align: "auto" }); }
  }, [data, visible]);
  const mutate = action.mutate;
  const select = useCallback((t: Thread) => {
    useView.setState({ selected: t.id });
    if (t.unread) mutate({ id: t.id, action: "read" });
  }, [mutate]);
  // Done and Unsubscribe on the selected row move the selection to the next row.
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const act = useCallback((t: Thread, a: string) => {
    if ((a === "done" || a === "unsubscribe") && useView.getState().selected === t.id) {
      const list = visibleRef.current;
      const at = list.findIndex((x) => x.id === t.id);
      const next = at < 0 ? undefined : list[at + 1] ?? list[at - 1];
      if (next) select(next); else useView.setState({ selected: null });
    }
    mutate({ id: t.id, action: a });
  }, [mutate, select]);
  // Warm the next few cached previews without making GitHub requests.
  useEffect(() => {
    const at = visible.findIndex((t) => t.id === view.selected);
    for (const t of visible.slice(Math.max(0, at + 1), Math.max(0, at + 1) + 3)) { if(t.number && (t.kind === "Issue" || t.kind === "PullRequest")) void qc.prefetchQuery({queryKey:itemKey(t.repo,t.number),queryFn:()=>api.get(itemPath(t.repo,t.number!)),staleTime:60_000}); }
    for (const t of visible.slice(Math.max(0, at + 1), Math.max(0, at + 1) + 3)) void qc.prefetchQuery({ queryKey: ["inbox-preview", t.id], queryFn: () => api.get<ThreadPreview | null>(`/api/inbox/${t.id}/preview`), staleTime: 60_000 });
  }, [view.selected, visible, qc]);
  if (phone && selected) return <div className="flex h-full"><PreviewPane key={selected.id} t={selected} projects={projects} act={act} busy={action.isPending} /></div>;
  return <div className="flex h-full flex-col">
    <div className={`flex flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-4 ${phone ? "py-2" : "h-14"}`}>
      {!phone && <h1 className="text-lg font-semibold">Inbox</h1>}<span className="text-xs text-ink-3">{threads.filter((t) => t.unread).length} unread</span>
      {scope.workspace && <span className="flex items-center gap-1" title="Notifications of this workspace's repositories, or of every repository">
        <button className={`btn btn-sm ${!allRepos ? "bg-select!" : ""}`} aria-pressed={!allRepos} onClick={() => useView.setState({ allRepos: false, scroll: 0 })}>{scope.workspace.name} repos</button>
        <button className={`btn btn-sm ${allRepos ? "bg-select!" : ""}`} aria-pressed={allRepos} onClick={() => useView.setState({ allRepos: true, scroll: 0 })}>All repos{!allRepos && hidden > 0 ? ` · ${hidden} more` : ""}</button>
      </span>}
      <span className="ml-auto text-xs text-ink-3">{data?.status.syncing ? "Syncing…" : data?.status.last_sync ? `Synced ${formatAge(data.status.last_sync)} ago` : "Waiting for GitHub"}</span>
      <button className="btn btn-sm" disabled={sync.isPending || data?.status.syncing} onClick={() => sync.mutate()}>Refresh</button>
    </div>
    {(error || data?.status.error) && <div className="border-b border-line bg-attn-soft px-4 py-2 text-xs text-gh-closed">{String(error ?? data?.status.error)} · Cached notifications remain available. Sign in with gh auth login if needed.</div>}
    <div className="flex flex-none flex-wrap items-center gap-1.5 border-b border-line px-3 py-2">
      {whys.map(([k, label, title]) => <button key={k} title={title} aria-pressed={view.why === k} className={`inline-flex h-[26px] items-center gap-1.5 rounded-full border px-2.5 text-xs whitespace-nowrap ${view.why === k ? "border-ink bg-ink text-paper" : "border-line bg-paper hover:bg-chrome"}`} onClick={() => useView.setState({ why: k, scroll: 0 })}>{label}<b className={`font-semibold tabular-nums ${view.why === k ? "text-paper/80" : "text-ink-3"}`}>{counts[k]}</b></button>)}
      <label className="ml-auto flex items-center gap-2 text-xs"><input type="checkbox" checked={view.unread} onChange={(e) => useView.setState({ unread: e.target.checked, scroll: 0 })} />Unread only</label>
      <RepoPicker threads={counted} selected={view.repo} />
    </div>
    <div className="flex min-h-0 flex-1">
      <div data-inbox-list ref={ref} className="min-w-0 flex-1 overflow-y-auto" onScroll={() => useView.setState({ scroll: ref.current?.scrollTop ?? 0 })}>
        {visible.length === 0 && <p className="p-8 text-center text-ink-3">{data?.status.syncing ? "Fetching notifications…" : view.unread || view.repo || view.why !== "all" ? "No notifications match these filters." : hidden > 0 ? `No notifications for the repositories of ${scope.workspace?.name}. Click the repos filter to see all ${hidden}.` : "Your inbox is clear."}</p>}
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((item) => <div key={item.key} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${item.start}px)` }}><ThreadRow t={visible[item.index]} selected={visible[item.index].id === view.selected} projects={projects} select={select} act={act} busy={action.isPending} /></div>)}
        </div>
      </div>
      {selected && <PreviewPane key={selected.id} t={selected} projects={projects} act={act} busy={action.isPending} />}
    </div>
  </div>;
}
