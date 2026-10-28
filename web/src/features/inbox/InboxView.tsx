import { PhoneHeader, PhoneActions } from "@/mobile/PhoneHeader";
import { openExternal } from "@/lib/platform";
import { Link } from "@tanstack/react-router";
import { AddContext, GitHubHTML, reference, itemKey, itemPath } from "@/features/github/lib";
import { GitHubReplyBox } from "@/features/github/ReplyBox";
import { reviewWithAgent } from "@/features/github/reviewAgent";
import { itemSuggestionKey, useSuggestionsByItem } from "@/features/review/data";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Thread } from "@/gen/Thread";
import type { ThreadPreview } from "@/gen/ThreadPreview";
import type { InboxSnapshot } from "@/gen/InboxSnapshot";
import type { Project } from "@/gen/Project";
import type { Suggestion } from "@/gen/Suggestion";
import { Markdown } from "@/components/Markdown";
import { MenuItem, Popover } from "@/components/Popover";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { Badge, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader } from "@/components/ui";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useProjects } from "@/lib/queries";
import { isPhone, usePhone } from "@/lib/phone";
import { useComposer } from "@/stores/composer";
import { forYou, useInbox, useInboxThreads, useView, whyOf, whys, type Why } from "./data";
export { useInbox, useInboxNeeds, useInboxThreads } from "./data";

function launchContext(t: Thread, projects: Project[]) {
  const project = projects.find((p) => p.gh_repo?.toLowerCase() === t.repo.toLowerCase()) ?? projects.find((p) => p.gh_upstream?.toLowerCase() === t.repo.toLowerCase());
  return { projectId: project?.id ?? null, githubRefs: [{ repo: t.repo, number: t.number, title: t.title, url: t.url, kind: t.kind === "PullRequest" ? "pr" : t.kind === "Issue" ? "issue" : t.kind.toLowerCase() }] };
}
const labels: Record<string, string> = { review_requested: "Review requested", mention: "Mentioned", team_mention: "Team mention", assign: "Assigned", ci_activity: "CI activity", state_change: "State changed", author: "Author", comment: "Commented", subscribed: "Subscribed", manual: "Subscribed", security_alert: "Security alert" };
const reason = (s: string) => labels[s] ?? s.replaceAll("_", " ");
const decision: Record<string, string> = { APPROVED: "Approved", CHANGES_REQUESTED: "Changes requested", REVIEW_REQUIRED: "Review required" };
const checkText: Record<string, string> = { SUCCESS: "Checks passing", FAILURE: "Checks failing", ERROR: "Checks failing", PENDING: "Checks pending", EXPECTED: "Checks pending" };
const ghParams = (t: Thread) => ({ owner: t.repo.split("/")[0], repo: t.repo.split("/")[1], number: String(t.number) });
const isReviewRequest = (t: Thread) => t.reason === "review_requested" && t.kind === "PullRequest" && t.number !== null && t.state !== "closed" && t.state !== "merged";

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

/** "Suggestion ready": an agent review (or comment) for this PR waits on the PR page. */
function ReadyChip({ t, s }: { t: Thread; s: Suggestion }) {
  return <Link to="/github/$owner/$repo/$number" params={ghParams(t)} search={{ draft: s.id }} className="flex-none no-underline" title="Open the PR at the agent's suggestion; it posts only when you click" onClick={(e) => e.stopPropagation()}>
    <Badge tone="teal" icon="comment">{s.kind === "review" ? "Agent review ready" : "Suggestion ready"}</Badge>
  </Link>;
}

