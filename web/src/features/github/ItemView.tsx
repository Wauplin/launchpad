import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useParams, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api } from "@/lib/api";
import { toast } from "@/components/Toast";
import { Icon, type IconName } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { Badge, ConfirmButton, Empty, ErrorState, FilterChip, FilterChips, Loading, PageHeader, Tabs, type BadgeTone } from "@/components/ui";
import { useInbox, useInboxAction } from "@/features/inbox/InboxView";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { useProjects } from "@/lib/queries";
import { formatAge } from "@/lib/format";
import { AddContext, GitHubHTML, itemKey, itemPath, openGitHub, reference } from "./lib";
import { SuggestionCard, useSuggestions } from "@/features/suggestions/Suggestions";
import { LabelChip } from "./LabelChip";
import { Picker, useRepoMeta } from "./Picker";
import { FilesChanged, usePrNotes } from "./FilesChanged";
import { GitHubReplyBox } from "./ReplyBox";
import { projectForRepo, reviewWithAgent } from "./reviewAgent";
import type { Item, Timeline, Comment } from "./types";

// Human words for GitHub's enums.
const REVIEW_DECISION: Record<string, string> = { APPROVED: "Approved", CHANGES_REQUESTED: "Changes requested", REVIEW_REQUIRED: "Review required" };
const REVIEW_STATE: Record<string, string> = { APPROVED: "approved", CHANGES_REQUESTED: "requested changes", COMMENTED: "commented", DISMISSED: "review dismissed", PENDING: "review pending" };
const human = (s: string) => s.toLowerCase().replaceAll("_", " ");
const PASS = new Set(["SUCCESS", "NEUTRAL", "SKIPPED"]);
const FAIL = new Set(["FAILURE", "ERROR", "TIMED_OUT", "CANCELLED", "ACTION_REQUIRED", "STARTUP_FAILURE"]);
type Check = NonNullable<NonNullable<NonNullable<Item["commits"]>["nodes"][number]["commit"]["statusCheckRollup"]>>["contexts"]["nodes"][number];
const checkResult = (c: Check): "pass" | "fail" | "pending" => { const r = c.conclusion ?? c.state ?? ""; return PASS.has(r) ? "pass" : FAIL.has(r) ? "fail" : "pending"; };

function stateOf(item: Item | undefined): { label: string; tone: BadgeTone; icon: IconName } {
  const pr = item?.__typename === "PullRequest";
  if (!item) return { label: "Loading", tone: "neutral", icon: "pr" };
  if (item.merged) return { label: "Merged", tone: "neutral", icon: "pr-merged" };
  if (item.state === "CLOSED") return { label: "Closed", tone: "danger", icon: pr ? "pr" : "issue-done" };
  if (item.isDraft) return { label: "GitHub draft", tone: "neutral", icon: "pr-draft" };
  return { label: "Open", tone: "ok", icon: pr ? "pr" : "issue" };
}

