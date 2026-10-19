import { usePhone } from "@/lib/phone";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useParams, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { abandonOperation } from "@/lib/operations";
import { toast, toastError } from "@/components/Toast";
import { useInbox, useInboxAction } from "@/features/inbox/InboxView";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { replyKey, useReviewState } from "@/lib/reviewState";
import { useProjects } from "@/lib/queries";
import { formatAge } from "@/lib/format";
import {
  AddContext,
  GitHubHTML,
  itemKey,
  itemPath,
  openGitHub,
  reference,
} from "./lib";
import { SuggestionCard, useSuggestions } from "@/features/suggestions/Suggestions";
import { useComposer } from "@/stores/composer";
import { LabelChip } from "./LabelChip";
import { Picker, useRepoMeta } from "./Picker";
import { FilesChanged } from "./FilesChanged";
import type { Item, Timeline, Comment } from "./types";
const EventCard = memo(function EventCard({
  event,
  item,
}: {
  event: Timeline;
  item: Item;
}) {
  const time =
    event.createdAt ?? event.submittedAt ?? event.commit?.committedDate;
  const author = event.author ?? event.actor;
  return (
    <article className="rounded-lg border border-line bg-paper p-4">
      <div className="mb-3 flex flex-wrap items-center gap-2 text-xs text-ink-3">
        {author?.avatarUrl && (
          <img
            src={author.avatarUrl}
            width={24}
            height={24}
            className="rounded-full"
          />
        )}
        <b className="text-ink">
          {author?.login ?? event.commit?.author.name ?? "GitHub"}
        </b>
        <span>
          {event.__typename
            .replace(/Event$/, "")
            .replace(/([a-z])([A-Z])/g, "$1 $2")}
        </span>
        {event.state && (
          <span>{event.state.toLowerCase().replaceAll("_", " ")}</span>
        )}
        {time && <span>{formatAge(Date.parse(time))} ago</span>}
        {event.url && (
          <>
            <button
              className="text-teal-ink"
              onClick={() => openGitHub(event.url!)}
            >
              Open
            </button>
            <AddContext
              refItem={{
                ...reference(item.repo, item, "comment"),
                url: event.url,
              }}
            />
          </>
        )}
      </div>
      {event.bodyHTML && <GitHubHTML html={event.bodyHTML} />}
      {event.label && <span>{event.label.name}</span>}
      {event.assignee && <span>{event.assignee.login}</span>}
      {event.requestedReviewer && (
        <span>
          {event.requestedReviewer.login ?? event.requestedReviewer.name}
        </span>
      )}
      {event.previousTitle && (
        <p>
          {event.previousTitle} → {event.currentTitle}
        </p>
      )}
      {event.source && (
        <button
          className="text-teal-ink"
          onClick={() => openGitHub(event.source!.url)}
        >
          {event.source.title}
        </button>
      )}
      {event.commit && (
        <button
          className="text-teal-ink"
          onClick={() => openGitHub(event.commit!.url)}
        >
          {event.commit.oid.slice(0, 7)} {event.commit.messageHeadline}
        </button>
      )}
    </article>
  );
});
function Conversation({ item, focus }: { item: Item; focus: { id: string | null; n: number } }) {
  const ref = useRef<HTMLDivElement>(null);
  const qc = useQueryClient();
  const suggestions = useSuggestions({ kind: "git_hub", repo: item.repo, number: item.number });
  const events: Timeline[] = [
    {
      __typename: "Description",
      id: item.id,
      author: item.author,
      createdAt: item.createdAt,
      body: item.body,
      bodyHTML: item.bodyHTML,
      url: item.url,
    },
    ...item.timelineItems.nodes.filter((e) => e.__typename),
  ];
  const v = useVirtualizer({
    count: suggestions.data.length + events.length,
    getItemKey: (i) => i < suggestions.data.length ? suggestions.data[i].id : events[i - suggestions.data.length].id ?? `event:${i - suggestions.data.length}`,
    getScrollElement: () => ref.current,
    estimateSize: () => 160,
    overscan: 3,
    useFlushSync: false,
  });
  // "Review & post" and ?draft= scroll to the agent draft (the first one when no id is given).
  const loaded = suggestions.isSuccess;
  useEffect(() => {
    if (!focus.n || !loaded || !suggestions.data.length) return;
    const i = Math.max(0, suggestions.data.findIndex((s) => s.id === focus.id));
    requestAnimationFrame(() => v.scrollToIndex(i, { align: "start" }));
  }, [focus.n, loaded]);
  return (
    <div ref={ref} className="min-h-0 flex-1 overflow-auto p-4">
      {suggestions.error && <p role="alert" className="text-gh-closed">{String(suggestions.error)}</p>}
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((r) => (
          <div
            key={r.key}
            data-index={r.index}
            ref={v.measureElement}
            className="absolute left-0 top-0 w-full pb-3"
            style={{ transform: `translateY(${r.start}px)` }}
          >
            {r.index < suggestions.data.length ? <SuggestionCard suggestion={suggestions.data[r.index]} head={item.headRefOid} onPosted={() => { void api.post<Item>(itemPath(item.repo, item.number), { force: true }).then((fresh) => qc.setQueryData(itemKey(item.repo, item.number), fresh)).catch(() => {}); }} /> : <EventCard event={events[r.index - suggestions.data.length]} item={item} />}
          </div>
        ))}
      </div>
    </div>
  );
}
function ReviewThreads({ item }: { item: Item }) {
  const ref = useRef<HTMLDivElement>(null);
  const rows = useMemo(
    () =>
      (item.reviewThreads?.nodes ?? []).flatMap((t) => [
        { thread: t, comment: null as Comment | null },
        ...t.comments.nodes.map((c) => ({ thread: t, comment: c })),
      ]),
    [item.reviewThreads],
  );
  const v = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: (i) => (rows[i].comment ? 160 : 48),
    overscan: 3,
    useFlushSync: false,
  });
  return (
    <div ref={ref} className="min-h-0 flex-1 overflow-auto p-4">
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((r) => {
          const { thread: t, comment: c } = rows[r.index];
          return (
            <div
              key={c?.id ?? t.id}
              data-index={r.index}
              ref={v.measureElement}
              className="absolute top-0 left-0 w-full pb-3"
              style={{ transform: `translateY(${r.start}px)` }}
            >
              {c ? (
                <EventCard
                  item={item}
                  event={{ ...c, __typename: "Review comment" }}
                />
              ) : (
                <div className="rounded border border-line bg-chrome p-3">
                  <b className="font-mono text-xs">
                    {t.path}
                    {t.line && `:${t.line}`}
                  </b>
                  <span className="ml-2 text-xs text-ink-3">
                    {t.isResolved ? "Resolved" : "Unresolved"}
                    {t.isOutdated && " · outdated"}
                  </span>
                </div>
              )}
            </div>
          );
        })}
      </div>
      {!rows.length && <p className="text-ink-3">No review threads.</p>}
    </div>
  );
}
function Checks({ item }: { item: Item }) {
  const checks = item.commits?.nodes[0]?.commit.statusCheckRollup;
  const list = checks?.contexts.nodes ?? [];
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: list.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 38,
    overscan: 2,
    useFlushSync: false,
  });
  return (
    <div>
      <b>Checks: {checks?.state?.toLowerCase() ?? "none"}</b>
      <div
        ref={ref}
        className="mt-2 overflow-auto"
        style={{ height: Math.min(228, list.length * 38) }}
      >
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((r) => {
            const c = list[r.index],
              text = `${c.name ?? c.context}: ${(c.conclusion ?? c.status ?? c.state ?? "").toLowerCase()}`;
            return (
              <button
                key={r.key}
                className="absolute left-0 h-[38px] w-full truncate text-left text-teal-ink hover:underline"
                style={{ top: r.start }}
                title={text}
                disabled={!(c.detailsUrl ?? c.targetUrl)}
                onClick={() => openGitHub((c.detailsUrl ?? c.targetUrl)!)}
              >
                {text}
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
/** Why GitHub would refuse or warn about the merge, from the cached merge state. */
function blocker(item: Item) {
  const checks = item.commits?.nodes[0]?.commit.statusCheckRollup?.state;
  if (item.isDraft) return "Blocked: draft pull request";
  if (item.mergeable === "CONFLICTING" || item.mergeStateStatus === "DIRTY") return "Blocked: merge conflicts";
  if (item.mergeable === "UNKNOWN" || item.mergeStateStatus === "UNKNOWN") return "GitHub is still checking mergeability. Refresh in a moment.";
  if (item.mergeStateStatus === "BEHIND") return "Blocked: the branch is behind its base";
  if (item.mergeStateStatus === "UNSTABLE") return "Checks are failing or pending";
  if (item.mergeStateStatus === "BLOCKED") {
    if (item.reviewDecision === "REVIEW_REQUIRED") return "Blocked: a review is required";
    if (item.reviewDecision === "CHANGES_REQUESTED") return "Blocked: changes were requested";
    if (checks && checks !== "SUCCESS") return `Blocked: checks ${checks.toLowerCase()}`;
    return "Blocked by branch protection";
  }
  return `${item.mergeable?.toLowerCase() ?? "unknown"} · ${item.mergeStateStatus?.toLowerCase() ?? "unknown"}`;
}
function Controls({
  item,
  act,
  busy,
}: {
  item: Item;
  act: (action: string, body: unknown) => Promise<boolean>;
  busy: boolean;
}) {
  const isPr = item.__typename === "PullRequest",
    open = item.state === "OPEN";
  const meta = useRepoMeta(item.repo).data;
  const current = item.labels.nodes.map((l) => l.name);
  const [reviewers, setReviewers] = useState<string[]>([]);
  const [labels, setLabels] = useState(current);
  const colors = useMemo(() => new Map([...(meta?.labels ?? []), ...item.labels.nodes].map((l) => [l.name, l.color])), [meta, item.labels]);
  const labelsChanged = labels.length !== current.length || labels.some((l) => !current.includes(l));
  const requested = new Set((item.reviewRequests?.nodes ?? []).map((r) => r.requestedReviewer.login ?? `team:${r.requestedReviewer.slug}`));
  const [method, setMethod] = useState(
    () => localStorage.getItem(`merge-method:${item.repo}`) ?? "squash",
  );
  const [confirm, setConfirm] = useState(false);
  const methods = [
    ["merge", "mergeCommitAllowed"],
    ["squash", "squashMergeAllowed"],
    ["rebase", "rebaseMergeAllowed"],
  ] as const;
  const allowed = methods.filter(([, key]) => item.repository?.[key]);
  const ready = item.mergeable === "MERGEABLE" && ["CLEAN", "HAS_HOOKS"].includes(item.mergeStateStatus ?? "") && !item.isDraft;
  const actual = allowed.some(([m]) => m === method)
    ? method
    : (allowed[0]?.[0] ?? method);
  return (
    <aside className="phone-item-controls w-[260px] flex-none overflow-auto border-l border-line bg-chrome p-3 text-xs">
      <div className="grid gap-3">
        <div>
          <b>Labels</b>
          <div className="mt-2 flex flex-wrap gap-1">
            {item.labels.nodes.map((l) => (
              <LabelChip key={l.name} label={l} />
            ))}
          </div>
          <Picker label={labelsChanged ? `Labels · ${labels.length} selected` : "Edit labels"} options={meta?.labels.map((l) => l.name) ?? []} selected={labels} onChange={setLabels} disabled={busy} render={(name) => <LabelChip label={{ name, color: colors.get(name) ?? "d0d7de" }} />} />
          {labelsChanged && (
            <div className="mt-2 flex gap-2">
              <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void act("labels", { labels })}>Save labels</button>
              <button className="btn btn-sm" disabled={busy} onClick={() => setLabels(current)}>Revert</button>
            </div>
          )}
        </div>
        <div>
          <b>Assignees</b>
          <p>{item.assignees.nodes.map((a) => a.login).join(", ") || "None"}</p>
          {item.milestone && <p>Milestone: {item.milestone.title}</p>}
        </div>
        {isPr && (
          <>
            <div>
              <b>Reviews</b>
              <p>
                {item.reviewDecision?.replaceAll("_", " ").toLowerCase() ??
                  "No decision"}
              </p>
              {item.reviews?.nodes.map((r, i) => (
                <p key={i}>
                  {r.author.login}: {r.state.toLowerCase().replaceAll("_", " ")}
                </p>
              ))}
              <p>
                Requested:{" "}
                {item.reviewRequests?.nodes
                  .map(
                    (r) =>
                      r.requestedReviewer.login ?? r.requestedReviewer.name,
                  )
                  .join(", ") || "None"}
              </p>
              {open && (
                <>
                  <Picker label={reviewers.length ? `Reviewers · ${reviewers.length} selected` : "Request reviewers"} options={(meta?.reviewers ?? []).filter((r) => !requested.has(r))} selected={reviewers} onChange={setReviewers} allowNew="Add" disabled={busy} />
                  {reviewers.length > 0 && <button
                    className="btn btn-sm btn-primary mt-2"
                    disabled={busy}
                    onClick={() => {
                      const names = reviewers;
                      void act("reviewers", {
                        reviewers: names.filter((s) => !s.startsWith("team:")),
                        team_reviewers: names
                          .filter((s) => s.startsWith("team:"))
                          .map((s) => s.slice(5)),
                      });
                    }}
                  >
                    Request review from {reviewers.length}
                  </button>}
                </>
              )}
            </div>
            <Checks item={item} />
            {item.closingIssuesReferences?.nodes.length !== 0 && (
              <div>
                {item.closingIssuesReferences?.nodes.map((i) => (
                  <button
                    key={i.url}
                    className="block text-left text-teal-ink"
                    onClick={() => openGitHub(i.url)}
                  >
                    Closes #{i.number} {i.title}
                  </button>
                ))}
              </div>
            )}
            {open && (
              <div className="rounded-lg border border-line bg-paper p-3">
                <b>Merge</b>
                <p className={ready ? "text-gh-open" : "text-attn"}>{ready ? "Ready to merge" : blocker(item)}</p>
                <select
                  className="field-input"
                  aria-label="Merge method"
                  value={actual}
                  onChange={(e) => {
                    setMethod(e.target.value);
                    setConfirm(false);
                  }}
                >
                  {allowed.map(([m]) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
                <button
                  className={`btn mt-2 w-full justify-center ${ready ? "btn-primary" : ""}`}
                  disabled={
                    busy ||
                    item.isDraft ||
                    item.mergeable !== "MERGEABLE" ||
                    !allowed.length
                  }
                  onClick={() => {
                    if (!confirm) {
                      setConfirm(true);
                      return;
                    }
                    void act("merge", {
                      sha: item.headRefOid,
                      merge_method: actual,
                    }).then((ok) => {
                      if (ok)
                        localStorage.setItem(
                          `merge-method:${item.repo}`,
                          actual,
                        );
                      setConfirm(false);
                    });
                  }}
                >
                  {confirm ? `Confirm ${actual} merge` : `Merge pull request`}
                </button>
                {item.isDraft && <p>Mark ready for review before merging.</p>}
              </div>
            )}
          </>
        )}
        {!item.merged && (
          <div className="flex flex-wrap gap-2">
            {open ? (
              <>
                <button
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={() =>
                    void act("state", {
                      kind: isPr ? "pr" : "issue",
                      state: "closed",
                      state_reason: "completed",
                    })
                  }
                >
                  {isPr ? "Close PR" : "Close as completed"}
                </button>
                {!isPr && (
                  <button
                    className="btn btn-sm"
                    disabled={busy}
                    onClick={() =>
                      void act("state", {
                        kind: "issue",
                        state: "closed",
                        state_reason: "not_planned",
                      })
                    }
                  >
                    Close as not planned
                  </button>
                )}
              </>
            ) : (
              <button
                className="btn btn-sm"
                disabled={busy}
                onClick={() =>
                  void act("state", {
                    kind: isPr ? "pr" : "issue",
                    state: "open",
                    state_reason: "reopened",
                  })
                }
              >
                Reopen
              </button>
            )}
            {isPr && open && (
              <button
                className="btn btn-sm"
                disabled={busy}
                onClick={() => void act(item.isDraft ? "ready" : "draft", {})}
              >
                {item.isDraft ? "Ready for review" : "Convert to draft"}
              </button>
            )}
          </div>
        )}
      </div>
    </aside>
  );
}
export function ItemView() {
  const { owner, repo, number } = useParams({ strict: false }) as {
    owner: string;
    repo: string;
    number: string;
  };
  return (
    <ItemDetail
      key={`${owner}/${repo}/${number}`}
      repo={`${owner}/${repo}`}
      number={Number(number)}
    />
  );
}
export function ItemDetail({ repo, number }: { repo: string; number: number }) {
  const phone = usePhone();
  const key = itemKey(repo, number),
    path = itemPath(repo, number),
    qc = useQueryClient();
  const q = useQuery({
    queryKey: key,
    queryFn: () => api.get<Item>(path),
    staleTime: 60_000,
  });
  const [refreshing, setRefreshing] = useState(false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null),
    [tab, setTab] = useState("conversation");
  const rawDraft = (useSearch({ strict: false }) as { draft?: unknown }).draft;
  const draftParam = rawDraft == null ? undefined : String(rawDraft);
  const [focus, setFocus] = useState<{ id: string | null; n: number }>({ id: null, n: 0 });
  const reviewDraft = (id: string | null) => { setTab("conversation"); setFocus((f) => ({ id, n: f.n + 1 })); };
  useEffect(() => { if (draftParam) reviewDraft(draftParam === "1" ? null : draftParam); }, [draftParam]);
  const [composing, setComposing] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  // WebKit does not focus clicked buttons, so the box collapses on a click outside it rather than on blur.
  useEffect(() => {
    if (!composing) return;
    const down = (e: MouseEvent) => { if (!box.current?.contains(e.target as Node)) setComposing(false); };
    document.addEventListener("mousedown", down);
    return () => document.removeEventListener("mousedown", down);
  }, [composing]);
  // The unsent reply lives in the daemon so it follows the owner across devices.
  const reply = useReviewState(replyKey(repo, number), "");
  const draft = reply.value;
  const [event, setEvent] = useState("COMMENT");
  const [failedPost, setFailedPost] = useState<{ path: string; body: unknown } | null>(null);
  const projects = useProjects().data ?? [];
  const project = projects.find(
    (p) =>
      p.gh_repo?.toLowerCase() === repo.toLowerCase() ||
      p.gh_upstream?.toLowerCase() === repo.toLowerCase(),
  );
  async function refresh(force = true) {
    setRefreshing(true);
    try {
      qc.setQueryData(key, await api.post<Item>(path, { force }));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshing(false);
    }
  }
  useEffect(() => {
    if (q.data && Date.now() - q.data.fetched_at > 60_000) void refresh(false);
  }, [q.data?.fetched_at]);
  async function act(action: string, body: unknown) {
    setBusy(true);
    setError(null);
    try {
      await api.post(`${path}/${action}`, body);
      setFailedPost(null);
      toast("GitHub action completed");
      void qc.invalidateQueries({ queryKey: ["repo-items", repo] });
      await refresh();
      return true;
    } catch (e) {
      if (action === "comment" || action === "review") setFailedPost({ path: `${path}/${action}`, body });
      setError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  }
  const inbox = useInbox().data;
  const read = useInboxAction();
  const readIds = useRef(new Set<string>());
  useEffect(() => {
    if (!q.data) return;
    for (const t of inbox?.threads ?? []) {
      if (
        t.repo.toLowerCase() === repo.toLowerCase() &&
        t.number === number &&
        t.unread &&
        !readIds.current.has(t.id)
      ) {
        readIds.current.add(t.id);
        read.mutate({ id: t.id, action: "read" });
      }
    }
  }, [q.data, inbox?.threads, repo, number]);
  const item = q.data;
  const isPr = item?.__typename === "PullRequest";
  const launchControl = (
          <LaunchButton
            small
            ctx={{
              projectId: project?.id ?? null,
              githubRefs: item
                ? [reference(repo, item, isPr ? "pr" : "issue")]
                : [],
            }}
          />
  );
  const headerActions = (
        <div className="flex flex-wrap gap-2">
          {!phone && launchControl}
          {isPr && item && <button className="btn btn-sm" onClick={() => {
            useComposer.getState().start({ projectId: project?.id ?? null, githubRefs: [reference(repo, item, "pr")] });
            useComposer.getState().update((s) => ({ ...s, text: `Review ${repo}#${number} at commit ${item.headRefOid}. Inspect the changes and relevant context, summarize concrete findings, and save a local suggested review using launchpad suggest review --repo ${repo} --number ${number} --head ${item.headRefOid} --body-file <file> with your session id. Use --event request-changes for blocking findings, otherwise a comment review. Do not post to GitHub or change the code.` }));
          }}>Review with agent</button>}
          {item && (
            <AddContext
              refItem={reference(repo, item, isPr ? "pr" : "issue")}
            />
          )}
          <button
            className="btn btn-sm"
            disabled={refreshing}
            onClick={() => void refresh()}
          >
            {refreshing ? "Refreshing…" : "Refresh"}
          </button>
          <button
            className="btn btn-sm"
            onClick={() =>
              openGitHub(
                item?.url ?? `https://github.com/${repo}/issues/${number}`,
              )
            }
          >
            Open on GitHub
          </button>
          {item && (
            <span className="ml-auto text-xs text-ink-3">
              Cached {formatAge(item.fetched_at)} ago
            </span>
          )}
        </div>
  );
  return (
    <div className="flex h-full flex-col">
      {phone ? <PhoneHeader title={item?.title ?? `#${number}`} subtitle={`${repo} #${number} · ${item?.merged ? "merged" : item?.isDraft ? "draft" : item?.state.toLowerCase() ?? "loading"}`} primary={launchControl} actions={headerActions} details={<>
        {isPr && <p className="break-all font-mono text-xs text-ink-3">{item.headRefName} → {item.baseRefName} · {item.headRefOid?.slice(0, 7)}</p>}
        {item && <Controls key={item.updatedAt} item={item} act={act} busy={busy} />}
      </>} /> : (
      <header className="flex-none border-b border-line p-4">
        <div className="text-xs text-ink-3">
          {repo} #{number} ·{" "}
          {item?.merged
            ? "merged"
            : item?.isDraft
              ? "draft"
              : item?.state.toLowerCase()}
        </div>
        <h2 className="mt-1 mb-3 text-lg font-semibold">
          {item?.title ?? `#${number}`}
        </h2>
        {isPr && (
          <p className="mb-3 text-xs text-ink-3">
            {item.headRefName} → {item.baseRefName} ·{" "}
            {item.headRefOid?.slice(0, 7)}
          </p>
        )}
        {headerActions}
      </header>
      )}
      {(error || q.error) && (
        <p className="m-0 border-b border-line p-3 text-gh-closed">
          {error ?? String(q.error)}
        </p>
      )}
      {item && <DraftBanner item={item} onReview={reviewDraft} />}
      {isPr && (
        <nav className="phone-tabs flex gap-2 border-b border-line px-4 py-2" aria-label="Pull request views">
          {["conversation", "files", "threads"].map((t) => (
            <button
              key={t}
              className={`btn btn-sm ${tab === t && !phone ? "bg-select!" : ""}`}
              aria-pressed={tab === t}
              onClick={() => setTab(t)}
            >
              {t === "files"
                ? "Files changed"
                : t === "threads"
                  ? "Review threads"
                  : "Conversation"}
            </button>
          ))}
        </nav>
      )}
      {item ? (
        <div className="flex min-h-0 flex-1">
          <div className="flex min-w-0 flex-1 flex-col">
            {tab === "files" && isPr ? (
              <FilesChanged
                repo={repo}
                number={number}
                head={item.headRefOid!}
                base={item.baseRefOid}
              />
            ) : tab === "threads" && isPr ? (
              <ReviewThreads item={item} />
            ) : (
              <Conversation item={item} focus={focus} />
            )}
            <div
              className="flex-none border-t border-line bg-chrome p-3"
              ref={box}
              onFocus={() => setComposing(true)}
            >
              {failedPost && <button className="btn btn-sm mb-2" disabled={busy} onClick={() => {
                if (window.confirm("Check GitHub for this comment or review first. Start another attempt only if it was not posted; otherwise it could be duplicated.")) {
                  void abandonOperation(failedPost.path, failedPost.body).then(() => { setFailedPost(null); setError(null); }).catch(toastError);
                }
              }}>I checked GitHub — start a new attempt</button>}
              <textarea
                className={`field-input ${composing || draft.trim() ? "h-24 resize-y" : "h-[34px] resize-none overflow-hidden"}`}
                rows={1}
                aria-label="Comment or review"
                placeholder="Write a comment or review…"
                value={draft}
                disabled={busy}
                onChange={(e) => {
                  reply.set(e.target.value);
                }}
              />
              {(composing || draft.trim()) && <div className="mt-2 flex flex-wrap items-center gap-2">
                <button
                  className="btn btn-primary btn-sm"
                  disabled={busy || !draft.trim()}
                  onClick={() =>
                    void act("comment", { body: draft }).then((ok) => {
                      if (ok) void reply.clear();
                    })
                  }
                >
                  Post comment
                </button>
                {isPr && item.state === "OPEN" && (
                  <>
                    <select
                      className="field-input w-auto!"
                      aria-label="Review type"
                      value={event}
                      onChange={(e) => setEvent(e.target.value)}
                    >
                      <option value="COMMENT">Comment review</option>
                      <option value="APPROVE">Approve</option>
                      <option value="REQUEST_CHANGES">Request changes</option>
                    </select>
                    <button
                      className="btn btn-sm"
                      disabled={busy || (event !== "APPROVE" && !draft.trim())}
                      onClick={() =>
                        void act("review", {
                          body: draft,
                          event,
                          commit_id: item.headRefOid,
                        }).then((ok) => {
                          if (ok) void reply.clear();
                        })
                      }
                    >
                      Submit review
                    </button>
                  </>
                )}
                <span className="ml-auto text-xs text-ink-3">
                  Posts when you click.
                </span>
              </div>}
            </div>
          </div>
          {!phone && <Controls key={item.updatedAt} item={item} act={act} busy={busy} />}
        </div>
      ) : (
        <p className="p-4 text-ink-3">Fetching this item for the first time…</p>
      )}
    </div>
  );
}

/** Shown on every tab while an agent draft for this item waits for the owner. */
function DraftBanner({ item, onReview }: { item: Item; onReview: (id: string | null) => void }) {
  const all = useSuggestions({ kind: "git_hub", repo: item.repo, number: item.number }).data;
  const waiting = all.filter((s) => s.status !== "posted");
  if (!waiting.length) return null;
  const describe = (s: (typeof waiting)[number]) => s.kind === "review" ? `${s.event === "REQUEST_CHANGES" ? "Request changes" : s.event === "APPROVE" ? "Approve" : "Comment"} review` : "Comment";
  const first = waiting[0];
  return (
    <div className="flex flex-none flex-wrap items-center gap-2 border-b border-teal-ink/30 bg-teal-soft px-4 py-1.5 text-[12.5px]">
      <b className="text-teal-ink">{waiting.length === 1 ? "Agent draft" : `${waiting.length} agent drafts`}:</b>
      <span>{waiting.length === 1 ? describe(first) : waiting.map(describe).join(" · ")}</span>
      <span className="text-ink-3">· by {first.author}{["posting", "uncertain"].includes(first.status) ? " · publication attempted, check GitHub" : ""}</span>
      <button className="btn btn-primary btn-sm ml-auto" onClick={() => onReview(first.id)}>Review &amp; post</button>
    </div>
  );
}
