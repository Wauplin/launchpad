// Follow-up context uses the same picker as the launch composer, with compact removable chips.
import type { MessageContext } from "@/gen/MessageContext";
import { Icon, type IconName } from "@/components/Icon";
import { SkillChips } from "@/features/launch/SkillChips";
import { AddContextMenu } from "@/features/launch/AddContextMenu";
import { hostOf } from "@/lib/hosts";
import { useProjects, useSession, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";

export const EMPTY_CONTEXT: MessageContext = { context_projects: [], github_refs: [], agent_traces: [], mentions: [], skills: [] };
export function hasMessageContext(context: MessageContext): boolean {
  return !!(context.context_projects.length || context.github_refs.length || context.agent_traces.length || context.skills?.length);
}

export function MessageContextPicker({ sessionId, value, onChange, disabled }: { sessionId: string; value: MessageContext; onChange: (value: MessageContext) => void; disabled: boolean }) {
  const projects = useProjects().data ?? [];
  const sessions = useSessions().data ?? [];
  const session = useSession(sessionId);
  const scope = useScope();
  const noFolders = session?.launch_spec.target.kind === "assistant" || (session?.launch_spec.target.kind === "browser" && !session.launch_spec.target.project_id);
  const available = (noFolders ? [] : projects).filter((p) => p.id !== session?.project_id && hostOf(p.path) === hostOf(session?.cwd ?? "") && !value.context_projects.some((c) => c.project_id === p.id));
  const addProject = (id: string) => onChange({ ...value, context_projects: [...value.context_projects, { project_id: id, branch: null, editable: false }] });
  const recent: { key: string; label: string; add: () => void }[] = [];
  const seen = new Set<string>();
  for (const s of [...sessions].filter(scope.hasSession).sort((a, b) => b.started_at - a.started_at)) {
    for (const c of s.launch_spec.context_projects) {
      const p = available.find((p) => p.id === c.project_id);
      if (!p || seen.has(p.id)) continue;
      seen.add(p.id);
      recent.push({ key: p.id, label: p.name, add: () => addProject(p.id) });
    }
    for (const g of s.launch_spec.github_refs) {
      if (seen.has(g.url) || value.github_refs.some((r) => r.url === g.url)) continue;
      seen.add(g.url);
      recent.push({ key: g.url, label: g.number ? `${g.repo}#${g.number}` : g.title ?? g.url, add: () => onChange({ ...value, github_refs: [...value.github_refs, g] }) });
    }
    if (recent.length >= 6) break;
  }
  const chip = (key: string, icon: IconName, label: string, detail: string, remove: () => void) => <span key={key} title={detail} className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-md border border-line bg-chrome pl-2 text-xs text-ink-2">
    <Icon name={icon} size={12} className="shrink-0" />
    <span className="truncate">{label}</span>
    <button className="iconbtn shrink-0" aria-label={`Remove ${label}`} disabled={disabled} onClick={remove}><Icon name="x" size={12} /></button>
  </span>;
  return <div className="flex min-w-0 flex-wrap items-center gap-1.5">
    <AddContextMenu compact disabled={disabled} projects={available} recent={recent} sessions={sessions.filter((s) => s.id !== sessionId && scope.hasSession(s))} selectedTraces={value.agent_traces}
      onProject={(p) => addProject(p.id)}
      onGitHub={(g) => { if (!value.github_refs.some((r) => r.url === g.url)) onChange({ ...value, github_refs: [...value.github_refs, g] }); }}
      onToggleTrace={(trace) => onChange({ ...value, agent_traces: value.agent_traces.some((t) => t.session_id === trace.session_id) ? value.agent_traces.filter((t) => t.session_id !== trace.session_id) : [...value.agent_traces, trace] })} />
    <SkillChips skills={value.skills} disabled={disabled} onRemove={(path) => onChange({ ...value, skills: (value.skills ?? []).filter((v) => v.path !== path) })} />
    {value.context_projects.map((c) => chip(c.project_id, "folder", projects.find((p) => p.id === c.project_id)?.name ?? "Removed folder", "Read-only folder reference", () => onChange({ ...value, context_projects: value.context_projects.filter((r) => r.project_id !== c.project_id) })))}
    {value.github_refs.map((g) => chip(g.url, g.kind === "pr" ? "pr" : "issue", g.number ? `${g.repo}#${g.number}` : g.title ?? g.url, g.url, () => onChange({ ...value, github_refs: value.github_refs.filter((r) => r.url !== g.url) })))}
    {value.agent_traces.map((t) => chip(`${t.session_id}:${t.through_seq}`, "agents", sessions.find((s) => s.id === t.session_id)?.title ?? "Removed agent", `Stored trace through event ${t.through_seq}`, () => onChange({ ...value, agent_traces: value.agent_traces.filter((r) => r.session_id !== t.session_id || r.through_seq !== t.through_seq) })))}
  </div>;
}