/** One line for a timeline event without text: "alice added label bug · 2 d ago". */
function eventText(e: Timeline): ReactNode {
  switch (e.__typename) {
    case "ClosedEvent": return "closed this";
    case "ReopenedEvent": return "reopened this";
    case "MergedEvent": return "merged this";
    case "LabeledEvent": return <>added the label <b>{e.label?.name}</b></>;
    case "UnlabeledEvent": return <>removed the label <b>{e.label?.name}</b></>;
    case "AssignedEvent": return <>assigned <b>{e.assignee?.login}</b></>;
    case "UnassignedEvent": return <>unassigned <b>{e.assignee?.login}</b></>;
    case "RenamedTitleEvent": return <>changed the title from <i>{e.previousTitle}</i> to <i>{e.currentTitle}</i></>;
    case "ReadyForReviewEvent": return "marked this ready for review";
    case "ConvertToDraftEvent": return "converted this to a GitHub draft";
    case "ReviewRequestedEvent": return <>requested a review from <b>{e.requestedReviewer?.login ?? e.requestedReviewer?.name}</b></>;
    case "CrossReferencedEvent": return <>mentioned this in {e.source ? <button className="text-teal-ink hover:underline" onClick={() => openGitHub(e.source!.url)}>{e.source.title}</button> : "another item"}</>;
    case "PullRequestCommit": return e.commit ? <>added commit <button className="font-mono text-teal-ink hover:underline" onClick={() => openGitHub(e.commit!.url)}>{e.commit.oid.slice(0, 7)}</button> {e.commit.messageHeadline}</> : "added a commit";
    case "PullRequestReview": return REVIEW_STATE[e.state ?? ""] ?? "reviewed";
    default: return human(e.__typename.replace(/Event$/, "").replace(/([a-z])([A-Z])/g, "$1 $2"));
  }
}
const eventIcon = (t: string): IconName => t === "PullRequestCommit" ? "commit" : t === "MergedEvent" ? "pr-merged" : t === "ClosedEvent" ? "xcircle" : t === "ReopenedEvent" ? "refresh" : t === "PullRequestReview" ? "eye" : t.includes("Label") ? "preset" : t === "CrossReferencedEvent" ? "ext" : "info";
const cardVerb = (e: Timeline) => e.__typename === "Description" ? "opened this" : e.__typename === "PullRequestReview" ? REVIEW_STATE[e.state ?? ""] ?? "reviewed" : "commented";

const EventCard = memo(function EventCard({ event, item }: { event: Timeline; item: Item }) {
  const time = event.createdAt ?? event.submittedAt ?? event.commit?.committedDate;
  const author = event.author ?? event.actor;
  const who = author?.login ?? event.commit?.author.name ?? "GitHub";
  if (!event.bodyHTML?.trim() && event.__typename !== "Description") {
    return <div className="flex min-h-7 items-center gap-2 px-1 text-[12px] text-ink-3">
      <span className="grid size-6 flex-none place-items-center rounded-full bg-sunk text-ink-2"><Icon name={eventIcon(event.__typename)} size={12} /></span>
      <span className="min-w-0 truncate"><b className="text-ink-2">{who}</b> {eventText(event)}</span>
      {time && <span className="flex-none">· {formatAge(Date.parse(time))} ago</span>}
    </div>;
  }
  return <article className="rounded-lg border border-line bg-paper p-4">
    <div className="mb-3 flex flex-wrap items-center gap-2 text-[12px] text-ink-3">
      {author?.avatarUrl && <img src={author.avatarUrl} width={24} height={24} className="rounded-full" alt="" />}
      <b className="text-ink">{who}</b>
      <span>{event.__typename === "Review comment" ? "commented on a line" : cardVerb(event)}</span>
      {time && <span>· {formatAge(Date.parse(time))} ago</span>}
      {event.url && <span className="ml-auto flex items-center gap-2">
        <button className="iconbtn" title="Open on GitHub" aria-label="Open on GitHub" onClick={() => openGitHub(event.url!)}><Icon name="ext" size={12} /></button>
        <AddContext refItem={{ ...reference(item.repo, item, "comment"), url: event.url }} />
      </span>}
    </div>
    {event.bodyHTML ? <GitHubHTML html={event.bodyHTML} /> : <p className="text-ink-3">No description.</p>}
  </article>;
});

