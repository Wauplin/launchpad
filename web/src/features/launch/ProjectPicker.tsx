import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Project } from "@/gen/Project";
import { Icon } from "@/components/Icon";
import { MenuItem } from "@/components/Popover";

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
    <MenuItem icon={<Icon name="folder" size={13} />} label={project.name} detail={project.description || project.gh_repo || project.path} active={active} onClick={() => onPick(project)} />
  </div>;
});

export function ProjectPicker({ projects, workspaceName, hasProject, selected, temporary, projectsRoot, onPick }: {
  projects: Project[];
  workspaceName?: string;
  hasProject: (id: string) => boolean;
  selected?: string;
  temporary: boolean;
  projectsRoot: string;
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
  }).sort((a, b) => (workspaceName ? Number(hasProject(b.project.id)) - Number(hasProject(a.project.id)) : 0) || a.score - b.score);
  const rows: (Project | null)[] = matches.map(({ project }) => project);
  const firstOther = workspaceName ? rows.findIndex((project) => project && !hasProject(project.id)) : -1;
  if (firstOther >= 0) rows.splice(firstOther, 0, null);
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => scroll.current, estimateSize: (index) => rows[index] ? 48 : 24, getItemKey: (index) => rows[index]?.id ?? "outside-workspace", overscan: 4, useFlushSync: false });
  const height = Math.min(matches.length * 48 + (firstOther >= 0 ? 24 : 0), 200);
  return <div className="grid gap-1">
    <input autoFocus className="field-input w-full" aria-label="Find project" placeholder="Find a project…" value={search} onChange={(e) => { setSearch(e.target.value); scroll.current?.scrollTo({ top: 0 }); }} />
    <MenuItem icon={<Icon name="plus" size={13} />} label="New temporary folder" detail={`in ${projectsRoot}`} active={temporary} onClick={() => onPick(null)} />
    <div ref={scroll} className="overflow-y-auto" style={{ height }}>
      <div className="relative" style={{ height: virtual.getTotalSize() }}>
        {virtual.getVirtualItems().map((row) => {
          const project = rows[row.index];
          return <div key={row.key} className="absolute left-0 top-0 w-full" style={{ transform: `translateY(${row.start}px)`, height: row.size }}>
            {project ? <ProjectRow project={project} active={project.id === selected} onPick={onPick} /> : <div className="px-[9px] pt-2 pb-1 seclabel" title="Launching here adds the project to this workspace">Not in {workspaceName}</div>}
          </div>;
        })}
      </div>
    </div>
    {!matches.length && <div className="px-2 py-1.5 text-xs text-ink-3">{projects.length ? "No matching projects." : "No project yet: add some in Projects."}</div>}
  </div>;
}
