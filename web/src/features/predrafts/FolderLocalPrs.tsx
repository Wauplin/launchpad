// A folder's Local PRs (open ones highlighted, discarded ones on request), plus links to the repository's pull
// requests and issues on github.com: Launchpad does not list remote items.

import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import type { Predraft } from "@/gen/Predraft";
import { githubRepoUrl } from "@/lib/githubLinks";
import { externalLinkProps } from "@/lib/platform";

/** A Local PR still to publish or merge (not discarded, published or merged). */
export const isOpenLocalPr = (p: Predraft) => !p.archived && !p.published_url && !p.merged;

export function FolderLocalPrs({ repo, predrafts }: { repo: string | null; predrafts: Predraft[] }) {
  const [showArchived, setShowArchived] = useState(false);
  const list = predrafts.filter((p) => showArchived || !p.archived).sort((a, b) => Number(isOpenLocalPr(b)) - Number(isOpenLocalPr(a)) || b.created_at - a.created_at);
  const open = predrafts.filter(isOpenLocalPr).length;
  const discarded = predrafts.some((p) => p.archived);
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: list.length, getScrollElement: () => ref.current, estimateSize: () => 52, overscan: 3, useFlushSync: false });
  const pulls = repo ? githubRepoUrl(repo, "pulls") : null;
  const issues = repo ? githubRepoUrl(repo, "issues") : null;
  return (
    <section className="grid gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="seclabel">Local PRs · {open} open</div>
        <span className="ml-auto flex flex-wrap items-center gap-2 text-[12px]">
          {discarded && <label className="flex items-center gap-1.5 text-ink-3"><input type="checkbox" checked={showArchived} onChange={(e) => setShowArchived(e.target.checked)} />Show discarded</label>}
          {pulls && <a data-tour="folder.github" className="btn btn-sm no-underline" {...externalLinkProps(pulls)}><Icon name="ext" size={12} />Pull requests</a>}
          {issues && <a className="btn btn-sm no-underline" {...externalLinkProps(issues)}><Icon name="ext" size={12} />Issues</a>}
        </span>
      </div>
      {list.length ? (
        <div ref={ref} className="overflow-auto rounded-card border border-line" style={{ height: Math.min(list.length * 52, 468) + 2 }}>
          <div className="relative" style={{ height: v.getTotalSize() }}>
            {v.getVirtualItems().map((row) => (
              <div key={list[row.index].id} className="absolute w-full" style={{ transform: `translateY(${row.start}px)` }}>
                <LocalRow p={list[row.index]} />
              </div>
            ))}
          </div>
        </div>
      ) : <p className="m-0 text-[13px] text-ink-3">No Local PRs in this folder.</p>}
    </section>
  );
}

/** A Local PR row: highlighted with "Review" only while it is open; otherwise its state (merged, published, discarded). */
const LocalRow = memo(function LocalRow({ p }: { p: Predraft }) {
  const open = isOpenLocalPr(p);
  return (
    <Link
      to="/predrafts/$id"
      params={{ id: p.id }}
      className={`flex h-[52px] items-center gap-3 border-b border-line-2 px-3 no-underline ${open ? "bg-teal-soft text-teal-ink" : "text-ink-2"}`}
    >
      <Icon name="pr-local" size={16} />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-semibold">{p.title}</span>
        <span className="text-[12px]">
          {p.archived ? "Discarded" : p.merged ? `Merged into ${p.base_branch}` : p.published_number ? `Published #${p.published_number}` : "Local PR"} · {p.branch}
        </span>
      </span>
      {open && <span className="text-[12px]">Review</span>}
    </Link>
  );
});