function Conversation({ item, focus }: { item: Item; focus: { id: string | null; n: number } }) {
  const ref = useRef<HTMLDivElement>(null);
  const qc = useQueryClient();
  const suggestions = useSuggestions({ kind: "git_hub", repo: item.repo, number: item.number });
  const events: Timeline[] = [{ __typename: "Description", id: item.id, author: item.author, createdAt: item.createdAt, body: item.body, bodyHTML: item.bodyHTML, url: item.url }, ...item.timelineItems.nodes.filter((e) => e.__typename)];
  const v = useVirtualizer({
    count: suggestions.data.length + events.length,
    getItemKey: (i) => i < suggestions.data.length ? suggestions.data[i].id : events[i - suggestions.data.length].id ?? `event:${i - suggestions.data.length}`,
    getScrollElement: () => ref.current,
    estimateSize: (i) => i < suggestions.data.length ? 200 : events[i - suggestions.data.length].bodyHTML ? 160 : 32,
    overscan: 3,
    useFlushSync: false,
  });
  // "Review & post" and ?draft= scroll to the suggestion (the first one when no id is given).
  const loaded = suggestions.isSuccess;
  useEffect(() => {
    if (!focus.n || !loaded || !suggestions.data.length) return;
    const i = Math.max(0, suggestions.data.findIndex((s) => s.id === focus.id));
    requestAnimationFrame(() => v.scrollToIndex(i, { align: "start" }));
  }, [focus.n, loaded]);
  return <div ref={ref} className="min-h-0 flex-1 overflow-auto p-4">
    {suggestions.error && <ErrorState className="mb-3" error={suggestions.error} />}
    <div className="relative" style={{ height: v.getTotalSize() }}>
      {v.getVirtualItems().map((r) => <div key={r.key} data-index={r.index} ref={v.measureElement} className="absolute top-0 left-0 w-full pb-3" style={{ transform: `translateY(${r.start}px)` }}>
        {r.index < suggestions.data.length ? <SuggestionCard suggestion={suggestions.data[r.index]} head={item.headRefOid} onPosted={() => { void api.post<Item>(itemPath(item.repo, item.number), { force: true }).then((fresh) => qc.setQueryData(itemKey(item.repo, item.number), fresh)).catch(() => {}); }} /> : <EventCard event={events[r.index - suggestions.data.length]} item={item} />}
      </div>)}
    </div>
  </div>;
}

function ReviewThreads({ item }: { item: Item }) {
  const ref = useRef<HTMLDivElement>(null);
  const rows = useMemo(() => (item.reviewThreads?.nodes ?? []).flatMap((t) => [{ thread: t, comment: null as Comment | null }, ...t.comments.nodes.map((c) => ({ thread: t, comment: c }))]), [item.reviewThreads]);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: (i) => (rows[i].comment ? 160 : 40), overscan: 3, useFlushSync: false });
  return <div ref={ref} className="min-h-0 flex-1 overflow-auto p-4">
    <div className="relative" style={{ height: v.getTotalSize() }}>
      {v.getVirtualItems().map((r) => {
        const { thread: t, comment: c } = rows[r.index];
        return <div key={c?.id ?? t.id} data-index={r.index} ref={v.measureElement} className="absolute top-0 left-0 w-full pb-3" style={{ transform: `translateY(${r.start}px)` }}>
          {c ? <EventCard item={item} event={{ ...c, __typename: "Review comment" }} /> : <div className="flex items-center gap-2 rounded border border-line bg-chrome px-3 py-2">
            <b className="min-w-0 truncate font-mono text-[12px]">{t.path}{t.line && `:${t.line}`}</b>
            <Badge tone={t.isResolved ? "done" : "attn"}>{t.isResolved ? "Resolved" : "Unresolved"}</Badge>
            {t.isOutdated && <Badge tone="neutral">Outdated</Badge>}
          </div>}
        </div>;
      })}
    </div>
    {!rows.length && <Empty icon="comment">No review threads.</Empty>}
  </div>;
}

function Section({ title, children, aside }: { title: string; children: ReactNode; aside?: ReactNode }) {
  return <section className="grid gap-2 border-b border-line-2 pb-3 last:border-b-0">
    <div className="flex items-center gap-2"><h3 className="seclabel m-0 flex-1">{title}</h3>{aside}</div>
    {children}
  </section>;
}

