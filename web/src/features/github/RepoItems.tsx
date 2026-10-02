import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { toastError } from "@/components/Toast";
import { LaunchButton } from "@/features/launch/LaunchButton";
import type { Predraft } from "@/gen/Predraft";
import { LabelChip } from "./LabelChip";
import type { ListItem, RepoItems as Items } from "./types";
import { AddContext, itemKey, itemPath, openGitHub, reference } from "./lib";
const Row = memo(function Row({
  repo,
  i,
  kind,
  selected,
  toggle,
}: {
  repo: string;
  i: ListItem;
  kind: string;
  selected: boolean;
  toggle: () => void;
}) {
  const qc = useQueryClient();
  return (
    <div className="flex h-[76px] items-center gap-2 border-b border-line-2 px-3">
      <input
        type="checkbox"
        aria-label={`Select #${i.number}`}
        checked={selected}
        onChange={toggle}
      />
      <Link
        className="min-w-0 flex-1 no-underline text-ink"
        to="/github/$owner/$repo/$number"
        params={{
          owner: repo.split("/")[0],
          repo: repo.split("/")[1],
          number: String(i.number),
        }}
        onMouseEnter={() =>
          void qc.prefetchQuery({
            queryKey: itemKey(repo, i.number),
            queryFn: () => api.get(itemPath(repo, i.number)),
            staleTime: 60_000,
          })
        }
      >
        <span className="block truncate font-semibold">{i.title}</span>
        <span className="text-xs text-ink-3">
          #{i.number} · {i.isDraft ? "draft" : i.state.toLowerCase()} ·{" "}
          {i.author.login}
        </span>
        <span
          className="mt-1 flex gap-1 overflow-hidden"
          title={i.labels?.nodes.map((l) => l.name).join(", ")}
        >
          {i.labels?.nodes.map((l) => (
            <LabelChip key={l.name} label={l} />
          ))}
        </span>
      </Link>
      <AddContext refItem={reference(repo, i, kind)} />
      <button
        className="btn btn-sm"
        title="Open on GitHub"
        onClick={() => openGitHub(i.url)}
      >
        GitHub
      </button>
    </div>
  );
});
export function RepoItems({
  repo,
  projectId,
}: {
  repo: string;
  projectId: string;
}) {
  const qc = useQueryClient();
  const key = ["repo-items", repo];
  const q = useQuery({
    queryKey: key,
    queryFn: () => api.get<Items>(`/api/repo-items/${repo}`),
    staleTime: 300_000,
    enabled: !!repo,
  });
  const local = useQuery({
    queryKey: ["predrafts"],
    queryFn: () => api.get<Predraft[]>("/api/predrafts"),
  });
  const [showArchived, setShowArchived] = useState(false);
  const predrafts = (local.data ?? []).filter(
    (p) => p.project_id === projectId && (showArchived || !p.archived),
  );
  const [tab, setTab] = useState<"prs" | "issues">("prs");
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [labelFilter, setLabelFilter] = useState("all");
  const [busy, setBusy] = useState(false);
  async function refresh() {
    setBusy(true);
    try {
      qc.setQueryData(key, await api.post<Items>(`/api/repo-items/${repo}`));
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    if (
      q.data &&
      (Date.now() - q.data.fetched_at > 300_000 ||
        [...q.data.prs, ...q.data.issues].some((i) => !i.labels))
    )
      void refresh();
  }, [q.data?.fetched_at]);
  const labels = useMemo(
    () =>
      [
        ...new Set(
          [...(q.data?.prs ?? []), ...(q.data?.issues ?? [])].flatMap(
            (i) => i.labels?.nodes.map((l) => l.name) ?? [],
          ),
        ),
      ].sort((a, b) => a.localeCompare(b)),
    [q.data],
  );
  const items = q.data?.[tab] ?? [];
  const githubList = items.filter(
    (i) =>
      labelFilter === "all" ||
      (labelFilter === "none"
        ? i.labels?.nodes.length === 0
        : i.labels?.nodes.some((l) => l.name === labelFilter.slice(6))),
  );
  const list = [...(tab === "prs" ? predrafts : []), ...githubList];
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: list.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 76,
    overscan: 3,
    useFlushSync: false,
  });
  useEffect(() => {
    ref.current?.scrollTo({ top: 0 });
  }, [tab, labelFilter]);
  const refs = [
    ...(q.data?.prs ?? [])
      .filter((i) => selected[`pr:${i.number}`])
      .map((i) => reference(repo, i, "pr")),
    ...(q.data?.issues ?? [])
      .filter((i) => selected[`issue:${i.number}`])
      .map((i) => reference(repo, i, "issue")),
  ];
  return (
    <section className="rounded-lg border border-line overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 bg-chrome p-3">
        <button
          className={`btn ${tab === "prs" ? "bg-select!" : ""}`}
          onClick={() => setTab("prs")}
        >
          Pull requests {(q.data?.prs.length ?? 0) + predrafts.length}
        </button>
        <button
          className={`btn ${tab === "issues" ? "bg-select!" : ""}`}
          onClick={() => setTab("issues")}
        >
          Issues {q.data?.issues.length ?? ""}
        </button>
        <button
          className="btn ml-auto"
          disabled={busy || !repo}
          onClick={() => void refresh()}
        >
          {busy ? "Refreshing…" : "Refresh"}
        </button>
        <button
          className="btn"
          disabled={!repo}
          onClick={() =>
            openGitHub(
              `https://github.com/${repo}/${tab === "prs" ? "pulls" : "issues"}`,
            )
          }
        >
          Open on GitHub
        </button>
        <label className="flex items-center gap-1 text-xs">
          <input
            type="checkbox"
            checked={showArchived}
            onChange={(e) => setShowArchived(e.target.checked)}
          />
          Discarded local PRs
        </label>
      </div>
      {repo && (
        <div className="flex flex-wrap items-center gap-2 border-b border-line bg-paper px-3 py-2 text-xs">
          <label className="flex items-center gap-2">
            GitHub label
            <select
              className="field-input w-auto!"
              value={labelFilter}
              onChange={(e) => setLabelFilter(e.target.value)}
            >
              <option value="all">All labels</option>
              <option value="none">No labels</option>
              {labels.map((name) => (
                <option key={name} value={`label:${name}`}>
                  {name}
                </option>
              ))}
            </select>
          </label>
          {labelFilter !== "all" && (
            <button
              className="btn btn-sm"
              onClick={() => setLabelFilter("all")}
            >
              Clear filter
            </button>
          )}
          <span className="text-ink-3">
            {githubList.length} of {items.length} recent open{" "}
            {tab === "prs" ? "pull requests" : "issues"}
            {tab === "prs" &&
              predrafts.length > 0 &&
              ` · ${predrafts.length} local PRs`}
          </span>
        </div>
      )}
      {refs.length > 0 && (
        <div className="flex gap-2 p-3">
          <span>{refs.length} selected</span>
          <LaunchButton
            small
            ctx={{ projectId, githubRefs: refs }}
            label="Launch with selection"
          />
          <button className="btn btn-sm" onClick={() => setSelected({})}>
            Clear
          </button>
        </div>
      )}
      {q.error && <p className="p-3 text-gh-closed">{String(q.error)}</p>}
      <div ref={ref} className="h-[456px] overflow-auto">
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((row) => {
            const i = list[row.index];
            if ("branch" in i)
              return (
                <div
                  key={i.id}
                  className="absolute w-full"
                  style={{ transform: `translateY(${row.start}px)` }}
                >
                  <LocalRow p={i} />
                </div>
              );
            const kind = tab === "prs" ? "pr" : "issue",
              id = `${kind}:${i.number}`;
            return (
              <div
                key={id}
                className="absolute w-full"
                style={{ transform: `translateY(${row.start}px)` }}
              >
                <Row
                  repo={repo}
                  i={i}
                  kind={kind}
                  selected={!!selected[id]}
                  toggle={() => setSelected((s) => ({ ...s, [id]: !s[id] }))}
                />
              </div>
            );
          })}
          {!list.length && (
            <p className="p-4 text-ink-3">
              {q.isPending && repo
                ? "Fetching recent items…"
                : labelFilter === "all"
                  ? "No open items."
                  : "No recent open items match this label."}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

const LocalRow = memo(function LocalRow({ p }: { p: Predraft }) {
  return (
    <Link
      to="/predrafts/$id"
      params={{ id: p.id }}
      className="flex h-[76px] items-center gap-3 border-b border-line-2 bg-[#eaf8f5] px-3 no-underline text-teal-ink"
    >
      <span className="rounded border border-teal-ink px-2 py-1 text-[11px]">
        Local
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-semibold">{p.title}</span>
        <span className="text-xs">
          {p.archived
            ? "Discarded"
            : p.published_number
              ? `Published #${p.published_number}`
              : "Pre-draft PR"}{" "}
          · {p.branch}
        </span>
      </span>
      <span className="text-xs">Review</span>
    </Link>
  );
});
