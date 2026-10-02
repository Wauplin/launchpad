import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { toast } from "@/components/Toast";
import { useInbox, useInboxAction } from "@/features/inbox/InboxView";
import { LaunchButton } from "@/features/launch/LaunchButton";
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
import { LabelChip } from "./LabelChip";
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
function Conversation({ item }: { item: Item }) {
  const ref = useRef<HTMLDivElement>(null);
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
    count: events.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 160,
    overscan: 3,
    useFlushSync: false,
  });
  return (
    <div ref={ref} className="min-h-0 flex-1 overflow-auto p-4">
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((r) => (
          <div
            key={r.key}
            data-index={r.index}
            ref={v.measureElement}
            className="absolute left-0 top-0 w-full pb-3"
            style={{ transform: `translateY(${r.start}px)` }}
          >
            <EventCard event={events[r.index]} item={item} />
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
  const [reviewers, setReviewers] = useState("");
  const [labels, setLabels] = useState(
    item.labels.nodes.map((l) => l.name).join(", "),
  );
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
  const actual = allowed.some(([m]) => m === method)
    ? method
    : (allowed[0]?.[0] ?? method);
  return (
    <aside className="w-[260px] flex-none overflow-auto border-l border-line bg-chrome p-3 text-xs">
      <div className="grid gap-3">
        <div>
          <b>Labels</b>
          <div className="mt-2 flex flex-wrap gap-1">
            {item.labels.nodes.map((l) => (
              <LabelChip key={l.name} label={l} />
            ))}
          </div>
          <input
            className="field-input mt-2"
            aria-label="Labels"
            value={labels}
            onChange={(e) => setLabels(e.target.value)}
            placeholder="Labels, separated by commas"
          />
          <button
            className="btn btn-sm mt-2"
            disabled={busy}
            onClick={() =>
              void act("labels", {
                labels: labels
                  .split(",")
                  .map((s) => s.trim())
                  .filter(Boolean),
              })
            }
          >
            Save labels
          </button>
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
                  <input
                    className="field-input"
                    aria-label="Request reviewers"
                    placeholder="Usernames or team:slug, comma separated"
                    value={reviewers}
                    onChange={(e) => setReviewers(e.target.value)}
                  />
                  <button
                    className="btn btn-sm mt-2"
                    disabled={busy || !reviewers.trim()}
                    onClick={() => {
                      const names = reviewers
                        .split(",")
                        .map((s) => s.trim())
                        .filter(Boolean);
                      void act("reviewers", {
                        reviewers: names.filter((s) => !s.startsWith("team:")),
                        team_reviewers: names
                          .filter((s) => s.startsWith("team:"))
                          .map((s) => s.slice(5)),
                      });
                    }}
                  >
                    Request review
                  </button>
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
                <p>
                  {item.mergeable?.toLowerCase()} ·{" "}
                  {item.mergeStateStatus?.toLowerCase()}
                </p>
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
                  className="btn btn-primary mt-2 w-full"
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
  const draftKey = `github-reply:${repo}:${number}`;
  const [draft, setDraft] = useState(
    () => localStorage.getItem(draftKey) ?? "",
  );
  const [event, setEvent] = useState("COMMENT");
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
      toast("GitHub action completed");
      void qc.invalidateQueries({ queryKey: ["repo-items", repo] });
      await refresh();
      return true;
    } catch (e) {
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
  return (
    <div className="flex h-full flex-col">
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
        <div className="flex flex-wrap gap-2">
          <LaunchButton
            small
            ctx={{
              projectId: project?.id ?? null,
              githubRefs: item
                ? [reference(repo, item, isPr ? "pr" : "issue")]
                : [],
            }}
          />
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
      </header>
      {(error || q.error) && (
        <p className="m-0 border-b border-line p-3 text-gh-closed">
          {error ?? String(q.error)}
        </p>
      )}
      {isPr && (
        <nav className="flex gap-2 border-b border-line px-4 py-2">
          {["conversation", "files", "threads"].map((t) => (
            <button
              key={t}
              className={`btn btn-sm ${tab === t ? "bg-select!" : ""}`}
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
              />
            ) : tab === "threads" && isPr ? (
              <ReviewThreads item={item} />
            ) : (
              <Conversation item={item} />
            )}
            <div className="flex-none border-t border-line bg-chrome p-3">
              <textarea
                className="field-input h-20 resize-y"
                aria-label="Comment or review"
                placeholder="Write a comment or review…"
                value={draft}
                disabled={busy}
                onChange={(e) => {
                  setDraft(e.target.value);
                  localStorage.setItem(draftKey, e.target.value);
                }}
              />
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <button
                  className="btn btn-primary btn-sm"
                  disabled={busy || !draft.trim()}
                  onClick={() =>
                    void act("comment", { body: draft }).then((ok) => {
                      if (ok) {
                        setDraft("");
                        localStorage.removeItem(draftKey);
                      }
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
                          if (ok) {
                            setDraft("");
                            localStorage.removeItem(draftKey);
                          }
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
              </div>
            </div>
          </div>
          <Controls key={item.updatedAt} item={item} act={act} busy={busy} />
        </div>
      ) : (
        <p className="p-4 text-ink-3">Fetching this item for the first time…</p>
      )}
    </div>
  );
}