function Checks({ item }: { item: Item }) {
  const list = item.commits?.nodes[0]?.commit.statusCheckRollup?.contexts.nodes ?? [];
  const [open, setOpen] = useState(false);
  const sorted = useMemo(() => [...list].sort((a, b) => ({ fail: 0, pending: 1, pass: 2 })[checkResult(a)] - ({ fail: 0, pending: 1, pass: 2 })[checkResult(b)]), [list]);
  const n = { pass: 0, fail: 0, pending: 0 };
  list.forEach((c) => n[checkResult(c)]++);
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: open ? sorted.length : 0, getScrollElement: () => ref.current, estimateSize: () => 28, overscan: 4, useFlushSync: false });
  const summary = [n.pass && `${n.pass} passing`, n.fail && `${n.fail} failing`, n.pending && `${n.pending} pending`].filter(Boolean).join(" · ");
  return <Section title="Checks" aside={list.length > 0 && <button className="text-[12px] text-teal-ink hover:underline" aria-expanded={open} onClick={() => setOpen(!open)}>{open ? "Hide" : "Show all"}</button>}>
    {list.length ? <span className={`flex items-center gap-1.5 ${n.fail ? "text-danger-ink" : n.pending ? "text-attn" : "text-gh-open"}`}><Icon name={n.fail ? "xcircle" : n.pending ? "clock" : "okcircle"} size={13} />{summary}</span> : <span className="text-ink-3">No checks</span>}
    {open && <div ref={ref} className="overflow-auto" style={{ height: Math.min(224, sorted.length * 28) }}>
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((r) => {
          const c = sorted[r.index], result = checkResult(c), url = c.detailsUrl ?? c.targetUrl;
          return <button key={r.key} className="absolute left-0 flex h-7 w-full items-center gap-1.5 text-left hover:underline disabled:no-underline" style={{ top: r.start }} title={`${c.name ?? c.context}: ${human(c.conclusion ?? c.status ?? c.state ?? "")}`} disabled={!url} onClick={() => openGitHub(url!)}>
            <Icon name={result === "fail" ? "xcircle" : result === "pending" ? "clock" : "okcircle"} size={12} className={result === "fail" ? "text-danger-ink" : result === "pending" ? "text-attn" : "text-gh-open"} />
            <span className="min-w-0 flex-1 truncate">{c.name ?? c.context}</span>
          </button>;
        })}
      </div>
    </div>}
  </Section>;
}

/** Why GitHub would refuse or warn about the merge, from the cached merge state. */
function blocker(item: Item) {
  const checks = item.commits?.nodes[0]?.commit.statusCheckRollup?.state;
  if (item.isDraft) return "Blocked: GitHub draft. Mark it ready for review first.";
  if (item.mergeable === "CONFLICTING" || item.mergeStateStatus === "DIRTY") return "Blocked: merge conflicts";
  if (item.mergeable === "UNKNOWN" || item.mergeStateStatus === "UNKNOWN") return "GitHub is still checking mergeability. Refresh in a moment.";
  if (item.mergeStateStatus === "BEHIND") return "Blocked: the branch is behind its base";
  if (item.mergeStateStatus === "UNSTABLE") return "Checks are failing or pending";
  if (item.mergeStateStatus === "BLOCKED") {
    if (item.reviewDecision === "REVIEW_REQUIRED") return "Blocked: a review is required";
    if (item.reviewDecision === "CHANGES_REQUESTED") return "Blocked: changes were requested";
    if (checks && checks !== "SUCCESS") return `Blocked: checks ${human(checks)}`;
    return "Blocked by branch protection";
  }
  return `GitHub reports ${human(item.mergeable ?? "unknown")} · ${human(item.mergeStateStatus ?? "unknown")}`;
}

