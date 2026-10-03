// Composer drawer (sketch S2): stays open on the right while you navigate, so context can be added
// from anywhere. The preview shows the exact prompt sent to the agent.

import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import type { GitHubRef } from "@/gen/GitHubRef";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { LaunchSpec } from "@/gen/LaunchSpec";
import type { Preview } from "@/gen/Preview";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import { HarnessMark, HARNESS_NAME } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { MenuItem, Popover } from "@/components/Popover";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useConfig, useHarnessDefaults, useProjects, useSessions } from "@/lib/queries";
import { rememberSuccessfulSession, useComposer } from "@/stores/composer";
import { useLayout } from "@/stores/layout";
import { DelaySelector, conditionsError } from "./DelaySelector";
import { itemIcon, PromptField } from "./PromptField";

function Field({ label, hint, end, children }: { label: string; hint?: string; end?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="grid gap-1.5">
      <div className="flex items-center gap-2 text-[11.5px] font-[650] text-ink-2">
        {label}
        {hint && <span className="font-normal text-ink-3">{hint}</span>}
        {end && <span className="ml-auto">{end}</span>}
      </div>
      {children}
    </div>
  );
}

function SelectButton({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button onClick={onClick} className="flex h-8 w-full min-w-0 items-center gap-2 rounded-[7px] border border-line bg-paper px-2.5 text-left text-[12.5px] hover:border-[#c4ccd7]">
      <span className="flex min-w-0 flex-1 items-center gap-2 truncate">{children}</span>
      <Icon name="chev" size={14} className="flex-none text-ink-3" />
    </button>
  );
}

