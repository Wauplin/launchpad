import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";
import { HostLabel } from "@/components/HostLabel";
import { hostOf, linkLabel, useHosts } from "@/lib/hosts";

// Match characters in order, preferring contiguous matches and shorter gaps.
function matchScore(value: string, query: string): number | null {
  const text = value.toLowerCase();
  const contiguous = text.indexOf(query);
  if (contiguous >= 0) return contiguous / Math.max(text.length, 1);
  let position = -1;
  let score = text.length;
  for (const character of query) {
    const next = text.indexOf(character, position + 1);
    if (next < 0) return null;
    score += next - position - 1;
    position = next;
  }
  return score;
}

const ProjectRow = memo(function ProjectRow({ project, active, onPick }: { project: Project; active: boolean; onPick: (project: Project | null) => void }) {
  return <div className="grid h-[48px]">
    <MenuItem icon={<Icon name="folder" size={13} />} label={<span className="inline-flex items-center gap-1.5">{project.name}<HostLabel path={project.path} /></span>} detail={project.description || project.gh_repo || project.path} active={active} onClick={() => onPick(project)} />
  </div>;
});

export function ProjectPicker({ projects, workspaceName, hasProject, selected, temporary, projectsRoot, emptyMessage = "No folder yet: add one from the sidebar.", onPick }: {
  projects: Project[];
  workspaceName?: string;
  hasProject: (id: string) => boolean;
  selected?: string;
  temporary: boolean;
  projectsRoot?: string;
  emptyMessage?: string;
  onPick: (project: Project | null) => void;
}) {
  const [search, setSearch] = useState("");
  const scroll = useRef<HTMLDivElement>(null);
  const query = search.trim().toLowerCase();
  const matches = projects.flatMap((project) => {
    if (!query) return [{ project, score: 0 }];
    const scores = [project.name, project.gh_repo, project.gh_upstream, project.description, project.path]
      .flatMap((value) => {
        const score = value ? matchScore(value, query) : null;
        return score === null ? [] : [score];
      });
    return scores.length ? [{ project, score: Math.min(...scores) }] : [];
  }).sort((a, b) => (workspaceName ? Number(hasProject(b.project.id)) - Number(hasProject(a.project.id)) : 0) || (hostOf(a.project.path) ?? "").localeCompare(hostOf(b.project.path) ?? "") || a.score - b.score);
  // Grouped by machine (this computer first) when some projects live on SSH hosts, then by workspace.
  const hosts = useHosts();
  const byHost = projects.some((p) => hostOf(p.path));
  const rows: (Project | string)[] = [];
  let group: string | null = null;
  for (const { project } of matches) {
    const host = hostOf(project.path);
    const outside = !!workspaceName && !hasProject(project.id);
    const machine = host ? `${host}${(() => { const h = hosts.find((x) => x.host.id === host); return h && h.state !== "connected" ? ` · ${linkLabel[h.state].toLowerCase()}` : ""; })()}` : "This computer";
    const label = [outside ? `Not in ${workspaceName}` : null, byHost ? machine : null].filter(Boolean).join(" · ");
    if (label && label !== group) rows.push(label);
    group = label;
    rows.push(project);
  }
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => scroll.current, estimateSize: (index) => typeof rows[index] === "string" ? 24 : 48, getItemKey: (index) => { const r = rows[index]; return typeof r === "string" ? `group-${r}` : r.id; }, overscan: 4, useFlushSync: false });
  const height = Math.min(rows.reduce((n, r) => n + (typeof r === "string" ? 24 : 48), 0), 220);
  return <div className="grid gap-1">
    <input autoFocus className="field-input w-full" aria-label="Find folder" placeholder="Find a folder…" value={search} onChange={(e) => { setSearch(e.target.value); scroll.current?.scrollTo({ top: 0 }); }} />
    {projectsRoot !== undefined && <MenuItem icon={<Icon name="plus" size={13} />} label="New scratch folder" detail={`in ${projectsRoot}`} active={temporary} onClick={() => onPick(null)} />}
    <div ref={scroll} className="overflow-y-auto" style={{ height }}>
      <div className="relative" style={{ height: virtual.getTotalSize() }}>
        {virtual.getVirtualItems().map((row) => {
          const project = rows[row.index];
          return <div key={row.key} className="absolute left-0 top-0 w-full" style={{ transform: `translateY(${row.start}px)`, height: row.size }}>
            {typeof project !== "string" ? <ProjectRow project={project} active={project.id === selected} onPick={onPick} /> : <div className="truncate px-[9px] pt-2 pb-1 seclabel" title={project.startsWith("Not in") ? "Launching here adds the project to this workspace" : "Agents of these projects run on this machine"}>{project}</div>}
          </div>;
        })}
      </div>
    </div>
    {!matches.length && <div className="px-2 py-1.5 text-xs text-ink-3">{projects.length ? "No matching folder." : emptyMessage}</div>}
  </div>;
}