function Controls({ item, act, busy }: { item: Item; act: (action: string, body: unknown) => Promise<boolean>; busy: boolean }) {
  const isPr = item.__typename === "PullRequest", open = item.state === "OPEN";
  const meta = useRepoMeta(item.repo).data;
  const current = item.labels.nodes.map((l) => l.name);
  const [reviewers, setReviewers] = useState<string[]>([]);
  const [labels, setLabels] = useState(current);
  const colors = useMemo(() => new Map([...(meta?.labels ?? []), ...item.labels.nodes].map((l) => [l.name, l.color])), [meta, item.labels]);
  const labelsChanged = labels.length !== current.length || labels.some((l) => !current.includes(l));
  const requested = new Set((item.reviewRequests?.nodes ?? []).map((r) => r.requestedReviewer.login ?? `team:${r.requestedReviewer.slug}`));
  const [method, setMethod] = useState(() => localStorage.getItem(`merge-method:${item.repo}`) ?? "squash");
  const methods = [["merge", "mergeCommitAllowed", "Merge commit"], ["squash", "squashMergeAllowed", "Squash"], ["rebase", "rebaseMergeAllowed", "Rebase"]] as const;
  const allowed = methods.filter(([, key]) => item.repository?.[key]);
  const ready = item.mergeable === "MERGEABLE" && ["CLEAN", "HAS_HOOKS"].includes(item.mergeStateStatus ?? "") && !item.isDraft;
  const actual = allowed.some(([m]) => m === method) ? method : (allowed[0]?.[0] ?? method);
  const state = stateOf(item);
  // Latest review per author.
  const reviews = useMemo(() => { const m = new Map<string, string>(); (item.reviews?.nodes ?? []).forEach((r) => m.set(r.author.login ?? "someone", r.state)); return [...m]; }, [item.reviews]);
  const requestedNames = (item.reviewRequests?.nodes ?? []).map((r) => r.requestedReviewer.login ?? r.requestedReviewer.name).filter(Boolean);
  return <aside className="phone-item-controls w-[280px] flex-none overflow-auto border-l border-line bg-chrome p-3 text-[12px]">
    <div className="grid gap-3">
      <Section title="Status">
        <div className="flex flex-wrap items-center gap-2"><Badge tone={state.tone} icon={state.icon}>{state.label}</Badge>{!open && !item.merged && <button className="btn btn-sm" disabled={busy} onClick={() => void act("state", { kind: isPr ? "pr" : "issue", state: "open", state_reason: "reopened" })}>Reopen</button>}</div>
        {isPr && open && <div className="grid gap-2 rounded-lg border border-line bg-paper p-3">
          <span className={`flex items-start gap-1.5 ${ready ? "text-gh-open" : "text-attn"}`}><Icon name={ready ? "okcircle" : "alert"} size={13} className="mt-px flex-none" />{ready ? "Ready to merge" : blocker(item)}</span>
          {allowed.length > 1 && <FilterChips label="Merge method">{allowed.map(([m, , label]) => <FilterChip key={m} active={actual === m} onClick={() => setMethod(m)}>{label}</FilterChip>)}</FilterChips>}
          <ConfirmButton tone="publish" small={false} className={`w-full justify-center ${ready ? "btn-primary" : ""}`} disabled={busy || item.isDraft || item.mergeable !== "MERGEABLE" || !allowed.length}
            consequence={`${methods.find(([m]) => m === actual)?.[2] ?? actual} into ${item.baseRefName} on GitHub.`} confirmLabel={`Confirm ${actual} merge`}
            onConfirm={() => act("merge", { sha: item.headRefOid, merge_method: actual }).then((ok) => { if (ok) localStorage.setItem(`merge-method:${item.repo}`, actual); })}>
            Merge pull request
          </ConfirmButton>
          <button className="btn btn-sm w-full justify-center" disabled={busy} onClick={() => void act(item.isDraft ? "ready" : "draft", {})}>{item.isDraft ? "Ready for review" : "Convert to GitHub draft"}</button>
        </div>}
        {!!item.closingIssuesReferences?.nodes.length && <div className="grid gap-1">{item.closingIssuesReferences.nodes.map((i) => <button key={i.url} className="truncate text-left text-teal-ink hover:underline" title={i.title} onClick={() => openGitHub(i.url)}>Closes #{i.number} {i.title}</button>)}</div>}
        <div className="text-ink-2"><span className="text-ink-3">Assignees:</span> {item.assignees.nodes.map((a) => a.login).join(", ") || "none"}{item.milestone && <> · <span className="text-ink-3">Milestone:</span> {item.milestone.title}</>}</div>
      </Section>
      {isPr && <Section title="Reviewers">
        <span className="font-semibold">{REVIEW_DECISION[item.reviewDecision ?? ""] ?? "No review decision"}</span>
        {reviews.map(([who, s]) => <span key={who} className="flex items-center gap-1.5"><Icon name={s === "APPROVED" ? "okcircle" : s === "CHANGES_REQUESTED" ? "xcircle" : "comment"} size={12} className={s === "APPROVED" ? "text-gh-open" : s === "CHANGES_REQUESTED" ? "text-danger-ink" : "text-ink-3"} />{who} {REVIEW_STATE[s] ?? human(s)}</span>)}
        {requestedNames.length > 0 && <span className="text-ink-2"><span className="text-ink-3">Requested:</span> {requestedNames.join(", ")}</span>}
        {open && <>
          <Picker label={reviewers.length ? `Reviewers · ${reviewers.length} selected` : "Request reviewers"} options={(meta?.reviewers ?? []).filter((r) => !requested.has(r))} selected={reviewers} onChange={setReviewers} allowNew="Add" disabled={busy} />
          {reviewers.length > 0 && <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void act("reviewers", { reviewers: reviewers.filter((s) => !s.startsWith("team:")), team_reviewers: reviewers.filter((s) => s.startsWith("team:")).map((s) => s.slice(5)) }).then((ok) => { if (ok) setReviewers([]); })}>Request review from {reviewers.length}</button>}
        </>}
      </Section>}
      {isPr && <Checks item={item} />}
      <Section title="Labels">
        {item.labels.nodes.length > 0 ? <div className="flex flex-wrap gap-1">{item.labels.nodes.map((l) => <LabelChip key={l.name} label={l} />)}</div> : <span className="text-ink-3">No labels</span>}
        <Picker label={labelsChanged ? `Labels · ${labels.length} selected` : "Edit labels"} options={meta?.labels.map((l) => l.name) ?? []} selected={labels} onChange={setLabels} disabled={busy} render={(name) => <LabelChip label={{ name, color: colors.get(name) ?? "d0d7de" }} />} />
        {labelsChanged && <div className="flex gap-2"><button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void act("labels", { labels })}>Save labels</button><button className="btn btn-sm" disabled={busy} onClick={() => setLabels(current)}>Revert</button></div>}
      </Section>
      {open && <Section title="Danger zone">
        <div className="grid justify-items-start gap-2">
          <ConfirmButton disabled={busy} consequence={isPr ? "Closes the PR on GitHub without merging." : "Closes the issue on GitHub as completed."} confirmLabel={isPr ? "Close PR" : "Close issue"} onConfirm={() => act("state", { kind: isPr ? "pr" : "issue", state: "closed", state_reason: "completed" })}>{isPr ? "Close PR" : "Close as completed"}</ConfirmButton>
          {!isPr && <ConfirmButton disabled={busy} consequence="Closes the issue on GitHub as not planned." confirmLabel="Close issue" onConfirm={() => act("state", { kind: "issue", state: "closed", state_reason: "not_planned" })}>Close as not planned</ConfirmButton>}
        </div>
      </Section>}
    </div>
  </aside>;
}

export function ItemView() {
  const { owner, repo, number } = useParams({ strict: false }) as { owner: string; repo: string; number: string };
  return <ItemDetail key={`${owner}/${repo}/${number}`} repo={`${owner}/${repo}`} number={Number(number)} />;
}

const DONE: Record<string, (n: number, body: Record<string, unknown>) => string> = {
  labels: (n) => `Labels saved on #${n}`,
  reviewers: (n) => `Review requested on #${n}`,
  merge: (n) => `Merged #${n}`,
  state: (n, b) => b.state === "open" ? `Reopened #${n}` : `Closed #${n}`,
  ready: (n) => `#${n} is ready for review`,
  draft: (n) => `#${n} is now a GitHub draft`,
};

export function ItemDetail({ repo, number }: { repo: string; number: number }) {
  const phone = usePhone();
  const key = itemKey(repo, number), path = itemPath(repo, number), qc = useQueryClient();
  const q = useQuery({ queryKey: key, queryFn: () => api.get<Item>(path), staleTime: 60_000 });
  const [refreshing, setRefreshing] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState<unknown>(null), [tab, setTab] = useState<"conversation" | "files" | "threads">("conversation");
  const rawDraft = (useSearch({ strict: false }) as { draft?: unknown }).draft;
  const draftParam = rawDraft == null ? undefined : String(rawDraft);
  const [focus, setFocus] = useState<{ id: string | null; n: number }>({ id: null, n: 0 });
  const reviewDraft = (id: string | null) => { setTab("conversation"); setFocus((f) => ({ id, n: f.n + 1 })); };
  useEffect(() => { if (draftParam) reviewDraft(draftParam === "1" ? null : draftParam); }, [draftParam]);
  const projects = useProjects().data ?? [];
  const project = projectForRepo(projects, repo);
  const notes = usePrNotes(repo, number);
  async function refresh(force = true) {
    setRefreshing(true);
    try { qc.setQueryData(key, await api.post<Item>(path, { force })); setError(null); }
    catch (e) { setError(e); }
    finally { setRefreshing(false); }
  }
  useEffect(() => { if (q.data && Date.now() - q.data.fetched_at > 60_000) void refresh(false); }, [q.data?.fetched_at]);
  async function act(action: string, body: unknown) {
    setBusy(true); setError(null);
    try {
      await api.post(`${path}/${action}`, body);
      toast(DONE[action]?.(number, body as Record<string, unknown>) ?? `Updated #${number}`);
      void qc.invalidateQueries({ queryKey: ["repo-items", repo] });
      await refresh();
      return true;
    } catch (e) { setError(e); return false; }
    finally { setBusy(false); }
  }
  const inbox = useInbox().data;
  const read = useInboxAction();
  const readIds = useRef(new Set<string>());
  useEffect(() => {
    if (!q.data) return;
    for (const t of inbox?.threads ?? []) {
      if (t.repo.toLowerCase() === repo.toLowerCase() && t.number === number && t.unread && !readIds.current.has(t.id)) { readIds.current.add(t.id); read.mutate({ id: t.id, action: "read" }); }
    }
  }, [q.data, inbox?.threads, repo, number]);
  const item = q.data;
  const isPr = item?.__typename === "PullRequest";
  const state = stateOf(item);
  const head = item?.headRefOid;
  const current = notes.value.filter((n) => n.head === head);
  const outdated = notes.value.length - current.length;
  const launch = <LaunchButton small primary={!isPr} ctx={{ projectId: project?.id ?? null, githubRefs: item ? [reference(repo, item, isPr ? "pr" : "issue")] : [] }} />;
  const agentReview = isPr && item && <button className="btn btn-sm btn-primary" title="Opens a launch draft that asks an agent for a local suggested review" onClick={() => reviewWithAgent({ projects, repo, number, title: item.title, url: item.url, head })}>Review with agent</button>;
  const openOnGitHub = () => openGitHub(item?.url ?? `https://github.com/${repo}/issues/${number}`);
  const synced = item && <span className="inline-flex items-center gap-1 whitespace-nowrap">Synced {formatAge(item.fetched_at)} ago<button className="iconbtn" aria-label="Refresh from GitHub" title="Refresh from GitHub" disabled={refreshing} onClick={() => void refresh()}><Icon name="refresh" size={13} className={refreshing ? "animate-spin" : ""} /></button></span>;
  const more = <Popover align="right" width={220} label="More actions" trigger={({ open, toggle }) => <button className="iconbtn" aria-label="More actions" aria-expanded={open} title="More actions" onClick={toggle}><Icon name="more" size={16} /></button>}>
    {(close) => <div className="grid gap-0.5"><MenuItem icon={<Icon name="ext" size={13} />} label="Open on GitHub" onClick={() => { openOnGitHub(); close(); }} /></div>}
  </Popover>;
  const actions = <>{isPr ? <>{agentReview}{launch}</> : launch}{item && <AddContext refItem={reference(repo, item, isPr ? "pr" : "issue")} />}{more}</>;
  const meta = <>
    <Badge tone={state.tone} icon={state.icon}>{state.label}</Badge>
    <span>{repo} #{number}</span>
    {isPr && item && <span className="font-mono">{item.headRefName} → {item.baseRefName} · {item.headRefOid?.slice(0, 7)}</span>}
    {synced}
  </>;
  const tabs = isPr ? <Tabs label="Pull request views" value={tab} onChange={setTab} items={[{ id: "conversation", label: "Conversation" }, { id: "files", label: "Changes", count: current.length || undefined, title: current.length ? `${current.length} line comments for your next review` : undefined }, { id: "threads", label: "Review threads", count: item?.reviewThreads?.nodes.length || undefined }]} /> : undefined;
  return <div className="flex h-full flex-col">
    {phone ? <>
      <PhoneHeader title={item?.title ?? `#${number}`} subtitle={<span className="flex flex-wrap items-center gap-1.5">{meta}</span>} primary={isPr ? agentReview : launch} actions={<>{isPr && launch}{item && <AddContext refItem={reference(repo, item, isPr ? "pr" : "issue")} />}<button className="btn btn-sm" onClick={openOnGitHub}>Open on GitHub</button><button className="btn btn-sm" disabled={refreshing} onClick={() => void refresh()}>Refresh</button></>} details={item && <Controls key={item.updatedAt} item={item} act={act} busy={busy} />} />
      {tabs && <div className="px-3">{tabs}</div>}
    </> : <PageHeader title={item?.title ?? `#${number}`} meta={meta} actions={actions} tabs={tabs} />}
    {(!!error || q.error) && <ErrorState className="m-2" error={error ?? q.error} onRetry={() => void refresh()} />}
    {item && <SuggestionBanner item={item} onReview={reviewDraft} />}
    {item ? <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        {tab === "files" && isPr ? <FilesChanged repo={repo} number={number} head={item.headRefOid!} base={item.baseRefOid} />
          : tab === "threads" && isPr ? <ReviewThreads item={item} />
            : <Conversation item={item} focus={focus} />}
        <GitHubReplyBox
          repo={repo} number={number} collapsible disabled={busy}
          comment={{ path: `${path}/comment`, body: (text) => ({ body: text }) }}
          review={isPr && item.state === "OPEN" ? { path: `${path}/review`, extra: current.length, body: (text, event) => ({ body: text, event, commit_id: head, comments: current.map((n) => ({ path: n.path, line: n.line, side: n.side === "old" ? "LEFT" : "RIGHT", body: n.text })) }) } : undefined}
          notes={(current.length > 0 || outdated > 0) && <span className="flex flex-wrap items-center gap-2 text-[12px]">
            {current.length > 0 && <Badge tone="attn" icon="pencil" title="Written in Changes; posted with Submit review">{current.length} line comment{current.length === 1 ? "" : "s"} in this review</Badge>}
            {outdated > 0 && <span className="text-ink-3">{outdated} on an older commit, not included</span>}
            <ConfirmButton consequence="Deletes the unsent line comments." confirmLabel="Discard" onConfirm={() => notes.clear()}>Discard line comments</ConfirmButton>
          </span>}
          onPosted={(kind) => { if (kind === "review") void notes.set((all) => all.filter((n) => n.head !== head)); void refresh(); }}
        />
      </div>
      {!phone && <Controls key={item.updatedAt} item={item} act={act} busy={busy} />}
    </div> : !q.error && <Loading what="this item from GitHub" />}
  </div>;
}

/** Shown on every tab while a suggestion for this item waits for the owner. */
function SuggestionBanner({ item, onReview }: { item: Item; onReview: (id: string | null) => void }) {
  const all = useSuggestions({ kind: "git_hub", repo: item.repo, number: item.number }).data;
  const waiting = all.filter((s) => s.status !== "posted");
  if (!waiting.length) return null;
  const describe = (s: (typeof waiting)[number]) => s.kind === "review" ? `${s.event === "REQUEST_CHANGES" ? "Request changes" : s.event === "APPROVE" ? "Approve" : "Comment"} review` : "Comment";
  const first = waiting[0];
  return <div className="flex flex-none flex-wrap items-center gap-2 border-b border-teal/40 bg-teal-soft px-4 py-1.5 text-[13px]">
    <b className="text-teal-ink">{waiting.length === 1 ? "Suggestion" : `${waiting.length} suggestions`}:</b>
    <span>{waiting.length === 1 ? describe(first) : waiting.map(describe).join(" · ")}</span>
    <span className="text-ink-3">· by {first.author}</span>
    {["posting", "uncertain"].includes(first.status) && <Badge tone="attn" icon="alert">Publication attempted: check GitHub</Badge>}
    <button className="btn btn-primary btn-sm ml-auto" onClick={() => onReview(first.id)}>Review &amp; post</button>
  </div>;
}
