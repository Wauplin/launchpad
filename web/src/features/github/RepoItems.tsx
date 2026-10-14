import { Icon } from "@/components/Icon";
import { usePhone } from "@/lib/phone";
import { PhoneActions } from "@/mobile/PhoneHeader";
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
import { itemKey, itemPath, openGitHub, reference } from "./lib";
import { useComposer } from "@/stores/composer";
const Row = memo(function Row({
  repo,
  i,
  selected,
  toggle,
}: {
  repo: string;
  i: ListItem;
  selected: boolean;
  toggle: () => void;
}) {
  const qc = useQueryClient();
  return (
    <div className="flex h-[52px] items-center gap-2.5 border-b border-line-2 px-3">
      <input
        type="checkbox"
        aria-label={`Select #${i.number} for a launch`}
        title="Select for a launch"
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
        <span className="block truncate font-semibold hover:underline">{i.title}</span>
        <span
          className="flex items-center gap-1.5 overflow-hidden text-xs whitespace-nowrap text-ink-3"
          title={i.labels?.nodes.map((l) => l.name).join(", ")}
        >
          #{i.number} · {i.isDraft ? "draft" : i.state.toLowerCase()} · {i.author.login}
          {i.labels?.nodes.map((l) => (
            <LabelChip key={l.name} label={l} />
          ))}
        </span>
      </Link>
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
  const phone = usePhone();
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
  const composerOpen = useComposer((s) => s.open);
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
        [...(q.data.prs ?? []), ...(q.data.issues ?? [])].some((i) => !i.labels))
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
    estimateSize: () => 52,
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
    <section className="rounded-lg border border-line overflow-clip">
      {phone ? <div className="flex min-w-0 items-center gap-1 border-b border-line bg-chrome px-2">
        <div className="phone-tabs flex min-w-0 flex-1" role="group" aria-label="Project GitHub items"><button className="btn btn-sm" aria-pressed={tab === "prs"} onClick={() => setTab("prs")}>PRs {(q.data?.prs?.length ?? 0) + predrafts.length}</button><button className="btn btn-sm" aria-pressed={tab === "issues"} onClick={() => setTab("issues")}>Issues {q.data?.issues?.length ?? 0}</button></div>
        <button className="iconbtn" aria-label="Refresh project items" title="Refresh project items" disabled={busy || !repo} onClick={() => void refresh()}><Icon name="refresh" size={16} /></button>
        <PhoneActions label="Project item options" closeOnAction={false} details={<div className="grid gap-3"><button className="btn" disabled={!repo} onClick={() => openGitHub(`https://github.com/${repo}/${tab === "prs" ? "pulls" : "issues"}`)}>Open on GitHub</button><label className="flex min-h-11 items-center gap-2 text-sm"><input type="checkbox" checked={showArchived} onChange={(e) => setShowArchived(e.target.checked)} />Discarded local PRs</label></div>} />
      </div> : <>
      <div className="flex flex-wrap items-center gap-2 bg-chrome p-3">
        <button
          className={`btn ${tab === "prs" ? "bg-select!" : ""}`}
          onClick={() => setTab("prs")}
        >
          Pull requests {(q.data?.prs?.length ?? 0) + predrafts.length}
        </button>
        <button
          className={`btn ${tab === "issues" ? "bg-select!" : ""}`}
          onClick={() => setTab("issues")}
        >
          Issues {q.data?.issues?.length ?? ""}
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
      </>}
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
      {q.error && <p className="p-3 text-gh-closed">{String(q.error)}</p>}
      <div ref={ref} className="overflow-auto" style={list.length ? { height: Math.min(list.length * 52, 468) } : undefined}>
        <div className="relative" style={list.length ? { height: v.getTotalSize() } : undefined}>
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
                  selected={!!selected[id]}
                  toggle={() => setSelected((s) => ({ ...s, [id]: !s[id] }))}
                />
              </div>
            );
          })}
          {!list.length && (
            <p className="m-0 px-3 py-2 text-[12.5px] text-ink-3">
              {q.isPending && repo
                ? "Fetching recent items…"
                : labelFilter === "all"
                  ? "No open items."
                  : "No recent open items match this label."}
            </p>
          )}
        </div>
      </div>
      {refs.length > 0 && (
        <div className="sticky bottom-0 z-10 flex items-center gap-2 border-t border-line bg-teal-soft px-3 py-2 text-[12.5px]">
          <b>{refs.length} selected</b>
          <span className="text-ink-3">as context for an agent</span>
          <span className="ml-auto flex gap-2">
            <LaunchButton small ctx={{ projectId, githubRefs: refs }} label="Launch with these" />
            {composerOpen && <button className="btn btn-sm" title="Add the selected items to the open launch draft" onClick={() => { useComposer.getState().start({ githubRefs: refs }); setSelected({}); }}>Add to draft</button>}
            <button className="btn btn-sm" onClick={() => setSelected({})}>Clear</button>
          </span>
        </div>
      )}
    </section>
  );
}

const LocalRow = memo(function LocalRow({ p }: { p: Predraft }) {
  return (
    <Link
      to="/predrafts/$id"
      params={{ id: p.id }}
      className="flex h-[52px] items-center gap-3 border-b border-line-2 bg-teal-soft px-3 no-underline text-teal-ink"
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