const ThreadRow = memo(function ThreadRow({ t, selected, projects, ready, select, act, busy }: { t: Thread; selected: boolean; projects: Project[]; ready: Suggestion | undefined; select: (t: Thread) => void; act: (t: Thread, action: string) => void; busy: boolean }) {
  const phone = usePhone();
  return <div className={`flex ${phone ? "h-[72px]" : "h-14"} items-center gap-2 border-b border-line-2 px-3 ${selected ? "bg-select" : "hover:bg-chrome"}`}>
    <button className="flex min-w-0 flex-1 items-start gap-2 text-left" onClick={() => select(t)} title={t.title} aria-current={selected || undefined}>
      <span className={`mt-1.5 size-2 flex-none rounded-full ${t.unread ? "bg-work" : "bg-transparent"}`} aria-label={t.unread ? "Unread" : undefined} />
      <span className="min-w-0 flex-1">
        <span className={`block truncate text-[13px] ${t.unread ? "font-semibold" : "text-ink-2"}`}>{t.title}</span>
        <span className="mt-1 flex items-center gap-2 overflow-hidden text-[11px] text-ink-3">
          <span className="truncate">{t.repo}{t.number !== null && ` #${t.number}`}</span>
          {t.state && <span className={`flex-none ${t.state === "merged" ? "text-gh-merged" : t.state === "open" ? "text-gh-open" : ""}`}>{t.state}</span>}
          <span className={`flex-none ${whyOf(t.reason) === "other" ? "text-ink-2" : "font-semibold text-attn"}`}>{reason(t.reason)}</span>
          {ready && t.number !== null && <ReadyChip t={t} s={ready} />}
          {!phone && t.participant_count > 0 && <span className="truncate" title={t.participants.join(", ")}>{t.participants.slice(0, 2).join(", ")}{t.participant_count > 2 && ` +${t.participant_count - 2}`}</span>}
          <span className="ml-auto flex-none">{formatAge(Date.parse(t.updated_at))}</span>
          {t.local_unread && <span title="Unread only in Launchpad">local</span>}
        </span>
      </span>
    </button>
    <button className="btn btn-sm flex-none" title="Mark as done on GitHub" aria-label={`Mark ${t.title} as done`} disabled={busy} onClick={() => act(t, "done")}><Icon name="check" size={12} />{!phone && "Done"}</button>
    {/* The phone offers Launch and + Add in the preview. */}
    {!phone && <LaunchButton small primary={false} ctx={launchContext(t, projects)} />}
    {!phone && t.number !== null && (t.kind === "Issue" || t.kind === "PullRequest") && <AddContext refItem={reference(t.repo, { number: t.number, title: t.title, url: t.url }, t.kind === "PullRequest" ? "pr" : "issue")} />}
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
  return <Popover align={phone ? "left" : "right"} width={phone ? "min(300px, calc(100vw - 24px))" : 300} className={phone ? "w-full" : ""} trigger={({ open, toggle }) => <button className="lp-chip max-w-[min(260px,100%)]" aria-pressed={!!selected} aria-expanded={open} title={selected ?? "Filter by repository"} onClick={toggle}><span className="truncate">Repository: {selected ?? "all"}</span><Icon name="chev" size={12} /></button>}>
    {(close) => <RepoList repos={repos} total={threads.length} selected={selected} query={query} setQuery={setQuery} pick={(repo) => { useView.setState({ repo, scroll: 0 }); setQuery(""); close(); }} />}
  </Popover>;
}
function RepoList({ repos, total, selected, query, setQuery, pick }: { repos: [string, number][]; total: number; selected: string | null; query: string; setQuery: (q: string) => void; pick: (repo: string | null) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: repos.length, getScrollElement: () => ref.current, estimateSize: () => isPhone ? 44 : 28, overscan: 6, useFlushSync: false });
  const row = `flex ${isPhone ? "h-11" : "h-7"} w-full items-center gap-2 rounded-md px-2 text-left text-[12px] hover:bg-select`;
  return <div className="grid gap-1">
    <input autoFocus aria-label="Search repositories" className="field-input h-8" placeholder="Search repositories…" value={query} onChange={(e) => setQuery(e.target.value)} />
    <button className={`${row} ${selected === null ? "bg-select font-semibold" : ""}`} onClick={() => pick(null)}><span className="flex-1">All repositories</span><span className="text-ink-3">{total}</span></button>
    <div ref={ref} className="max-h-60 overflow-y-auto">
      {repos.length === 0 && <Empty>No repository matches.</Empty>}
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((item) => { const [repo, count] = repos[item.index]; return <button key={repo} title={repo} className={`${row} absolute top-0 left-0 ${selected === repo ? "bg-select font-semibold" : ""}`} style={{ transform: `translateY(${item.start}px)` }} onClick={() => pick(repo)}><span className="min-w-0 flex-1 truncate">{repo}</span><span className="text-ink-3">{count}</span></button>; })}
      </div>
    </div>
  </div>;
}