function Branch({ projectId, branch, onChange }: { projectId: string; branch: string | null; onChange: (b: string | null) => void }) {
  const q = useQuery({
    queryKey: ["branches", projectId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${projectId}/branches`),
    staleTime: 30_000,
  });
  return (
    <Popover
      width={260}
      trigger={({ toggle }) => (
        <button onClick={toggle} className="inline-flex h-5 items-center gap-1 rounded-[5px] border border-line bg-paper px-1.5 font-mono text-[11.5px] whitespace-nowrap">
          <Icon name="branch" size={12} />
          {branch ?? `checkout${q.data?.current ? ` (${q.data.current})` : ""}`}
          <Icon name="chev" size={12} className="text-ink-3" />
        </button>
      )}
    >
      {(close) => (
        <>
          <MenuItem label="Current checkout" detail={q.data?.current ?? undefined} active={branch === null} onClick={() => (onChange(null), close())} />
          {(q.data?.branches ?? []).map((b) => (
            <MenuItem key={b} icon={<Icon name="branch" size={13} />} label={<span className="font-mono text-xs">{b}</span>} active={b === branch} onClick={() => (onChange(b), close())} />
          ))}
          {q.isLoading && <div className="px-2 py-1 text-xs text-ink-3">Loading branches…</div>}
        </>
      )}
    </Popover>
  );
}

function GitHubLinkField({ onAdd }: { onAdd: (r: GitHubRef) => void }) {
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  if (!open) {
    return (
      <button className="btn btn-sm" onClick={() => setOpen(true)}>
        <Icon name="plus" size={12} />
        GitHub link
      </button>
    );
  }
  const add = async () => {
    if (!url.trim()) return;
    setBusy(true);
    try {
      onAdd(await api.post<GitHubRef>("/api/gh/resolve", { url: url.trim() }));
      setUrl("");
      setOpen(false);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-1 gap-1.5">
      <input
        autoFocus
        className="field-input h-6 flex-1 text-xs"
        placeholder="https://github.com/owner/repo/issues/123"
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void add();
          if (e.key === "Escape") setOpen(false);
        }}
      />
      <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => void add()}>
        {busy ? "Adding…" : "Add"}
      </button>
      <button className="btn btn-sm btn-ghost" onClick={() => setOpen(false)}>
        Cancel
      </button>
    </div>
  );
}

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

export function Composer() {
  const { open, spec, update, close, discard, openAfterLaunch, setOpenAfterLaunch, conditions, setConditions } = useComposer();
  const projects = useProjects().data ?? [];
  const sessions = useSessions().data;
  useEffect(() => {
    // A queued launch may have started while the browser was disconnected.
    for (const session of sessions ?? []) rememberSuccessfulSession(session);
  }, [sessions]);
  const config = useConfig().data?.config;
  const defaults = useHarnessDefaults(spec.harness).data;
  const navigate = useNavigate();
  const showSession = useLayout((s) => s.show);
  const [delayValid, setDelayValid] = useState(true);
  const [launching, setLaunching] = useState(false);
  const [showAll, setShowAll] = useState(false);
  const debouncedSpec = useDebounced(spec, 250);
  const preview = useQuery({
    queryKey: ["preview", debouncedSpec],
    queryFn: () => api.post<Preview>("/api/launch/preview", debouncedSpec),
    enabled: open,
    placeholderData: (prev) => prev,
  });

  const localPrs = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<import("@/gen/Predraft").Predraft[]>("/api/predrafts"), enabled: open });
  const existing = spec.target.kind === "existing_predraft" ? localPrs.data?.find((p) => p.id === (spec.target as { predraft_id: string }).predraft_id) : undefined;
  const recentSessions = sessions ?? [];
  if (!open) return null;

  const set = (f: (s: LaunchSpec) => LaunchSpec) => update(f);
  const target = (spec.target.kind === "project" || spec.target.kind === "predraft") ? projects.find((p) => p.id === (spec.target as { project_id: string }).project_id) : existing ? projects.find((p) => p.id === existing.project_id) : null;
  const repo = target?.gh_repo ?? target?.gh_upstream ?? null;
  const sorted = [...projects.filter((p) => !p.is_temp)].sort((a, b) => Number(b.pinned) - Number(a.pinned) || a.name.localeCompare(b.name));
  const model = spec.model ? defaults?.models.find((m) => m.id === spec.model) : defaults?.models.find((m) => m.id === defaults?.model);
  const efforts = model?.efforts.length ? model.efforts : (defaults?.efforts ?? []);
  const scheduled = conditions.length > 0;
  const canLaunch = delayValid && !conditionsError(conditions) && !!spec.harness && spec.target.kind !== "none" && !!preview.data?.prompt.trim();

  const addProjectContext = (p: Project) =>
    set((s) => (s.context_projects.some((c) => c.project_id === p.id) || ((s.target.kind === "project" || s.target.kind === "predraft") && s.target.project_id === p.id) ? s : { ...s, context_projects: [...s.context_projects, { project_id: p.id, branch: null }] }));

  const recentContexts: { key: string; label: string; add: () => void }[] = [];
  const seenContexts = new Set<string>();
  for (const session of recentSessions.filter((s) => s.launch_spec.harness && s.last_seq > 0 && s.status !== "error")) {
    for (const c of session.launch_spec.context_projects) {
      const p = projects.find((p) => p.id === c.project_id);
      const key = `project:${c.project_id}:${c.branch ?? ""}`;
      if (!p || seenContexts.has(key)) continue;
      seenContexts.add(key);
      if (p.id !== target?.id && !spec.context_projects.some((p) => p.project_id === c.project_id)) recentContexts.push({ key, label: `${p.name}${c.branch ? ` · ${c.branch}` : ""}`, add: () => set((s) => ({ ...s, context_projects: [...s.context_projects,c] })) });
    }
    for (const g of session.launch_spec.github_refs) {
      if (seenContexts.has(g.url)) continue;
      seenContexts.add(g.url);
      if (!spec.github_refs.some((r) => r.url === g.url)) recentContexts.push({ key:g.url, label:g.number ? `${g.repo}#${g.number}` : g.title ?? g.url, add:() => set((s) => ({ ...s, github_refs:[...s.github_refs,g] })) });
    }
    for (const f of session.launch_spec.files ?? []) {
      const key = JSON.stringify(f);
      if (seenContexts.has(key)) continue;
      seenContexts.add(key);
      if (!(spec.files ?? []).some((r) => JSON.stringify(r) === key)) recentContexts.push({ key, label:`${f.path}:${f.start_line}`, add:() => set((s) => ({ ...s, files:[...(s.files ?? []),f] })) });
    }
  }
  const launch = async () => {
    setLaunching(true);
    try {
      if (scheduled) {
        await api.post("/api/launch/schedules", { spec, conditions });
        discard();
        void navigate({ to: "/agents" });
        toast("Agent scheduled. You can start it now or cancel in Agents.");
        return;
      }
      const s = await api.post<Session>("/api/sessions", spec);
      if (s.status !== "error") useComposer.getState().rememberLaunch(spec);
      discard();
      const where = projects.find((p) => p.id === s.project_id)?.name ?? s.cwd.split("/").pop();
      const openIt = () => {
        showSession(s.id);
        void navigate({ to: "/agents" });
      };
      if (openAfterLaunch) openIt();
      else toast(`${HARNESS_NAME[s.harness]} started in ${where}`, { label: "Open", run: openIt });
    } catch (e) {
      toastError(e);
    } finally {
      setLaunching(false);
    }
  };

  return (
    <aside className="relative flex w-[440px] flex-none flex-col border-l border-line bg-paper shadow-[-14px_0_28px_-20px_rgba(23,32,43,.35)]">
      <div className="flex items-center gap-2.5 border-b border-line px-3.5 py-3">
        <span className="text-sm font-bold">New agent</span>
        <span className="inline-flex gap-0.5 rounded-lg bg-sunk p-0.5">
          {(["claude", "codex"] as HarnessKind[]).map((h) => (
            <button
              key={h}
              onClick={() => set((s) => ({ ...s, harness: h, model: s.harness === h ? s.model : null, effort: s.harness === h ? s.effort : null }))}
              className={`inline-flex h-[26px] items-center gap-1.5 rounded-md px-2.5 text-[12.5px] ${
                spec.harness === h ? "bg-paper font-semibold text-ink shadow-[0_1px_2px_rgba(23,32,43,.12),inset_0_0_0_1px_var(--color-line)]" : "text-ink-2"
              }`}
            >
              <HarnessMark harness={h} size={15} />
              {HARNESS_NAME[h]}
            </button>
          ))}
        </span>
        <button className="iconbtn ml-auto" title="Close (the draft is kept)" onClick={close}>
          <Icon name="x" size={14} />
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3.5 py-3">
        <div className="grid grid-cols-2 gap-2.5">
          <Field label="Project">
            <Popover
              width={300}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  <Icon name="folder" size={14} />
                  {spec.target.kind === "temp" ? "New temporary folder" : (target?.name ?? <span className="text-ink-3">Pick a project</span>)}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <>
                  <MenuItem
                    icon={<Icon name="plus" size={13} />}
                    label="New temporary folder"
                    detail={`in ${config?.projects_root ?? "~/projects"}`}
                    active={spec.target.kind === "temp"}
                    onClick={() => (set((s) => ({ ...s, target: { kind: "temp" } })), closeMenu())}
                  />
                  {sorted.map((p) => (
                    <MenuItem
                      key={p.id}
                      icon={<Icon name="folder" size={13} />}
                      label={p.name}
                      detail={p.description || p.gh_repo || undefined}
                      active={target?.id === p.id}
                      onClick={() => (
                        set((s) => ({ ...s, target: { kind: s.target.kind === "predraft" ? "predraft" : "project", project_id: p.id }, context_projects: s.context_projects.filter((c) => c.project_id !== p.id) })), closeMenu()
                      )}
                    />
                  ))}
                  {sorted.length === 0 && <div className="px-2 py-1.5 text-xs text-ink-3">No project yet: add some in Projects.</div>}
                </>
              )}
            </Popover>
          </Field>
          <Field label="Run in">
            {target ? <select className="field-input h-8 w-full" aria-label="Run in" value={spec.target.kind} onChange={(e) => set((s) => ({ ...s, target: { kind: e.target.value === "predraft" ? "predraft" : "project", project_id: target.id }, presets: s.presets.filter((id) => id !== "draft-pr" && id !== "worktree") }))}>
              {existing && <option value="existing_predraft">Pre-draft: {existing.title}</option>}<option value="project">Project checkout</option><option value="predraft">Pre-draft PR</option>
            </select> : <div className="field-input h-8">{spec.target.kind === "temp" ? "A new folder" : "—"}</div>}
            {target && spec.target.kind === "project" && <div className="mt-1 text-xs text-ink-3"><TargetCheckout projectId={target.id} /></div>}
          </Field>
        </div>

        <Field label="Prompt">
          <PromptField
            value={spec.text}
            onChange={(text) => set((s) => ({ ...s, text }))}
            projects={sorted}
            repo={repo}
            onPickProject={addProjectContext}
            onPickItem={(it) =>
              set((s) =>
                s.github_refs.some((g) => g.url === it.url)
                  ? s
                  : { ...s, github_refs: [...s.github_refs, { url: it.url, kind: it.kind, repo: it.repo, number: it.number, title: it.title }] },
              )
            }
          />
        </Field>

        <Field label="Context" hint="type @ or # in the prompt, use the buttons, or add lines from the code viewer">
          <div className="grid gap-[5px]">
            {!!recentContexts.length && <div className="grid gap-1"><span className="text-[11.5px] text-ink-3">Recent contexts</span><div className="flex flex-wrap gap-1">{recentContexts.slice(0,6).map((c) => <button key={c.key} className="btn btn-sm max-w-full truncate" title={c.label} onClick={c.add}>+ {c.label}</button>)}</div></div>}
            {spec.github_refs.map((g) => {
              const ic = itemIcon(g.kind === "pr" ? "pr" : "issue", "open");
              return (
                <div key={g.url} className="grid grid-cols-[16px_auto_1fr_16px] items-center gap-2 rounded-[7px] border border-line bg-chrome px-2 py-1.5 text-[12.5px]">
                  <Icon name={g.kind === "comment" || g.kind === "review_comment" ? "comment" : ic.name} size={14} className={ic.color} />
                  <span className="font-mono text-xs">{g.number === null ? g.kind : `#${g.number}`}</span>
                  <span className="truncate text-ink-2">{g.title ?? g.url}</span>
                  <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, github_refs: s.github_refs.filter((x) => x.url !== g.url) }))}>
                    <Icon name="x" size={13} />
                  </button>
                </div>
              );
            })}
            {(spec.files ?? []).map((f, i) => (
              <div key={`${f.root}\n${f.rev}\n${f.path}\n${f.start_line}-${f.end_line}`} className="grid grid-cols-[16px_minmax(0,auto)_1fr_16px] items-center gap-2 rounded-[7px] border border-line bg-chrome px-2 py-1.5 text-[12.5px]">
                <Icon name="file" size={14} />
                <span className="truncate font-mono text-xs" title={`${f.root}/${f.path}`}>
                  {f.path}
                  {f.start_line !== null && `:${f.start_line}${f.end_line !== null && f.end_line > f.start_line ? `-${f.end_line}` : ""}`}
                </span>
                <span className="truncate text-xs text-ink-3">{f.rev ? `at ${f.rev}` : f.root.split("/").pop()}</span>
                <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, files: s.files.filter((_, j) => j !== i) }))}>
                  <Icon name="x" size={13} />
                </button>
              </div>
            ))}
            {spec.context_projects.map((c) => {
              const p = projects.find((x) => x.id === c.project_id);
              return (
                <div key={c.project_id} className="grid grid-cols-[16px_auto_1fr_16px] items-center gap-2 rounded-[7px] border border-line bg-chrome px-2 py-1.5 text-[12.5px]">
                  <Icon name="folder" size={14} />
                  <span className="font-mono text-xs">{p?.name ?? "?"}</span>
                  <span className="min-w-0">
                    <Branch
                      projectId={c.project_id}
                      branch={c.branch}
                      onChange={(branch) => set((s) => ({ ...s, context_projects: s.context_projects.map((x) => (x.project_id === c.project_id ? { ...x, branch } : x)) }))}
                    />
                  </span>
                  <button className="text-ink-3 hover:text-ink" title="Remove" onClick={() => set((s) => ({ ...s, context_projects: s.context_projects.filter((x) => x.project_id !== c.project_id) }))}>
                    <Icon name="x" size={13} />
                  </button>
                </div>
              );
            })}
            <div className="flex flex-wrap gap-1.5">
              <Popover
                width={280}
                trigger={({ toggle }) => (
                  <button className="btn btn-sm" onClick={toggle}>
                    <Icon name="plus" size={12} />
                    Project
                  </button>
                )}
              >
                {(closeMenu) => (
                  <>
                    {sorted
                      .filter((p) => p.id !== target?.id && !spec.context_projects.some((c) => c.project_id === p.id))
                      .map((p) => (
                        <MenuItem key={p.id} icon={<Icon name="folder" size={13} />} label={p.name} detail={p.description || undefined} onClick={() => (addProjectContext(p), closeMenu())} />
                      ))}
                  </>
                )}
              </Popover>
              <GitHubLinkField onAdd={(r) => set((s) => (s.github_refs.some((g) => g.url === r.url) ? s : { ...s, github_refs: [...s.github_refs, r] }))} />
            </div>
          </div>
        </Field>

        <Field
          label="Presets"
          end={
            <button className="font-semibold text-teal-ink hover:underline" onClick={() => void navigate({ to: "/settings", hash: "presets" })}>
              Edit presets
            </button>
          }
        >
          <div className="flex flex-wrap gap-1.5">
            {(config?.presets ?? []).filter((p) => p.id !== "draft-pr" && ((spec.target.kind !== "predraft" && spec.target.kind !== "existing_predraft") || p.id !== "worktree")).map((p) => {
              const on = spec.presets.includes(p.id);
              return (
                <button
                  key={p.id}
                  title={p.text}
                  onClick={() => set((s) => ({ ...s, presets: on ? s.presets.filter((x) => x !== p.id) : [...s.presets, p.id] }))}
                  className={`inline-flex h-7 items-center gap-1.5 rounded-full border px-[11px] text-[12.5px] ${on ? "border-ink bg-ink font-semibold text-white" : "border-line bg-paper text-ink-2 hover:border-[#c4ccd7]"}`}
                >
                  {on && <Icon name="check" size={12} />}
                  {p.label}
                </button>
              );
            })}
            {config?.presets.length === 0 && <span className="text-xs text-ink-3">No preset.</span>}
          </div>
        </Field>

        <div className="grid grid-cols-2 gap-2.5">
          <Field label="Model">
            <Popover
              width={300}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  {spec.model ?? (
                    <>
                      Default <span className="text-ink-3">· {defaults?.model ?? "harness default"}</span>
                    </>
                  )}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <>
                  <MenuItem label={`Default (${defaults?.model ?? "harness default"})`} detail="What the agent picks on its own" active={!spec.model} onClick={() => (set((s) => ({ ...s, model: null })), closeMenu())} />
                  {(defaults?.models ?? []).map((m) => (
                    <MenuItem key={m.id} label={m.display_name || m.id} detail={`${m.id} · ${m.description}`} active={spec.model === m.id} onClick={() => (set((s) => ({ ...s, model: m.id })), closeMenu())} />
                  ))}
                </>
              )}
            </Popover>
          </Field>
          <Field label="Effort">
            <Popover
              width={220}
              trigger={({ toggle }) => (
                <SelectButton onClick={toggle}>
                  {spec.effort ?? (
                    <>
                      Default <span className="text-ink-3">· {defaults?.effort ?? "harness default"}</span>
                    </>
                  )}
                </SelectButton>
              )}
            >
              {(closeMenu) => (
                <>
                  <MenuItem label={`Default (${defaults?.effort ?? "harness default"})`} active={!spec.effort} onClick={() => (set((s) => ({ ...s, effort: null })), closeMenu())} />
                  {efforts.map((e) => (
                    <MenuItem key={e} label={e} active={spec.effort === e} onClick={() => (set((s) => ({ ...s, effort: e })), closeMenu())} />
                  ))}
                </>
              )}
            </Popover>
          </Field>
        </div>

        <DelaySelector conditions={conditions} harness={spec.harness} onChange={setConditions} onValidityChange={setDelayValid} />

        <div className="rounded-lg border border-line bg-chrome">
          <div className="flex items-center gap-2 border-b border-line px-2.5 py-1.5 text-[11.5px] font-[650] text-ink-2">
            Prompt sent to {spec.harness ? HARNESS_NAME[spec.harness] : "the agent"}
            <button className="ml-auto font-normal text-ink-3 hover:text-ink" onClick={() => setShowAll((v) => !v)}>
              {preview.data?.prompt.length ?? 0} characters · {showAll ? "show less" : "show all"}
            </button>
          </div>
          <pre
            className={`m-0 px-2.5 py-2 font-mono text-[11.5px] leading-normal whitespace-pre-wrap text-ink-2 ${showAll ? "" : "max-h-[110px] overflow-hidden [mask-image:linear-gradient(#000_70%,transparent)]"}`}
          >
            {preview.data?.prompt || <span className="text-ink-3">The prompt appears here as you type.</span>}
          </pre>
        </div>
      </div>

      <div className="flex items-center gap-2 border-t border-line bg-chrome px-3.5 py-3">
        <label className="mr-auto inline-flex items-center gap-[7px] text-xs text-ink-2" style={scheduled ? { visibility: "hidden" } : undefined}>
          <input type="checkbox" className="accent-teal" disabled={scheduled} checked={openAfterLaunch} onChange={(e) => setOpenAfterLaunch(e.target.checked)} />
          Open after launch
        </label>
        <button className="btn btn-ghost" onClick={discard} title="Clear the composer">
          Discard
        </button>
        <button className="btn btn-primary" disabled={!canLaunch || launching} onClick={() => void launch()}>
          <Icon name="play" size={14} />
          {launching ? (scheduled ? "Scheduling…" : "Launching…") : scheduled ? "Schedule" : "Launch"}
        </button>
      </div>
    </aside>
  );
}

function TargetCheckout({ projectId }: { projectId: string }) {
  const q = useQuery({
    queryKey: ["branches", projectId],
    queryFn: () => api.get<{ current: string | null; branches: string[] }>(`/api/projects/${projectId}/branches`),
    staleTime: 30_000,
  });
  return (
    <>
      Checkout{q.data?.current && <span className="font-mono text-xs">· {q.data.current}</span>}
    </>
  );
}
