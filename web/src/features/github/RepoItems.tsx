import { Icon } from "@/components/Icon";
import { usePhone } from "@/lib/phone";
import { PhoneActions } from "@/mobile/PhoneHeader";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { toastError } from "@/components/Toast";
import { MenuItem, Popover } from "@/components/Popover";
import { ErrorState, Loading, Tabs } from "@/components/ui";
import { formatAge } from "@/lib/format";
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
          #{i.number} · {i.isDraft ? "GitHub draft" : i.state.toLowerCase()} · {i.author.login}
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
  const colors = useMemo(() => new Map([...(q.data?.prs ?? []), ...(q.data?.issues ?? [])].flatMap((i) => i.labels?.nodes ?? []).map((l) => [l.name, l.color])), [q.data]);
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
      <div className="flex min-w-0 flex-wrap items-center gap-2 border-b border-line bg-chrome px-3">
        <Tabs<"prs" | "issues"> label="Folder GitHub items" value={tab} onChange={setTab} className="min-w-0 flex-1" items={[{ id: "prs", label: phone ? "PRs" : "Pull requests", count: (q.data?.prs?.length ?? 0) + predrafts.length }, { id: "issues", label: "Issues", count: q.data?.issues?.length ?? 0 }]} />
        {q.data && !phone && <span className="text-[12px] text-ink-3">Synced {formatAge(q.data.fetched_at)} ago</span>}
        <button className="iconbtn" aria-label="Refresh from GitHub" title="Refresh from GitHub" disabled={busy || !repo} onClick={() => void refresh()}><Icon name="refresh" size={phone ? 16 : 13} className={busy ? "animate-spin" : ""} /></button>
        {phone ? <PhoneActions label="Item options" closeOnAction={false} details={<div className="grid gap-3"><button className="btn" disabled={!repo} onClick={() => openGitHub(`https://github.com/${repo}/${tab === "prs" ? "pulls" : "issues"}`)}>Open on GitHub</button><label className="flex min-h-11 items-center gap-2 text-sm"><input type="checkbox" checked={showArchived} onChange={(e) => setShowArchived(e.target.checked)} />Discarded Local PRs</label></div>} />
          : <Popover align="right" width={240} label="More options" trigger={({ open, toggle }) => <button className="iconbtn" aria-label="More options" aria-expanded={open} title="More options" onClick={toggle}><Icon name="more" size={15} /></button>}>
            {(close) => <div className="grid gap-1">
              <MenuItem icon={<Icon name="ext" size={13} />} label="Open on GitHub" disabled={!repo} onClick={() => { openGitHub(`https://github.com/${repo}/${tab === "prs" ? "pulls" : "issues"}`); close(); }} />
              <label className="flex items-center gap-2 px-[9px] py-[7px] text-[12px]"><input type="checkbox" checked={showArchived} onChange={(e) => setShowArchived(e.target.checked)} />Show discarded Local PRs</label>
            </div>}
          </Popover>}
      </div>
      {repo && (
        <div className="flex flex-wrap items-center gap-2 border-b border-line bg-paper px-3 py-2 text-[12px]">
          <Popover width={260} label="GitHub label" trigger={({ open, toggle }) => <button className="lp-chip" aria-pressed={labelFilter !== "all"} aria-expanded={open} onClick={toggle}>Label: {labelFilter === "all" ? "all" : labelFilter === "none" ? "no labels" : labelFilter.slice(6)}<Icon name="chev" size={12} /></button>}>
            {(close) => <div className="grid max-h-72 gap-0.5 overflow-auto">
              <MenuItem label="All labels" active={labelFilter === "all"} onClick={() => { setLabelFilter("all"); close(); }} />
              <MenuItem label="No labels" active={labelFilter === "none"} onClick={() => { setLabelFilter("none"); close(); }} />
              {labels.map((name) => <MenuItem key={name} label={<LabelChip label={{ name, color: colors.get(name) ?? "d0d7de" }} />} active={labelFilter === `label:${name}`} onClick={() => { setLabelFilter(`label:${name}`); close(); }} />)}
            </div>}
          </Popover>
          {labelFilter !== "all" && <button className="btn btn-sm" onClick={() => setLabelFilter("all")}>Clear filter</button>}
          <span className="text-ink-3">
            {githubList.length} of {items.length} recent open {tab === "prs" ? "pull requests" : "issues"}
            {tab === "prs" && predrafts.length > 0 && ` · ${predrafts.length} Local PR${predrafts.length === 1 ? "" : "s"}`}
          </span>
        </div>
      )}
      {q.error && <ErrorState className="m-2" error={q.error} onRetry={() => void refresh()} />}
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
          {!list.length && (q.isPending && repo ? <Loading what="recent items" /> : <p className="m-0 px-3 py-2 text-[12px] text-ink-3">{labelFilter === "all" ? "No open items." : "No recent open items match this label."}</p>)}
        </div>
      </div>
      {refs.length > 0 && (
        <div className="sticky bottom-0 z-10 flex items-center gap-2 border-t border-line bg-teal-soft px-3 py-2 text-[12px]">
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
      <Icon name="pr-local" size={15} />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-semibold">{p.title}</span>
        <span className="text-xs">
          {p.archived
            ? "Discarded"
            : p.published_number
              ? `Published #${p.published_number}`
              : "Local PR"}{" "}
          · {p.branch}
        </span>
      </span>
      <span className="text-[12px]">Review</span>
    </Link>
  );
});
