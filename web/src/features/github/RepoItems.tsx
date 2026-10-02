import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import { toastError } from "@/components/Toast";
import { LaunchButton } from "@/features/launch/LaunchButton";
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
    <div className="flex h-[56px] items-center gap-2 border-b border-line-2 px-3">
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
  });
  const [tab, setTab] = useState<"prs" | "issues">("prs");
  const [selected, setSelected] = useState<Record<string, boolean>>({});
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
    if (q.data && Date.now() - q.data.fetched_at > 300_000) void refresh();
  }, [q.data?.fetched_at]);
  const list = q.data?.[tab] ?? [];
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: list.length,
    getScrollElement: () => ref.current,
    estimateSize: () => 56,
    overscan: 3,
    useFlushSync: false,
  });
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
          Pull requests {q.data?.prs.length ?? ""}
        </button>
        <button
          className={`btn ${tab === "issues" ? "bg-select!" : ""}`}
          onClick={() => setTab("issues")}
        >
          Issues {q.data?.issues.length ?? ""}
        </button>
        <button
          className="btn ml-auto"
          disabled={busy}
          onClick={() => void refresh()}
        >
          {busy ? "Refreshing…" : "Refresh"}
        </button>
        <button
          className="btn"
          onClick={() =>
            openGitHub(
              `https://github.com/${repo}/${tab === "prs" ? "pulls" : "issues"}`,
            )
          }
        >
          Open on GitHub
        </button>
      </div>
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
      <div ref={ref} className="h-[336px] overflow-auto">
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((row) => {
            const i = list[row.index],
              kind = tab === "prs" ? "pr" : "issue",
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
              {q.isPending ? "Fetching recent items…" : "No open items."}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}