function Synced({ label, busy, onRefresh, title }: { label: string; busy: boolean; onRefresh: () => void; title: string }) {
  return <span className="inline-flex items-center gap-1 whitespace-nowrap">
    {label}
    <button className="iconbtn" aria-label={title} title={title} disabled={busy} onClick={onRefresh}><Icon name="refresh" size={13} className={busy ? "animate-spin" : ""} /></button>
  </span>;
}

function PreviewPane({ t, projects, ready, act, busy }: { t: Thread; projects: Project[]; ready: Suggestion | undefined; act: (t: Thread, action: string) => void; busy: boolean }) {
  const qc = useQueryClient();
  const phone = usePhone();
  const composerOpen = useComposer((s) => s.open);
  const preview = useQuery({ queryKey: ["inbox-preview", t.id], queryFn: () => api.get<ThreadPreview | null>(`/api/inbox/${t.id}/preview`), staleTime: Infinity });
  const [refreshError, setRefreshError] = useState<unknown>(null);
  const refresh = useMutation({ mutationFn: (force: boolean) => api.post(`/api/inbox/${t.id}/preview`, { force }), onSuccess: () => { setRefreshError(null); void qc.invalidateQueries({ queryKey: ["inbox-preview", t.id] }); }, onError: (e) => setRefreshError(e) });
  const supported = (t.kind === "Issue" || t.kind === "PullRequest") && t.number !== null;
  const hasSummary = supported || t.kind === "Release" || t.kind === "Commit";
  useEffect(() => { if (hasSummary) refresh.mutate(false); }, [t.id, t.updated_at]); // Refresh in the background while the cached summary renders.
  useEffect(() => { if (supported) void qc.prefetchQuery({ queryKey: itemKey(t.repo, t.number!), queryFn: () => api.get(itemPath(t.repo, t.number!)), staleTime: 60_000 }); }, [t.id, t.updated_at]);
  const p = preview.data;
  const request = isReviewRequest(t);
  const head = supported ? (qc.getQueryData<{ headRefOid?: string }>(itemKey(t.repo, t.number!))?.headRefOid ?? null) : null;
  const conversation = supported && <Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={ghParams(t)}>{t.kind === "PullRequest" ? "Open PR" : "Open issue"}</Link>;
  const done = (primary: boolean) => <button className={`btn btn-sm ${primary ? "btn-primary" : ""}`} title="Mark as done on GitHub" disabled={busy} onClick={() => act(t, "done")}><Icon name="check" size={12} />Done</button>;
  // One primary, two secondary, then ⋯. Review requests lead with the agent review.
  const primary = request
    ? ready
      ? <Link className="btn btn-sm btn-primary no-underline" to="/github/$owner/$repo/$number" params={ghParams(t)} search={{ draft: ready.id }}>Review &amp; post</Link>
      : <button className="btn btn-sm btn-primary" title="Opens a launch draft that asks an agent for a local suggested review" onClick={() => reviewWithAgent({ projects, repo: t.repo, number: t.number!, title: t.title, url: t.url, head })}>Review with agent</button>
    : done(true);
  const secondary = request ? <>{done(false)}{conversation}</> : <><LaunchButton small primary={false} ctx={launchContext(t, projects)} />{conversation || <button className="btn btn-sm" onClick={() => openExternal(t.url)}>Open on GitHub</button>}</>;
  const readLabel = t.unread ? "Mark read" : "Mark unread";
  const readTitle = t.unread ? "Mark as read" : "Mark as unread locally (github.com stays read)";
  const unsubscribe = <ConfirmButton consequence="Mutes this thread on GitHub and marks it done." confirmLabel="Unsubscribe" disabled={busy} onConfirm={() => act(t, "unsubscribe")} icon="bell-off">Unsubscribe</ConfirmButton>;
  const more = <Popover align="right" width={240} label="More actions" trigger={({ open, toggle }) => <button className="iconbtn" aria-label="More actions" aria-expanded={open} title="More actions" onClick={toggle}><Icon name="more" size={16} /></button>}>
    {(close) => <div className="grid gap-0.5">
      <MenuItem icon={<Icon name="mail" size={13} />} label={readLabel} detail={readTitle} disabled={busy} onClick={() => { act(t, t.unread ? "read" : "unread"); close(); }} />
      {request && <MenuItem icon={<Icon name="play" size={13} />} label="Launch an agent" onClick={() => { useComposer.getState().start(launchContext(t, projects)); close(); }} />}
      {supported && composerOpen && <MenuItem icon={<Icon name="plus" size={13} />} label="Add to draft" detail="As context of the open launch draft" onClick={() => { useComposer.getState().start({ githubRefs: [reference(t.repo, { number: t.number!, title: t.title, url: t.url }, t.kind === "PullRequest" ? "pr" : "issue")] }); close(); }} />}
      {conversation && <MenuItem icon={<Icon name="ext" size={13} />} label="Open on GitHub" onClick={() => { openExternal(t.url); close(); }} />}
      <div className="border-t border-line-2 px-2 pt-2 pb-1">{unsubscribe}</div>
    </div>}
  </Popover>;
  const facts = [t.review_decision && (decision[t.review_decision] ?? reason(t.review_decision.toLowerCase())), t.checks && (checkText[t.checks] ?? `Checks ${t.checks.toLowerCase()}`)].filter(Boolean).join(" · ");
  const subtitle = `${t.repo}${t.number !== null ? ` #${t.number}` : ""} · ${t.kind === "PullRequest" ? "PR" : t.kind}${t.state ? ` · ${t.state}` : ""} · ${reason(t.reason)}`;
  return <section className={`flex min-h-0 flex-col bg-paper ${phone ? "w-full" : "w-[40%] min-w-[320px] border-l border-line"}`}>
    {phone ? <PhoneHeader title={t.title} onBack={() => useView.setState({ selected: null })} backLabel="Back to inbox" subtitle={subtitle} primary={primary} actions={<>
      {secondary}
      <button className="btn btn-sm" title={readTitle} disabled={busy} onClick={() => act(t, t.unread ? "read" : "unread")}>{readLabel}</button>
      {request && <LaunchButton small primary={false} ctx={launchContext(t, projects)} />}
      {supported && <AddContext refItem={reference(t.repo, { number: t.number!, title: t.title, url: t.url }, t.kind === "PullRequest" ? "pr" : "issue")} />}
      {conversation && <button className="btn btn-sm" onClick={() => openExternal(t.url)}>Open on GitHub</button>}
    </>} details={<>{facts && <p className="text-xs text-ink-2">{facts}</p>}{unsubscribe}</>} /> : <>
    <div className="flex-none border-b border-line p-4">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="text-[12px] text-ink-3">{subtitle}</div>
          <h2 className="mt-1 mb-3 text-[15px] font-semibold">{t.title}</h2>
        </div>
        <button className="iconbtn flex-none" aria-label="Close preview" title="Close preview" onClick={() => useView.setState({ selected: null })}><Icon name="x" size={14} /></button>
      </div>
      <div className="flex flex-wrap items-center gap-2">{primary}{secondary}{more}</div>
      {(facts || ready) && <div className="mt-2 flex flex-wrap items-center gap-2 text-[12px] text-ink-2">{facts}{ready && <ReadyChip t={t} s={ready} />}</div>}
    </div>
    </>}
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      {!!refreshError && <ErrorState className="mb-3" error={refreshError} onRetry={() => refresh.mutate(true)} />}
      {preview.isError && <ErrorState className="mb-3" error={preview.error} />}
      {p ? <>
        <div className="mb-2 flex items-center gap-1 text-[12px] text-ink-3">{p.author} · <Synced label={`Summary cached ${formatAge(p.fetched_at)} ago`} busy={refresh.isPending} title="Refresh the summary from GitHub" onRefresh={() => refresh.mutate(true)} /></div>
        {p.body_html ? <GitHubHTML html={p.body_html} /> : <Markdown text={p.body || "No description."} />}
        <h3 className="mt-6 mb-3 text-[12px] font-semibold text-ink-2">Latest comments (up to 8)</h3>
        {p.comments.map((c) => <article key={c.url} className="mb-4 border-t border-line pt-3"><div className="mb-2 flex flex-wrap items-center gap-2 text-[12px] text-ink-3">{c.author} · {formatAge(Date.parse(c.created_at))} ago <button className="text-teal-ink hover:underline" onClick={() => openExternal(c.url)}>Open comment</button><AddContext refItem={{ repo: t.repo, number: t.number, title: t.title, url: c.url, kind: "comment" }} /></div>{c.body_html ? <GitHubHTML html={c.body_html} /> : <Markdown text={c.body} />}</article>)}
      </> : hasSummary
        ? refresh.isPending ? <Loading what="the thread summary" /> : <Empty icon="info" action={<button className="btn btn-sm" onClick={() => refresh.mutate(true)}><Icon name="refresh" size={12} />Refresh</button>}>No cached summary.</Empty>
        : <Empty icon="info" action={<button className="btn btn-sm" onClick={() => openExternal(t.url)}>Open on GitHub</button>}>This notification has no issue or PR preview.</Empty>}
    </div>
    {supported && <GitHubReplyBox repo={t.repo} number={t.number!} comment={{ path: `${itemPath(t.repo, t.number!)}/comment`, body: (text) => ({ body: text }) }} onPosted={() => { void qc.invalidateQueries({ queryKey: ["inbox-preview", t.id] }); void qc.invalidateQueries({ queryKey: itemKey(t.repo, t.number!) }); }} />}
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
  const byItem = useSuggestionsByItem();
  const sync = useMutation({ mutationFn: () => api.post("/api/inbox/sync"), onError: toastError, onSettled: () => void qc.invalidateQueries({ queryKey: ["inbox"] }) });
  const { threads, total, scope, allRepos } = useInboxThreads();
  const hidden = total - threads.length;
  // Threads read while For you is shown stay listed until the filter changes.
  const kept = useRef(new Set<string>());
  useEffect(() => { kept.current.clear(); }, [view.why, view.repo, allRepos]);
  const counted = useMemo(() => threads.filter((t) => !view.unread || t.unread), [threads, view.unread]);
  const inRepo = useMemo(() => counted.filter((t) => !view.repo || t.repo === view.repo), [counted, view.repo]);
  const counts = useMemo(() => {
    const c: Record<Why, number> = { you: 0, review: 0, mention: 0, assign: 0, other: 0, all: inRepo.length };
    inRepo.forEach((t) => { c[whyOf(t.reason)]++; if (forYou(t)) c.you++; });
    return c;
  }, [inRepo]);
  const visible = useMemo(() => view.why === "all" ? inRepo : view.why === "you" ? inRepo.filter((t) => forYou(t) || kept.current.has(t.id)) : inRepo.filter((t) => whyOf(t.reason) === view.why), [inRepo, view.why]);
  const selected = threads.find((t) => t.id === view.selected);
  const unread = threads.filter((t) => t.unread).length;
  const mine = threads.filter(forYou).length;
  // A repository picked among all repos may not belong to the workspace.
  useEffect(() => { if (view.repo && !threads.some((t) => t.repo === view.repo)) useView.setState({ repo: null }); }, [allRepos, scope.workspace?.id]);
  const v = useVirtualizer({ count: visible.length, getScrollElement: () => ref.current, estimateSize: () => (phone ? 72 : 56), getItemKey: (i) => visible[i].id, overscan: 4, useFlushSync: false, initialOffset: view.scroll });
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
    if (forYou(t)) kept.current.add(t.id);
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
    for (const t of visible.slice(Math.max(0, at + 1), Math.max(0, at + 1) + 3)) { if (t.number && (t.kind === "Issue" || t.kind === "PullRequest")) void qc.prefetchQuery({ queryKey: itemKey(t.repo, t.number), queryFn: () => api.get(itemPath(t.repo, t.number!)), staleTime: 60_000 }); }
    for (const t of visible.slice(Math.max(0, at + 1), Math.max(0, at + 1) + 3)) void qc.prefetchQuery({ queryKey: ["inbox-preview", t.id], queryFn: () => api.get<ThreadPreview | null>(`/api/inbox/${t.id}/preview`), staleTime: 60_000 });
  }, [view.selected, visible, qc]);
  const readyOf = (t: Thread) => t.number !== null ? byItem.get(itemSuggestionKey(t.repo, t.number))?.[0] : undefined;
  if (phone && selected) return <div className="flex h-full"><PreviewPane key={selected.id} t={selected} projects={projects} ready={readyOf(selected)} act={act} busy={action.isPending} /></div>;
  const syncing = sync.isPending || !!data?.status.syncing;
  const synced = <Synced label={data?.status.syncing ? "Syncing…" : data?.status.last_sync ? `Synced ${formatAge(data.status.last_sync)} ago` : "Waiting for GitHub"} busy={syncing} title="Refresh the inbox from GitHub" onRefresh={() => sync.mutate()} />;
  const repoScope: ReactNode = scope.workspace && <FilterChips label="Repositories">
    <FilterChip active={!allRepos} title="Notifications of this workspace's repositories" onClick={() => useView.setState({ allRepos: false, scroll: 0 })}>{scope.workspace.name} repos</FilterChip>
    <FilterChip active={allRepos} title="Notifications of every repository" count={!allRepos && hidden > 0 ? hidden : undefined} onClick={() => useView.setState({ allRepos: true, scroll: 0 })}>All repos</FilterChip>
  </FilterChips>;
  const counters = <><span title="Unread notifications in this list">{unread} unread</span><span>·</span><span title="Unread review requests, mentions and assignments: the sidebar count">{mine} for you</span><span>·</span>{synced}</>;
  return <div className="flex h-full flex-col">
    {phone ? <div className="flex shrink-0 items-center gap-2 border-b border-line px-3 py-1">
      <span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-1.5 text-xs text-ink-3">{counters}</span>
      <PhoneActions label="Inbox filters" closeOnAction={false} details={<div className="grid gap-3">
        {repoScope}
        <label className="flex min-h-11 items-center gap-2"><input type="checkbox" checked={view.unread} onChange={(e) => useView.setState({ unread: e.target.checked, scroll: 0 })} />Unread only</label>
        <RepoPicker threads={counted} selected={view.repo} />
      </div>} />
    </div> : <PageHeader title="Inbox" meta={counters} actions={repoScope} />}
    {(error || data?.status.error) && <ErrorState className="m-2" error={error ?? data?.status.error} action={<span className="text-ink-2">Cached notifications remain available. Sign in with gh auth login if needed.</span>} />}
    <div className="inbox-filters flex flex-none flex-wrap items-center gap-1.5 border-b border-line px-3 py-2">
      <FilterChips tour="inbox.reasons" label="Why you were notified">
        {whys.map(([k, label, title]) => <FilterChip key={k} active={view.why === k} title={title} count={counts[k]} onClick={() => useView.setState({ why: k, scroll: 0 })}>{label}</FilterChip>)}
      </FilterChips>
      {!phone && <><label className="ml-auto flex items-center gap-2 text-[12px]"><input type="checkbox" checked={view.unread} onChange={(e) => useView.setState({ unread: e.target.checked, scroll: 0 })} />Unread only</label>
      <RepoPicker threads={counted} selected={view.repo} /></>}
    </div>
    <div className="flex min-h-0 flex-1">
      <div data-inbox-list ref={ref} className="min-w-0 flex-1 overflow-y-auto" onScroll={() => useView.setState({ scroll: ref.current?.scrollTop ?? 0 })}>
        {visible.length === 0 && (data?.status.syncing || !data ? <Loading what="notifications" /> : <Empty fill icon="inbox">{view.unread || view.repo || view.why !== "all" ? "No notifications match these filters." : hidden > 0 ? `No notifications for the repositories of ${scope.workspace?.name}. Choose All repos to see all ${hidden}.` : "Your inbox is clear."}</Empty>)}
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((item) => { const t = visible[item.index]; return <div key={item.key} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${item.start}px)` }}><ThreadRow t={t} selected={t.id === view.selected} projects={projects} ready={readyOf(t)} select={select} act={act} busy={action.isPending} /></div>; })}
        </div>
      </div>
      {selected && <PreviewPane key={selected.id} t={selected} projects={projects} ready={readyOf(selected)} act={act} busy={action.isPending} />}
    </div>
  </div>;
}
